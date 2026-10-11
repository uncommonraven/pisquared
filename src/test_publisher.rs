//! A signing key shaped as the publisher's is, for tests that need an image signed the way the publisher signs.

use pgp::composed::{
    ArmorOptions, DetachedSignature, KeyType, SecretKeyParamsBuilder, SignedPublicKey,
    SignedSecretSubKey, SubkeyParamsBuilder,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::types::{KeyDetails, Password};

/// A certifying primary with a signing subkey, which makes the signatures. Ed25519 keeps generation fast; the shape
/// is what the verification depends on.
pub(crate) struct Publisher {
    cert: SignedPublicKey,
    subkey: SignedSecretSubKey,
    pub(crate) primary_fingerprint: String,
}

impl Publisher {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn key_id(&self) -> String {
        self.subkey
            .key
            .legacy_key_id()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect()
    }

    pub(crate) fn armored_key(&self) -> Vec<u8> {
        self.cert
            .to_armored_string(ArmorOptions::default())
            .unwrap()
            .into_bytes()
    }

    pub(crate) fn sign(&self, image: &[u8]) -> Vec<u8> {
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
