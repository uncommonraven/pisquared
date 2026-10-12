//! Runs the obtaining procedure for one device from start to finish, writing every step to the step log.
//!
//! The steps are resolve, download with its checksum, signature, decompress, and configure, in that order. A
//! run completes only when the image's signature verifies, so only an image whose signature verified is ever decompressed.
//!
//! The downloaded image is removed from the output path whenever the run does not vouch for it: when its signature
//! does not verify, when its signature cannot be checked, or when the run is cancelled. A checksum that cannot be
//! had does not stop the run, because the signature is what gates success; the outcome says the checksum was
//! unverified. Cancellation stops the run at the step it is in, because the operator asked it to stop rather than
//! because anything failed.
//!
//! The decompressed file is edited by the configure step. If that edit fails or is cancelled, the decompressed file is
//! removed, so no image with unwritten Wi-Fi choices is presented as finished.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::backend::catalogue::{self, ADDRESS_PREFIX, NotListed, Resolution};
use crate::backend::configure::Settings;
use crate::backend::decompress::{self, DecompressError};
use crate::backend::fetch::{self, Download, FetchError, Verification};
use crate::backend::image::{self, ImageError};
use crate::backend::log::{Step, StepLog};
use crate::backend::signature::{self, SIGNING_KEY_URL, SignatureCheck, SignatureError};

/// What a completed run left on disk: the compressed image and the decompressed file beside it. The signature
/// gates success, so the image's checksum may still be unverified; `verification` says which it was. The decompressed
/// file has the operator's choices written into it, so it no longer matches the publisher's bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct Obtained {
    pub image: PathBuf,
    pub flashable: PathBuf,
    pub verification: Verification,
}

/// What the run is doing at this moment, reported as it changes so an attached interface can show a live state
/// without reading the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Resolving,
    Downloading { written: u64, total: Option<u64> },
    Verifying,
    Decompressing,
    Configuring,
}

#[derive(Debug, Error)]
pub enum RunError {
    #[error(transparent)]
    NotListed(#[from] NotListed),

    #[error(transparent)]
    Download(FetchError),

    #[error(transparent)]
    Signature(#[from] SignatureError),

    #[error("{file} is not verified by its signature, so the run halts: {reason}")]
    Unverified { file: String, reason: String },

    #[error(transparent)]
    Decompress(DecompressError),

    #[error("the image was not configured, so it is removed: {0}")]
    Configure(ImageError),

    #[error("the run was cancelled, and nothing is presented as finished")]
    Cancelled,
}

/// A transfer the operator stopped is a cancelled run, not a download failure.
impl From<FetchError> for RunError {
    fn from(err: FetchError) -> Self {
        match err {
            FetchError::Cancelled { .. } => RunError::Cancelled,
            err => RunError::Download(err),
        }
    }
}

/// A decode the operator stopped is a cancelled run, not a decompression failure.
impl From<DecompressError> for RunError {
    fn from(err: DecompressError) -> Self {
        match err {
            DecompressError::Cancelled { .. } => RunError::Cancelled,
            err => RunError::Decompress(err),
        }
    }
}

/// Where the publisher's images and signing key are fetched from.
pub struct Sources<'a> {
    /// The directory the publisher serves images from, with its trailing slash.
    pub images: &'a str,
    /// The address of the publisher's signing key.
    pub signing_key: &'a str,
}

/// Obtains the image for `device_id` into `output_dir`, and writes `settings` into it. Every step records itself in `log`
/// as it runs, so a failed run still shows what it established before it stopped. Each change of what the run is doing
/// goes to `report`, and a cancelled token ends the run at the step it is in.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: &reqwest::Client,
    device_id: &str,
    settings: &Settings,
    sources: &Sources<'_>,
    output_dir: &Path,
    cancel: &CancellationToken,
    mut report: impl FnMut(Activity),
    log: &mut StepLog,
) -> Result<Obtained, RunError> {
    report(Activity::Resolving);
    let resolution = resolve_device(device_id, sources.images, log)?;
    if cancel.is_cancelled() {
        return Err(RunError::Cancelled);
    }

    let image = output_dir.join(resolution.device.image_name());
    let download = download_image(
        client,
        &resolution.address,
        &image,
        cancel,
        &mut report,
        log,
    )
    .await?;

    report(Activity::Verifying);
    verify_signature(
        client,
        &resolution.address,
        sources.signing_key,
        &image,
        cancel,
        log,
    )
    .await?;

    let flashable = image.with_extension("");
    report(Activity::Decompressing);
    decompress_image(&image, &flashable, cancel, log).await?;

    report(Activity::Configuring);
    configure_image(&flashable, settings, cancel, log).await?;

    Ok(Obtained {
        image,
        flashable,
        verification: download.verification,
    })
}

