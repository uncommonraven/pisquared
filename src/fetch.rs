//! Downloads an image and verifies it against the checksum its publisher publishes beside it.
//!
//! The bytes stream to a `.partial` file beside the output path and are renamed onto it only once the whole
//! file has arrived, so an interrupted transfer leaves nothing at the output path. A checksum that does not
//! match removes the partial file, so bytes that failed their checksum are kept nowhere. A checksum that cannot
//! be retrieved is reported as unverified with its reason, and never as verified or as a mismatch.

use std::io;
use std::path::{Path, PathBuf};

use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

/// Appended to an image address for the publisher's published checksum file.
pub const CHECKSUM_SUFFIX: &str = ".sha256";

/// Appended to the output path for the file while it is still arriving.
pub const PARTIAL_SUFFIX: &str = ".partial";

/// Bounds how much of a published checksum file is read. The real ones carry one line per image.
const MAX_PUBLISHED_SIZE: usize = 1 << 20;

/// The reason a checksum could not be fetched because the run was cancelled first.
const CANCELLED_BEFORE_CHECKSUM: &str = "the run was cancelled before the checksum was fetched";

/// What a transfer reports as it runs, so an interface can show progress. `total` is what the publisher
/// declared in the response, and is absent when it declared none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub written: u64,
    pub total: Option<u64>,
}

/// How the downloaded file was checked. Only the two outcomes that leave a file in place appear here: a
/// mismatch is an error, not a verification result.
#[derive(Debug, PartialEq, Eq)]
pub enum Verification {
    /// The computed digest equals the published one.
    Verified { source: String, digest: String },
    /// The published checksum could not be retrieved or read. The reason is kept so nothing reads it as a pass.
    Unverified { source: String, reason: String },
}

/// A completed download: how big it is and what it was verified against.
#[derive(Debug)]
pub struct Download {
    pub bytes: u64,
    pub verification: Verification,
}

/// Why a download stopped. A cancellation is a reason a transfer stopped, not a failure the tool caused.
#[derive(Debug, Error)]
pub enum FetchError {
    #[error("the transfer of {file} was cancelled with nothing presented as finished")]
    Cancelled { file: String },

    #[error("{url} answered {status}")]
    Status { url: String, status: StatusCode },

    #[error(
        "the transfer of {file} stopped at {written} of the {total} bytes the publisher declared"
    )]
    Truncated {
        file: String,
        written: u64,
        total: u64,
    },

    #[error(
        "{file} does not match the checksum published at {checksum_url}: expected {expected}, observed {observed}"
    )]
    Mismatch {
        file: String,
        checksum_url: String,
        expected: String,
        observed: String,
    },

    #[error("cannot write {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

/// Fetches `image_url` to `output`, verifies it against the checksum published at `image_url` plus
/// [`CHECKSUM_SUFFIX`], and places it at `output` unless it fails verification.
///
/// A transfer that is interrupted or fails leaves its partial file behind, which is nowhere near the output path.
/// A retry truncates that file and starts over rather than resuming it. A cancellation or a mismatch removes the
/// partial file, so bytes that were stopped or failed are kept nowhere.
pub async fn download(
    client: &reqwest::Client,
    image_url: &str,
    output: &Path,
    cancel: &CancellationToken,
    mut progress: impl FnMut(Progress),
) -> Result<Download, FetchError> {
    let partial = partial_path(output);
    let (bytes, verification) =
        match fetch_and_verify(client, image_url, &partial, cancel, &mut progress).await {
            Ok(checked) => checked,
            Err(err) => {
                if matches!(
                    err,
                    FetchError::Cancelled { .. } | FetchError::Mismatch { .. }
                ) {
                    let _ = tokio::fs::remove_file(&partial).await;
                }
                return Err(err);
            }
        };

    // A cancel that arrives during the checksum fetch reads as unverified, so the token is checked here to keep
    // a cancelled run from presenting the image at all.
    if cancel.is_cancelled() {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(FetchError::Cancelled {
            file: filename(image_url),
        });
    }
    tokio::fs::rename(&partial, output)
        .await
        .map_err(|source| FetchError::Io {
            path: output.to_path_buf(),
            source,
        })?;
    Ok(Download {
        bytes,
        verification,
    })
}

