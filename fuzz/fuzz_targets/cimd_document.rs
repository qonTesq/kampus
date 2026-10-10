//! Client ID Metadata Document validation: the client ID URL, a newline,
//! then the document body (no newline: a fixed URL and the whole input).
#![no_main]

use libfuzzer_sys::fuzz_target;

const URL: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";

fuzz_target!(|data: &[u8]| {
    let (url, body) = match data.iter().position(|b| *b == b'\n') {
        Some(i) => match std::str::from_utf8(&data[..i]) {
            Ok(url) => (url, &data[i + 1..]),
            Err(_) => return,
        },
        None => (URL, data),
    };
    let _ = kampus_auth::fuzzing::cimd_document(url, body);
});
