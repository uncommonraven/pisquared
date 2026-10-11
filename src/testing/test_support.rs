//! Helpers the tests across the crate share: the image bytes they serve, their digests and compressed forms, and
//! the checksum file the publisher serves beside them.

use std::io::Write;

use liblzma::write::XzEncoder;
use sha2::{Digest, Sha256};

/// Three megabytes, larger than the decompression buffer and any single read, so transfers take several chunks.
pub(crate) fn payload() -> Vec<u8> {
    (0..3_000_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect()
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn xz(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = XzEncoder::new(Vec::new(), 6);
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// A checksum file in sha256sum's format, as the publisher serves it.
pub(crate) fn checksum_file(bytes: &[u8], name: &str) -> Vec<u8> {
    format!("{}  {name}\n", sha256_hex(bytes)).into_bytes()
}
