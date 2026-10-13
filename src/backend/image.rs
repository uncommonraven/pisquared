//! Edits files on the boot partition of a decompressed image in place, without mounting it.
//!
//! The partition is presented as its own stream with `fscommon::StreamSlice`, and `fatfs` reads and writes the FAT
//! filesystem inside it.

use std::fs::OpenOptions;
use std::io::{self, Read, Seek, Write};
use std::path::Path;

use fatfs::{Dir, File, FileSystem, FsOptions, ReadWriteSeek};
use fscommon::StreamSlice;
use thiserror::Error;

use super::configure::{
    self, CMDLINE, CREDENTIALS, ConfigureError, DIETPI, DIETPI_WIFI, Editable, SCRIPT, Settings,
};
use super::partition::{self, PartitionError};

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("the image could not be opened")]
    Open(#[source] io::Error),
    #[error(transparent)]
    Partition(#[from] PartitionError),
    #[error("the boot partition could not be opened or saved")]
    Volume(#[source] io::Error),
    #[error(transparent)]
    Configure(#[from] ConfigureError),
}

/// Sets `settings` in the boot partition of the decompressed image at `image`, in place.
pub fn configure_image(image: &Path, settings: &Settings) -> Result<(), ImageError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(image)
        .map_err(ImageError::Open)?;
    configure_disk(&mut file, settings)
}

/// Sets `settings` in the first partition of a disk, which must be a FAT partition. The new files are created here,
/// after the files that must already exist are open, and all of them are removed again if the configuration fails.
pub fn configure_disk<D: Read + Write + Seek>(
    disk: &mut D,
    settings: &Settings,
) -> Result<(), ImageError> {
    let (start, end) = partition::first_fat_partition(disk)?.byte_range();
    let mut slice = StreamSlice::new(disk, start, end).map_err(ImageError::Volume)?;
    let fs = FileSystem::new(&mut slice, FsOptions::new()).map_err(ImageError::Volume)?;
    {
        let root = fs.root_dir();
        let mut dietpi = open(&root, DIETPI)?;
        let mut dietpi_wifi = open(&root, DIETPI_WIFI)?;
        let mut cmdline = open(&root, CMDLINE)?;
        let mut credentials = create(&root, CREDENTIALS)?;
        let mut script = match create(&root, SCRIPT) {
            Ok(script) => script,
            Err(err) => {
                drop(credentials);
                let _ = root.remove(CREDENTIALS);
                return Err(err.into());
            }
        };
        let result = configure::configure_files(
            &mut dietpi,
            &mut dietpi_wifi,
            &mut cmdline,
            &mut credentials,
            &mut script,
            settings,
        );
        drop(credentials);
        drop(script);
        if result.is_err() {
            let _ = root.remove(CREDENTIALS);
            let _ = root.remove(SCRIPT);
        }
        result?;
    }
    fs.unmount().map_err(ImageError::Volume)
}

fn open<'a, T: ReadWriteSeek>(
    root: &Dir<'a, T>,
    name: &'static str,
) -> Result<File<'a, T>, ConfigureError> {
    root.open_file(name)
        .map_err(|source| ConfigureError::Io { file: name, source })
}

fn create<'a, T: ReadWriteSeek>(
    root: &Dir<'a, T>,
    name: &'static str,
) -> Result<File<'a, T>, ConfigureError> {
    root.create_file(name)
        .map_err(|source| ConfigureError::Io { file: name, source })
}

