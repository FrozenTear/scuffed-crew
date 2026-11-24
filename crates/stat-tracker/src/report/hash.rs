//! Lowercase hex SHA-256, as the report manifest lists each file and as
//! `reads.json` pairs with its `scoreboard.png`. Re-exported at
//! [`super::bundle::sha256_hex`], the path the app uses.

use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}
