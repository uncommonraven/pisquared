//! Finds the first partition of a disk image by reading its partition table, without mounting the image or using a loop
//! device. Both MBR and GPT are read. Sectors are 512 bytes.

use std::io::{self, Read, Seek, SeekFrom};

use thiserror::Error;

pub const SECTOR_LEN: u64 = 512;

const SIGNATURE_OFFSET: usize = 510;
const MBR_SIGNATURE: [u8; 2] = [0x55, 0xAA];
const MBR_TABLE_OFFSET: usize = 446;
const MBR_ENTRY_LEN: usize = 16;
const MBR_SLOTS: usize = 4;
const PROTECTIVE_MBR: u8 = 0xEE;
const FAT_MBR_TYPES: [u8; 6] = [0x01, 0x04, 0x06, 0x0B, 0x0C, 0x0E];

const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
const GPT_MIN_ENTRY_LEN: u64 = 128;
const GPT_MAX_TABLE_LEN: u64 = 1 << 20;
const GPT_TYPE_LEN: usize = 16;
/// The EFI System Partition type, C12A7328-F81F-11D2-BA4B-00A0C93EC93B, as it is stored on disk.
const EFI_SYSTEM_TYPE: [u8; GPT_TYPE_LEN] = [
    0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B,
];

#[derive(Debug, Error)]
pub enum PartitionError {
    #[error("the image has no partition table")]
    NoPartitionTable,
    #[error("the partition table lists no partitions")]
    NoPartitions,
    #[error("the first partition is not a FAT partition")]
    NotFat,
    #[error("the partition table could not be read")]
    Read(#[from] io::Error),
}

/// Where a partition sits on the disk, in whole sectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Partition {
    pub first_sector: u64,
    pub sector_count: u64,
}

impl Partition {
    /// The byte range the partition occupies, as the start and the end (exclusive).
    pub fn byte_range(self) -> (u64, u64) {
        let start = self.first_sector * SECTOR_LEN;
        (start, start + self.sector_count * SECTOR_LEN)
    }
}

struct Found {
    partition: Partition,
    fat: bool,
}

/// The first partition of the disk, which must be a FAT partition.
///
/// For MBR, the first partition is the entry in slot 1 when that slot is in use, otherwise the used entry with the lowest
/// starting sector. For GPT, it is the used entry with the lowest starting LBA. A protective MBR means GPT.
pub fn first_fat_partition<D: Read + Seek>(disk: &mut D) -> Result<Partition, PartitionError> {
    let found = first_partition(disk)?;
    if found.fat {
        Ok(found.partition)
    } else {
        Err(PartitionError::NotFat)
    }
}

fn first_partition<D: Read + Seek>(disk: &mut D) -> Result<Found, PartitionError> {
    let mbr = read_sector(disk, 0)?;
    if mbr[SIGNATURE_OFFSET..] != MBR_SIGNATURE {
        return Err(PartitionError::NoPartitionTable);
    }

    let slots: Vec<MbrSlot> = (0..MBR_SLOTS)
        .map(|index| MbrSlot::read(&mbr, index))
        .collect();
    if slots.iter().any(|slot| slot.kind == PROTECTIVE_MBR) {
        return first_gpt_partition(disk);
    }

    let first = if slots[0].kind != 0 {
        slots[0]
    } else {
        slots
            .iter()
            .copied()
            .filter(|slot| slot.kind != 0)
            .min_by_key(|slot| slot.first_sector)
            .ok_or(PartitionError::NoPartitions)?
    };
    Ok(Found {
        partition: Partition {
            first_sector: first.first_sector,
            sector_count: first.sector_count,
        },
        fat: FAT_MBR_TYPES.contains(&first.kind),
    })
}

