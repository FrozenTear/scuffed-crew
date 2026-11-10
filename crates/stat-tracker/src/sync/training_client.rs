//! The three training crop routes the tracker uses, with the sync token:
//! `GET /api/training-crops/consent`, `POST /api/training-crops?kind=` and
//! `DELETE /api/training-crops/mine`.
//!
//! There is no method for `PUT /api/training-crops/consent`: consent is set
//! on the site with a session only, and a token `PUT` is 401. None of these
//! calls touches the stats sync pause (`sync_auth.json`): a refusal here is
//! reported as a training crop warning and nothing else.

use std::time::Duration;

use scuffed_types::api::training_crops::{
    TrainingCropConsent, TrainingCropUploadResponse, TrainingCropsDeleted,
};

use super::client::{SyncClient, retry_after_from_response, send_error_text};
use super::training::{QueuedCrop, UploadKind, multipart_body};
use super::training_store::new_boundary;

pub(super) const CONSENT_PATH: &str = "/api/training-crops/consent";
pub(super) const UPLOAD_PATH: &str = "/api/training-crops";
pub(super) const DELETE_MINE_PATH: &str = "/api/training-crops/mine";

/// Largest answer body read from these routes.
const MAX_ANSWER_BYTES: usize = 256 * 1024;

/// A training crop call that did not work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrainingCallError {
    /// Nothing usable came back: unsafe URL, network, timeout, a redirect
    /// off the site, or an answer that did not parse.
    Transport(String),
    /// The server answered with an error status.
    Status {
        status: u16,
        /// `error` (or `code`) from the JSON body.
        code: Option<String>,
        /// `Retry-After` on a 429 or 503.
        retry_after: Option<Duration>,
    },
}

impl TrainingCallError {
    pub fn status(&self) -> Option<u16> {
        match self {
            TrainingCallError::Status { status, .. } => Some(*status),
            TrainingCallError::Transport(_) => None,
        }
    }

    pub fn code(&self) -> Option<&str> {
        match self {
            TrainingCallError::Status { code, .. } => code.as_deref(),
            TrainingCallError::Transport(_) => None,
        }
    }

    /// One short sentence for the desktop app.
    pub fn plain_text(&self) -> String {
        match self {
            TrainingCallError::Transport(_) => "Could not reach the website.".to_string(),
            TrainingCallError::Status { status, .. } => match status {
                401 | 403 => "The website refused the sync token.".to_string(),
                429 => "The website asked the tracker to wait. Try again in a minute.".to_string(),
                503 => "The website is not taking training crops right now.".to_string(),
                500..=599 => format!("The website had a problem (HTTP {status})."),
                _ => format!("The website refused the request (HTTP {status})."),
            },
        }
    }
}

fn error_code(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    ["error", "code"]
        .into_iter()
        .find_map(|key| value.get(key)?.as_str().map(str::to_string))
}

async fn read_json<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, TrainingCallError> {
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let retry_after = retry_after_from_response(&resp);
        let body = resp.text().await.unwrap_or_default();
        return Err(TrainingCallError::Status {
            status,
            code: error_code(&body),
            retry_after,
        });
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| TrainingCallError::Transport(e.to_string()))?;
    if bytes.len() > MAX_ANSWER_BYTES {
        return Err(TrainingCallError::Transport("answer too large".into()));
    }
    serde_json::from_slice(&bytes).map_err(|e| TrainingCallError::Transport(e.to_string()))
}

impl SyncClient {
    fn training_url(&self, path: &str) -> Result<(String, String), TrainingCallError> {
        self.refuse_unsafe_transport()
            .map_err(|e| TrainingCallError::Transport(e.to_string()))?;
        let creds = self.credentials();
        Ok((format!("{}{path}", creds.server_url), creds.token))
    }

    /// `GET /api/training-crops/consent`: the member's consent per kind.
    pub async fn fetch_training_consent(&self) -> Result<TrainingCropConsent, TrainingCallError> {
        let (url, token) = self.training_url(CONSENT_PATH)?;
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| TrainingCallError::Transport(send_error_text(&e)))?;
        read_json(resp).await
    }

    /// `POST /api/training-crops?kind=<kind>` with `crops` as meta and crop
    /// pairs. The caller has checked every crop and sized the batch
    /// ([`super::training::check_crop`], [`super::training::batch_len`]).
    pub async fn upload_training_crops(
        &self,
        kind: UploadKind,
        crops: &[QueuedCrop],
    ) -> Result<TrainingCropUploadResponse, TrainingCallError> {
        let (url, token) = self.training_url(UPLOAD_PATH)?;
        let boundary = new_boundary();
        let body = multipart_body(&boundary, crops);
        let resp = self
            .http
            .post(&url)
            .query(&[("kind", kind.as_str())])
            .bearer_auth(&token)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .map_err(|e| TrainingCallError::Transport(send_error_text(&e)))?;
        read_json(resp).await
    }

    /// `DELETE /api/training-crops/{id}`: one own crop. A crop already gone
    /// is 404 `crop_not_found`.
    pub async fn delete_training_crop(&self, id: &str) -> Result<u32, TrainingCallError> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(TrainingCallError::Transport("bad crop id".into()));
        }
        let (url, token) = self.training_url(&format!("{UPLOAD_PATH}/{id}"))?;
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| TrainingCallError::Transport(send_error_text(&e)))?;
        read_json::<TrainingCropsDeleted>(resp)
            .await
            .map(|answer| answer.deleted)
    }

    /// `DELETE /api/training-crops/mine`: every own training crop of every
    /// kind. The count the server removed.
    pub async fn delete_own_training_crops(&self) -> Result<u32, TrainingCallError> {
        let (url, token) = self.training_url(DELETE_MINE_PATH)?;
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| TrainingCallError::Transport(send_error_text(&e)))?;
        read_json::<TrainingCropsDeleted>(resp)
            .await
            .map(|answer| answer.deleted)
    }
}
