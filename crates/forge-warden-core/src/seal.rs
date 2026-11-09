//! SHA-256 seals: the lowercase hex digest of exact bytes, before any parsing.
//!
//! `sha256sum < file` reproduces every seal forge-warden reports.
//!
//! Q-TYPES-2 (Q1-04): a seal is a [`Sha256Hex`], checked once where it is made (by hashing, or by
//! parsing outside text), so every holder of one knows it is exactly 64 lowercase hex digits. There is
//! no other way to obtain one: the field is private and the only constructors are [`Sha256Hex::of`]
//! and [`Sha256Hex::parse`].

use serde::Serialize;
use sha2::{Digest, Sha256};

/// A well-formed seal: exactly 64 lowercase hex digits. Serialized as that string, unchanged.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Sha256Hex(String);

/// Why text is not a seal.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SealError {
    /// Not exactly 64 bytes long.
    #[error("a seal is 64 lowercase hex digits, not {len} bytes")]
    Length {
        /// The byte length found.
        len: usize,
    },
    /// A byte that is not `0-9` or `a-f` (uppercase hex is refused, not folded).
    #[error("a seal is 64 lowercase hex digits; byte {at} is not one")]
    NotLowercaseHex {
        /// The offset of the first offending byte.
        at: usize,
    },
}

impl Sha256Hex {
    /// The seal of `bytes`.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(format!("{:x}", Sha256::digest(bytes)))
    }

    /// `text` as a seal, refused unless it is exactly 64 lowercase hex digits.
    ///
    /// # Errors
    /// [`SealError`] naming the length or the first byte that is not lowercase hex.
    pub fn parse(text: &str) -> Result<Self, SealError> {
        if text.len() != 64 {
            return Err(SealError::Length { len: text.len() });
        }
        match text
            .bytes()
            .position(|b| !(b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        {
            Some(at) => Err(SealError::NotLowercaseHex { at }),
            None => Ok(Self(text.to_owned())),
        }
    }

    /// The 64 hex digits.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Sha256Hex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> Sha256Hex {
    Sha256Hex::of(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn empty_input_has_the_published_digest() {
        assert_eq!(
            sha256_hex(b"").as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn uppercase_short_long_and_non_hex_seals_are_refused_with_their_reason() {
        assert_eq!(
            Sha256Hex::parse(&"A".repeat(64)),
            Err(SealError::NotLowercaseHex { at: 0 })
        );
        assert_eq!(
            Sha256Hex::parse(&format!("{}g", "a".repeat(63))),
            Err(SealError::NotLowercaseHex { at: 63 })
        );
        assert_eq!(
            Sha256Hex::parse(&"a".repeat(63)),
            Err(SealError::Length { len: 63 })
        );
        assert_eq!(
            Sha256Hex::parse(&"a".repeat(65)),
            Err(SealError::Length { len: 65 })
        );
        assert_eq!(Sha256Hex::parse(""), Err(SealError::Length { len: 0 }));
        assert!(Sha256Hex::parse(&"0123456789abcdef".repeat(4)).is_ok());
    }

    #[test]
    fn a_seal_serializes_as_its_bare_digits() {
        let seal = sha256_hex(b"");
        assert_eq!(
            serde_json::to_string(&seal).unwrap(),
            format!("\"{}\"", seal.as_str())
        );
        assert_eq!(seal.to_string(), seal.as_str());
    }

    proptest! {
        #[test]
        fn every_digest_parses_back_to_itself(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let seal = sha256_hex(&bytes);
            prop_assert_eq!(Sha256Hex::parse(seal.as_str()), Ok(seal));
        }
    }
}
