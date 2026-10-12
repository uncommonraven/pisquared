//! Writes the operator's choices into `dietpi.txt` and `dietpi-wifi.txt`, following IMG-2 to IMG-7 of the credentials
//! requirements, and the hostname and password of the Hostname page.
//!
//! The edits are split in two. The functions that compute new text work on strings and know nothing about where the text
//! came from. The functions that read and write a file keep the file open for the whole edit. The image step can reuse
//! the first kind on a file inside the boot partition.

use std::io::{self, Read, Seek, SeekFrom, Write};

use thiserror::Error;

use super::network::{Connection, Wifi};

pub const DIETPI: &str = "dietpi.txt";
pub const DIETPI_WIFI: &str = "dietpi-wifi.txt";

/// What the operator chose for the Network and Hostname pages, as written into the boot partition.
pub struct Settings {
    pub connection: Connection,
    pub hostname: String,
    pub password: String,
}

#[derive(Debug, Error)]
pub enum ConfigureError {
    #[error("{file} could not be read or written")]
    Io {
        file: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("the value for {key} contains a line break")]
    LineBreak { key: &'static str },
}

/// Sets `settings` in two files that are already open for editing, such as files on a FAT volume.
///
/// Both files must exist, but `dietpi-wifi.txt` is written only for Wi-Fi (IMG-7). Both new texts are computed before
/// either is written, so a value that cannot be written leaves both files as they were.
pub fn configure_files<F: Editable>(
    dietpi: &mut F,
    dietpi_wifi: &mut F,
    settings: &Settings,
) -> Result<(), ConfigureError> {
    let dietpi_text = dietpi_txt(&read_text(dietpi, DIETPI)?, settings)?;
    let dietpi_wifi_text = match &settings.connection {
        Connection::Wifi(wifi) => Some(dietpi_wifi_txt(
            &read_text(dietpi_wifi, DIETPI_WIFI)?,
            wifi,
        )?),
        Connection::Ethernet => None,
    };

    write_text(dietpi, DIETPI, &dietpi_text)?;
    match dietpi_wifi_text {
        Some(text) => write_text(dietpi_wifi, DIETPI_WIFI, &text),
        None => Ok(()),
    }
}

/// A file that can be edited where it is: read, written, cut short at the position it was last written to, and saved.
pub trait Editable: Read + Write + Seek {
    fn cut_at_position(&mut self) -> io::Result<()>;
    fn save(&mut self) -> io::Result<()>;
}

fn read_text<F: Editable>(file: &mut F, name: &'static str) -> Result<String, ConfigureError> {
    let io_error = |source| ConfigureError::Io { file: name, source };
    let mut text = String::new();
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    file.read_to_string(&mut text).map_err(io_error)?;
    Ok(text)
}

/// Writes `text` from the start of the file and cuts off whatever was left after it.
fn write_text<F: Editable>(
    file: &mut F,
    name: &'static str,
    text: &str,
) -> Result<(), ConfigureError> {
    let io_error = |source| ConfigureError::Io { file: name, source };
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    file.write_all(text.as_bytes()).map_err(io_error)?;
    file.cut_at_position().map_err(io_error)?;
    file.save().map_err(io_error)
}

/// The `dietpi.txt` keys for the connection (NET-9, NET-10, and IMG-4), the hostname, the password, the SSH server, and
/// the automated setup that runs them without a dialogue.
pub fn dietpi_txt(text: &str, settings: &Settings) -> Result<String, ConfigureError> {
    let text = match &settings.connection {
        Connection::Wifi(wifi) => {
            let text = set_line(text, "AUTO_SETUP_NET_WIFI_ENABLED", "1");
            let text = set_line(&text, "AUTO_SETUP_NET_ETHERNET_ENABLED", "0");
            set_line(
                &text,
                "AUTO_SETUP_NET_WIFI_COUNTRY_CODE",
                &wifi.country_code,
            )
        }
        Connection::Ethernet => {
            let text = set_line(text, "AUTO_SETUP_NET_WIFI_ENABLED", "0");
            set_line(&text, "AUTO_SETUP_NET_ETHERNET_ENABLED", "1")
        }
    };
    let text = set_line(
        &text,
        "AUTO_SETUP_NET_HOSTNAME",
        plain("AUTO_SETUP_NET_HOSTNAME", &settings.hostname)?,
    );
    let text = set_line(
        &text,
        "AUTO_SETUP_GLOBAL_PASSWORD",
        plain("AUTO_SETUP_GLOBAL_PASSWORD", &settings.password)?,
    );
    let text = set_line(&text, "AUTO_SETUP_SSH_SERVER_INDEX", "-2");
    Ok(set_line(&text, "AUTO_SETUP_AUTOMATED", "1"))
}

/// Refuses a value that contains a line break, since it would end its line early, and otherwise returns it.
fn plain<'a>(key: &'static str, value: &'a str) -> Result<&'a str, ConfigureError> {
    if value.contains(['\n', '\r']) {
        return Err(ConfigureError::LineBreak { key });
    }
    Ok(value)
}

/// The entry `[0]` of `dietpi-wifi.txt`, for a WPA-PSK network (IMG-6).
pub fn dietpi_wifi_txt(text: &str, wifi: &Wifi) -> Result<String, ConfigureError> {
    let ssid = quote("aWIFI_SSID[0]", &wifi.ssid)?;
    let key = quote("aWIFI_KEY[0]", &wifi.passphrase)?;
    let text = set_line(text, "aWIFI_SSID[0]", &ssid);
    let text = set_line(&text, "aWIFI_KEY[0]", &key);
    Ok(set_line(&text, "aWIFI_KEYMGR[0]", "'WPA-PSK'"))
}

/// Replaces the value of the first uncommented line whose key is `key`, or appends `key=value` when there is none.
///
/// The line ending of a replaced line is kept. Every other line is copied unchanged.
fn set_line(text: &str, key: &str, value: &str) -> String {
    let mut out = String::with_capacity(text.len() + key.len() + value.len() + 2);
    let mut replaced = false;
    for line in text.split_inclusive('\n') {
        if !replaced && assigns(line, key) {
            out.push_str(key);
            out.push('=');
            out.push_str(value);
            out.push_str(line_ending(line));
            replaced = true;
        } else {
            out.push_str(line);
        }
    }
    if !replaced {
        let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(ending);
        }
        out.push_str(key);
        out.push('=');
        out.push_str(value);
        out.push_str(ending);
    }
    out
}

