//! Shared fixtures for the `sync` tests: a `SyncClient` with a test token.

use super::*;

pub(super) fn client(url: &str) -> SyncClient {
    SyncClient::new(SyncConfig {
        server_url: url.to_string(),
        token: "super-secret-token".to_string(),
    })
}
