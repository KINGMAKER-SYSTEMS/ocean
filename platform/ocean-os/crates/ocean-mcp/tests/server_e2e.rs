//! The real server executable interoperates with Ocean's MCP client over stdio.
use ocean_mcp::{McpClient, StdioTransport};
use ocean_protocol::Content;
use serde_json::json;
use std::{process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn actual_server_handshake_health_and_tool_errors() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = axum::Router::new().route("/health", axum::routing::get(|| async {
        axum::Json(json!({"ok":true,"service":"ocean-daemon","backend":"fixture","rev":"test-revision"}))
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let transport = StdioTransport::spawn(
        env!("CARGO_BIN_EXE_ocean-mcp"),
        &["--daemon".into(), base, "serve".into()],
        &[],
    )
    .unwrap();
    let client = McpClient::new(Box::new(transport)).with_timeout(Duration::from_secs(10));
    assert_eq!(
        client.initialize("ocean-server-test").await.unwrap(),
        "ocean"
    );
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 9);
    assert!(tools.iter().all(|tool| tool.name != "ocean_prompt"));
    let prompt = client
        .call_tool("ocean_prompt", json!({"prompt":"write a file","yolo":true}))
        .await
        .unwrap();
    assert!(prompt.is_error);
    assert!(
        matches!(&prompt.content[0], Content::Text { text } if text.contains("Direct prompts are unavailable"))
    );
    // A refused stale call leaves this same connection usable.
    let health = client.call_tool("ocean_health", json!({})).await.unwrap();
    assert!(!health.is_error);
    assert!(matches!(&health.content[0], Content::Text { text } if text.contains("test-revision")));
    let bad = client
        .call_tool(
            "ocean_room_post",
            json!({"room":"team","body":"forged","author_id":"peer"}),
        )
        .await
        .unwrap();
    assert!(bad.is_error);
    drop(client);
    server.abort();
}

#[tokio::test]
async fn actual_server_refuses_oversized_stdio_frames() {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_ocean-mcp"))
        .args(["--daemon", "http://127.0.0.1:1", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let _ = input.write_all(&vec![b'x'; 256 * 1024 + 1]).await;
    drop(input);
    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("size limit"));
}
