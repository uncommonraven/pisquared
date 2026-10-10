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

use crate::fetch::partial_path;

/// The size of the buffer the decoder writes through. Peak memory is this plus the decoder's own dictionary, which
/// does not grow with the image.
const BUFFER_SIZE: usize = 1024 * 1024;

/// A completed decompression: where the flashable file is and how many bytes it holds.
#[derive(Debug, PartialEq, Eq)]
pub struct Decompression {
    pub path: PathBuf,
    pub bytes: u64,
}

#[derive(Debug, Error)]
pub enum DecompressError {
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
/// failure removes the partial file, so nothing is presented as a flashable image.
pub async fn decompress(input: &Path, output: &Path) -> Result<Decompression, DecompressError> {
    let partial = partial_path(output);
    match stream_to(input, &partial).await {
        Ok(bytes) => {
            tokio::fs::rename(&partial, output)
                .await
                .map_err(|source| DecompressError::Io {
                    path: output.to_path_buf(),
                    source,
                })?;
            Ok(Decompression {
                path: output.to_path_buf(),
                bytes,
            })
        }
        Err(err) => {
            let _ = tokio::fs::remove_file(&partial).await;
            Err(err)
        }
    }
}

/// Decodes `input` into `partial`, returning how many bytes were written.
async fn stream_to(input: &Path, partial: &Path) -> Result<u64, DecompressError> {
    let source = File::open(input)
        .await
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
        let n = decoder
            .read(&mut buf)
            .await
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
    }
    out.sync_all().await.map_err(|source| DecompressError::Io {
        path: partial.to_path_buf(),
        source,
    })?;
    // Dropped here, before any rename: Windows refuses to rename a file that is still open.
    drop(out);

    Ok(written)
}

/// The reason a decode failed. liblzma reports input that ends before the end of the xz stream as a buffer error,
/// which arrives here as `OutOfMemory`, so that kind is read as truncation rather than shown as a memory failure.
fn stream_reason(err: &io::Error) -> String {
    if err.kind() == io::ErrorKind::OutOfMemory {
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
    use liblzma::write::XzEncoder;
    use std::io::Write;
    use tempfile::tempdir;

    /// Three megabytes, larger than the buffer, so the stream takes several reads.
    fn payload() -> Vec<u8> {
        (0..3_000_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect()
    }

    fn xz(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = XzEncoder::new(Vec::new(), 6);
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    /// Task 6.1: a complete stream appears at the output path with its decompressed contents.
    #[tokio::test]
    async fn a_complete_stream_appears_at_the_output_path() {
        let image = payload();
        let dir = tempdir().unwrap();
        let input = dir.path().join("image.img.xz");
        let output = dir.path().join("image.img");
        std::fs::write(&input, xz(&image)).unwrap();

        let done = decompress(&input, &output).await.unwrap();

        assert_eq!(done.path, output);
        assert_eq!(done.bytes, image.len() as u64);
        assert_eq!(std::fs::read(&output).unwrap(), image);
        assert!(!partial_path(&output).exists());
    }

    /// Task 6.2: a stream cut short fails with the reason shown, and nothing is left at either name.
    #[tokio::test]
    async fn a_truncated_stream_fails_and_leaves_nothing() {
        let compressed = xz(&payload());
        let dir = tempdir().unwrap();
        let input = dir.path().join("image.img.xz");
        let output = dir.path().join("image.img");
        std::fs::write(&input, &compressed[..compressed.len() / 2]).unwrap();

        let err = decompress(&input, &output).await.unwrap_err();

        assert!(matches!(err, DecompressError::Stream { .. }), "{err}");
        assert!(err.to_string().contains("truncated"), "{err}");
        assert!(!output.exists());
        assert!(!partial_path(&output).exists());
    }

    /// Task 6.2: a corrupt stream fails with the reason shown, and nothing is left at either name.
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

        let err = decompress(&input, &output).await.unwrap_err();

        assert!(matches!(err, DecompressError::Stream { .. }), "{err}");
        assert!(!output.exists());
        assert!(!partial_path(&output).exists());
    }
}
