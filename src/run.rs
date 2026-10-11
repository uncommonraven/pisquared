//! Runs the obtaining procedure for one device from start to finish, writing every step to the step log.
//!
//! The steps are resolve, download with its checksum, signature, and decompress, in that order. A signature that
//! cannot be checked halts the run, so an unverified image is never decompressed. Cancellation stops the run at
//! the step it is in, because the operator asked it to stop rather than because anything failed.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::catalogue::{self, ADDRESS_PREFIX, NotListed};
use crate::decompress::{self, DecompressError};
use crate::fetch::{self, FetchError, Verification};
use crate::log::{Step, StepLog};
use crate::signature::{self, SIGNING_KEY_URL, SignatureCheck, SignatureError};

/// What a completed run left on disk: the verified compressed image and the decompressed file beside it.
#[derive(Debug, PartialEq, Eq)]
pub struct Obtained {
    pub image: PathBuf,
    pub flashable: PathBuf,
}

/// What the run is doing at this moment, reported as it changes so an attached interface can show a live state
/// without reading the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Resolving,
    Downloading { written: u64, total: Option<u64> },
    Verifying,
    Decompressing,
}

#[derive(Debug, Error)]
pub enum RunError {
    #[error(transparent)]
    NotListed(#[from] NotListed),

    #[error(transparent)]
    Download(#[from] FetchError),

    #[error(transparent)]
    Signature(#[from] SignatureError),

    #[error("{file} is not verified by its signature, so the run halts: {reason}")]
    Unverified { file: String, reason: String },

    #[error(transparent)]
    Decompress(#[from] DecompressError),

    #[error("the run was cancelled, and nothing is presented as finished")]
    Cancelled,
}

/// Where the publisher's images and signing key are fetched from.
pub struct Sources<'a> {
    /// The directory the publisher serves images from, with its trailing slash.
    pub images: &'a str,
    /// The address of the publisher's signing key.
    pub signing_key: &'a str,
}

/// Obtains the image for `device_id` into `output_dir`. Every step records itself in `log` as it runs, so a failed
/// run still shows what it established before it stopped. Each change of what the run is doing goes to `report`,
/// and a cancelled token ends the run at the step it is in.
pub async fn run(
    client: &reqwest::Client,
    device_id: &str,
    sources: &Sources<'_>,
    output_dir: &Path,
    cancel: &CancellationToken,
    mut report: impl FnMut(Activity),
    log: &mut StepLog,
) -> Result<Obtained, RunError> {
    report(Activity::Resolving);
    let resolution = catalogue::resolve(device_id)
        .inspect_err(|err| log.record(Step::Resolve, err.to_string()))?;
    let device = resolution.device;
    let address = format!("{}{}", sources.images, device.image_name());
    log.record(
        Step::Resolve,
        format!("{} boots {}", device.name, device.image_name()),
    );
    log.record(Step::Resolve, format!("fetching from {address}"));
    for caveat in resolution.caveats {
        log.record(Step::Resolve, format!("caveat: {caveat}"));
    }
    if cancel.is_cancelled() {
        return Err(RunError::Cancelled);
    }

    let image = output_dir.join(device.image_name());
    report(Activity::Downloading {
        written: 0,
        total: None,
    });
    let download = match fetch::download(client, &address, &image, cancel, |p| {
        report(Activity::Downloading {
            written: p.written,
            total: p.total,
        });
    })
    .await
    {
        Ok(download) => download,
        Err(err) => {
            log.record(Step::Download, err.to_string());
            return Err(match err {
                FetchError::Cancelled { .. } => RunError::Cancelled,
                err => err.into(),
            });
        }
    };
    report(Activity::Verifying);
    log.record(
        Step::Download,
        format!("{} bytes written to {}", download.bytes, image.display()),
    );
    match &download.verification {
        Verification::Verified { source, digest } => log.record(
            Step::Checksum,
            format!("verified: sha256 {digest} matches {source}"),
        ),
        Verification::Unverified { source, reason } => log.record(
            Step::Checksum,
            format!("unverified against {source}: {reason}"),
        ),
    }

    let check =
        match signature::verify_download(client, &address, sources.signing_key, &image, cancel)
            .await
        {
            Ok(check) => check,
            Err(SignatureError::Cancelled) => {
                log.record(Step::Signature, "cancelled");
                // The bytes were checksummed but nothing checked their signature, so the operator is shown a
                // cancellation rather than a file they could mistake for one the run vouched for.
                let _ = tokio::fs::remove_file(&image).await;
                log.record(
                    Step::Signature,
                    format!("removed {} before it was verified", image.display()),
                );
                return Err(RunError::Cancelled);
            }
            Err(err) => {
                log.record(Step::Signature, err.to_string());
                return Err(err.into());
            }
        };
    match &check {
        SignatureCheck::Verified {
            key_id, key_source, ..
        } => log.record(
            Step::Signature,
            format!(
                "verified: made by signing key {key_id}, retrieved over HTTPS from {key_source} at the time of verification; the key is identified by its ID, not by a fingerprint checked against an independent source"
            ),
        ),
        SignatureCheck::Unverified { key_source, reason } => {
            let halt = RunError::Unverified {
                file: fetch::filename(&address),
                reason: reason.clone(),
            };
            log.record(
                Step::Signature,
                format!("unverified: key from {key_source}: {reason}"),
            );
            return Err(halt);
        }
    }

    let flashable = image.with_extension("");
    report(Activity::Decompressing);
    let decompression = match decompress::decompress(&image, &flashable, cancel).await {
        Ok(decompression) => decompression,
        Err(err) => {
            log.record(Step::Decompress, err.to_string());
            return Err(match err {
                DecompressError::Cancelled { .. } => RunError::Cancelled,
                err => err.into(),
            });
        }
    };
    log.record(
        Step::Decompress,
        format!(
            "{} bytes written to {}; the decompressed image is not verified, since the publisher publishes no checksum for it",
            decompression.bytes,
            flashable.display()
        ),
    );

    Ok(Obtained { image, flashable })
}

/// A run on its own thread, so the interface keeps drawing and taking keys while the transfer goes on.
pub struct Job {
    reports: Receiver<Activity>,
    current: Activity,
    cancel: CancellationToken,
    handle: Option<JoinHandle<(Result<Obtained, RunError>, StepLog)>>,
}

impl Job {
    /// Starts obtaining the image for `device_id` into `output_dir`, from the publisher's addresses.
    pub fn start(device_id: &str, output_dir: PathBuf) -> Self {
        let (sender, reports) = mpsc::channel();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let device_id = device_id.to_owned();
        let handle = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a single-threaded runtime can be built");
            let mut log = StepLog::default();
            let sources = Sources {
                images: ADDRESS_PREFIX,
                signing_key: SIGNING_KEY_URL,
            };
            let result = runtime.block_on(run(
                &reqwest::Client::new(),
                &device_id,
                &sources,
                &output_dir,
                &token,
                |activity| {
                    let _ = sender.send(activity);
                },
                &mut log,
            ));
            (result, log)
        });
        Self {
            reports,
            current: Activity::Resolving,
            cancel,
            handle: Some(handle),
        }
    }

    /// What the run is doing now.
    pub fn current(&self) -> Activity {
        self.current
    }

    /// Asks the run to stop at the step it is in. The run ends with [`RunError::Cancelled`] and nothing presented
    /// as finished.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Takes in what the run has reported since the last call. Once the run has ended, returns its outcome and
    /// its log, and returns `None` on every call before that.
    pub fn poll(&mut self) -> Option<(Result<Obtained, RunError>, StepLog)> {
        while let Ok(activity) = self.reports.try_recv() {
            self.current = activity;
        }
        if !self.handle.as_ref()?.is_finished() {
            return None;
        }
        self.handle
            .take()
            .map(|handle| handle.join().expect("the run does not panic"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::CHECKSUM_SUFFIX;
    use crate::signature::SIGNATURE_SUFFIX;
    use crate::test_publisher::Publisher;
    use crate::test_server::{Reply, client, serve};
    use liblzma::write::XzEncoder;
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;
    use std::io::Write;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    const IMAGE: &str = "DietPi_RPi5-ARMv8-Trixie.img.xz";
    const KEY: &str = "MichaIng.gpg";

    /// Three megabytes, larger than the decompression buffer, so the decompression takes several reads.
    fn payload() -> Vec<u8> {
        (0..3_000_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect()
    }

    fn compress(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = XzEncoder::new(Vec::new(), 6);
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn path(name: &str) -> String {
        format!("/images/{name}")
    }

    fn routes(image: &[u8], signature: Vec<u8>, key: Vec<u8>) -> HashMap<String, Reply> {
        HashMap::from([
            (path(IMAGE), Reply::Body(image.to_vec())),
            (
                path(&format!("{IMAGE}{CHECKSUM_SUFFIX}")),
                Reply::Body(format!("{}  {IMAGE}\n", sha256_hex(image)).into_bytes()),
            ),
            (
                path(&format!("{IMAGE}{SIGNATURE_SUFFIX}")),
                Reply::Body(signature),
            ),
            (path(KEY), Reply::Body(key)),
        ])
    }

    fn steps(log: &StepLog) -> Vec<Step> {
        let mut steps: Vec<Step> = log.entries().iter().map(|entry| entry.step).collect();
        steps.dedup();
        steps
    }

    /// Obtains the image for `rpi-5` from the server at `base`, with the publisher's key served at [`KEY`].
    async fn obtain(
        base: &str,
        dir: &Path,
        cancel: &CancellationToken,
        report: impl FnMut(Activity),
        log: &mut StepLog,
    ) -> Result<Obtained, RunError> {
        let signing_key = format!("{base}{KEY}");
        let sources = Sources {
            images: base,
            signing_key: &signing_key,
        };
        run(&client(), "rpi-5", &sources, dir, cancel, report, log).await
    }

    #[tokio::test]
    async fn a_full_run_writes_every_step_to_the_log() {
        let publisher = Publisher::new();
        let data = payload();
        let image = compress(&data);
        let base = serve(routes(
            &image,
            publisher.sign(&image),
            publisher.armored_key(),
        ));
        let dir = tempdir().unwrap();
        let mut log = StepLog::default();
        let cancel = CancellationToken::new();
        let mut seen = Vec::new();

        let obtained = obtain(&base, dir.path(), &cancel, |a| seen.push(a), &mut log)
            .await
            .unwrap();

        assert_eq!(
            obtained,
            Obtained {
                image: dir.path().join(IMAGE),
                flashable: dir.path().join("DietPi_RPi5-ARMv8-Trixie.img"),
            }
        );
        // The run reports each state in order, and the transfer's final state says every byte arrived.
        assert!(
            matches!(seen.first(), Some(Activity::Resolving)),
            "{seen:?}"
        );
        assert!(seen.last() == Some(&Activity::Decompressing), "{seen:?}");
        assert!(
            seen.iter()
                .any(|a| matches!(a, Activity::Downloading { total: Some(_), .. })),
            "{seen:?}"
        );
        assert_eq!(std::fs::read(&obtained.flashable).unwrap(), data);
        assert_eq!(
            steps(&log),
            [
                Step::Resolve,
                Step::Download,
                Step::Checksum,
                Step::Signature,
                Step::Decompress,
            ]
        );
        let rendered = log.to_string();
        assert!(rendered.contains(&publisher.key_id()), "{rendered}");
        assert!(rendered.contains(&format!("{base}{KEY}")), "{rendered}");
        assert!(
            rendered.contains("at the time of verification"),
            "{rendered}"
        );
        assert!(
            rendered.contains("not by a fingerprint checked against an independent source"),
            "{rendered}"
        );
    }

    #[tokio::test]
    async fn a_refused_signature_halts_the_run_and_the_log_says_why() {
        let publisher = Publisher::new();
        let data = payload();
        let image = compress(&data);
        let mut tampered = image.clone();
        tampered[1000] ^= 0x01;
        let base = serve(routes(
            &tampered,
            publisher.sign(&image),
            publisher.armored_key(),
        ));
        let dir = tempdir().unwrap();
        let mut log = StepLog::default();
        let cancel = CancellationToken::new();

        let err = obtain(&base, dir.path(), &cancel, |_| {}, &mut log)
            .await
            .unwrap_err();

        assert!(matches!(err, RunError::Signature(_)), "{err}");
        assert_eq!(steps(&log).last(), Some(&Step::Signature));
        assert!(log.to_string().contains("does not verify"), "{log}");
        assert!(!dir.path().join(IMAGE).exists());
        assert!(!dir.path().join("DietPi_RPi5-ARMv8-Trixie.img").exists());
    }

    /// Task 7.3: cancelling a run mid-transfer ends it with `Cancelled`, and the output directory is left empty.
    #[tokio::test]
    async fn a_run_cancelled_mid_transfer_leaves_the_directory_empty() {
        let publisher = Publisher::new();
        let image = compress(&payload());
        let mut served = routes(&image, publisher.sign(&image), publisher.armored_key());
        served.insert(
            path(IMAGE),
            Reply::Slow {
                body: image.clone(),
                pieces: 12,
                pause_ms: 50,
            },
        );
        let base = serve(served);
        let dir = tempdir().unwrap();
        let mut log = StepLog::default();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            token.cancel();
        });
        let mut seen = Vec::new();

        let err = obtain(&base, dir.path(), &cancel, |a| seen.push(a), &mut log)
            .await
            .unwrap_err();

        assert!(matches!(err, RunError::Cancelled), "{err}");
        assert!(
            matches!(
                seen.last(),
                Some(Activity::Downloading { written, total: Some(_) }) if *written > 0
            ),
            "{seen:?}"
        );
        assert_eq!(steps(&log), [Step::Resolve, Step::Download]);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "a cancelled run leaves nothing in the output directory"
        );
    }
}
