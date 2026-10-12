//! Validation of the Network page's fields, following NET-3 to NET-6 of the credentials requirements.
//!
//! The errors name the rule that failed and never echo the value, so they are safe to show and to log (SEC-1).

use std::fmt;

use thiserror::Error;

const SSID_MAX_BYTES: usize = 32;
const PASSPHRASE_MIN_LEN: usize = 8;
const PASSPHRASE_MAX_LEN: usize = 63;
const KEY_HEX_LEN: usize = 64;
const HOSTNAME_MAX_LEN: usize = 63;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SsidError {
    #[error("the network name is empty")]
    Empty,
    #[error("the network name is longer than 32 bytes")]
    TooLong,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PassphraseError {
    #[error("the passphrase is shorter than 8 characters")]
    TooShort,
    #[error("the passphrase is longer than 63 characters")]
    TooLong,
    #[error("the passphrase contains characters that are not printable ASCII")]
    NotPrintable,
    #[error("a 64-character key must contain only hexadecimal digits")]
    NotHex,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CountryCodeError {
    #[error("the country code is not an ISO 3166-1 alpha-2 code")]
    Unknown,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HostnameError {
    #[error("the hostname is empty")]
    Empty,
    #[error("the hostname is longer than 63 characters")]
    TooLong,
    #[error("the hostname may contain only letters, digits, and hyphens")]
    InvalidCharacter,
    #[error("the hostname cannot start or end with a hyphen")]
    HyphenAtEdge,
}

/// An SSID is 1 to 32 bytes of UTF-8. Case is preserved, and the length is counted in bytes, not characters.
pub fn validate_ssid(ssid: &str) -> Result<(), SsidError> {
    match ssid.len() {
        0 => Err(SsidError::Empty),
        1..=SSID_MAX_BYTES => Ok(()),
        _ => Err(SsidError::TooLong),
    }
}

/// A passphrase is 8 to 63 printable ASCII characters, or exactly 64 hexadecimal digits, which is a pre-computed key.
pub fn validate_passphrase(passphrase: &str) -> Result<(), PassphraseError> {
    if passphrase.len() == KEY_HEX_LEN {
        return if passphrase.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(())
        } else {
            Err(PassphraseError::NotHex)
        };
    }
    if passphrase.len() < PASSPHRASE_MIN_LEN {
        return Err(PassphraseError::TooShort);
    }
    if passphrase.len() > PASSPHRASE_MAX_LEN {
        return Err(PassphraseError::TooLong);
    }
    if passphrase.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        Ok(())
    } else {
        Err(PassphraseError::NotPrintable)
    }
}

/// A country code is one of the ISO 3166-1 alpha-2 codes known to `rust_iso3166`, in capitals.
pub fn validate_country_code(code: &str) -> Result<(), CountryCodeError> {
    country_name(code)
        .map(|_| ())
        .ok_or(CountryCodeError::Unknown)
}

/// The full name for a country code, such as "GB" for "United Kingdom of Great Britain and Northern Ireland".
pub fn country_name(code: &str) -> Option<&'static str> {
    rust_iso3166::from_alpha2(code).map(|country| country.name)
}

/// The countries whose code or name contains the query, ignoring case, in ISO order. An empty query matches nothing.
pub fn countries_matching(query: &str) -> Vec<(&'static str, &'static str)> {
    if query.is_empty() {
        return Vec::new();
    }
    let query = query.to_lowercase();
    rust_iso3166::ALL
        .iter()
        .filter(|country| {
            country.alpha2.to_lowercase().contains(&query)
                || country.name.to_lowercase().contains(&query)
        })
        .map(|country| (country.alpha2, country.name))
        .collect()
}

/// The Wi-Fi network the image connects the Pi to, once its fields are valid. Its `Display` never shows the passphrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wifi {
    pub ssid: String,
    pub passphrase: String,
    pub country_code: String,
}

impl fmt::Display for Wifi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Wi-Fi SSID \"{}\", passphrase ********, country {} ({}).",
            self.ssid,
            self.country_code,
            country_name(&self.country_code).unwrap_or("unknown")
        )
    }
}