impl<T: ReadWriteSeek> Editable for File<'_, T> {
    fn cut_at_position(&mut self) -> io::Result<()> {
        self.truncate()
    }

    fn save(&mut self) -> io::Result<()> {
        self.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Cursor, Read, Write};

    use fatfs::{FileSystem, FormatVolumeOptions, FsOptions};
    use fscommon::StreamSlice;
    use tempfile::tempdir;

    use super::*;
    use crate::backend::network::{Connection, Wifi};

    const SECTOR: u64 = 512;
    const FAT_FIRST_SECTOR: u64 = 2048;
    const FAT_SECTORS: u64 = 32 * 1024;
    const FAT_TYPE: u8 = 0x0C;
    const LINUX_TYPE: u8 = 0x83;
    const DISK_LEN: usize = ((FAT_FIRST_SECTOR + FAT_SECTORS) * SECTOR) as usize;

    const DIETPI_TXT: &str = "AUTO_SETUP_NET_ETHERNET_ENABLED=1\nAUTO_SETUP_NET_WIFI_ENABLED=0\nAUTO_SETUP_NET_WIFI_COUNTRY_CODE=GB\n";
    const DIETPI_WIFI_TXT: &str = "aWIFI_SSID[0]=''\naWIFI_KEY[0]=''\naWIFI_KEYMGR[0]='WPA-PSK'\n";
    const CMDLINE_TXT: &str = "root=PARTUUID=d6e7fee7-02 rootfstype=ext4 rootwait fsck.repair=yes net.ifnames=0 logo.nologo console=serial0,115200 console=tty1\n";
    const CMDLINE_CONFIGURED: &str = "root=PARTUUID=d6e7fee7-02 rootfstype=ext4 rootwait fsck.repair=yes net.ifnames=0 logo.nologo console=serial0,115200 console=tty1 lsm=landlock,lockdown,yama,integrity,apparmor\n";

    fn home() -> Wifi {
        Wifi {
            ssid: "Home Network".to_owned(),
            passphrase: "it's a test".to_owned(),
            country_code: "GB".to_owned(),
        }
    }

    fn settings() -> Settings {
        Settings {
            connection: Connection::Wifi(home()),
            hostname: "mypi".to_owned(),
            password: "a-long-password".to_owned(),
            mesh_network_auth_key: "tskey-auth-kABC123-xyz789".to_owned(),
        }
    }

    fn put(disk: &mut Cursor<Vec<u8>>, at: usize, bytes: &[u8]) {
        disk.get_mut()[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A disk with a FAT partition at the usual place, holding the two config files, and the given MBR slot-1 type.
    fn fat_disk(kind: u8, files: &[(&str, &str)]) -> Cursor<Vec<u8>> {
        let mut disk = Cursor::new(vec![0; DISK_LEN]);
        let mut mbr = [0; 16];
        mbr[4] = kind;
        mbr[8..12].copy_from_slice(&(FAT_FIRST_SECTOR as u32).to_le_bytes());
        mbr[12..16].copy_from_slice(&(FAT_SECTORS as u32).to_le_bytes());
        put(&mut disk, 446, &mbr);
        put(&mut disk, 510, &[0x55, 0xAA]);
        format_fat(&mut disk, files);
        disk
    }

    /// A disk with a protective MBR and a GPT whose first entry is the EFI System Partition with the FAT volume.
    fn gpt_disk(files: &[(&str, &str)]) -> Cursor<Vec<u8>> {
        let mut disk = Cursor::new(vec![0; DISK_LEN]);
        let mut mbr = [0; 16];
        mbr[4] = 0xEE;
        mbr[8..12].copy_from_slice(&1u32.to_le_bytes());
        put(&mut disk, 446, &mbr);
        put(&mut disk, 510, &[0x55, 0xAA]);

        let header = SECTOR as usize;
        put(&mut disk, header, b"EFI PART");
        put(&mut disk, header + 72, &2u64.to_le_bytes());
        put(&mut disk, header + 80, &128u32.to_le_bytes());
        put(&mut disk, header + 84, &128u32.to_le_bytes());

        let entry = 2 * SECTOR as usize;
        put(
            &mut disk,
            entry,
            &[
                0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E,
                0xC9, 0x3B,
            ],
        );
        put(&mut disk, entry + 32, &FAT_FIRST_SECTOR.to_le_bytes());
        put(
            &mut disk,
            entry + 40,
            &(FAT_FIRST_SECTOR + FAT_SECTORS - 1).to_le_bytes(),
        );

        format_fat(&mut disk, files);
        disk
    }

    fn format_fat(disk: &mut Cursor<Vec<u8>>, files: &[(&str, &str)]) {
        let start = FAT_FIRST_SECTOR * SECTOR;
        let end = start + FAT_SECTORS * SECTOR;
        let mut slice = StreamSlice::new(disk, start, end).unwrap();
        fatfs::format_volume(&mut slice, FormatVolumeOptions::new()).unwrap();
        let fs = FileSystem::new(&mut slice, FsOptions::new()).unwrap();
        for (name, text) in files {
            let mut file = fs.root_dir().create_file(name).unwrap();
            file.write_all(text.as_bytes()).unwrap();
        }
        fs.unmount().unwrap();
    }

    fn read_file(disk: &mut Cursor<Vec<u8>>, name: &str) -> String {
        let start = FAT_FIRST_SECTOR * SECTOR;
        let end = start + FAT_SECTORS * SECTOR;
        let mut slice = StreamSlice::new(disk, start, end).unwrap();
        let fs = FileSystem::new(&mut slice, FsOptions::new()).unwrap();
        let mut text = String::new();
        fs.root_dir()
            .open_file(name)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    fn file_exists(disk: &mut Cursor<Vec<u8>>, name: &str) -> bool {
        let start = FAT_FIRST_SECTOR * SECTOR;
        let end = start + FAT_SECTORS * SECTOR;
        let mut slice = StreamSlice::new(disk, start, end).unwrap();
        let fs = FileSystem::new(&mut slice, FsOptions::new()).unwrap();
        fs.root_dir().open_file(name).is_ok()
    }

    fn standard_files() -> [(&'static str, &'static str); 3] {
        [
            ("dietpi.txt", DIETPI_TXT),
            ("dietpi-wifi.txt", DIETPI_WIFI_TXT),
            ("cmdline.txt", CMDLINE_TXT),
        ]
    }

    fn assert_configured(disk: &mut Cursor<Vec<u8>>) {
        assert_eq!(
            read_file(disk, "dietpi.txt"),
            "AUTO_SETUP_NET_ETHERNET_ENABLED=0\nAUTO_SETUP_NET_WIFI_ENABLED=1\nAUTO_SETUP_NET_WIFI_COUNTRY_CODE=GB\nAUTO_SETUP_NET_HOSTNAME=mypi\nAUTO_SETUP_GLOBAL_PASSWORD=a-long-password\nAUTO_SETUP_SSH_SERVER_INDEX=-2\nAUTO_SETUP_INSTALL_SOFTWARE_ID=9 58\nAUTO_SETUP_CUSTOM_SCRIPT_EXEC=0\nAUTO_SETUP_AUTOMATED=1\n"
        );
        assert_eq!(
            read_file(disk, "dietpi-wifi.txt"),
            "aWIFI_SSID[0]='Home Network'\naWIFI_KEY[0]='it'\\''s a test'\naWIFI_KEYMGR[0]='WPA-PSK'\n"
        );
        assert_eq!(read_file(disk, "cmdline.txt"), CMDLINE_CONFIGURED);
        assert_eq!(
            read_file(disk, "user-defined-credentials.txt"),
            "MESH_NETWORK_AUTH_KEY=tskey-auth-kABC123-xyz789\n"
        );
        assert_eq!(
            read_file(disk, "Automation_Custom_Script.sh"),
            configure::AUTOMATION_SCRIPT
        );
    }

    #[test]
    fn an_mbr_image_has_its_boot_partition_configured_in_place() {
        let mut disk = fat_disk(FAT_TYPE, &standard_files());

        configure_disk(&mut disk, &settings()).unwrap();

        assert_configured(&mut disk);
    }

    #[test]
    fn a_gpt_image_has_its_boot_partition_configured_in_place() {
        let mut disk = gpt_disk(&standard_files());

        configure_disk(&mut disk, &settings()).unwrap();

        assert_configured(&mut disk);
    }

    #[test]
    fn a_non_fat_first_partition_is_refused_and_nothing_is_written() {
        let mut disk = fat_disk(LINUX_TYPE, &standard_files());
        let before = disk.get_ref().clone();

        assert!(matches!(
            configure_disk(&mut disk, &settings()),
            Err(ImageError::Partition(PartitionError::NotFat))
        ));
        assert_eq!(disk.get_ref(), &before);
    }

    #[test]
    fn a_missing_dietpi_wifi_fails_and_leaves_dietpi_untouched() {
        let mut disk = fat_disk(FAT_TYPE, &[("dietpi.txt", DIETPI_TXT)]);

        assert!(matches!(
            configure_disk(&mut disk, &settings()),
            Err(ImageError::Configure(ConfigureError::Io {
                file: "dietpi-wifi.txt",
                ..
            }))
        ));
        assert_eq!(read_file(&mut disk, "dietpi.txt"), DIETPI_TXT);
    }

    #[test]
    fn an_ethernet_image_leaves_dietpi_wifi_untouched() {
        let mut disk = fat_disk(FAT_TYPE, &standard_files());
        let ethernet = Settings {
            connection: Connection::Ethernet,
            ..settings()
        };

        configure_disk(&mut disk, &ethernet).unwrap();

        assert_eq!(read_file(&mut disk, "dietpi-wifi.txt"), DIETPI_WIFI_TXT);
        assert_eq!(
            read_file(&mut disk, "dietpi.txt"),
            "AUTO_SETUP_NET_ETHERNET_ENABLED=1\nAUTO_SETUP_NET_WIFI_ENABLED=0\nAUTO_SETUP_NET_WIFI_COUNTRY_CODE=GB\nAUTO_SETUP_NET_HOSTNAME=mypi\nAUTO_SETUP_GLOBAL_PASSWORD=a-long-password\nAUTO_SETUP_SSH_SERVER_INDEX=-2\nAUTO_SETUP_INSTALL_SOFTWARE_ID=9 58\nAUTO_SETUP_CUSTOM_SCRIPT_EXEC=0\nAUTO_SETUP_AUTOMATED=1\n"
        );
        assert_eq!(read_file(&mut disk, "cmdline.txt"), CMDLINE_CONFIGURED);
    }

    #[test]
    fn a_missing_dietpi_txt_fails_without_creating_it() {
        let mut disk = fat_disk(FAT_TYPE, &[("dietpi-wifi.txt", DIETPI_WIFI_TXT)]);

        assert!(matches!(
            configure_disk(&mut disk, &settings()),
            Err(ImageError::Configure(ConfigureError::Io {
                file: "dietpi.txt",
                ..
            }))
        ));
        assert_eq!(read_file(&mut disk, "dietpi-wifi.txt"), DIETPI_WIFI_TXT);
        assert!(!file_exists(&mut disk, "user-defined-credentials.txt"));
        assert!(!file_exists(&mut disk, "Automation_Custom_Script.sh"));
    }

    #[test]
    fn a_key_that_cannot_be_written_leaves_the_files_unchanged_and_creates_nothing() {
        let mut disk = fat_disk(FAT_TYPE, &standard_files());
        let broken = Settings {
            mesh_network_auth_key: "tskey-abc\nAUTO_SETUP_AUTOMATED=0".to_owned(),
            ..settings()
        };

        assert!(matches!(
            configure_disk(&mut disk, &broken),
            Err(ImageError::Configure(ConfigureError::LineBreak {
                key: "MESH_NETWORK_AUTH_KEY"
            }))
        ));
        assert!(!file_exists(&mut disk, "user-defined-credentials.txt"));
        assert!(!file_exists(&mut disk, "Automation_Custom_Script.sh"));
        assert_eq!(read_file(&mut disk, "dietpi.txt"), DIETPI_TXT);
        assert_eq!(read_file(&mut disk, "cmdline.txt"), CMDLINE_TXT);
    }

    #[test]
    fn a_shorter_value_leaves_no_bytes_of_the_old_file_behind() {
        let long_dietpi = "AUTO_SETUP_NET_ETHERNET_ENABLED=1\nAUTO_SETUP_NET_WIFI_ENABLED=0\nAUTO_SETUP_NET_WIFI_COUNTRY_CODE=GBGBGBGBGBGBGBGBGBGBGBGBGBGBGBGBGBGBGBGBGBGBGBGB\n";
        let long_wifi = "aWIFI_SSID[0]='a very long network name that is replaced'\naWIFI_KEY[0]=''\naWIFI_KEYMGR[0]='WPA-PSK'\n";
        let mut disk = fat_disk(
            FAT_TYPE,
            &[
                ("dietpi.txt", long_dietpi),
                ("dietpi-wifi.txt", long_wifi),
                ("cmdline.txt", CMDLINE_TXT),
            ],
        );

        configure_disk(&mut disk, &settings()).unwrap();

        assert_configured(&mut disk);
    }

    #[test]
    fn a_missing_cmdline_fails_and_leaves_the_other_files_untouched() {
        let mut disk = fat_disk(
            FAT_TYPE,
            &[
                ("dietpi.txt", DIETPI_TXT),
                ("dietpi-wifi.txt", DIETPI_WIFI_TXT),
            ],
        );

        assert!(matches!(
            configure_disk(&mut disk, &settings()),
            Err(ImageError::Configure(ConfigureError::Io {
                file: "cmdline.txt",
                ..
            }))
        ));
        assert_eq!(read_file(&mut disk, "dietpi.txt"), DIETPI_TXT);
        assert_eq!(read_file(&mut disk, "dietpi-wifi.txt"), DIETPI_WIFI_TXT);
        assert!(!file_exists(&mut disk, "user-defined-credentials.txt"));
        assert!(!file_exists(&mut disk, "Automation_Custom_Script.sh"));
    }

    #[test]
    fn an_image_file_is_configured_in_place() {
        let dir = tempdir().unwrap();
        let image = dir.path().join("disk.img");
        fs::write(&image, fat_disk(FAT_TYPE, &standard_files()).into_inner()).unwrap();

        configure_image(&image, &settings()).unwrap();

        let mut disk = Cursor::new(fs::read(&image).unwrap());
        assert_configured(&mut disk);
    }

    #[test]
    fn an_image_path_that_does_not_exist_is_refused() {
        let dir = tempdir().unwrap();

        assert!(matches!(
            configure_image(&dir.path().join("missing.img"), &settings()),
            Err(ImageError::Open(_))
        ));
    }
}