fn first_gpt_partition<D: Read + Seek>(disk: &mut D) -> Result<Found, PartitionError> {
    let header = read_sector(disk, 1)?;
    if &header[..GPT_SIGNATURE.len()] != GPT_SIGNATURE {
        return Err(PartitionError::NoPartitionTable);
    }
    let table_sector = u64_at(&header, 72);
    let count = u64::from(u32_at(&header, 80));
    let entry_len = u64::from(u32_at(&header, 84));
    if entry_len < GPT_MIN_ENTRY_LEN || count == 0 || count * entry_len > GPT_MAX_TABLE_LEN {
        return Err(PartitionError::NoPartitionTable);
    }

    let table = read_bytes(
        disk,
        table_sector * SECTOR_LEN,
        (count * entry_len) as usize,
    )?;
    let first = table
        .chunks_exact(entry_len as usize)
        .filter(|entry| entry[..GPT_TYPE_LEN].iter().any(|&byte| byte != 0))
        .min_by_key(|entry| u64_at(entry, 32))
        .ok_or(PartitionError::NoPartitions)?;

    let first_sector = u64_at(first, 32);
    let last_sector = u64_at(first, 40);
    if last_sector < first_sector {
        return Err(PartitionError::NoPartitionTable);
    }
    Ok(Found {
        partition: Partition {
            first_sector,
            sector_count: last_sector - first_sector + 1,
        },
        fat: first[..GPT_TYPE_LEN] == EFI_SYSTEM_TYPE,
    })
}

#[derive(Debug, Clone, Copy)]
struct MbrSlot {
    kind: u8,
    first_sector: u64,
    sector_count: u64,
}

impl MbrSlot {
    fn read(mbr: &[u8; 512], index: usize) -> Self {
        let at = MBR_TABLE_OFFSET + index * MBR_ENTRY_LEN;
        Self {
            kind: mbr[at + 4],
            first_sector: u64::from(u32_at(mbr, at + 8)),
            sector_count: u64::from(u32_at(mbr, at + 12)),
        }
    }
}

fn read_sector<D: Read + Seek>(disk: &mut D, sector: u64) -> io::Result<[u8; 512]> {
    let mut buffer = [0; 512];
    disk.seek(SeekFrom::Start(sector * SECTOR_LEN))?;
    disk.read_exact(&mut buffer)?;
    Ok(buffer)
}

fn read_bytes<D: Read + Seek>(disk: &mut D, offset: u64, len: usize) -> io::Result<Vec<u8>> {
    let mut buffer = vec![0; len];
    disk.seek(SeekFrom::Start(offset))?;
    disk.read_exact(&mut buffer)?;
    Ok(buffer)
}

fn u32_at(buffer: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(buffer[at..at + 4].try_into().unwrap())
}