async fn fetch_and_verify(
    client: &reqwest::Client,
    image_url: &str,
    partial: &Path,
    cancel: &CancellationToken,
    progress: &mut impl FnMut(Progress),
) -> Result<(u64, Verification), FetchError> {
    let (bytes, observed) = transfer(client, image_url, partial, cancel, progress).await?;
    let file = filename(image_url);
    let checksum_url = format!("{image_url}{CHECKSUM_SUFFIX}");
    let verification = match retrieve(client, &checksum_url, &file, cancel).await {
        Ok(expected) if expected == observed => Verification::Verified {
            source: checksum_url,
            digest: observed,
        },
        Ok(expected) => {
            return Err(FetchError::Mismatch {
                file,
                checksum_url,
                expected,
                observed,
            });
        }
        Err(reason) => Verification::Unverified {
            source: checksum_url,
            reason,
        },
    };
    Ok((bytes, verification))
}

/// Streams the image into `partial`, returning how many bytes arrived and their SHA-256 as lowercase hex.
///
/// Every wait is raced against the cancellation token, so a cancel stops a transfer before its first chunk as
/// readily as one mid-stream (the behaviour the 0.4 spike proved). Nothing here removes the partial file: the
/// caller decides what a stopped transfer means.
async fn transfer(
    client: &reqwest::Client,
    url: &str,
    partial: &Path,
    cancel: &CancellationToken,
    progress: &mut impl FnMut(Progress),
) -> Result<(u64, String), FetchError> {
    let cancelled = || FetchError::Cancelled {
        file: filename(url),
    };
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    if let Some(parent) = partial.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| FetchError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
    }

    let mut response = tokio::select! {
        _ = cancel.cancelled() => return Err(cancelled()),
        response = client.get(url).send() => response?,
    };
    if response.status() != StatusCode::OK {
        return Err(FetchError::Status {
            url: url.to_owned(),
            status: response.status(),
        });
    }
    let total = response.content_length();
    progress(Progress { written: 0, total });

    let mut file = File::create(partial)
        .await
        .map_err(|source| FetchError::Io {
            path: partial.to_path_buf(),
            source,
        })?;
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    while let Some(chunk) = tokio::select! {
        _ = cancel.cancelled() => return Err(cancelled()),
        chunk = response.chunk() => chunk?,
    } {
        file.write_all(&chunk)
            .await
            .map_err(|source| FetchError::Io {
                path: partial.to_path_buf(),
                source,
            })?;
        hasher.update(&chunk);
        written += chunk.len() as u64;
        progress(Progress { written, total });
    }
    file.sync_all().await.map_err(|source| FetchError::Io {
        path: partial.to_path_buf(),
        source,
    })?;
    // Dropped here, before any rename: Windows refuses to rename a file that is still open.
    drop(file);

    if let Some(total) = total
        && written < total
    {
        return Err(FetchError::Truncated {
            file: filename(url),
            written,
            total,
        });
    }

    Ok((written, format!("{:x}", hasher.finalize())))
}

/// Fetches the published digest for `file` from `checksum_url`. The error is the reason it could not be had.
///
/// A cancel is reported through the caller's token check, not as a reason here: the fetch is raced against the
/// token so a cancelled run does not wait on the checksum server.
async fn retrieve(
    client: &reqwest::Client,
    checksum_url: &str,
    file: &str,
    cancel: &CancellationToken,
) -> Result<String, String> {
    if cancel.is_cancelled() {
        return Err(CANCELLED_BEFORE_CHECKSUM.to_owned());
    }
    let body = tokio::select! {
        _ = cancel.cancelled() => return Err(CANCELLED_BEFORE_CHECKSUM.to_owned()),
        body = get(client, checksum_url) => body?,
    };
    if body.len() > MAX_PUBLISHED_SIZE {
        return Err(format!(
            "it is larger than the {MAX_PUBLISHED_SIZE} bytes a checksum file can be"
        ));
    }
    parse_published(&String::from_utf8_lossy(&body), file)
}