/// Whether the line is an assignment to `key`. The key is the text before the first `=`, compared exactly, so a commented
/// line (which starts with `#`) never matches.
fn assigns(line: &str, key: &str) -> bool {
    line.split_once('=').is_some_and(|(name, _)| name == key)
}

fn line_ending(line: &str) -> &str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

/// Wraps a value in single quotes for `dietpi-wifi.txt`, writing each single quote in it as `'\''`.
fn quote(key: &'static str, value: &str) -> Result<String, ConfigureError> {
    if value.contains(['\n', '\r']) {
        return Err(ConfigureError::LineBreak { key });
    }
    Ok(format!("'{}'", value.replace('\'', r"'\''")))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_wifi_connection_enables_wifi_and_disables_ethernet() {
        let text = "AUTO_SETUP_NET_ETHERNET_ENABLED=1\nAUTO_SETUP_NET_WIFI_ENABLED=0\n";
        assert_eq!(
            dietpi_txt(text, &wifi_settings()).unwrap(),
            "AUTO_SETUP_NET_ETHERNET_ENABLED=0\nAUTO_SETUP_NET_WIFI_ENABLED=1\nAUTO_SETUP_NET_WIFI_COUNTRY_CODE=GB\nAUTO_SETUP_NET_HOSTNAME=mypi\nAUTO_SETUP_GLOBAL_PASSWORD=a-long-password\nAUTO_SETUP_SSH_SERVER_INDEX=-2\nAUTO_SETUP_AUTOMATED=1\n"
        );
    }

    #[test]
    fn an_ethernet_connection_enables_ethernet_and_disables_wifi() {
        let text = "AUTO_SETUP_NET_ETHERNET_ENABLED=0\nAUTO_SETUP_NET_WIFI_ENABLED=1\nAUTO_SETUP_NET_WIFI_COUNTRY_CODE=GB\n";
        assert_eq!(
            dietpi_txt(text, &settings(Connection::Ethernet)).unwrap(),
            "AUTO_SETUP_NET_ETHERNET_ENABLED=1\nAUTO_SETUP_NET_WIFI_ENABLED=0\nAUTO_SETUP_NET_WIFI_COUNTRY_CODE=GB\nAUTO_SETUP_NET_HOSTNAME=mypi\nAUTO_SETUP_GLOBAL_PASSWORD=a-long-password\nAUTO_SETUP_SSH_SERVER_INDEX=-2\nAUTO_SETUP_AUTOMATED=1\n"
        );
    }

    #[test]
    fn the_automated_setup_replaces_its_default_and_leaves_commented_examples() {
        let text = "#AUTO_SETUP_AUTOMATED=0\nAUTO_SETUP_GLOBAL_PASSWORD=dietpi\nAUTO_SETUP_NET_HOSTNAME=DietPi\nAUTO_SETUP_AUTOMATED=0\n";
        assert_eq!(
            dietpi_txt(text, &settings(Connection::Ethernet)).unwrap(),
            "#AUTO_SETUP_AUTOMATED=0\nAUTO_SETUP_GLOBAL_PASSWORD=a-long-password\nAUTO_SETUP_NET_HOSTNAME=mypi\nAUTO_SETUP_AUTOMATED=1\nAUTO_SETUP_NET_WIFI_ENABLED=0\nAUTO_SETUP_NET_ETHERNET_ENABLED=1\nAUTO_SETUP_SSH_SERVER_INDEX=-2\n"
        );
    }

    #[test]
    fn the_ssh_server_is_set_to_openssh_in_place_of_the_dropbear_default() {
        let text = "# SSH server choice: 0=none/custom | -1=Dropbear | -2=OpenSSH\nAUTO_SETUP_SSH_SERVER_INDEX=-1\nAUTO_SETUP_AUTOMATED=0\n";
        assert_eq!(
            dietpi_txt(text, &wifi_settings()).unwrap(),
            "# SSH server choice: 0=none/custom | -1=Dropbear | -2=OpenSSH\nAUTO_SETUP_SSH_SERVER_INDEX=-2\nAUTO_SETUP_AUTOMATED=1\nAUTO_SETUP_NET_WIFI_ENABLED=1\nAUTO_SETUP_NET_ETHERNET_ENABLED=0\nAUTO_SETUP_NET_WIFI_COUNTRY_CODE=GB\nAUTO_SETUP_NET_HOSTNAME=mypi\nAUTO_SETUP_GLOBAL_PASSWORD=a-long-password\n"
        );
    }

    #[test]
    fn a_line_break_in_the_password_or_hostname_is_refused_without_echoing_it() {
        let password = Settings {
            password: "pass\nAUTO_SETUP_AUTOMATED=0".to_owned(),
            ..wifi_settings()
        };
        let error = dietpi_txt("", &password).unwrap_err();
        assert!(matches!(
            error,
            ConfigureError::LineBreak {
                key: "AUTO_SETUP_GLOBAL_PASSWORD"
            }
        ));
        assert!(!error.to_string().contains("pass"));

        let hostname = Settings {
            hostname: "my\rpi".to_owned(),
            ..wifi_settings()
        };
        assert!(matches!(
            dietpi_txt("", &hostname),
            Err(ConfigureError::LineBreak {
                key: "AUTO_SETUP_NET_HOSTNAME"
            })
        ));
    }

    #[test]
    fn a_key_is_matched_exactly_and_commented_lines_are_left_alone() {
        let text = "AUTO_SETUP_NET_WIFI_ENABLED_EXTRA=7\n#AUTO_SETUP_NET_WIFI_ENABLED=5\nAUTO_SETUP_NET_WIFI_ENABLED=0\n";
        assert_eq!(
            set_line(text, "AUTO_SETUP_NET_WIFI_ENABLED", "1"),
            "AUTO_SETUP_NET_WIFI_ENABLED_EXTRA=7\n#AUTO_SETUP_NET_WIFI_ENABLED=5\nAUTO_SETUP_NET_WIFI_ENABLED=1\n"
        );
    }

    #[test]
    fn only_the_first_uncommented_occurrence_is_replaced() {
        let text = "KEY=a\nKEY=b\n";
        assert_eq!(set_line(text, "KEY", "c"), "KEY=c\nKEY=b\n");
    }

    #[test]
    fn a_missing_key_is_appended_after_a_line_without_a_newline() {
        assert_eq!(set_line("A=1", "B", "2"), "A=1\nB=2\n");
        assert_eq!(set_line("A=1\n", "B", "2"), "A=1\nB=2\n");
    }

    #[test]
    fn a_missing_key_is_appended_with_crlf_when_the_file_uses_crlf() {
        assert_eq!(set_line("A=1\r\n", "B", "2"), "A=1\r\nB=2\r\n");
    }

    #[test]
    fn a_replaced_line_keeps_its_line_ending() {
        assert_eq!(set_line("A=1\r\nB=2\r\n", "A", "9"), "A=9\r\nB=2\r\n");
        assert_eq!(set_line("A=1", "A", "9"), "A=9");
    }

    #[test]
    fn the_quoted_wifi_entry_escapes_single_quotes() {
        let wifi = Wifi {
            passphrase: "it's a 'test'".to_owned(),
            ..home()
        };
        let text = dietpi_wifi_txt("aWIFI_KEY[0]=''\n", &wifi).unwrap();
        assert_eq!(
            text,
            "aWIFI_KEY[0]='it'\\''s a '\\''test'\\'''\naWIFI_SSID[0]='Home Network'\naWIFI_KEYMGR[0]='WPA-PSK'\n"
        );
    }

    #[test]
    fn a_line_break_in_a_wifi_value_is_refused_without_echoing_it() {
        let wifi = Wifi {
            ssid: "Home\nAWIFI".to_owned(),
            ..home()
        };
        let error = dietpi_wifi_txt("", &wifi).unwrap_err();
        assert!(matches!(
            error,
            ConfigureError::LineBreak {
                key: "aWIFI_SSID[0]"
            }
        ));
        assert!(!error.to_string().contains("Home"));
    }
}
