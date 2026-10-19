//! Pins how [`scuffed_api_client::ClientError`] turns an API 404 into a variant
//! and a user-facing message.
//!
//! Unknown `/api/*` paths respond with HTTP 404, `Content-Type: application/json`,
//! and `{"error":"Not found"}` instead of the site HTML page.

#[cfg(test)]
mod tests {
    use scuffed_api_client::ClientError;

    /// The response the API client reads for an unknown `/api/*` path.
    struct ApiNotFound {
        status: u16,
        content_type: &'static str,
        body: &'static str,
    }

    fn map_not_found(response: ApiNotFound) -> ClientError {
        assert_eq!(response.status, 404);
        assert_eq!(response.content_type, "application/json");
        // `ClientError::http` is what the web and native fetch paths call for
        // every status >= 400. The JSON body is kept on the Http variant.
        // Display reads the `error` field. Content-Type is not stored.
        ClientError::http(response.status, response.body, None)
    }

    #[test]
    fn http_404_json_not_found_is_the_http_variant() {
        let response = ApiNotFound {
            status: 404,
            content_type: "application/json",
            body: r#"{"error":"Not found"}"#,
        };
        let err = map_not_found(response);

        match &err {
            ClientError::Http {
                status,
                body,
                retry_after,
            } => {
                assert_eq!(*status, 404);
                assert_eq!(body, r#"{"error":"Not found"}"#);
                assert_eq!(*retry_after, None);
            }
            ClientError::Network(message) => {
                panic!("404 JSON must not become Network: {message}")
            }
            ClientError::Deserialize(message) => {
                panic!("404 JSON must not become Deserialize: {message}")
            }
        }

        assert_eq!(err.http_status(), Some(404));
        assert_eq!(err.http_body(), Some(r#"{"error":"Not found"}"#));
        assert_eq!(err.to_string(), "HTTP error 404: Not found");
        assert!(!err.to_string().contains('<'));
        assert!(!err.to_string().contains('\u{2014}'));

        // The old miss was the site HTML page, which has no JSON `error` field.
        let html_page = ClientError::http(
            404,
            "<!DOCTYPE html><html><title>Not found</title></html>",
            None,
        );
        assert_eq!(html_page.to_string(), "HTTP error: 404");
        assert_ne!(html_page.to_string(), err.to_string());

        // PR 377 also answers a wrong method with JSON, not an HTML page.
        let method_not_allowed = ClientError::http(405, r#"{"error":"Method not allowed"}"#, None);
        match &method_not_allowed {
            ClientError::Http {
                status,
                body,
                retry_after,
            } => {
                assert_eq!(*status, 405);
                assert_eq!(body, r#"{"error":"Method not allowed"}"#);
                assert_eq!(*retry_after, None);
            }
            ClientError::Network(message) => {
                panic!("405 JSON must not become Network: {message}")
            }
            ClientError::Deserialize(message) => {
                panic!("405 JSON must not become Deserialize: {message}")
            }
        }
        assert_eq!(
            method_not_allowed.to_string(),
            "HTTP error 405: Method not allowed"
        );
    }
}
