//! HTTP request classification for the read gate.
//!
//! The gate never splices a TCP connection to strfry. A NIP-11 GET is answered
//! here and the socket is closed. Any other non-upgrade request is rejected
//! with `Connection: close`. Only a real WebSocket handshake (both `Upgrade`
//! and `Connection` tokens) is forwarded, after the read rules.

use std::net::IpAddr;

/// What the first request on a connection is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpClass {
    /// WebSocket handshake. Both `Upgrade: websocket` and `Connection: upgrade`.
    WebSocket,
    /// `Accept: application/nostr+json`. Answer NIP-11 from the gate.
    Nip11,
    /// Anything else. 400 and close. Do not dial strfry.
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRequest {
    pub class: HttpClass,
    pub forwarded_for: Vec<IpAddr>,
}

pub fn parse_request(buf: &[u8]) -> Result<ParsedRequest, String> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);
    let status = req.parse(buf).map_err(|err| format!("bad http: {err}"))?;
    if status.is_partial() {
        return Err("incomplete http request".into());
    }
    let upgrade = header_has_token(req.headers, "upgrade", "websocket");
    let connection = header_has_token(req.headers, "connection", "upgrade");
    let class = if upgrade && connection {
        HttpClass::WebSocket
    } else if header_has_token(req.headers, "accept", "application/nostr+json") {
        HttpClass::Nip11
    } else {
        HttpClass::Reject
    };
    Ok(ParsedRequest {
        class,
        forwarded_for: forwarded_for(req.headers),
    })
}

fn header_has_token(headers: &[httparse::Header<'_>], name: &str, token: &str) -> bool {
    headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case(name))
        .any(|header| {
            let Ok(value) = std::str::from_utf8(header.value) else {
                return false;
            };
            value.split(',').any(|part| token_matches(part, token))
        })
}

/// Comma-separated header tokens, optional whitespace, optional `;parameter`.
fn token_matches(part: &str, token: &str) -> bool {
    let media = part.trim().split(';').next().unwrap_or("").trim();
    media.eq_ignore_ascii_case(token)
}

fn forwarded_for(headers: &[httparse::Header<'_>]) -> Vec<IpAddr> {
    let mut ips = Vec::new();
    for header in headers {
        if !header.name.eq_ignore_ascii_case("x-forwarded-for") {
            continue;
        }
        let Ok(value) = std::str::from_utf8(header.value) else {
            continue;
        };
        for part in value.split(',') {
            if let Ok(ip) = part.trim().parse::<IpAddr>() {
                ips.push(ip);
            }
        }
    }
    ips
}

pub fn nip11_response() -> Vec<u8> {
    let body = serde_json::json!({
        "name": "Scuffed Crew Relay",
        "description": "Private Nostr relay for the Scuffed Crew gaming community",
        "supported_nips": [1, 11, 29, 42, 59],
        "limitation": {
            "auth_required": true,
            "payment_required": false
        }
    })
    .to_string();
    http_response(
        "200 OK",
        "application/nostr+json",
        Some("Access-Control-Allow-Origin: *\r\n"),
        &body,
    )
}

pub fn reject_response() -> Vec<u8> {
    http_response(
        "400 Bad Request",
        "text/plain; charset=utf-8",
        None,
        "websocket upgrade required\n",
    )
}

pub fn limited_response() -> Vec<u8> {
    http_response(
        "429 Too Many Requests",
        "text/plain; charset=utf-8",
        Some("Retry-After: 60\r\n"),
        "rate-limited: too many connections\n",
    )
}

fn http_response(status: &str, content_type: &str, extra: Option<&str>, body: &str) -> Vec<u8> {
    let extra = extra.unwrap_or("");
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n{extra}\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> ParsedRequest {
        parse_request(raw.as_bytes()).unwrap()
    }

    #[test]
    fn upgrade_without_space_is_websocket() {
        let raw = "GET / HTTP/1.1\r\nHost: x\r\nConnection:Upgrade\r\nUpgrade:websocket\r\n\r\n";
        assert!(
            !raw.to_ascii_lowercase().contains("upgrade: websocket"),
            "the old substring check required a space"
        );
        assert_eq!(parse(raw).class, HttpClass::WebSocket);
    }

    #[test]
    fn upgrade_tokens_allow_optional_whitespace_and_ignore_case() {
        let raw = "GET / HTTP/1.1\r\nHost: x\r\nConnection: keep-alive, Upgrade\r\nUpgrade: WebSocket\r\n\r\n";
        assert_eq!(parse(raw).class, HttpClass::WebSocket);
    }

    #[test]
    fn upgrade_without_connection_token_is_not_a_websocket() {
        let raw = "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\nUpgrade:websocket\r\n\r\n";
        assert_eq!(parse(raw).class, HttpClass::Reject);
    }

    #[test]
    fn nip11_accept_is_not_a_proxy() {
        let raw = "GET / HTTP/1.1\r\nHost: x\r\nAccept: application/nostr+json\r\n\r\n";
        assert_eq!(parse(raw).class, HttpClass::Nip11);
        let mixed =
            "GET / HTTP/1.1\r\nHost: x\r\nAccept: text/html, application/nostr+json;q=1\r\n\r\n";
        assert_eq!(parse(mixed).class, HttpClass::Nip11);
    }

    #[test]
    fn plain_get_is_rejected() {
        let raw = "GET / HTTP/1.1\r\nHost: x\r\nAccept: text/html\r\n\r\n";
        assert_eq!(parse(raw).class, HttpClass::Reject);
    }

    #[test]
    fn responses_close_the_connection() {
        let nip11 = String::from_utf8(nip11_response()).unwrap();
        assert!(nip11.starts_with("HTTP/1.1 200 OK"));
        assert!(nip11.to_ascii_lowercase().contains("connection: close"));
        assert!(nip11.contains("application/nostr+json"));
        assert!(nip11.contains("auth_required"));
        let rejected = String::from_utf8(reject_response()).unwrap();
        assert!(rejected.starts_with("HTTP/1.1 400"));
        assert!(rejected.to_ascii_lowercase().contains("connection: close"));
    }

    #[test]
    fn forwarded_for_collects_every_header() {
        let raw = "GET / HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: 203.0.113.5, 198.51.100.2\r\nX-Forwarded-For: 192.0.2.10\r\n\r\n";
        let parsed = parse(raw);
        let expect = ["203.0.113.5", "198.51.100.2", "192.0.2.10"]
            .into_iter()
            .map(|text| text.parse::<IpAddr>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(parsed.forwarded_for, expect);
    }
}