/// The published digest for `filename` in a checksum file, lowercase.
///
/// Three shapes are accepted: sha256sum's format with its binary-mode asterisk optional, the same among digests
/// for several files where only the line naming this file counts, and a single bare digest. A file with several
/// digests and none naming `filename` is refused rather than guessed at, since the first digest may belong to
/// some other file.
pub fn parse_published(body: &str, filename: &str) -> Result<String, String> {
    let mut digests = Vec::new();
    for line in body.lines() {
        let Some((digest, rest)) = first_digest(line) else {
            continue;
        };
        let digest = digest.to_ascii_lowercase();
        let named = rest
            .trim_start_matches([' ', '\t', '*'])
            .split_whitespace()
            .next();
        if named == Some(filename) {
            return Ok(digest);
        }
        digests.push(digest);
    }

    match digests.len() {
        0 => Err("it carries no sha256 digest".to_owned()),
        1 => Ok(digests.swap_remove(0)),
        n => Err(format!(
            "it carries {n} digests and none is labelled {filename}"
        )),
    }
}

/// The first 64-character hex word on a line, and the text that follows it.
fn first_digest(line: &str) -> Option<(&str, &str)> {
    let mut start = None;
    for (i, c) in line
        .char_indices()
        .chain(std::iter::once((line.len(), ' ')))
    {
        if c.is_ascii_alphanumeric() || c == '_' {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            let word = &line[s..i];
            if word.len() == 64 && word.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Some((word, &line[i..]));
            }
        }
    }
    None
}

/// The publisher's filename for the file at `address`, taken from its path so a query string never leaks into it.
pub fn filename(address: &str) -> String {
    url::Url::parse(address)
        .ok()
        .and_then(|url| url.path_segments()?.next_back().map(str::to_owned))
        .unwrap_or_default()
}

pub(crate) fn partial_path(output: &Path) -> PathBuf {
    let mut name = output.as_os_str().to_owned();
    name.push(PARTIAL_SUFFIX);
    PathBuf::from(name)
}

/// The body of a small file the publisher serves, or the reason it could not be had.
pub(crate) async fn get(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client.get(url).send().await.map_err(|err| describe(&err))?;
    if response.status() != StatusCode::OK {
        return Err(format!("the server answered {}", response.status()));
    }
    response
        .bytes()
        .await
        .map(|body| body.to_vec())
        .map_err(|err| describe(&err))
}