fn resolve_device(
    device_id: &str,
    images: &str,
    log: &mut StepLog,
) -> Result<Resolution, RunError> {
    let resolution = catalogue::resolve(device_id, images)
        .inspect_err(|err| log.record(Step::Resolve, err.to_string()))?;
    log.record(
        Step::Resolve,
        format!(
            "{} boots {}",
            resolution.device.name,
            resolution.device.image_name()
        ),
    );
    log.record(
        Step::Resolve,
        format!("fetching from {}", resolution.address),
    );
    for caveat in resolution.caveats {
        log.record(Step::Resolve, format!("caveat: {caveat}"));
    }
    Ok(resolution)
}

async fn download_image(
    client: &reqwest::Client,
    address: &str,
    image: &Path,
    cancel: &CancellationToken,
    report: &mut impl FnMut(Activity),
    log: &mut StepLog,
) -> Result<Download, RunError> {
    report(Activity::Downloading {
        written: 0,
        total: None,
    });
    let download = fetch::download(client, address, image, cancel, |p| {
        report(Activity::Downloading {
            written: p.written,
            total: p.total,
        });
    })
    .await
    .inspect_err(|err| log.record(Step::Download, err.to_string()))?;

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
    Ok(download)
}

async fn verify_signature(
    client: &reqwest::Client,
    address: &str,
    signing_key: &str,
    image: &Path,
    cancel: &CancellationToken,
    log: &mut StepLog,
) -> Result<(), RunError> {
    let check = match signature::verify_download(client, address, signing_key, image, cancel).await
    {
        Ok(check) => check,
        Err(SignatureError::Cancelled) => {
            log.record(Step::Signature, "cancelled");
            remove_unvouched(image, log).await;
            return Err(RunError::Cancelled);
        }
        Err(err) => {
            log.record(Step::Signature, err.to_string());
            return Err(err.into());
        }
    };

    match check {
        SignatureCheck::Verified {
            key_id, key_source, ..
        } => {
            log.record(
                Step::Signature,
                format!(
                    "verified: made by signing key {key_id}, retrieved over HTTPS from {key_source} at the time of verification; the key is identified by its ID, not by a fingerprint checked against an independent source"
                ),
            );
            Ok(())
        }
        SignatureCheck::Unverified { key_source, reason } => {
            log.record(
                Step::Signature,
                format!("unverified: key from {key_source}: {reason}"),
            );
            remove_unvouched(image, log).await;
            Err(RunError::Unverified {
                file: fetch::filename(address),
                reason,
            })
        }
    }
}

/// Removes the image from the output path because the run does not vouch for it, and records that in the log.
async fn remove_unvouched(image: &Path, log: &mut StepLog) {
    let _ = tokio::fs::remove_file(image).await;
    log.record(
        Step::Signature,
        format!("removed {} before it was verified", image.display()),
    );
}

async fn decompress_image(
    image: &Path,
    flashable: &Path,
    cancel: &CancellationToken,
    log: &mut StepLog,
) -> Result<(), RunError> {
    let decompression = decompress::decompress(image, flashable, cancel, |_| {})
        .await
        .inspect_err(|err| log.record(Step::Decompress, err.to_string()))?;
    log.record(
        Step::Decompress,
        format!(
            "{} bytes written to {}; the decompressed image is not verified, since the publisher publishes no checksum for it",
            decompression.bytes,
            flashable.display()
        ),
    );
    Ok(())
}

/// Writes the operator's choices into the boot partition of the decompressed file. A cancel, or a failure to write, removes
/// the decompressed file, so an image without its choices is never presented as finished. The log says what was written
/// without saying what the values were.
async fn configure_image(
    flashable: &Path,
    settings: &Settings,
    cancel: &CancellationToken,
    log: &mut StepLog,
) -> Result<(), RunError> {
    let result = image::configure_image(flashable, settings);
    if cancel.is_cancelled() {
        log.record(Step::Configure, "cancelled");
        remove_unconfigured(flashable, log).await;
        return Err(RunError::Cancelled);
    }
    match result {
        Ok(()) => {
            log.record(
                Step::Configure,
                format!(
                    "choices written to the boot partition of {}; the Wi-Fi passphrase, if any, and the password are stored there in plain text",
                    flashable.display()
                ),
            );
            Ok(())
        }
        Err(err) => {
            log.record(Step::Configure, err.to_string());
            remove_unconfigured(flashable, log).await;
            Err(RunError::Configure(err))
        }
    }
}

