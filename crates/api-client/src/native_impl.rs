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

    let text = resp
        .text()
        .await
        .map_err(|e| ClientError::Network(e.to_string()))?;
    crate::decode_body(&text)
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

    let text = resp
        .text()
        .await
        .map_err(|e| ClientError::Network(e.to_string()))?;
    crate::decode_body(&text)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `get` treats status >= 400 as an HTTP error, not a decoded success body.
    #[tokio::test]
    async fn get_fails_on_4xx_and_5xx() {
        assert_http_error(
            "403 Forbidden",
            br#"{"error":"Forbidden"}"#,
            403,
            "Forbidden",
        )
        .await;
        assert_http_error(
            "500 Internal Server Error",
            br#"{"error":"Internal error"}"#,
            500,
            "Internal error",
        )
        .await;
    }

    async fn assert_http_error(status_line: &str, body: &'static [u8], status: u16, needle: &str) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let status_line = status_line.to_string();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
            let header = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            tokio::io::AsyncWriteExt::write_all(&mut socket, header.as_bytes())
                .await
                .unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut socket, body)
                .await
                .unwrap();
        });

        let err = get::<serde_json::Value>(&format!("http://{addr}"), "/roles", None)
            .await
            .expect_err("status >= 400 must not decode as success");
        match err {
            ClientError::Http {
                status: got,
                body: text,
            } => {
                assert_eq!(got, status);
                assert!(text.contains(needle), "{text}");
            }
            other => panic!("expected HTTP {status}, got {other}"),
        }
    }
}
