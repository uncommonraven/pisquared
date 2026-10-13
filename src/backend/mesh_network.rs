//! Validation of the Mesh Network auth key entered on the Mesh Network page.
//!
//! The errors name the rule that failed and never echo the key, so they are safe to show and to log.

use thiserror::Error;

const KEY_PREFIX: &str = "tskey-";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthKeyError {
    #[error("the auth key is empty")]
    Empty,
    #[error("the auth key must start with tskey-")]
    MissingPrefix,
    #[error("the auth key has nothing after tskey-")]
    NoSecret,
    #[error("the auth key cannot contain whitespace")]
    Whitespace,
    #[error("the auth key cannot contain quote characters")]
    Quote,
}

/// A Tailscale auth key starts with `tskey-` and has no whitespace or quote characters, since it is written to a file
/// that a shell reads.
pub fn validate_auth_key(key: &str) -> Result<(), AuthKeyError> {
    if key.is_empty() {
        return Err(AuthKeyError::Empty);
    }
    if key.chars().any(char::is_whitespace) {
        return Err(AuthKeyError::Whitespace);
    }
    if key.contains(['\'', '"']) {
        return Err(AuthKeyError::Quote);
    }
    match key.strip_prefix(KEY_PREFIX) {
        None => Err(AuthKeyError::MissingPrefix),
        Some("") => Err(AuthKeyError::NoSecret),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_with_the_prefix_is_valid() {
        assert_eq!(validate_auth_key("tskey-auth-kABC123-xyz789"), Ok(()));
    }

    #[test]
    fn an_empty_key_is_rejected() {
        assert_eq!(validate_auth_key(""), Err(AuthKeyError::Empty));
    }

    #[test]
    fn a_key_must_start_with_the_prefix() {
        assert_eq!(
            validate_auth_key("auth-kABC123"),
            Err(AuthKeyError::MissingPrefix)
        );
        assert_eq!(
            validate_auth_key("Xtskey-abc"),
            Err(AuthKeyError::MissingPrefix)
        );
    }

    #[test]
    fn the_prefix_alone_is_not_a_key() {
        assert_eq!(validate_auth_key("tskey-"), Err(AuthKeyError::NoSecret));
    }

    #[test]
    fn whitespace_and_quotes_are_rejected() {
        assert_eq!(
            validate_auth_key("tskey-abc def"),
            Err(AuthKeyError::Whitespace)
        );
        assert_eq!(
            validate_auth_key("tskey-abc\t"),
            Err(AuthKeyError::Whitespace)
        );
        assert_eq!(validate_auth_key("tskey-ab'c"), Err(AuthKeyError::Quote));
        assert_eq!(validate_auth_key("tskey-ab\"c"), Err(AuthKeyError::Quote));
    }
}