/// Removes the decompressed file because its choices were not written, and records that in the log.
async fn remove_unconfigured(flashable: &Path, log: &mut StepLog) {
    let _ = tokio::fs::remove_file(flashable).await;
    log.record(
        Step::Configure,
        format!("removed {} before it was configured", flashable.display()),
    );
}

/// A run on its own thread, so the interface keeps drawing and taking keys while the transfer goes on.
pub struct Job {
    reports: Receiver<Activity>,
    current: Activity,
    cancel: CancellationToken,
    handle: Option<JoinHandle<(Result<Obtained, RunError>, StepLog)>>,
}

impl Job {
    /// Starts obtaining the image for `device_id` into `output_dir`, from the publisher's addresses, and configuring it
    /// with `settings`.
    pub fn start(device_id: &str, settings: Settings, output_dir: PathBuf) -> Self {
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
                &settings,
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
    use crate::backend::fetch::CHECKSUM_SUFFIX;
    use crate::backend::network::{Connection, Wifi};
    use crate::backend::signature::SIGNATURE_SUFFIX;
    use crate::testing::test_publisher::Publisher;
    use crate::testing::test_server::{Reply, client, image_path, serve};
    use crate::testing::test_support::{
        boot_file, boot_image, checksum_file, payload, sha256_hex, xz,
    };
    use std::collections::HashMap;
    use tempfile::tempdir;

    const IMAGE: &str = "DietPi_RPi5-ARMv8-Trixie.img.xz";
    const FLASHABLE: &str = "DietPi_RPi5-ARMv8-Trixie.img";
    const KEY: &str = "MichaIng.gpg";
    const DIETPI_TXT: &str = "AUTO_SETUP_NET_ETHERNET_ENABLED=1\nAUTO_SETUP_NET_WIFI_ENABLED=0\nAUTO_SETUP_NET_WIFI_COUNTRY_CODE=GB\n";
    const DIETPI_WIFI_TXT: &str = "aWIFI_SSID[0]=''\naWIFI_KEY[0]=''\naWIFI_KEYMGR[0]='WPA-PSK'\n";

    fn routes(image: &[u8], signature: Vec<u8>, key: Vec<u8>) -> HashMap<String, Reply> {
        HashMap::from([
            (image_path(IMAGE), Reply::Body(image.to_vec())),
            (
                image_path(&format!("{IMAGE}{CHECKSUM_SUFFIX}")),
                Reply::Body(checksum_file(image, IMAGE)),
            ),
            (
                image_path(&format!("{IMAGE}{SIGNATURE_SUFFIX}")),
                Reply::Body(signature),
            ),
            (image_path(KEY), Reply::Body(key)),
        ])
    }

    fn steps(log: &StepLog) -> Vec<Step> {
        let mut steps: Vec<Step> = log.entries().iter().map(|entry| entry.step).collect();
        steps.dedup();
        steps
    }

    /// Obtains the image for `rpi-5` over Ethernet from the server at `base`, with the publisher's key served at [`KEY`].
    async fn obtain(
        base: &str,
        dir: &Path,
        cancel: &CancellationToken,
        report: impl FnMut(Activity),
        log: &mut StepLog,
    ) -> Result<Obtained, RunError> {
        obtain_with(
            &settings(Connection::Ethernet),
            base,
            dir,
            cancel,
            report,
            log,
        )
        .await
    }

    /// Obtains the image for `rpi-5` with `settings` from the server at `base`, as [`obtain`] does.
    async fn obtain_with(
        settings: &Settings,
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
        run(
            &client(),
            "rpi-5",
            settings,
            &sources,
            dir,
            cancel,
            report,
            log,
        )
        .await
    }

    fn home() -> Wifi {
        Wifi {
            ssid: "Home Network".to_owned(),
            passphrase: "correct horse".to_owned(),
            country_code: "GB".to_owned(),
        }
    }

    fn settings(connection: Connection) -> Settings {
        Settings {
            connection,
            hostname: "mypi".to_owned(),
            password: "a-long-password".to_owned(),
        }
    }

    fn wifi_settings() -> Settings {
        settings(Connection::Wifi(home()))
    }

    fn boot_disk() -> Vec<u8> {
        boot_image(&[
            ("dietpi.txt", DIETPI_TXT),
            ("dietpi-wifi.txt", DIETPI_WIFI_TXT),
        ])
    }

    #[tokio::test]
    async fn a_full_run_writes_every_step_to_the_log() {
        let publisher = Publisher::new();
        let image = xz(&boot_disk());
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
                flashable: dir.path().join(FLASHABLE),
                verification: Verification::Verified {
                    source: format!("{base}{IMAGE}{CHECKSUM_SUFFIX}"),
                    digest: sha256_hex(&image),
                },
            }
        );
        // The run reports each state in order, and the transfer's final state says every byte arrived.
        assert!(
            matches!(seen.first(), Some(Activity::Resolving)),
            "{seen:?}"
        );
        assert!(seen.last() == Some(&Activity::Configuring), "{seen:?}");
        assert!(
            seen.iter()
                .any(|a| matches!(a, Activity::Downloading { total: Some(_), .. })),
            "{seen:?}"
        );
        let flashable = std::fs::read(&obtained.flashable).unwrap();
        assert_eq!(
            boot_file(&flashable, "dietpi.txt"),
            "AUTO_SETUP_NET_ETHERNET_ENABLED=1\nAUTO_SETUP_NET_WIFI_ENABLED=0\nAUTO_SETUP_NET_WIFI_COUNTRY_CODE=GB\nAUTO_SETUP_NET_HOSTNAME=mypi\nAUTO_SETUP_GLOBAL_PASSWORD=a-long-password\nAUTO_SETUP_AUTOMATED=1\n"
        );
        assert_eq!(boot_file(&flashable, "dietpi-wifi.txt"), DIETPI_WIFI_TXT);
        assert_eq!(
            steps(&log),
            [
                Step::Resolve,
                Step::Download,
                Step::Checksum,
                Step::Signature,
                Step::Decompress,
                Step::Configure,
            ]
        );
        let rendered = log.to_string();
        assert!(rendered.contains(&publisher.key_id()), "{rendered}");
        assert!(rendered.contains(&format!("{base}{KEY}")), "{rendered}");
    }

    #[tokio::test]
    async fn a_refused_signature_halts_the_run_and_the_log_says_why() {
        let publisher = Publisher::new();
        let data = payload();
        let image = xz(&data);
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
        assert!(!dir.path().join(FLASHABLE).exists());
    }

    /// A signature that cannot be checked halts the run, removes the image so nothing the run did not vouch for is
    /// left at the output path, and never decompresses it.
    #[tokio::test]
    async fn an_unverified_signature_halts_the_run_and_removes_the_image() {
        let publisher = Publisher::new();
        let image = xz(&payload());
        let mut served = routes(&image, publisher.sign(&image), publisher.armored_key());
        served.insert(
            image_path(&format!("{IMAGE}{SIGNATURE_SUFFIX}")),
            Reply::Status(404),
        );
        let base = serve(served);
        let dir = tempdir().unwrap();
        let mut log = StepLog::default();
        let cancel = CancellationToken::new();

        let err = obtain(&base, dir.path(), &cancel, |_| {}, &mut log)
            .await
            .unwrap_err();

        assert!(
            matches!(&err, RunError::Unverified { reason, .. } if reason.contains("404")),
            "{err}"
        );
        assert_eq!(steps(&log).last(), Some(&Step::Signature));
        assert!(!dir.path().join(IMAGE).exists());
        assert!(!dir.path().join(FLASHABLE).exists());
    }

    /// A checksum that cannot be had does not stop a run whose signature verifies, and the outcome says the
    /// checksum was unverified.
    #[tokio::test]
    async fn an_unretrievable_checksum_does_not_stop_a_run_with_a_verified_signature() {
        let publisher = Publisher::new();
        let image = xz(&boot_disk());
        let mut served = routes(&image, publisher.sign(&image), publisher.armored_key());
        served.insert(
            image_path(&format!("{IMAGE}{CHECKSUM_SUFFIX}")),
            Reply::Status(404),
        );
        let base = serve(served);
        let dir = tempdir().unwrap();
        let mut log = StepLog::default();
        let cancel = CancellationToken::new();

        let obtained = obtain(&base, dir.path(), &cancel, |_| {}, &mut log)
            .await
            .unwrap();

        assert!(
            matches!(&obtained.verification, Verification::Unverified { reason, .. } if reason.contains("404")),
            "{obtained:?}"
        );
        assert!(obtained.flashable.exists());
    }

    /// Cancelling a run mid-transfer ends it with `Cancelled`, and the output directory is left empty. The cancel is
    /// issued from the progress report once bytes have arrived, so it is certain to land mid-transfer.
    #[tokio::test]
    async fn a_run_cancelled_mid_transfer_leaves_the_directory_empty() {
        let publisher = Publisher::new();
        let image = xz(&payload());
        let mut served = routes(&image, publisher.sign(&image), publisher.armored_key());
        served.insert(
            image_path(IMAGE),
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
        let mut seen = Vec::new();
        let report = |activity: Activity| {
            if matches!(activity, Activity::Downloading { written, .. } if written > 0) {
                token.cancel();
            }
            seen.push(activity);
        };

        let err = obtain(&base, dir.path(), &cancel, report, &mut log)
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

    /// A Wi-Fi run writes the choices into the boot partition of the decompressed file, after it has decompressed. The
    /// log says so, and neither the log nor the outcome shows the SSID or the passphrase.
    #[tokio::test]
    async fn a_wifi_run_writes_the_choices_into_the_decompressed_boot_partition() {
        let publisher = Publisher::new();
        let image = xz(&boot_disk());
        let base = serve(routes(
            &image,
            publisher.sign(&image),
            publisher.armored_key(),
        ));
        let dir = tempdir().unwrap();
        let mut log = StepLog::default();
        let cancel = CancellationToken::new();

        let obtained = obtain_with(
            &wifi_settings(),
            &base,
            dir.path(),
            &cancel,
            |_| {},
            &mut log,
        )
        .await
        .unwrap();

        let flashable = std::fs::read(&obtained.flashable).unwrap();
        assert_eq!(
            boot_file(&flashable, "dietpi.txt"),
            "AUTO_SETUP_NET_ETHERNET_ENABLED=0\nAUTO_SETUP_NET_WIFI_ENABLED=1\nAUTO_SETUP_NET_WIFI_COUNTRY_CODE=GB\nAUTO_SETUP_NET_HOSTNAME=mypi\nAUTO_SETUP_GLOBAL_PASSWORD=a-long-password\nAUTO_SETUP_AUTOMATED=1\n"
        );
        assert_eq!(
            boot_file(&flashable, "dietpi-wifi.txt"),
            "aWIFI_SSID[0]='Home Network'\naWIFI_KEY[0]='correct horse'\naWIFI_KEYMGR[0]='WPA-PSK'\n"
        );
        assert_eq!(steps(&log).last(), Some(&Step::Configure));
        let rendered = log.to_string();
        assert!(!rendered.contains("correct horse"), "{rendered}");
        assert!(!rendered.contains("Home Network"), "{rendered}");
        assert!(!rendered.contains("a-long-password"), "{rendered}");
    }

    /// A Wi-Fi run on an image with no FAT boot partition fails with the reason logged, and removes the decompressed file.
    #[tokio::test]
    async fn a_wifi_run_on_an_image_without_a_boot_partition_removes_the_decompressed_file() {
        let publisher = Publisher::new();
        let image = xz(&payload());
        let base = serve(routes(
            &image,
            publisher.sign(&image),
            publisher.armored_key(),
        ));
        let dir = tempdir().unwrap();
        let mut log = StepLog::default();
        let cancel = CancellationToken::new();

        let err = obtain_with(
            &wifi_settings(),
            &base,
            dir.path(),
            &cancel,
            |_| {},
            &mut log,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, RunError::Configure(_)), "{err}");
        assert_eq!(steps(&log).last(), Some(&Step::Configure));
        assert!(log.to_string().contains("removed"), "{log}");
        assert!(!dir.path().join(FLASHABLE).exists());
    }

    /// Cancelling a Wi-Fi run while it is configuring ends it with `Cancelled`, and the decompressed file is removed, so
    /// an image without its Wi-Fi choices is never presented.
    #[tokio::test]
    async fn a_run_cancelled_while_configuring_removes_the_decompressed_file() {
        let publisher = Publisher::new();
        let image = xz(&boot_disk());
        let base = serve(routes(
            &image,
            publisher.sign(&image),
            publisher.armored_key(),
        ));
        let dir = tempdir().unwrap();
        let mut log = StepLog::default();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let report = |activity: Activity| {
            if activity == Activity::Configuring {
                token.cancel();
            }
        };

        let err = obtain_with(
            &wifi_settings(),
            &base,
            dir.path(),
            &cancel,
            report,
            &mut log,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, RunError::Cancelled), "{err}");
        assert_eq!(steps(&log).last(), Some(&Step::Configure));
        assert!(!dir.path().join(FLASHABLE).exists());
    }
}
