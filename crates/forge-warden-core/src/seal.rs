//! SHA-256 seals: the lowercase hex digest of exact bytes, before any parsing.
//!
//! `sha256sum < file` reproduces every seal forge-warden reports.

use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        // Both nibbles are < 16, so `get` always succeeds; `unwrap_or` keeps this panic-free
        // without an index.
        let hi = HEX.get(usize::from(byte >> 4)).copied().unwrap_or(b'0');
        let lo = HEX.get(usize::from(byte & 0x0f)).copied().unwrap_or(b'0');
        out.push(char::from(hi));
        out.push(char::from(lo));
    }
    out
}

/// Whether `text` is a well-formed seal: exactly 64 lowercase hex digits.
#[must_use]
pub fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn empty_input_has_the_published_digest() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn uppercase_and_short_seals_are_refused() {
        assert!(!is_sha256_hex(&"A".repeat(64)));
        assert!(!is_sha256_hex(&"a".repeat(63)));
        assert!(!is_sha256_hex(&"a".repeat(65)));
        assert!(!is_sha256_hex(&"g".repeat(64)));
    }

    proptest! {
        #[test]
        fn every_digest_is_a_well_formed_seal(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            prop_assert!(is_sha256_hex(&sha256_hex(&bytes)));
        }
    }
}
