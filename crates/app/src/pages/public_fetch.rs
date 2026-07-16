//! Tri-state fetch for public pages: a genuine 404 must render differently
//! from a network/server failure, otherwise outages read as "not found".

use scuffed_api_client::{ApiClient, ClientError};
use serde::de::DeserializeOwned;

#[derive(Debug, Clone)]
pub(crate) enum PublicFetch<T> {
    Found(T),
    NotFound,
    Failed,
}

pub(crate) async fn fetch_public<T: DeserializeOwned>(path: &str) -> PublicFetch<T> {
    match ApiClient::web().fetch::<T>(path).await {
        Ok(v) => PublicFetch::Found(v),
        Err(ClientError::Http { status: 404, .. }) => PublicFetch::NotFound,
        Err(_) => PublicFetch::Failed,
    }
}
