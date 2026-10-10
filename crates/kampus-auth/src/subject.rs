//! The pseudonymous token subject for an upstream user (design §7):
//! `"k1." + hex(HMAC-SHA-256(key, len32(iss) ‖ iss ‖ len32(sub) ‖ sub)[..16])`.
//! The length prefixes make the input unambiguous; the key is per school,
//! from the environment. `k1.` cannot collide with demo `synthetic:` subjects.

use aws_lc_rs::hmac;

use crate::util::hex;

/// The kampus subject for upstream `(issuer, subject)` under `key`.
pub(crate) fn pseudonym(key: &[u8], issuer: &str, subject: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    let mut context = hmac::Context::with_key(&key);
    for part in [issuer, subject] {
        // Issuers and subjects are short; u32::MAX is unreachable in practice.
        let len = u32::try_from(part.len()).unwrap_or(u32::MAX);
        context.update(&len.to_be_bytes());
        context.update(part.as_bytes());
    }
    let tag = context.sign();
    format!("k1.{}", hex(&tag.as_ref()[..16]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn stable_prefixed_and_unambiguous() {
        let a = pseudonym(&KEY, "https://login.school.example", "u-123");
        assert_eq!(a, pseudonym(&KEY, "https://login.school.example", "u-123"));
        assert!(a.starts_with("k1.") && a.len() == 3 + 32, "{a}");
        assert!(!a.contains("u-123"));
        // Moving bytes between issuer and subject changes the input.
        assert_ne!(
            pseudonym(&KEY, "https://a.example/x", "y"),
            pseudonym(&KEY, "https://a.example/", "xy")
        );
        assert_ne!(a, pseudonym(&KEY, "https://login.school.example", "u-124"));
        assert_ne!(a, pseudonym(&KEY, "https://other.example", "u-123"));
        // Another school's key gives another subject for the same user.
        assert_ne!(
            a,
            pseudonym(&[8; 32], "https://login.school.example", "u-123")
        );
    }

    #[test]
    fn matches_an_independent_hmac_computation() {
        // HMAC-SHA-256 computed by hand from the RFC 2104 definition.
        use aws_lc_rs::digest::{SHA256, digest};
        let mut message = Vec::new();
        for part in ["iss", "sub"] {
            message.extend_from_slice(&u32::try_from(part.len()).unwrap().to_be_bytes());
            message.extend_from_slice(part.as_bytes());
        }
        let pad = |byte: u8| {
            KEY.iter()
                .map(|k| k ^ byte)
                .chain([byte; 32])
                .collect::<Vec<_>>()
        };
        let inner = digest(&SHA256, &[pad(0x36), message].concat());
        let outer = digest(&SHA256, &[pad(0x5c), inner.as_ref().to_vec()].concat());
        assert_eq!(
            pseudonym(&KEY, "iss", "sub"),
            format!("k1.{}", hex(&outer.as_ref()[..16]))
        );
    }
}
