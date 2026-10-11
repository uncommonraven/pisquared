//! Decompresses the downloaded image to a flashable file, streaming it through a buffer so that memory is bounded by
//! the buffer rather than by the image.
//!
//! The output streams to a `.partial` file beside the output path and is renamed onto it only once the whole stream
//! has decompressed, so a truncated or corrupt stream leaves nothing at the output path. The publisher publishes no
//! checksum or size for the decompressed file, so nothing here claims the result was verified against one: what is
//! guaranteed is that the decoder consumed the whole stream.

use std::io;
use std::path::{Path, PathBuf};

use async_compression::tokio::bufread::XzDecoder;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

use crate::backend::fetch::partial_path;

/// The size of the buffer the decoder writes through. Peak memory is this plus the decoder's own dictionary, which
/// does not grow with the image.
const BUFFER_SIZE: usize = 1024 * 1024;

/// A completed decompression: how many bytes the flashable file holds.
#[derive(Debug, PartialEq, Eq)]
pub struct Decompression {
    pub bytes: u64,
}

#[derive(Debug, Error)]
pub enum DecompressError {
    #[error("{file} was not decompressed: the run was cancelled")]
    Cancelled { file: String },

    #[error("{file} did not decompress: {reason}")]
    Stream { file: String, reason: String },

