//! Helpers the tests across the crate share: the image bytes they serve, their digests and compressed forms, the
//! checksum file the publisher serves beside them, and a disk image with a FAT boot partition.

use std::io::{Cursor, Read, Write};

use fatfs::{FileSystem, FormatVolumeOptions, FsOptions};
use fscommon::StreamSlice;
use liblzma::write::XzEncoder;
use sha2::{Digest, Sha256};

const SECTOR: u64 = 512;
const BOOT_FIRST_SECTOR: u64 = 2048;
const BOOT_SECTORS: u64 = 32 * 1024;
const FAT_MBR_TYPE: u8 = 0x0C;

/// Three megabytes, larger than the decompression buffer and any single read, so transfers take several chunks.
pub(crate) fn payload() -> Vec<u8> {
    (0..3_000_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect()
}

/// A disk image whose first MBR partition is a FAT volume holding `files` at its root, as a DietPi image's boot
/// partition is laid out.
pub(crate) fn boot_image(files: &[(&str, &str)]) -> Vec<u8> {
    let mut disk = vec![0u8; ((BOOT_FIRST_SECTOR + BOOT_SECTORS) * SECTOR) as usize];
    let entry = 446;
    disk[entry + 4] = FAT_MBR_TYPE;
    disk[entry + 8..entry + 12].copy_from_slice(&(BOOT_FIRST_SECTOR as u32).to_le_bytes());
    disk[entry + 12..entry + 16].copy_from_slice(&(BOOT_SECTORS as u32).to_le_bytes());
    disk[510..512].copy_from_slice(&[0x55, 0xAA]);

    let mut cursor = Cursor::new(disk);
    let (start, end) = boot_range();
    {
        let mut slice = StreamSlice::new(&mut cursor, start, end).unwrap();
        fatfs::format_volume(&mut slice, FormatVolumeOptions::new()).unwrap();
        let fs = FileSystem::new(&mut slice, FsOptions::new()).unwrap();
        for (name, text) in files {
            let mut file = fs.root_dir().create_file(name).unwrap();
            file.write_all(text.as_bytes()).unwrap();
        }
        fs.unmount().unwrap();
    }
    cursor.into_inner()
}

/// The text of a file at the root of the boot partition of an image made by [`boot_image`].
pub(crate) fn boot_file(image: &[u8], name: &str) -> String {
    let mut cursor = Cursor::new(image.to_vec());
    let (start, end) = boot_range();
    let mut slice = StreamSlice::new(&mut cursor, start, end).unwrap();
    let fs = FileSystem::new(&mut slice, FsOptions::new()).unwrap();
    let mut text = String::new();
    fs.root_dir()
        .open_file(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

fn boot_range() -> (u64, u64) {
    let start = BOOT_FIRST_SECTOR * SECTOR;
    (start, start + BOOT_SECTORS * SECTOR)
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
