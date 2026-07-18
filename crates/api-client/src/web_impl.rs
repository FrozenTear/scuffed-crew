use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestCredentials, RequestInit, RequestMode, Response};

use crate::ClientError;

fn init_opts(method: &str) -> RequestInit {
    let opts = RequestInit::new();
    opts.set_method(method);
    opts.set_mode(RequestMode::SameOrigin);
    opts.set_credentials(RequestCredentials::SameOrigin);
    opts
}

async fn do_fetch(request: &Request) -> Result<(u16, String), ClientError> {
    let window = web_sys::window().ok_or_else(|| ClientError::Network("No window".into()))?;
    let resp_value = JsFuture::from(window.fetch_with_request(request))
        .await
        .map_err(|e| ClientError::Network(format!("{e:?}")))?;

    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| ClientError::Network("Response cast failed".into()))?;

    let status = resp.status();
    let text = JsFuture::from(
        resp.text()
            .map_err(|e| ClientError::Network(format!("{e:?}")))?,
    )
    .await
    .map_err(|e| ClientError::Network(format!("{e:?}")))?
    .as_string()
    .unwrap_or_default();

    Ok((status, text))
}

fn build_json_request(method: &str, url: &str, json_body: &str) -> Result<Request, ClientError> {
    let opts = init_opts(method);
    opts.set_body(&wasm_bindgen::JsValue::from_str(json_body));

    let request = Request::new_with_str_and_init(url, &opts)
        .map_err(|e| ClientError::Network(format!("{e:?}")))?;

    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|e| ClientError::Network(format!("{e:?}")))?;

    Ok(request)
}

async fn json_request<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    method: &str,
    base_url: &str,
    path: &str,
    body: &B,
) -> Result<T, ClientError> {
    let url = format!("{base_url}{path}");
    let json_body =
        serde_json::to_string(body).map_err(|e| ClientError::Network(format!("Serialize: {e}")))?;

    let request = build_json_request(method, &url, &json_body)?;
    let (status, text) = do_fetch(&request).await?;

    if status >= 400 {
        return Err(ClientError::Http { status, body: text });
    }

    serde_json::from_str(&text).map_err(|e| ClientError::Deserialize(e.to_string()))
}

pub async fn get<T: serde::de::DeserializeOwned>(
    base_url: &str,
    path: &str,
) -> Result<T, ClientError> {
    let url = format!("{base_url}{path}");
    let opts = init_opts("GET");

    let request = Request::new_with_str_and_init(&url, &opts)
        .map_err(|e| ClientError::Network(format!("{e:?}")))?;

    let (status, text) = do_fetch(&request).await?;

    if status >= 400 {
        return Err(ClientError::Http { status, body: text });
    }

    serde_json::from_str(&text).map_err(|e| ClientError::Deserialize(e.to_string()))
}

pub async fn post_empty(base_url: &str, path: &str) -> Result<(), ClientError> {
    let url = format!("{base_url}{path}");
    let opts = init_opts("POST");

    let request = Request::new_with_str_and_init(&url, &opts)
        .map_err(|e| ClientError::Network(format!("{e:?}")))?;

    let (status, text) = do_fetch(&request).await?;

    if status >= 400 {
        return Err(ClientError::Http { status, body: text });
    }

    Ok(())
}

pub async fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    base_url: &str,
    path: &str,
    body: &B,
) -> Result<T, ClientError> {
    json_request("POST", base_url, path, body).await
}

pub async fn put_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    base_url: &str,
    path: &str,
    body: &B,
) -> Result<T, ClientError> {
    json_request("PUT", base_url, path, body).await
}

pub async fn patch_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    base_url: &str,
    path: &str,
    body: &B,
) -> Result<T, ClientError> {
    json_request("PATCH", base_url, path, body).await
}

pub async fn delete(base_url: &str, path: &str) -> Result<(), ClientError> {
    let url = format!("{base_url}{path}");
    let opts = init_opts("DELETE");

    let request = Request::new_with_str_and_init(&url, &opts)
        .map_err(|e| ClientError::Network(format!("{e:?}")))?;

    let (status, text) = do_fetch(&request).await?;

    if status >= 400 {
        return Err(ClientError::Http { status, body: text });
    }

    Ok(())
}
