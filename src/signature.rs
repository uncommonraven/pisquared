//! Verifies a downloaded image against the detached signature its publisher ships beside it.
//!
//! The signing key is fetched over HTTPS from the publisher's GitHub account at the time of verification and is
//! held in memory, so nothing is written to the operator's keyring. The signing subkey is the one whose key ID the
//! signature names. A signature that does not verify is a refusal: the image is removed and the run halts. A key or
//! signature that cannot be retrieved is reported as unverified with its reason, and is never reported as verified
//! or as a bad signature.

use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey};
use pgp::types::KeyDetails;
use reqwest::StatusCode;
use thiserror::Error;

use crate::fetch::describe;

/// Appended to an image address for the publisher's detached signature.
pub const SIGNATURE_SUFFIX: &str = ".asc";

/// Where the publisher publishes the signing key.
pub const SIGNING_KEY_URL: &str = "https://github.com/MichaIng.gpg";

/// How the downloaded image was checked against its signature. A refusal is an error, not a result.
#[derive(Debug, PartialEq, Eq)]
pub enum SignatureCheck {
    /// The signature verifies. `key_id` is the key that made it, which may be a signing subkey of
    /// `primary_fingerprint`.
    Verified {
        key_id: String,
        primary_fingerprint: String,
        key_source: String,
    },
    /// The signing key or the signature could not be had. The reason is kept so nothing reads it as a pass.
    Unverified { key_source: String, reason: String },
}