fn u64_at(buffer: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(buffer[at..at + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    const DISK_LEN: usize = 4 * 1024 * 1024;
    const FAT: u8 = 0x0C;
    const LINUX: u8 = 0x83;

    fn blank_disk() -> Cursor<Vec<u8>> {
        Cursor::new(vec![0; DISK_LEN])
    }

    fn put(disk: &mut Cursor<Vec<u8>>, at: usize, bytes: &[u8]) {
        disk.get_mut()[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// An MBR disk whose table has the given `(slot, type, first sector, sector count)` entries.
    fn mbr_disk(entries: &[(usize, u8, u32, u32)]) -> Cursor<Vec<u8>> {
        let mut disk = blank_disk();
        put(&mut disk, SIGNATURE_OFFSET, &MBR_SIGNATURE);
        for &(slot, kind, first, count) in entries {
            let at = MBR_TABLE_OFFSET + slot * MBR_ENTRY_LEN;
            put(&mut disk, at + 4, &[kind]);
            put(&mut disk, at + 8, &first.to_le_bytes());
            put(&mut disk, at + 12, &count.to_le_bytes());
        }
        disk
    }

    /// A GPT disk with a protective MBR and one entry per `(type, first LBA, last LBA)`.
    fn gpt_disk(entries: &[([u8; 16], u64, u64)]) -> Cursor<Vec<u8>> {
        let mut disk = blank_disk();
        put(&mut disk, MBR_TABLE_OFFSET + 4, &[PROTECTIVE_MBR]);
        put(&mut disk, MBR_TABLE_OFFSET + 8, &1u32.to_le_bytes());
        put(&mut disk, SIGNATURE_OFFSET, &MBR_SIGNATURE);

        let header = SECTOR_LEN as usize;
        put(&mut disk, header, GPT_SIGNATURE);
        put(&mut disk, header + 72, &2u64.to_le_bytes());
        put(&mut disk, header + 80, &128u32.to_le_bytes());
        put(&mut disk, header + 84, &128u32.to_le_bytes());

        for (index, (kind, first, last)) in entries.iter().enumerate() {
            let at = 2 * SECTOR_LEN as usize + index * 128;
            put(&mut disk, at, kind);
            put(&mut disk, at + 32, &first.to_le_bytes());
            put(&mut disk, at + 40, &last.to_le_bytes());
        }
        disk
    }

    #[test]
    fn mbr_slot_one_is_the_first_partition_when_it_is_used() {
        let mut disk = mbr_disk(&[(0, LINUX, 2048, 100), (1, FAT, 4096, 200)]);
        assert!(matches!(
            first_fat_partition(&mut disk),
            Err(PartitionError::NotFat)
        ));

        let mut disk = mbr_disk(&[(0, FAT, 2048, 100), (1, LINUX, 4096, 200)]);
        assert_eq!(
            first_fat_partition(&mut disk).unwrap(),
            Partition {
                first_sector: 2048,
                sector_count: 100
            }
        );
    }

    #[test]
    fn mbr_with_slot_one_empty_uses_the_lowest_starting_sector() {
        let mut disk = mbr_disk(&[(2, FAT, 8192, 100), (3, LINUX, 4096, 200)]);
        assert!(matches!(
            first_fat_partition(&mut disk),
            Err(PartitionError::NotFat)
        ));

        let mut disk = mbr_disk(&[(2, LINUX, 8192, 100), (3, FAT, 4096, 200)]);
        assert_eq!(
            first_fat_partition(&mut disk).unwrap(),
            Partition {
                first_sector: 4096,
                sector_count: 200
            }
        );
    }

    #[test]
    fn every_mbr_fat_type_is_accepted() {
        for kind in FAT_MBR_TYPES {
            let mut disk = mbr_disk(&[(0, kind, 2048, 100)]);
            assert!(first_fat_partition(&mut disk).is_ok(), "type {kind:#x}");
        }
    }

    #[test]
    fn gpt_first_partition_is_the_used_entry_with_the_lowest_lba() {
        let other = [0x11; GPT_TYPE_LEN];
        let mut disk = gpt_disk(&[(EFI_SYSTEM_TYPE, 2048, 4095), (other, 1000, 1999)]);
        assert!(matches!(
            first_fat_partition(&mut disk),
            Err(PartitionError::NotFat)
        ));

        let mut disk = gpt_disk(&[(other, 4096, 8191), (EFI_SYSTEM_TYPE, 2048, 4095)]);
        assert_eq!(
            first_fat_partition(&mut disk).unwrap(),
            Partition {
                first_sector: 2048,
                sector_count: 2048
            }
        );
    }

    #[test]
    fn a_disk_without_a_signature_has_no_partition_table() {
        let mut disk = blank_disk();
        assert!(matches!(
            first_fat_partition(&mut disk),
            Err(PartitionError::NoPartitionTable)
        ));
    }

    #[test]
    fn a_signed_table_with_no_used_entries_has_no_partitions() {
        let mut disk = mbr_disk(&[]);
        assert!(matches!(
            first_fat_partition(&mut disk),
            Err(PartitionError::NoPartitions)
        ));
    }

    #[test]
    fn a_disk_too_short_for_its_table_is_a_read_failure() {
        let mut disk = Cursor::new(vec![0; 100]);
        assert!(matches!(
            first_fat_partition(&mut disk),
            Err(PartitionError::Read(_))
        ));
    }
}
