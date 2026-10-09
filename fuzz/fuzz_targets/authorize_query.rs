//! `GET {issuer}/authorize` query parsing and single-value reads.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|query: &str| {
    let _ = kampus_auth::fuzzing::authorize_query(query);
});