#[derive(Debug, Error)]
pub enum SignatureError {
    #[error("{file} does not verify against the signature published at {signature_url}: {reason}")]
    Refused {
        file: String,
        signature_url: String,
        reason: String,
    },

    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// The key a signature verified against: the key ID that made it and the primary key it belongs to.
#[derive(Debug, PartialEq, Eq)]
pub struct Signer {
    pub key_id: String,
    pub primary_fingerprint: String,
}

/// Verifies the image at `image` against the signature published at `image_url` plus [`SIGNATURE_SUFFIX`], using
/// the signing key at `key_url`. A signature that does not verify removes `image` and returns an error, so nothing
/// is left presented as usable. A key or signature that cannot be retrieved is reported as unverified.
pub async fn verify_download(
    client: &reqwest::Client,
    image_url: &str,
    key_url: &str,
    image: &Path,
) -> Result<SignatureCheck, SignatureError> {
    let signature_url = format!("{image_url}{SIGNATURE_SUFFIX}");
    let unverified = |reason| SignatureCheck::Unverified {
        key_source: key_url.to_owned(),
        reason,
    };

    let cert = match retrieve_key(client, key_url).await {
        Ok(cert) => cert,
        Err(reason) => return Ok(unverified(reason)),
    };
    let armored = match retrieve_signature(client, &signature_url).await {
        Ok(armored) => armored,
        Err(reason) => return Ok(unverified(reason)),
    };

    let image_path = image.to_path_buf();
    let check_url = signature_url.clone();
    let checked =
        tokio::task::spawn_blocking(move || verify_file(&cert, &armored, &image_path, &check_url))
            .await
            .map_err(|err| SignatureError::Io {
                path: image.to_path_buf(),
                source: io::Error::other(err),
            })?;

    match checked {
        Ok(signer) => Ok(SignatureCheck::Verified {
            key_id: signer.key_id,
            primary_fingerprint: signer.primary_fingerprint,
            key_source: key_url.to_owned(),
        }),
        Err(refusal @ SignatureError::Refused { .. }) => {
            let _ = tokio::fs::remove_file(image).await;
            Err(refusal)
        }
        Err(err) => Err(err),
    }
}

/// Fetches the publisher's signing key. The key is returned in memory; nothing is written to disk.
pub async fn retrieve_key(
    client: &reqwest::Client,
    key_url: &str,
) -> Result<SignedPublicKey, String> {
    let body = get(client, key_url)
        .await
        .map_err(|reason| format!("the signing key could not be retrieved: {reason}"))?;
    let (key, _) = SignedPublicKey::from_string(&String::from_utf8_lossy(&body))
        .map_err(|err| format!("the signing key could not be read: {err}"))?;
    Ok(key)
}

/// Fetches the detached signature published beside an image, armored, as the publisher serves it.
pub async fn retrieve_signature(
    client: &reqwest::Client,
    signature_url: &str,
) -> Result<String, String> {
    let body = get(client, signature_url)
        .await
        .map_err(|reason| format!("the signature could not be retrieved: {reason}"))?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Checks the file at `image` against the armored signature `armored`, made by a key in `cert`. The file is read
/// once as a stream. The key is selected by the key ID the signature names: the primary key, or the signing subkey
/// with that ID.
pub fn verify_file(
    cert: &SignedPublicKey,
    armored: &str,
    image: &Path,
    signature_url: &str,
) -> Result<Signer, SignatureError> {
    let file = image
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let refused = |reason: String| SignatureError::Refused {
        file: file.clone(),
        signature_url: signature_url.to_owned(),
        reason,
    };

    let (sig, _) = DetachedSignature::from_string(armored)
        .map_err(|err| refused(format!("the signature could not be read: {err}")))?;
    let issuer = sig
        .signature
        .issuer_key_id()
        .first()
        .map(hex)
        .ok_or_else(|| refused("the signature names no key ID".to_owned()))?;

    let primary_fingerprint = format!("{:X}", cert.primary_key.fingerprint());
    let primary_key_id = hex(cert.primary_key.legacy_key_id());
    cert.verify_bindings().map_err(|err| {
        refused(format!(
            "the signing key's subkey bindings do not verify: {err}"
        ))
    })?;

    let source = std::fs::File::open(image).map_err(|source| SignatureError::Io {
        path: image.to_path_buf(),
        source,
    })?;
    let reader = BufReader::new(source);
    let checked = if issuer == primary_key_id {
        sig.signature.verify(cert, reader)
    } else if let Some(subkey) = cert
        .public_subkeys
        .iter()
        .find(|subkey| hex(subkey.key.legacy_key_id()) == issuer)
    {
        sig.signature.verify(subkey, reader)
    } else {
        return Err(refused(format!(
            "it is made by key {issuer}, which is not among the keys of {primary_fingerprint}"
        )));
    };
    checked.map_err(|err| {
        refused(format!(
            "the signature made by key {issuer} does not match: {err}"
        ))
    })?;

    Ok(Signer {
        key_id: issuer,
        primary_fingerprint,
    })
}

async fn get(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
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

fn hex(id: impl AsRef<[u8]>) -> String {
    id.as_ref().iter().map(|b| format!("{b:02X}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{Reply, client, serve};
    use pgp::composed::{
        ArmorOptions, KeyType, SecretKeyParamsBuilder, SignedSecretSubKey, SubkeyParamsBuilder,
    };
    use pgp::crypto::hash::HashAlgorithm;
    use pgp::types::Password;
    use std::collections::HashMap;
    use tempfile::{TempDir, tempdir};

    const IMAGE: &str = "DietPi_RPi5-ARMv8-Trixie.img.xz";
    const KEY: &str = "MichaIng.gpg";

    /// A signing key shaped as the publisher's is: a certifying primary with a signing subkey, which makes the
    /// signatures. Ed25519 keeps generation fast; the shape is what the verification depends on.
    struct Publisher {
        cert: SignedPublicKey,
        subkey: SignedSecretSubKey,
        primary_fingerprint: String,
    }

    impl Publisher {
        fn new() -> Self {
            let secret = SecretKeyParamsBuilder::default()
                .key_type(KeyType::Ed25519)
                .can_certify(true)
                .can_sign(true)
                .primary_user_id("MichaIng <micha@dietpi.com>".into())
                .passphrase(None)
                .subkey(
                    SubkeyParamsBuilder::default()
                        .key_type(KeyType::Ed25519)
                        .can_sign(true)
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap()
                .generate(rand::thread_rng())
                .unwrap();
            let subkey = secret.secret_subkeys[0].clone();
            let cert = SignedPublicKey::from(secret);
            let primary_fingerprint = format!("{:X}", cert.primary_key.fingerprint());
            Self {
                cert,
                subkey,
                primary_fingerprint,
            }
        }

        fn key_id(&self) -> String {
            hex(self.subkey.key.legacy_key_id())
        }

        fn armored_key(&self) -> Vec<u8> {
            self.cert
                .to_armored_string(ArmorOptions::default())
                .unwrap()
                .into_bytes()
        }

        fn sign(&self, image: &[u8]) -> Vec<u8> {
            DetachedSignature::sign_binary_data(
                rand::thread_rng(),
                &self.subkey.key,
                &Password::empty(),
                HashAlgorithm::Sha256,
                image,
            )
            .unwrap()
            .to_armored_string(ArmorOptions::default())
            .unwrap()
            .into_bytes()
        }
    }

    /// The image as the operator's machine holds it: a stand-in for the compressed bytes the signature covers.
    fn image_bytes() -> Vec<u8> {
        b"compressed image bytes ".repeat(20_000)
    }

    fn path(name: &str) -> String {
        format!("/images/{name}")
    }

    fn routes(key: Reply, signature: Reply) -> HashMap<String, Reply> {
        HashMap::from([
            (path(KEY), key),
            (path(&format!("{IMAGE}{SIGNATURE_SUFFIX}")), signature),
        ])
    }

    /// Writes the image the operator's machine holds, returning its path within the directory.
    fn write_image(dir: &TempDir, bytes: &[u8]) -> PathBuf {
        let image = dir.path().join(IMAGE);
        std::fs::write(&image, bytes).unwrap();
        image
    }

    /// Task 5.1: the key is named at the publisher's GitHub account over HTTPS.
    #[test]
    fn the_signing_key_is_named_where_the_publisher_publishes_it() {
        assert_eq!(SIGNING_KEY_URL, "https://github.com/MichaIng.gpg");
    }

    /// Task 5.1: the key and the detached signature are obtained, and the key is the publisher's.
    #[tokio::test]
    async fn the_key_and_signature_are_obtained_in_memory() {
        let publisher = Publisher::new();
        let image = image_bytes();
        let base = serve(routes(
            Reply::Body(publisher.armored_key()),
            Reply::Body(publisher.sign(&image)),
        ));

        let key = retrieve_key(&client(), &format!("{base}{KEY}"))
            .await
            .unwrap();
        let armored = retrieve_signature(&client(), &format!("{base}{IMAGE}{SIGNATURE_SUFFIX}"))
            .await
            .unwrap();

        assert_eq!(
            format!("{:X}", key.primary_key.fingerprint()),
            publisher.primary_fingerprint
        );
        assert!(armored.starts_with("-----BEGIN PGP SIGNATURE-----"));
    }

    /// Task 5.2: a good signature reports the key that made it, found by its key ID among the subkeys.
    #[tokio::test]
    async fn a_good_signature_reports_the_key_that_made_it() {
        let publisher = Publisher::new();
        let image = image_bytes();
        let base = serve(routes(
            Reply::Body(publisher.armored_key()),
            Reply::Body(publisher.sign(&image)),
        ));
        let dir = tempdir().unwrap();
        let image_path = write_image(&dir, &image);

        let check = verify_download(
            &client(),
            &format!("{base}{IMAGE}"),
            &format!("{base}{KEY}"),
            &image_path,
        )
        .await
        .unwrap();

        assert_eq!(
            check,
            SignatureCheck::Verified {
                key_id: publisher.key_id(),
                primary_fingerprint: publisher.primary_fingerprint.clone(),
                key_source: format!("{base}{KEY}"),
            }
        );
        assert!(image_path.exists());
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from(IMAGE)]);
    }

    /// Task 5.2 and 5.4: a tampered image fails, halts the run with the reason, and is left nowhere.
    #[tokio::test]
    async fn a_tampered_image_halts_with_the_reason_and_is_removed() {
        let publisher = Publisher::new();
        let image = image_bytes();
        let base = serve(routes(
            Reply::Body(publisher.armored_key()),
            Reply::Body(publisher.sign(&image)),
        ));
        let dir = tempdir().unwrap();
        let mut tampered = image.clone();
        tampered[1000] ^= 0x01;
        let image_path = write_image(&dir, &tampered);

        let err = verify_download(
            &client(),
            &format!("{base}{IMAGE}"),
            &format!("{base}{KEY}"),
            &image_path,
        )
        .await
        .unwrap_err();

        let message = err.to_string();
        assert!(message.contains(IMAGE), "{message}");
        assert!(message.contains("does not verify"), "{message}");
        assert!(!image_path.exists());
    }

    /// Task 5.2: a signature made by a key the publisher's key does not contain fails, naming that key.
    #[tokio::test]
    async fn a_signature_from_an_unrelated_key_fails_naming_the_key() {
        let publisher = Publisher::new();
        let unrelated = Publisher::new();
        let image = image_bytes();
        let base = serve(routes(
            Reply::Body(publisher.armored_key()),
            Reply::Body(unrelated.sign(&image)),
        ));
        let dir = tempdir().unwrap();
        let image_path = write_image(&dir, &image);

        let err = verify_download(
            &client(),
            &format!("{base}{IMAGE}"),
            &format!("{base}{KEY}"),
            &image_path,
        )
        .await
        .unwrap_err();

        let message = err.to_string();
        assert!(message.contains(&unrelated.key_id()), "{message}");
        assert!(!image_path.exists());
    }

    /// Task 5.3: a signing key that cannot be retrieved is unverified with its reason, and the image is kept.
    #[tokio::test]
    async fn an_unretrievable_key_is_unverified_with_its_reason() {
        let publisher = Publisher::new();
        let image = image_bytes();
        let base = serve(routes(
            Reply::Status(404),
            Reply::Body(publisher.sign(&image)),
        ));
        let dir = tempdir().unwrap();
        let image_path = write_image(&dir, &image);

        let check = verify_download(
            &client(),
            &format!("{base}{IMAGE}"),
            &format!("{base}{KEY}"),
            &image_path,
        )
        .await
        .unwrap();

        match check {
            SignatureCheck::Unverified { reason, .. } => {
                assert!(reason.contains("404"), "reason {reason:?} keeps the status");
            }
            other => panic!("expected unverified, got {other:?}"),
        }
        assert!(image_path.exists());
    }

    /// Task 5.3: a signing key that never answers is unverified, not a bad signature.
    #[tokio::test]
    async fn an_unreachable_key_is_unverified_not_a_bad_signature() {
        let publisher = Publisher::new();
        let image = image_bytes();
        let base = serve(routes(Reply::Silent, Reply::Body(publisher.sign(&image))));
        let dir = tempdir().unwrap();
        let image_path = write_image(&dir, &image);

        let check = verify_download(
            &client(),
            &format!("{base}{IMAGE}"),
            &format!("{base}{KEY}"),
            &image_path,
        )
        .await
        .unwrap();

        match check {
            SignatureCheck::Unverified { reason, .. } => assert!(!reason.is_empty()),
            other => panic!("expected unverified, got {other:?}"),
        }
        assert!(image_path.exists());
    }

    /// Task 5.3: a key that is served but cannot be read is unverified, not a bad signature.
    #[tokio::test]
    async fn an_unreadable_key_is_unverified_not_a_bad_signature() {
        let publisher = Publisher::new();
        let image = image_bytes();
        let base = serve(routes(
            Reply::Body(b"this is not a key\n".to_vec()),
            Reply::Body(publisher.sign(&image)),
        ));
        let dir = tempdir().unwrap();
        let image_path = write_image(&dir, &image);

        let check = verify_download(
            &client(),
            &format!("{base}{IMAGE}"),
            &format!("{base}{KEY}"),
            &image_path,
        )
        .await
        .unwrap();

        match check {
            SignatureCheck::Unverified { reason, .. } => {
                assert!(reason.contains("could not be read"), "{reason}");
            }
            other => panic!("expected unverified, got {other:?}"),
        }
    }

    /// Task 5.3: a signature that cannot be retrieved is unverified with its reason, and never verified.
    #[tokio::test]
    async fn an_absent_signature_is_unverified_with_its_reason() {
        let publisher = Publisher::new();
        let image = image_bytes();
        let base = serve(routes(
            Reply::Body(publisher.armored_key()),
            Reply::Status(404),
        ));
        let dir = tempdir().unwrap();
        let image_path = write_image(&dir, &image);

        let check = verify_download(
            &client(),
            &format!("{base}{IMAGE}"),
            &format!("{base}{KEY}"),
            &image_path,
        )
        .await
        .unwrap();

        match check {
            SignatureCheck::Unverified { reason, .. } => {
                assert!(
                    reason.contains("signature could not be retrieved"),
                    "{reason}"
                );
            }
            other => panic!("expected unverified, got {other:?}"),
        }
        assert!(image_path.exists());
    }
}
