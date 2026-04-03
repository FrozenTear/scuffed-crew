use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Deserialize)]
pub struct Nip05Query {
    pub name: Option<String>,
}

#[derive(Serialize)]
pub struct Nip05Response {
    pub names: HashMap<String, String>,
    pub relays: HashMap<String, Vec<String>>,
}

/// Normalize a display name to a NIP-05 local name: lowercase, keep alphanumeric + underscores.
fn normalize_nip05_name(display_name: &str) -> String {
    display_name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// GET /.well-known/nostr.json — NIP-05 identity verification endpoint.
///
/// Query params:
///   - `name`: local part of the NIP-05 identifier (e.g., "devadmin" for devadmin@scuffedclan.gg)
///   - `name=_`: returns all linked identities (directory listing)
///
/// Response includes `Access-Control-Allow-Origin: *` as required by NIP-05.
pub async fn nostr_json(
    State(state): State<AppState>,
    Query(query): Query<Nip05Query>,
) -> impl IntoResponse {
    let members = match state.db.list_nostr_identities().await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to list Nostr identities: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")],
                Json(Nip05Response {
                    names: HashMap::new(),
                    relays: HashMap::new(),
                }),
            );
        }
    };

    let mut names = HashMap::new();
    let requested_name = query.name.unwrap_or_default().to_lowercase();

    for member in &members {
        if let Some(ref pubkey) = member.nostr_pubkey {
            let nip05_name = normalize_nip05_name(&member.display_name);
            if nip05_name.is_empty() {
                continue;
            }
            // Return all if wildcard, or only matching name
            if requested_name == "_" || requested_name == nip05_name {
                names.insert(nip05_name, pubkey.clone());
            }
        }
    }

    (
        StatusCode::OK,
        [(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")],
        Json(Nip05Response {
            names,
            relays: HashMap::new(),
        }),
    )
}