/// A hostname is one DNS label: 1 to 63 ASCII letters, digits, or hyphens, not starting or ending with a hyphen.
pub fn validate_hostname(hostname: &str) -> Result<(), HostnameError> {
    if hostname.is_empty() {
        return Err(HostnameError::Empty);
    }
    if hostname.len() > HOSTNAME_MAX_LEN {
        return Err(HostnameError::TooLong);
    }
    if !hostname
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(HostnameError::InvalidCharacter);
    }
    if hostname.starts_with('-') || hostname.ends_with('-') {
        return Err(HostnameError::HyphenAtEdge);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn country_codes_are_unique_and_complete() {
        let codes: Vec<&str> = rust_iso3166::ALL
            .iter()
            .map(|country| country.alpha2)
            .collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len());
        assert_eq!(codes.len(), 249);
        assert!(
            rust_iso3166::ALL
                .iter()
                .all(|country| !country.name.is_empty())
        );
    }

    #[test]
    fn country_code_maps_to_its_full_name() {
        assert_eq!(
            country_name("GB"),
            Some("United Kingdom of Great Britain and Northern Ireland")
        );
        assert_eq!(country_name("BO"), Some("Bolivia (Plurinational State of)"));
        assert_eq!(country_name("CI"), Some("Côte d'Ivoire"));
        assert_eq!(country_name("gb"), None);
        assert_eq!(country_name("XX"), None);
    }

    #[test]
    fn country_search_matches_codes_and_names_ignoring_case() {
        assert!(countries_matching("").is_empty());
        assert!(
            countries_matching("united k")
                .contains(&("GB", "United Kingdom of Great Britain and Northern Ireland"))
        );
        assert!(countries_matching("gb").contains(&("GB", country_name("GB").unwrap())));
        assert!(countries_matching("zzz").is_empty());
    }

    #[test]
    fn the_wifi_summary_never_shows_the_passphrase() {
        let wifi = Wifi {
            ssid: "Home".to_owned(),
            passphrase: "correct horse".to_owned(),
            country_code: "GB".to_owned(),
        };
        let summary = wifi.to_string();

        assert!(!summary.contains("correct"), "{summary}");
        assert!(summary.contains("passphrase ********"), "{summary}");
        assert!(summary.contains("SSID \"Home\""), "{summary}");
        assert!(summary.contains("GB (United Kingdom"), "{summary}");
    }

    #[test]
    fn ssid_length_is_counted_in_bytes_at_the_boundaries() {
        assert_eq!(validate_ssid(""), Err(SsidError::Empty));
        assert_eq!(validate_ssid("a"), Ok(()));
        assert_eq!(validate_ssid(&"a".repeat(32)), Ok(()));
        assert_eq!(validate_ssid(&"a".repeat(33)), Err(SsidError::TooLong));
    }

    #[test]
    fn ssid_multibyte_characters_count_as_their_bytes() {
        // "é" is two bytes, so sixteen of them fill 32 bytes and seventeen do not fit.
        assert_eq!(validate_ssid(&"é".repeat(16)), Ok(()));
        assert_eq!(validate_ssid(&"é".repeat(17)), Err(SsidError::TooLong));
    }

    #[test]
    fn ssid_keeps_case_and_spaces() {
        assert_eq!(validate_ssid("Home Network"), Ok(()));
    }

    #[test]
    fn passphrase_length_boundaries() {
        assert_eq!(
            validate_passphrase("1234567"),
            Err(PassphraseError::TooShort)
        );
        assert_eq!(validate_passphrase("12345678"), Ok(()));
        assert_eq!(validate_passphrase(&"a".repeat(63)), Ok(()));
        assert_eq!(
            validate_passphrase(&"a".repeat(65)),
            Err(PassphraseError::TooLong)
        );
    }

    #[test]
    fn passphrase_must_be_printable_ascii() {
        assert_eq!(validate_passphrase("pass word!"), Ok(()));
        assert_eq!(
            validate_passphrase("passwörd"),
            Err(PassphraseError::NotPrintable)
        );
        assert_eq!(
            validate_passphrase("pass\tword"),
            Err(PassphraseError::NotPrintable)
        );
    }

    #[test]
    fn sixty_four_characters_must_be_a_hex_key() {
        let key = "0123456789abcdefABCDEF0123456789abcdefABCDEF0123456789abcdef0123";
        assert_eq!(key.len(), 64);
        assert_eq!(validate_passphrase(key), Ok(()));
        assert_eq!(
            validate_passphrase(&"g".repeat(64)),
            Err(PassphraseError::NotHex)
        );
    }

    #[test]
    fn country_code_must_be_a_listed_code_in_capitals() {
        assert_eq!(validate_country_code("GB"), Ok(()));
        assert_eq!(validate_country_code("AQ"), Ok(()));
        assert_eq!(validate_country_code("ZW"), Ok(()));
        assert_eq!(validate_country_code("gb"), Err(CountryCodeError::Unknown));
        assert_eq!(validate_country_code("XX"), Err(CountryCodeError::Unknown));
        assert_eq!(validate_country_code("GBR"), Err(CountryCodeError::Unknown));
        assert_eq!(validate_country_code(""), Err(CountryCodeError::Unknown));
    }

    #[test]
    fn hostname_length_boundaries() {
        assert_eq!(validate_hostname(""), Err(HostnameError::Empty));
        assert_eq!(validate_hostname("a"), Ok(()));
        assert_eq!(validate_hostname(&"a".repeat(63)), Ok(()));
        assert_eq!(
            validate_hostname(&"a".repeat(64)),
            Err(HostnameError::TooLong)
        );
    }

    #[test]
    fn hostname_allows_letters_digits_and_inner_hyphens() {
        assert_eq!(validate_hostname("pisquared"), Ok(()));
        assert_eq!(validate_hostname("Pi-2-Squared"), Ok(()));
        assert_eq!(
            validate_hostname("under_score"),
            Err(HostnameError::InvalidCharacter)
        );
        assert_eq!(
            validate_hostname("dot.name"),
            Err(HostnameError::InvalidCharacter)
        );
    }

    #[test]
    fn hostname_cannot_start_or_end_with_a_hyphen() {
        assert_eq!(validate_hostname("-pi"), Err(HostnameError::HyphenAtEdge));
        assert_eq!(validate_hostname("pi-"), Err(HostnameError::HyphenAtEdge));
        assert_eq!(validate_hostname("-"), Err(HostnameError::HyphenAtEdge));
    }
}