    #[error("cannot write {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Decompresses the xz file at `input` to `output`. The output appears only if the whole stream decompressed; any
/// failure removes the partial file, so nothing is presented as a flashable image. A cancellation counts as a
/// failure here: the decode stops and the partial file is removed. `progress` receives the running total of
/// bytes written each time a block of output lands.
pub async fn decompress(
    input: &Path,
    output: &Path,
    cancel: &CancellationToken,
    progress: impl FnMut(u64),
) -> Result<Decompression, DecompressError> {
    let partial = partial_path(output);
    match stream_to(input, &partial, cancel, progress).await {
        Ok(bytes) => {
            tokio::fs::rename(&partial, output)
                .await
                .map_err(|source| DecompressError::Io {
                    path: output.to_path_buf(),
                    source,
                })?;
            Ok(Decompression { bytes })
        }
        Err(err) => {
            let _ = tokio::fs::remove_file(&partial).await;
            Err(err)
        }
    }
}

/// Decodes `input` into `partial`, returning how many bytes were written.
///
/// Every read is raced against the cancellation token, so a cancel stops the decode mid-stream. The token is
/// checked once more after the stream ends so a cancel that lands during the last read still counts.
async fn stream_to(
    input: &Path,
    partial: &Path,
    cancel: &CancellationToken,
    mut progress: impl FnMut(u64),
) -> Result<u64, DecompressError> {
    let cancelled = || DecompressError::Cancelled {
        file: filename(input),
    };
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let source = tokio::select! {
        _ = cancel.cancelled() => return Err(cancelled()),
        source = File::open(input) => source,
    }
    .map_err(|source| DecompressError::Io {
        path: input.to_path_buf(),
        source,
    })?;
    let mut decoder = XzDecoder::new(BufReader::new(source));
    let mut out = File::create(partial)
        .await
        .map_err(|source| DecompressError::Io {
            path: partial.to_path_buf(),
            source,
        })?;

    let mut buf = vec![0u8; BUFFER_SIZE];
    let mut written: u64 = 0;
    loop {
        let n = tokio::select! {
            _ = cancel.cancelled() => return Err(cancelled()),
            n = decoder.read(&mut buf) => n,
        }
        .map_err(|err| DecompressError::Stream {
            file: filename(input),
            reason: stream_reason(&err),
        })?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .await
            .map_err(|source| DecompressError::Io {
                path: partial.to_path_buf(),
                source,
            })?;
        written += n as u64;
        progress(written);
    }
    out.sync_all().await.map_err(|source| DecompressError::Io {
        path: partial.to_path_buf(),
        source,
    })?;
    // Dropped here, before any rename: Windows refuses to rename a file that is still open.
    drop(out);

    // The decode of the last block races the cancel too: a token set while the final read ran makes this a
    // cancelled decode even though the bytes are all there, because the operator asked for it to stop.
    if cancel.is_cancelled() {
        return Err(cancelled());
    }

    Ok(written)
}

/// The reason a decode failed. When the xz data ends before the stream is complete, the decoder reports it in one of
/// two ways, depending on where the input runs out. liblzma returns a buffer error (`LZMA_BUF_ERROR`), which the xz
/// decoder in `compression-codecs` reports as `io::ErrorKind::OutOfMemory`. Or async-compression's own flush reports
/// `io::ErrorKind::UnexpectedEof` when the input ends first. Both mean the same thing here, since the input is a file
/// and nothing else can end it early, so both are read as truncation. A genuine allocation failure would arrive as
/// `OutOfMemory` and be reported as truncation too, which is why the truncated case is the one the tests pin.
fn stream_reason(err: &io::Error) -> String {
    if matches!(
        err.kind(),
        io::ErrorKind::OutOfMemory | io::ErrorKind::UnexpectedEof
    ) {
        "the xz data ends before the stream is complete, so the image is truncated".to_owned()
    } else {
        err.to_string()
    }
}

fn filename(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::test_support::{payload, xz};
    use tempfile::tempdir;

    /// A complete stream appears at the output path with its decompressed contents.
    #[tokio::test]
    async fn a_complete_stream_appears_at_the_output_path() {
        let image = payload();
        let dir = tempdir().unwrap();
        let input = dir.path().join("image.img.xz");
        let output = dir.path().join("image.img");
        std::fs::write(&input, xz(&image)).unwrap();

        let done = decompress(&input, &output, &CancellationToken::new(), |_| {})
            .await
            .unwrap();

        assert_eq!(done.bytes, image.len() as u64);
        assert_eq!(std::fs::read(&output).unwrap(), image);
        assert!(!partial_path(&output).exists());
    }

    /// A stream cut short fails with the reason shown, and nothing is left at either name.
    #[tokio::test]
    async fn a_truncated_stream_fails_and_leaves_nothing() {
        let compressed = xz(&payload());
        let dir = tempdir().unwrap();
        let input = dir.path().join("image.img.xz");
        let output = dir.path().join("image.img");
        std::fs::write(&input, &compressed[..compressed.len() / 2]).unwrap();

        let err = decompress(&input, &output, &CancellationToken::new(), |_| {})
            .await
            .unwrap_err();

        assert!(matches!(err, DecompressError::Stream { .. }), "{err}");
        assert!(err.to_string().contains("truncated"), "{err}");
        assert!(!output.exists());
        assert!(!partial_path(&output).exists());
    }

    /// Both ways the decoder reports a stream that ends early read as truncation, whichever one a platform produces.
    #[test]
    fn an_early_end_of_input_reads_as_truncation() {
        for kind in [io::ErrorKind::OutOfMemory, io::ErrorKind::UnexpectedEof] {
            let reason = stream_reason(&io::Error::from(kind));
            assert!(reason.contains("truncated"), "{kind:?}: {reason}");
        }
    }

    /// A stream with corrupted bytes fails with the reason shown, and nothing is left at either name.
    #[tokio::test]
    async fn a_corrupt_stream_fails_and_leaves_nothing() {
        let mut compressed = xz(&payload());
        let middle = compressed.len() / 2;
        for byte in &mut compressed[middle..middle + 64] {
            *byte ^= 0xFF;
        }
        let dir = tempdir().unwrap();
        let input = dir.path().join("image.img.xz");
        let output = dir.path().join("image.img");
        std::fs::write(&input, &compressed).unwrap();

        let err = decompress(&input, &output, &CancellationToken::new(), |_| {})
            .await
            .unwrap_err();

        assert!(matches!(err, DecompressError::Stream { .. }), "{err}");
        assert!(!output.exists());
        assert!(!partial_path(&output).exists());
    }

    /// A cancel issued once output has been written stops the decode and leaves nothing at either name. The cancel
    /// is issued from the progress callback, so it is certain to land mid-decode. The stream is long enough that
    /// many reads follow the first write.
    #[tokio::test]
    async fn a_cancelled_decode_leaves_nothing() {
        let image = vec![0u8; 128 * 1024 * 1024];
        let dir = tempdir().unwrap();
        let input = dir.path().join("image.img.xz");
        let output = dir.path().join("image.img");
        std::fs::write(&input, xz(&image)).unwrap();
        let cancel = CancellationToken::new();
        let c = cancel.clone();

        let err = decompress(&input, &output, &cancel, |written| {
            if written > 0 {
                c.cancel();
            }
        })
        .await
        .unwrap_err();

        assert!(matches!(err, DecompressError::Cancelled { .. }), "{err}");
        assert!(!output.exists(), "the output path must hold nothing");
        assert!(
            !partial_path(&output).exists(),
            "the partial must be removed"
        );
    }

    /// A token already cancelled before the call stops the decode before it opens the input.
    #[tokio::test]
    async fn a_cancel_before_the_call_starts_nothing() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("image.img.xz");
        let output = dir.path().join("image.img");
        std::fs::write(&input, xz(&payload())).unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();

        let err = decompress(&input, &output, &cancel, |_| {})
            .await
            .unwrap_err();

        assert!(matches!(err, DecompressError::Cancelled { .. }), "{err}");
        assert!(!output.exists());
        assert!(!partial_path(&output).exists());
    }
}
