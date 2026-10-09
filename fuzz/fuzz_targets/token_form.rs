//! `POST {issuer}/token`: the form body plus an optional `Authorization`
//! header, split at the first NUL byte (no NUL: no header).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (body, authorization) = match data.iter().position(|b| *b == 0) {
        Some(i) => (&data[..i], std::str::from_utf8(&data[i + 1..]).ok()),
        None => (data, None),
    };
    let _ = kampus_auth::fuzzing::token_form(body, authorization);
});
