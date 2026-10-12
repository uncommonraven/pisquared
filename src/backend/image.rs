//! Edits files on the boot partition of a decompressed image in place, without mounting it.
//!
//! The partition is presented as its own stream with `fscommon::StreamSlice`, and `fatfs` reads and writes the FAT
//! filesystem inside it.

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Seek, SeekFrom, Write};

    use fatfs::{FileSystem, FormatVolumeOptions, FsOptions};
    use fscommon::StreamSlice;

    const PARTITION_START: u64 = 2048 * 512;
    const PARTITION_LEN: u64 = 16 * 1024 * 1024;

    fn blank_image() -> Cursor<Vec<u8>> {
        Cursor::new(vec![0; (PARTITION_START + PARTITION_LEN) as usize])
    }

    #[test]
    fn rewritten_file_round_trips_inside_a_partition_slice() {
        let mut image = blank_image();
        {
            let mut partition = StreamSlice::new(&mut image, PARTITION_START, PARTITION_START + PARTITION_LEN).unwrap();
            fatfs::format_volume(&mut partition, FormatVolumeOptions::new()).unwrap();
            let fs = FileSystem::new(&mut partition, FsOptions::new()).unwrap();
            let mut file = fs.root_dir().create_file("dietpi.txt").unwrap();
            file.write_all(b"AUTO_SETUP_AUTOMATED=0\nAUTO_SETUP_NET_HOSTNAME=DietPi\n").unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(b"AUTO_SETUP_AUTOMATED=1\n").unwrap();
            file.truncate().unwrap();
            drop(file);
            fs.unmount().unwrap();
        }

        let mut partition = StreamSlice::new(&mut image, PARTITION_START, PARTITION_START + PARTITION_LEN).unwrap();
        let fs = FileSystem::new(&mut partition, FsOptions::new()).unwrap();
        let mut contents = String::new();
        fs.root_dir().open_file("dietpi.txt").unwrap().read_to_string(&mut contents).unwrap();
        assert_eq!(contents, "AUTO_SETUP_AUTOMATED=1\n");
    }
}
