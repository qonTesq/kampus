//! Small encoding and randomness helpers (no `base64` crate, per design §13).

use aws_lc_rs::digest;
use aws_lc_rs::rand::{SecureRandom, SystemRandom};

const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url without padding (RFC 4648 section 5).
pub(crate) fn b64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(char::from(URL_SAFE[(n >> (18 - 6 * i) & 63) as usize]));
        }
    }
    out
}

/// Decode base64 in either alphabet, with or without padding. `None` for
/// anything malformed (including non-zero trailing bits).
pub(crate) fn b64_decode(text: &str) -> Option<Vec<u8>> {
    let trimmed = text.trim_end_matches('=');
    if text.len() - trimmed.len() > 2 || trimmed.len() % 4 == 1 {
        return None;
    }
    let value = |c: u8| -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        }))
    };
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    for chunk in trimmed.as_bytes().chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= value(*c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        let keep = chunk.len() - 1;
        // Bits past the last whole byte must be zero (canonical encoding).
        let mask = match keep {
            1 => 0x0000_ffff,
            2 => 0x0000_00ff,
            _ => 0,
        };
        if n & mask != 0 {
            return None;
        }
        out.extend_from_slice(&bytes[1..=keep]);
    }
    Some(out)
}

/// Lowercase hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Decode lowercase or uppercase hex.
pub(crate) fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digits = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(digits, 16).ok()
        })
        .collect()
}

/// Undo `application/x-www-form-urlencoded` escaping (`+` and `%XX`).
/// `None` for a malformed escape or a result that is not UTF-8.
pub(crate) fn form_unescape(text: &str) -> Option<String> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(b) = bytes.next() {
        match b {
            b'+' => out.push(b' '),
            b'%' => {
                let pair = [bytes.next()?, bytes.next()?];
                out.push(*hex_decode(std::str::from_utf8(&pair).ok()?)?.first()?);
            }
            _ => out.push(b),
        }
    }
    String::from_utf8(out).ok()
}

/// `n` random bytes from the system CSPRNG, hex encoded. `None` only if the
/// system generator fails.
pub(crate) fn random_hex(n: usize) -> Option<String> {
    let mut bytes = vec![0u8; n];
    SystemRandom::new().fill(&mut bytes).ok()?;
    Some(hex(&bytes))
}

/// SHA-256 of `text`, hex encoded: how codes and refresh tokens are stored.
pub(crate) fn sha256_hex(text: &str) -> String {
    hex(digest::digest(&digest::SHA256, text.as_bytes()).as_ref())
}

/// Constant-time equality for secrets.
pub(crate) fn secret_eq(a: &str, b: &str) -> bool {
    aws_lc_rs::constant_time::verify_slices_are_equal(a.as_bytes(), b.as_bytes()).is_ok()
}

/// Current Unix time in seconds.
pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_in_both_alphabets() {
        for len in 0..40u8 {
            let bytes: Vec<u8> = (0..len).map(|i| i.wrapping_mul(37) ^ 0xa5).collect();
            let encoded = b64url(&bytes);
            assert_eq!(b64_decode(&encoded).unwrap(), bytes);
            let standard = encoded.replace('-', "+").replace('_', "/");
            let padded = format!("{standard}{}", "=".repeat((4 - standard.len() % 4) % 4));
            assert_eq!(b64_decode(&padded).unwrap(), bytes);
        }
        assert_eq!(b64_decode("a").as_deref(), None);
        assert_eq!(b64_decode("ab*c").as_deref(), None);
        assert_eq!(b64_decode("YQ===").as_deref(), None);
        // Non-canonical trailing bits.
        assert_eq!(b64_decode("YR").as_deref(), None);
        assert_eq!(b64_decode("dXNlcjpwYXNz").unwrap(), b"user:pass");
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
        assert_eq!(hex_decode("000fFF").unwrap(), [0, 15, 255]);
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("zz").is_none());
        assert_eq!(random_hex(16).unwrap().len(), 32);
    }
}
