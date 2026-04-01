use serde::{de::DeserializeOwned, Deserialize, Serialize};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestCredentials, RequestInit, RequestMode, Response};

#[derive(Debug, Clone)]
pub enum ApiError {
    Network(String),
    Status { status: u16, message: String },
    Parse(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Network(msg) => write!(f, "Network error: {msg}"),
            ApiError::Status { status, message } => write!(f, "HTTP {status}: {message}"),
            ApiError::Parse(msg) => write!(f, "Parse error: {msg}"),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

async fn do_fetch(method: &str, url: &str, body: Option<String>) -> ApiResult<Response> {
    let opts = RequestInit::new();
    opts.set_method(method);
    opts.set_mode(RequestMode::SameOrigin);
    opts.set_credentials(RequestCredentials::SameOrigin);

    if let Some(json_body) = body {
        let headers = web_sys::Headers::new().map_err(|e| ApiError::Network(format!("{e:?}")))?;
        headers
            .set("Content-Type", "application/json")
            .map_err(|e| ApiError::Network(format!("{e:?}")))?;
        opts.set_headers(&headers);
        opts.set_body(&wasm_bindgen::JsValue::from_str(&json_body));
    }

    let request = Request::new_with_str_and_init(url, &opts)
        .map_err(|e| ApiError::Network(format!("{e:?}")))?;

    let window = web_sys::window().ok_or_else(|| ApiError::Network("No window".into()))?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| ApiError::Network(format!("{e:?}")))?;

    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| ApiError::Network("Response cast failed".into()))?;

    if !resp.ok() {
        let status = resp.status();
        let text = JsFuture::from(resp.text().map_err(|e| ApiError::Network(format!("{e:?}")))?)
            .await
            .map_err(|e| ApiError::Network(format!("{e:?}")))?
            .as_string()
            .unwrap_or_default();
        return Err(ApiError::Status {
            status,
            message: text,
        });
    }

    Ok(resp)
}

async fn parse_json<T: DeserializeOwned>(resp: Response) -> ApiResult<T> {
    let text = JsFuture::from(resp.text().map_err(|e| ApiError::Parse(format!("{e:?}")))?)
        .await
        .map_err(|e| ApiError::Parse(format!("{e:?}")))?
        .as_string()
        .unwrap_or_default();
    serde_json::from_str(&text).map_err(|e| ApiError::Parse(e.to_string()))
}

pub async fn get<T: DeserializeOwned>(url: &str) -> ApiResult<T> {
    let resp = do_fetch("GET", url, None).await?;
    parse_json(resp).await
}

/// Response shape for cursor-paginated list endpoints.
#[derive(Debug, Clone, Deserialize)]
pub struct CursorResponse<T> {
    pub data: Vec<T>,
    pub next_cursor: Option<String>,
}

/// Fetch all items from a cursor-paginated list endpoint (first page, no cursor).
pub async fn get_list<T: DeserializeOwned>(url: &str) -> ApiResult<Vec<T>> {
    let resp: CursorResponse<T> = get(url).await?;
    Ok(resp.data)
}

pub async fn post<B: Serialize, T: DeserializeOwned>(url: &str, body: &B) -> ApiResult<T> {
    let json = serde_json::to_string(body).map_err(|e| ApiError::Parse(e.to_string()))?;
    let resp = do_fetch("POST", url, Some(json)).await?;
    parse_json(resp).await
}

pub async fn put<B: Serialize, T: DeserializeOwned>(url: &str, body: &B) -> ApiResult<T> {
    let json = serde_json::to_string(body).map_err(|e| ApiError::Parse(e.to_string()))?;
    let resp = do_fetch("PUT", url, Some(json)).await?;
    parse_json(resp).await
}

pub async fn patch<B: Serialize, T: DeserializeOwned>(url: &str, body: &B) -> ApiResult<T> {
    let json = serde_json::to_string(body).map_err(|e| ApiError::Parse(e.to_string()))?;
    let resp = do_fetch("PATCH", url, Some(json)).await?;
    parse_json(resp).await
}

pub async fn delete(url: &str) -> ApiResult<()> {
    do_fetch("DELETE", url, None).await?;
    Ok(())
}

/// Upload a file via multipart form data.
pub async fn upload_file<T: DeserializeOwned>(url: &str, file: web_sys::File) -> ApiResult<T> {
    let form_data =
        web_sys::FormData::new().map_err(|e| ApiError::Network(format!("{e:?}")))?;
    form_data
        .append_with_blob("file", &file)
        .map_err(|e| ApiError::Network(format!("{e:?}")))?;

    let opts = RequestInit::new();
    opts.set_method("POST");
    opts.set_mode(RequestMode::SameOrigin);
    opts.set_credentials(RequestCredentials::SameOrigin);
    opts.set_body(&form_data);

    let request = Request::new_with_str_and_init(url, &opts)
        .map_err(|e| ApiError::Network(format!("{e:?}")))?;

    let window = web_sys::window().ok_or_else(|| ApiError::Network("No window".into()))?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| ApiError::Network(format!("{e:?}")))?;

    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| ApiError::Network("Response cast failed".into()))?;

    if !resp.ok() {
        let status = resp.status();
        let text = JsFuture::from(resp.text().map_err(|e| ApiError::Network(format!("{e:?}")))?)
            .await
            .map_err(|e| ApiError::Network(format!("{e:?}")))?
            .as_string()
            .unwrap_or_default();
        return Err(ApiError::Status {
            status,
            message: text,
        });
    }

    parse_json(resp).await
}
