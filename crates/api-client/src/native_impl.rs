use crate::ClientError;

use std::time::Duration;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

async fn json_request<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    method: reqwest::Method,
    base_url: &str,
    path: &str,
    body: &B,
    token: Option<&str>,
) -> Result<T, ClientError> {
    let url = format!("{base_url}{path}");
    let client = client();
    let mut req = client.request(method, &url).json(body);

    if let Some(tok) = token {
        req = req.bearer_auth(tok);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| ClientError::Network(e.to_string()))?;

    let status = resp.status().as_u16();
    if status >= 400 {
        let body = resp.text().await.unwrap_or_default();
        return Err(ClientError::Http { status, body });
    }

    resp.json()
        .await
        .map_err(|e| ClientError::Deserialize(e.to_string()))
}

pub async fn get<T: serde::de::DeserializeOwned>(
    base_url: &str,
    path: &str,
    token: Option<&str>,
) -> Result<T, ClientError> {
    let url = format!("{base_url}{path}");
    let client = client();
    let mut req = client.get(&url);

    if let Some(tok) = token {
        req = req.bearer_auth(tok);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| ClientError::Network(e.to_string()))?;

    let status = resp.status().as_u16();
    if status >= 400 {
        let body = resp.text().await.unwrap_or_default();
        return Err(ClientError::Http { status, body });
    }

    resp.json()
        .await
        .map_err(|e| ClientError::Deserialize(e.to_string()))
}

pub async fn post_empty(
    base_url: &str,
    path: &str,
    token: Option<&str>,
) -> Result<(), ClientError> {
    let url = format!("{base_url}{path}");
    let client = client();
    let mut req = client.post(&url);

    if let Some(tok) = token {
        req = req.bearer_auth(tok);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| ClientError::Network(e.to_string()))?;

    let status = resp.status().as_u16();
    if status >= 400 {
        let body = resp.text().await.unwrap_or_default();
        return Err(ClientError::Http { status, body });
    }

    Ok(())
}

pub async fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    base_url: &str,
    path: &str,
    body: &B,
    token: Option<&str>,
) -> Result<T, ClientError> {
    json_request(reqwest::Method::POST, base_url, path, body, token).await
}

pub async fn put_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    base_url: &str,
    path: &str,
    body: &B,
    token: Option<&str>,
) -> Result<T, ClientError> {
    json_request(reqwest::Method::PUT, base_url, path, body, token).await
}

pub async fn patch_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    base_url: &str,
    path: &str,
    body: &B,
    token: Option<&str>,
) -> Result<T, ClientError> {
    json_request(reqwest::Method::PATCH, base_url, path, body, token).await
}

pub async fn delete(base_url: &str, path: &str, token: Option<&str>) -> Result<(), ClientError> {
    let url = format!("{base_url}{path}");
    let client = client();
    let mut req = client.delete(&url);

    if let Some(tok) = token {
        req = req.bearer_auth(tok);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| ClientError::Network(e.to_string()))?;

    let status = resp.status().as_u16();
    if status >= 400 {
        let body = resp.text().await.unwrap_or_default();
        return Err(ClientError::Http { status, body });
    }

    Ok(())
}
