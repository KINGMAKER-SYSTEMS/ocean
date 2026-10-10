//! Sign-out keeps ChatGPT registration identity but detaches all credentials.
use crate::{resolve_auth_path, store, OAuthProvider};
use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::Value;
use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteRevocation {
    NotRequired,
    Confirmed,
    Unconfirmed,
}

#[derive(Debug)]
pub struct LogoutOutcome {
    /// Whether local credential material was removed (registration may remain).
    pub removed: bool,
    pub remote_revocation: RemoteRevocation,
}
impl LogoutOutcome {
    pub fn message(&self) -> &'static str {
        match self.remote_revocation {
            RemoteRevocation::Unconfirmed => "Signed out locally. Remote revocation was not confirmed; disconnect the app in ChatGPT Settings.",
            _ => "Signed out.",
        }
    }
}

/// Detach credential usability before network work so concurrent refresh cannot
/// restore this session. The bounded task retains its credentials even if its
/// waiter is cancelled. Callers serialize login/logout for the same provider.
pub async fn logout(provider: OAuthProvider, auth_file: Option<PathBuf>) -> Result<LogoutOutcome> {
    let path = resolve_auth_path(auth_file)?;
    tokio::spawn(async move {
        let prior = tokio::task::spawn_blocking(move || {
            if provider == OAuthProvider::ChatGptPlan {
                store::detach_chatgpt_tokens(&path)
            } else {
                store::remove_and_write(&path, provider.auth_json_key())
                    .map(|removed| removed.then_some(Value::Bool(true)))
            }
        })
        .await
        .map_err(|_| anyhow!("logout custody failed"))??;
        if provider != OAuthProvider::ChatGptPlan {
            return Ok(LogoutOutcome {
                removed: prior.is_some(),
                remote_revocation: RemoteRevocation::NotRequired,
            });
        }
        let Some(prior) = prior else {
            return Ok(LogoutOutcome {
                removed: false,
                remote_revocation: RemoteRevocation::NotRequired,
            });
        };
        let removed = ["access", "refresh", "id_token"]
            .iter()
            .any(|key| nonempty(&prior, key).is_some());
        let remote_revocation = match (nonempty(&prior, "refresh"), nonempty(&prior, "client_id")) {
            (Some(refresh), Some(client_id)) if client_id != "dynamic_agent_client" => {
                if revoke_chatgpt(client_id, refresh).await {
                    RemoteRevocation::Confirmed
                } else {
                    RemoteRevocation::Unconfirmed
                }
            }
            (Some(_), _) => RemoteRevocation::Unconfirmed,
            _ => RemoteRevocation::NotRequired,
        };
        Ok(LogoutOutcome {
            removed,
            remote_revocation,
        })
    })
    .await
    .map_err(|_| anyhow!("logout task failed"))?
}
fn nonempty<'a>(block: &'a Value, key: &str) -> Option<&'a str> {
    block
        .get(key)?
        .as_str()
        .filter(|value| !value.trim().is_empty())
}
async fn revoke_chatgpt(client_id: &str, refresh: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return false;
    };
    revoke_at(
        &client,
        "https://auth.openai.com/.well-known/openid-configuration",
        "https://auth.openai.com",
        client_id,
        refresh,
        Duration::from_millis(150),
    )
    .await
}
// Parameters permit loopback synthetic fixtures without production env overrides.
async fn revoke_at(
    client: &reqwest::Client,
    discovery_url: &str,
    approved_origin: &str,
    client_id: &str,
    refresh: &str,
    backoff: Duration,
) -> bool {
    let mut endpoint = None;
    for attempt in 0..3 {
        if attempt != 0 {
            tokio::time::sleep(backoff * attempt).await;
        }
        if endpoint.is_none() {
            match client.get(discovery_url).send().await {
                Ok(response) if response.status() == reqwest::StatusCode::OK => {
                    let Ok(bytes) = response.bytes().await else {
                        continue;
                    };
                    let Ok(discovery) = serde_json::from_slice::<Value>(&bytes) else {
                        return false;
                    };
                    if discovery["issuer"] != "https://auth.openai.com" {
                        return false;
                    }
                    let Some(raw) = discovery["revocation_endpoint"].as_str() else {
                        return false;
                    };
                    let Ok(url) = reqwest::Url::parse(raw) else {
                        return false;
                    };
                    if url.origin().ascii_serialization() != approved_origin
                        || !url.username().is_empty()
                        || url.password().is_some()
                        || url.fragment().is_some()
                    {
                        return false;
                    }
                    endpoint = Some(url);
                }
                Ok(response) if response.status().is_server_error() => continue,
                Err(_) => continue,
                _ => return false,
            }
        }
        match client
            .post(endpoint.as_ref().unwrap().clone())
            .form(&[
                ("token", refresh),
                ("token_type_hint", "refresh_token"),
                ("client_id", client_id),
            ])
            .send()
            .await
        {
            Ok(response) if response.status() == reqwest::StatusCode::OK => return true,
            Ok(response) if response.status().is_server_error() => continue,
            Err(_) => continue,
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    // status zero closes the connection to exercise a transport failure.
    async fn fixture(
        statuses: Vec<u16>,
        discovery_failure: bool,
        foreign_endpoint: bool,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let endpoint = if foreign_endpoint {
            "https://untrusted.invalid/revoke".to_owned()
        } else {
            format!("{origin}/revoke")
        };
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut first = true;
            let mut statuses = statuses.into_iter();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 1024];
                    let n = socket.read(&mut bytes).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&bytes[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .unwrap_or("0")
                            .parse()
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(request).unwrap();
                let discovery = request.starts_with("GET ");
                requests.push(request);
                if first && discovery_failure {
                    first = false;
                    socket.write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    continue;
                }
                let (status, body) = if discovery {
                    (200, serde_json::json!({"issuer":"https://auth.openai.com", "revocation_endpoint":endpoint}).to_string())
                } else {
                    (statuses.next().unwrap(), String::new())
                };
                if status != 0 {
                    socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                }
                if foreign_endpoint
                    || (!discovery && (status == 200 || (status != 0 && status < 500)))
                    || (!discovery && statuses.len() == 0)
                {
                    break;
                }
            }
            requests
        });
        (origin, task)
    }

    #[tokio::test]
    async fn chatgpt_revocation_success_retries_and_unconfirmed_are_bounded() {
        for (statuses, confirmed) in [
            (vec![200], true),
            (vec![503, 200], true),
            (vec![0, 200], true),
            (vec![503, 503, 503], false),
            (vec![400], false),
            (vec![302], false),
        ] {
            let attempts = statuses.len();
            let (origin, server) = fixture(statuses, false, false).await;
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(1))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            assert_eq!(
                revoke_at(
                    &client,
                    &format!("{origin}/discovery"),
                    &origin,
                    "issued-synthetic",
                    "refresh-synthetic",
                    Duration::ZERO
                )
                .await,
                confirmed
            );
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), attempts + 1);
            for request in &requests[1..] {
                assert!(request.starts_with("POST /revoke "));
                assert!(request.contains("token=refresh-synthetic"));
                assert!(request.contains("token_type_hint=refresh_token"));
                assert!(request.contains("client_id=issued-synthetic"));
            }
        }
    }

    #[tokio::test]
    async fn chatgpt_discovery_retries_and_rejects_untrusted_endpoint() {
        for foreign in [false, true] {
            let (origin, server) = fixture(vec![200], !foreign, foreign).await;
            let client = reqwest::Client::new();
            assert_eq!(
                revoke_at(
                    &client,
                    &format!("{origin}/discovery"),
                    &origin,
                    "issued",
                    "synthetic",
                    Duration::ZERO
                )
                .await,
                !foreign
            );
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), if foreign { 1 } else { 3 });
        }
    }
}
