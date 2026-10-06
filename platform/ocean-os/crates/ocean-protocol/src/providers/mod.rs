pub mod anthropic;
pub mod codex;
pub mod google;
pub mod openai;

use async_trait::async_trait;

use crate::error::Result;
use crate::stream::AssistantMessageEventStream;
use crate::types::{Context, Model, StreamOptions};

/// Generic provider interface — invoked by `stream_simple` based on `model.api`.
#[async_trait]
pub trait Provider: Send + Sync {
    async fn stream(
        &self,
        model: &Model,
        context: &Context,
        options: &StreamOptions,
    ) -> Result<AssistantMessageEventStream>;
}

#[cfg(test)]
pub(crate) mod test_support {
    /// Answer one request on `listener` with `events` as an SSE body, after
    /// reading the whole request so the client sees a clean exchange.
    pub(crate) async fn serve_one_sse(listener: tokio::net::TcpListener, events: &'static str) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        let expected = loop {
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0, "request ended before headers");
            bytes.extend_from_slice(&chunk[..n]);
            assert!(bytes.len() < 64 * 1024, "fixture request exceeds bound");
            if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                break end + 4 + length;
            }
        };
        while bytes.len() < expected {
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{events}",
            events.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }
}