/// An error with its cause chain, so a reason reads "connection refused" rather than only "error sending request".
pub(crate) fn describe(err: &reqwest::Error) -> String {
    let mut text = err.to_string();
    let mut cause = std::error::Error::source(err);
    while let Some(next) = cause {
        text.push_str(": ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{Reply, client, serve};
    use std::collections::HashMap;
    use tempfile::tempdir;

    const NAME: &str = "DietPi_RPi5-ARMv8-Trixie.img.xz";

    /// Three megabytes, larger than any single read, so the transfer takes several chunks.
    fn payload() -> Vec<u8> {
        (0..3_000_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect()
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// A checksum file in sha256sum's format, as the publisher serves it.
    fn checksum_file(bytes: &[u8], name: &str) -> Vec<u8> {
        format!("{}  {name}\n", sha256_hex(bytes)).into_bytes()
    }

    /// The path a file is served at under the server's base.
    fn path(name: &str) -> String {
        format!("/images/{name}")
    }

    fn routes_for(image: Reply, checksum: Reply) -> HashMap<String, Reply> {
        HashMap::from([
            (path(NAME), image),
            (path(&format!("{NAME}{CHECKSUM_SUFFIX}")), checksum),
        ])
    }

    /// Task 4.1: a completed transfer appears at the output path, and nothing is left at the partial name.
    #[tokio::test]
    async fn a_completed_transfer_appears_at_the_output_path() {
        let image = payload();
        let base = serve(routes_for(
            Reply::Body(image.clone()),
            Reply::Body(checksum_file(&image, NAME)),
        ));
        let dir = tempdir().unwrap();
        let output = dir.path().join(NAME);

        let download = download(
            &client(),
            &format!("{base}{NAME}"),
            &output,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(std::fs::read(&output).unwrap(), image);
        assert_eq!(download.bytes, image.len() as u64);
        assert!(!partial_path(&output).exists());
    }

    /// Task 4.1: an interrupted transfer does not appear at the output path.
    #[tokio::test]
    async fn an_interrupted_transfer_does_not_appear_at_the_output_path() {
        let image = payload();
        let base = serve(routes_for(
            Reply::Truncated {
                declared: image.len(),
                sent: image[..image.len() / 2].to_vec(),
            },
            Reply::Body(checksum_file(&image, NAME)),
        ));
        let dir = tempdir().unwrap();
        let output = dir.path().join(NAME);

        let result = download(
            &client(),
            &format!("{base}{NAME}"),
            &output,
            &CancellationToken::new(),
            |_| {},
        )
        .await;

        assert!(result.is_err(), "an interrupted transfer must fail");
        assert!(
            !output.exists(),
            "the output path must not hold a partial image"
        );
    }

    /// Task 4.1: a retry starts over and reaches the same bytes an uninterrupted transfer would.
    #[tokio::test]
    async fn a_retry_after_an_interruption_reaches_the_same_bytes() {
        let image = payload();
        let interrupted = serve(routes_for(
            Reply::Truncated {
                declared: image.len(),
                sent: image[..1000].to_vec(),
            },
            Reply::Body(checksum_file(&image, NAME)),
        ));
        let complete = serve(routes_for(
            Reply::Body(image.clone()),
            Reply::Body(checksum_file(&image, NAME)),
        ));
        let dir = tempdir().unwrap();
        let output = dir.path().join(NAME);

        assert!(
            download(
                &client(),
                &format!("{interrupted}{NAME}"),
                &output,
                &CancellationToken::new(),
                |_| {}
            )
            .await
            .is_err()
        );
        download(
            &client(),
            &format!("{complete}{NAME}"),
            &output,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(std::fs::read(&output).unwrap(), image);
    }

    /// Task 4.2: a matching checksum is reported as verified.
    #[tokio::test]
    async fn a_matching_checksum_is_reported_verified() {
        let image = payload();
        let base = serve(routes_for(
            Reply::Body(image.clone()),
            Reply::Body(checksum_file(&image, NAME)),
        ));
        let dir = tempdir().unwrap();
        let output = dir.path().join(NAME);

        let download = download(
            &client(),
            &format!("{base}{NAME}"),
            &output,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(
            download.verification,
            Verification::Verified {
                source: format!("{base}{NAME}{CHECKSUM_SUFFIX}"),
                digest: sha256_hex(&image),
            }
        );
    }

    /// Task 4.2: a mismatch fails naming both values, and leaves nothing at the output path.
    #[tokio::test]
    async fn a_mismatch_fails_naming_both_values_and_leaves_nothing() {
        let image = payload();
        let published = sha256_hex(b"some other image");
        let base = serve(routes_for(
            Reply::Body(image.clone()),
            Reply::Body(format!("{published}  {NAME}\n").into_bytes()),
        ));
        let dir = tempdir().unwrap();
        let output = dir.path().join(NAME);

        let err = download(
            &client(),
            &format!("{base}{NAME}"),
            &output,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();

        let message = err.to_string();
        assert!(message.contains(&published), "{message}");
        assert!(message.contains(&sha256_hex(&image)), "{message}");
        assert!(!output.exists());
        assert!(!partial_path(&output).exists());
    }

    /// Task 4.3: a checksum the publisher refuses is unverified with its reason, and is not verified or a mismatch.
    #[tokio::test]
    async fn an_unretrievable_checksum_is_unverified_with_its_reason() {
        let image = payload();
        let base = serve(routes_for(Reply::Body(image.clone()), Reply::Status(404)));
        let dir = tempdir().unwrap();
        let output = dir.path().join(NAME);

        let download = download(
            &client(),
            &format!("{base}{NAME}"),
            &output,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();

        match download.verification {
            Verification::Unverified { reason, .. } => {
                assert!(reason.contains("404"), "reason {reason:?} keeps the status");
            }
            other => panic!("expected unverified, got {other:?}"),
        }
        assert_eq!(std::fs::read(&output).unwrap(), image);
    }

    /// Task 4.3: a checksum server that cannot be reached is unverified, not a mismatch.
    #[tokio::test]
    async fn an_unreachable_checksum_is_unverified_not_a_mismatch() {
        let image = payload();
        let base = serve(routes_for(Reply::Body(image.clone()), Reply::Silent));
        let dir = tempdir().unwrap();
        let output = dir.path().join(NAME);

        let download = download(
            &client(),
            &format!("{base}{NAME}"),
            &output,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();

        match download.verification {
            Verification::Unverified { reason, .. } => assert!(!reason.is_empty()),
            other => panic!("expected unverified, got {other:?}"),
        }
    }

    /// Task 4.3: a checksum file that cannot be read is unverified, not a mismatch.
    #[tokio::test]
    async fn an_unreadable_checksum_is_unverified_not_a_mismatch() {
        let image = payload();
        let base = serve(routes_for(
            Reply::Body(image.clone()),
            Reply::Body(b"no digest here\n".to_vec()),
        ));
        let dir = tempdir().unwrap();
        let output = dir.path().join(NAME);

        let download = download(
            &client(),
            &format!("{base}{NAME}"),
            &output,
            &CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();

        match download.verification {
            Verification::Unverified { reason, .. } => {
                assert!(reason.contains("no sha256 digest"), "{reason}");
            }
            other => panic!("expected unverified, got {other:?}"),
        }
    }

    #[test]
    fn a_labelled_line_is_read_with_or_without_binary_mode() {
        let digest = sha256_hex(b"image");
        assert_eq!(
            parse_published(&format!("{digest} *{NAME}\n"), NAME),
            Ok(digest.clone())
        );
        assert_eq!(
            parse_published(&format!("{digest}  {NAME}\n"), NAME),
            Ok(digest)
        );
    }

    #[test]
    fn the_line_naming_this_file_is_used_among_several() {
        let mine = sha256_hex(b"mine");
        let other = sha256_hex(b"other");
        let body = format!("{other}  DietPi_RPi1-ARMv6-Trixie.img.xz\n{mine}  {NAME}\n");
        assert_eq!(parse_published(&body, NAME), Ok(mine));
    }

    #[test]
    fn several_digests_with_none_for_this_file_are_refused() {
        let body = format!(
            "{}  other-a.img.xz\n{}  other-b.img.xz\n",
            sha256_hex(b"a"),
            sha256_hex(b"b")
        );
        assert!(parse_published(&body, NAME).is_err());
    }

    #[test]
    fn a_bare_digest_is_read() {
        let digest = sha256_hex(b"image");
        assert_eq!(parse_published(&format!("{digest}\n"), NAME), Ok(digest));
    }

    #[test]
    fn a_filename_comes_from_the_path_not_the_query() {
        assert_eq!(
            filename("https://dietpi.com/downloads/images/DietPi_RPi5-ARMv8-Trixie.img.xz?x=1"),
            NAME
        );
    }

    /// Task 7.3: cancelling a transfer with no bytes yet stops it and leaves nothing at the output path.
    #[tokio::test]
    async fn a_cancel_before_the_first_chunk_leaves_nothing() {
        let image = payload();
        let base = serve(routes_for(
            Reply::Hang,
            Reply::Body(checksum_file(&image, NAME)),
        ));
        let dir = tempdir().unwrap();
        let output = dir.path().join(NAME);
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            c.cancel();
        });

        let mut ticks = Vec::new();
        let err = download(&client(), &format!("{base}{NAME}"), &output, &cancel, |p| {
            ticks.push(p)
        })
        .await
        .unwrap_err();

        assert!(matches!(err, FetchError::Cancelled { .. }), "{err}");
        assert!(
            ticks.is_empty(),
            "a transfer that never answered reported no progress"
        );
        assert!(!output.exists(), "the output path must hold nothing");
        assert!(
            !partial_path(&output).exists(),
            "the partial must be removed"
        );
    }

    /// Task 7.3: cancelling a transfer in progress stops it with some bytes arrived, and leaves nothing.
    #[tokio::test]
    async fn a_cancel_during_a_transfer_leaves_nothing() {
        let image = payload();
        let base = serve(routes_for(
            Reply::Slow {
                body: image.clone(),
                pieces: 12,
                pause_ms: 50,
            },
            Reply::Body(checksum_file(&image, NAME)),
        ));
        let dir = tempdir().unwrap();
        let output = dir.path().join(NAME);
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            c.cancel();
        });

        let mut ticks = Vec::new();
        let err = download(&client(), &format!("{base}{NAME}"), &output, &cancel, |p| {
            ticks.push(p)
        })
        .await
        .unwrap_err();

        assert!(matches!(err, FetchError::Cancelled { .. }), "{err}");
        let last = ticks.last().copied().expect("progress was reported");
        assert!(last.written > 0, "the cancel came after bytes arrived");
        assert_eq!(last.total, Some(image.len() as u64));
        assert!(!output.exists(), "the output path must hold nothing");
        assert!(
            !partial_path(&output).exists(),
            "the partial must be removed"
        );
    }
}
