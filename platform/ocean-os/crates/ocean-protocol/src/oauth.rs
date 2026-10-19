//! OAuth refresh-token exchange for provider subscription logins.
//!
//! Pure HTTP: POST a `refresh_token` grant to a token endpoint and return the
//! new tokens. Which blocks to refresh, and persisting them back into Ocean's
//! `auth.json`, is the caller's job (`ocean-agent`) — this module knows nothing
//! about files or providers, so it is trivially testable against a local fake
//! endpoint.

use std::time::Duration;

use serde::Deserialize;

use crate::error::{Error, Result};

/// Deadline for one token-endpoint round trip. A refresh runs on the turn's
/// critical path (before credential resolution), so it must fail fast rather
/// than hold the turn hostage.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(15);

/// A successful refresh: the new access token, an optional rotated refresh
/// token (some issuers rotate on every use, some don't), and the lifetime.
#[derive(Debug, Clone)]
pub struct RefreshedToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in_secs: Option<i64>,
    pub id_token: Option<String>,
    pub scope: Option<String>,
}

#[derive(Deserialize)]
struct WireResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

/// Exchange a refresh token at `endpoint` for fresh credentials.
///
/// Sends the JSON `refresh_token` grant shape both Anthropic's and OpenAI's
/// public-client token endpoints accept. The refresh token itself is a
/// secret — it is never logged, and errors carry only the HTTP status plus the
/// endpoint, never the body echoed verbatim (bodies can quote the token).
pub async fn refresh_token(
    endpoint: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<RefreshedToken> {
    refresh_token_request(endpoint, client_id, refresh_token, None).await
}

/// Refresh a Sign in with ChatGPT token. This issuer requires a form-encoded
/// request and the resource bound to the issued client registration.
pub async fn refresh_token_with_resource(
    endpoint: &str,
    client_id: &str,
    refresh_token: &str,
    resource: &str,
) -> Result<RefreshedToken> {
    refresh_token_request(endpoint, client_id, refresh_token, Some(resource)).await
}

async fn refresh_token_request(
    endpoint: &str,
    client_id: &str,
    refresh_token: &str,
    resource: Option<&str>,
) -> Result<RefreshedToken> {
    let client = reqwest::Client::builder()
        .timeout(REFRESH_TIMEOUT)
        .build()
        .map_err(Error::Http)?;
    let request = client.post(endpoint);
    let resp = if let Some(resource) = resource {
        request
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("refresh_token", refresh_token),
                ("resource", resource),
            ])
            .send()
            .await
    } else {
        request
            .json(&serde_json::json!({
                "grant_type": "refresh_token",
                "client_id": client_id,
                "refresh_token": refresh_token,
            }))
            .send()
            .await
    }
    .map_err(Error::Http)?;
    let status = resp.status();
    if !status.is_success() {
        return Err(Error::ProviderError {
            status: status.as_u16(),
            body: format!("token refresh at {endpoint} failed"),
        });
    }
    let wire: WireResponse = resp
        .json()
        .await
        .map_err(|e| Error::InvalidResponse(format!("token refresh response: {e}")))?;
    if wire.access_token.trim().is_empty() {
        return Err(Error::InvalidResponse(
            "token refresh returned an empty access_token".into(),
        ));
    }
    if resource.is_some()
        && wire.scope.as_deref().is_some_and(|scope| {
            !scope
                .split_whitespace()
                .any(|scope| scope == "chatgpt.tokens.use.direct")
        })
    {
        return Err(Error::InvalidResponse(
            "ChatGPT refresh did not retain plan inference access".into(),
        ));
    }
    Ok(RefreshedToken {
        access_token: wire.access_token,
        refresh_token: wire.refresh_token,
        expires_in_secs: wire.expires_in,
        id_token: wire.id_token,
        scope: wire.scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    /// A one-shot fake token endpoint returning `body` with `status`.
    async fn fake_endpoint(status: &'static str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Drain the request head (we don't parse it; the test asserts on
            // the response handling only).
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf).await;
            let resp = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        });
        format!("http://{addr}/oauth/token")
    }

    #[tokio::test]
    async fn successful_refresh_parses_tokens() {
        let url = fake_endpoint(
            "200 OK",
            r#"{"access_token":"new-at","refresh_token":"new-rt","expires_in":3600}"#,
        )
        .await;
        let out = refresh_token(&url, "client-1", "old-rt").await.unwrap();
        assert_eq!(out.access_token, "new-at");
        assert_eq!(out.refresh_token.as_deref(), Some("new-rt"));
        assert_eq!(out.expires_in_secs, Some(3600));
        assert_eq!(out.id_token, None);
        assert_eq!(out.scope, None);
    }

    #[tokio::test]
    async fn http_error_maps_to_provider_error_without_leaking_body() {
        let url = fake_endpoint("400 Bad Request", r#"{"error":"invalid_grant"}"#).await;
        let err = refresh_token(&url, "client-1", "old-rt")
            .await
            .expect_err("400 must be an error");
        let msg = format!("{err}");
        assert!(msg.contains("400"), "{msg}");
        assert!(
            !msg.contains("invalid_grant"),
            "response body must not be echoed into the error (may quote secrets): {msg}"
        );
    }

    #[tokio::test]
    async fn empty_access_token_is_rejected() {
        let url = fake_endpoint("200 OK", r#"{"access_token":""}"#).await;
        assert!(refresh_token(&url, "c", "r").await.is_err());
    }

    #[tokio::test]
    async fn chatgpt_refresh_uses_form_grant_issued_client_and_resource() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(socket);
            let mut request_line = String::new();
            reader.read_line(&mut request_line).await.unwrap();
            let mut content_length = 0;
            let mut content_type = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        content_length = value.trim().parse::<usize>().unwrap();
                    }
                    if name.eq_ignore_ascii_case("content-type") {
                        content_type = value.trim().to_owned();
                    }
                }
            }
            let mut body = vec![0; content_length];
            reader.read_exact(&mut body).await.unwrap();
            let body = String::from_utf8(body).unwrap();
            assert!(request_line.starts_with("POST /oauth/token "));
            assert_eq!(content_type, "application/x-www-form-urlencoded");
            assert!(body.contains("grant_type=refresh_token"), "{body}");
            assert!(body.contains("client_id=issued-client"), "{body}");
            assert!(body.contains("refresh_token=rotating-token"), "{body}");
            assert!(
                body.contains("resource=https%3A%2F%2Fapi.openai.com%2Fv1"),
                "{body}"
            );
            let response_body = r#"{"access_token":"fresh","refresh_token":"rotated"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            reader
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
        });
        let url = format!("http://{addr}/oauth/token");
        let result = refresh_token_with_resource(
            &url,
            "issued-client",
            "rotating-token",
            "https://api.openai.com/v1",
        )
        .await
        .unwrap();
        assert_eq!(result.access_token, "fresh");
        server.await.unwrap();
    }
}
