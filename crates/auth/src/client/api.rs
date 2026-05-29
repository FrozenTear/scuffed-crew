//! API client helpers for making HTTP requests to the backend (WASM).

use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;
use wasm_bindgen::JsCast;
use web_sys::{Request, RequestCredentials, RequestInit, RequestMode, Response};

/// API error types
#[derive(Error, Debug, Clone)]
pub enum ApiError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("Request failed with status {status}: {message}")]
    Status { status: u16, message: String },

    #[error("Failed to parse response: {0}")]
    Parse(String),

    #[error("Unauthorized - please log in")]
    Unauthorized,

    #[error("Not found")]
    NotFound,
}

/// Result type for API operations
pub type ApiResult<T> = Result<T, ApiError>;

/// Fetch JSON from a URL with GET request
pub async fn fetch_json<T: DeserializeOwned>(url: &str) -> ApiResult<T> {
    let opts = RequestInit::new();
    opts.set_method("GET");
    opts.set_mode(RequestMode::SameOrigin);
    opts.set_credentials(RequestCredentials::Include);

    let request = Request::new_with_str_and_init(url, &opts)
        .map_err(|_| ApiError::Network("Failed to create request".into()))?;

    request
        .headers()
        .set("Accept", "application/json")
        .map_err(|_| ApiError::Network("Failed to set headers".into()))?;

    send_request(request).await
}

/// Fetch a cursor-paginated list endpoint, returning just the data vec.
pub async fn fetch_json_list<T: DeserializeOwned>(url: &str) -> ApiResult<Vec<T>> {
    #[derive(serde::Deserialize)]
    struct CursorResponse<T> {
        data: Vec<T>,
    }
    let resp: CursorResponse<T> = fetch_json(url).await?;
    Ok(resp.data)
}

/// POST JSON to a URL
pub async fn post_json<T: Serialize, R: DeserializeOwned>(url: &str, body: &T) -> ApiResult<R> {
    send_json_body("POST", url, body).await
}

/// PUT JSON to a URL
pub async fn put_json<T: Serialize, R: DeserializeOwned>(url: &str, body: &T) -> ApiResult<R> {
    send_json_body("PUT", url, body).await
}

/// DELETE request to a URL
pub async fn delete(url: &str) -> ApiResult<()> {
    let opts = RequestInit::new();
    opts.set_method("DELETE");
    opts.set_mode(RequestMode::SameOrigin);
    opts.set_credentials(RequestCredentials::Include);

    let request = Request::new_with_str_and_init(url, &opts)
        .map_err(|_| ApiError::Network("Failed to create request".into()))?;

    let resp = send_request_raw(request).await?;

    if resp.ok() {
        Ok(())
    } else {
        Err(status_error(&resp).await)
    }
}

/// POST without expecting a response body
pub async fn post_empty<T: Serialize>(url: &str, body: &T) -> ApiResult<()> {
    let json = serde_json::to_string(body)
        .map_err(|e| ApiError::Parse(format!("Serialization error: {}", e)))?;

    let opts = RequestInit::new();
    opts.set_method("POST");
    opts.set_mode(RequestMode::SameOrigin);
    opts.set_credentials(RequestCredentials::Include);
    opts.set_body(&wasm_bindgen::JsValue::from_str(&json));

    let request = Request::new_with_str_and_init(url, &opts)
        .map_err(|_| ApiError::Network("Failed to create request".into()))?;

    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|_| ApiError::Network("Failed to set headers".into()))?;

    let resp = send_request_raw(request).await?;

    if resp.ok() {
        Ok(())
    } else {
        Err(status_error(&resp).await)
    }
}

async fn send_json_body<T: Serialize, R: DeserializeOwned>(
    method: &str,
    url: &str,
    body: &T,
) -> ApiResult<R> {
    let json = serde_json::to_string(body)
        .map_err(|e| ApiError::Parse(format!("Serialization error: {}", e)))?;

    let opts = RequestInit::new();
    opts.set_method(method);
    opts.set_mode(RequestMode::SameOrigin);
    opts.set_credentials(RequestCredentials::Include);
    opts.set_body(&wasm_bindgen::JsValue::from_str(&json));

    let request = Request::new_with_str_and_init(url, &opts)
        .map_err(|_| ApiError::Network("Failed to create request".into()))?;

    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|_| ApiError::Network("Failed to set headers".into()))?;

    send_request(request).await
}

async fn send_request<T: DeserializeOwned>(request: Request) -> ApiResult<T> {
    let resp = send_request_raw(request).await?;

    if resp.ok() {
        let text = get_response_text(&resp).await?;
        serde_json::from_str(&text).map_err(|e| ApiError::Parse(format!("JSON parse error: {}", e)))
    } else {
        Err(status_error(&resp).await)
    }
}

async fn send_request_raw(request: Request) -> ApiResult<Response> {
    let window = web_sys::window().ok_or_else(|| ApiError::Network("No window".into()))?;

    let resp_value = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| ApiError::Network(format!("Fetch failed: {:?}", e)))?;

    resp_value
        .dyn_into::<Response>()
        .map_err(|_| ApiError::Network("Invalid response object".into()))
}

async fn get_response_text(resp: &Response) -> ApiResult<String> {
    let text_promise = resp
        .text()
        .map_err(|_| ApiError::Parse("Failed to get response text".into()))?;

    let text_value = wasm_bindgen_futures::JsFuture::from(text_promise)
        .await
        .map_err(|_| ApiError::Parse("Failed to read response text".into()))?;

    text_value
        .as_string()
        .ok_or_else(|| ApiError::Parse("Response text is not a string".into()))
}

async fn status_error(resp: &Response) -> ApiError {
    let status = resp.status();

    match status {
        401 => ApiError::Unauthorized,
        404 => ApiError::NotFound,
        _ => {
            let message = get_response_text(resp)
                .await
                .unwrap_or_else(|_| "Unknown error".into());
            ApiError::Status { status, message }
        }
    }
}
