//! Discovery of the models a configured local or OpenAI-compatible endpoint
//! actually serves, for the model picker and `POST /v1/model` validation.
//!
//! `ocean-providers` decides which endpoints may be asked
//! ([`ocean_providers::local_model_sources`]) and how a discovered id routes;
//! this module owns the HTTP requests and their cache. Discovery is advisory:
//! it never blocks the picker for longer than [`REFRESH_TIMEOUT`], and an
//! endpoint that is down contributes nothing rather than an error.

use ocean_providers::{DiscoveredLocalModels, LocalModelSource, ProviderId};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Per-request budget. The endpoints are local; a slow answer is a busy or
/// absent server, and the picker must not wait on it.
const REQUEST_TIMEOUT: Duration = Duration::from_millis(1500);
/// Upper bound on one endpoint's whole refresh (listing plus capabilities).
const REFRESH_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a successful listing is served before it is refreshed.
const FRESH_FOR: Duration = Duration::from_secs(30);
/// How long a failed refresh is remembered before the endpoint is retried.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(5);
/// How long the last good listing outlives a failing endpoint, so a restart
/// or a dropped request does not empty the picker.
const KEEP_LAST_GOOD_FOR: Duration = Duration::from_secs(300);
/// Concurrent capability lookups against one endpoint.
const SHOW_CONCURRENCY: usize = 8;

struct Entry {
    checked_at: Instant,
    refresh_failed: bool,
    last_good: Option<(Instant, Vec<String>)>,
}

/// Cached, time-bounded model discovery keyed by provider and base URL.
pub(super) struct LocalModelDiscovery {
    client: reqwest::Client,
    entries: Mutex<HashMap<(String, String), Entry>>,
    fresh_for: Duration,
    retry_after_failure: Duration,
    keep_last_good_for: Duration,
}

/// The daemon's process-wide discovery cache.
pub(super) fn shared() -> &'static LocalModelDiscovery {
    static SHARED: OnceLock<LocalModelDiscovery> = OnceLock::new();
    SHARED.get_or_init(|| {
        LocalModelDiscovery::new(FRESH_FOR, RETRY_AFTER_FAILURE, KEEP_LAST_GOOD_FOR)
    })
}

impl LocalModelDiscovery {
    pub(super) fn new(
        fresh_for: Duration,
        retry_after_failure: Duration,
        keep_last_good_for: Duration,
    ) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
            entries: Mutex::new(HashMap::new()),
            fresh_for,
            retry_after_failure,
            keep_last_good_for,
        }
    }

    /// Tool-capable models per source, refreshing any source whose cached
    /// listing has expired. A source with no usable listing is omitted.
    pub(super) async fn discovered(
        &self,
        sources: &[LocalModelSource],
    ) -> Vec<DiscoveredLocalModels> {
        let mut refreshes = tokio::task::JoinSet::new();
        for (index, source) in sources.iter().enumerate() {
            if !self.needs_refresh(source) {
                continue;
            }
            let client = self.client.clone();
            let source = source.clone();
            refreshes.spawn(async move {
                let models = tokio::time::timeout(
                    REFRESH_TIMEOUT,
                    fetch_tool_capable_models(&client, &source),
                )
                .await
                .ok()
                .flatten();
                (index, models)
            });
        }
        while let Some(joined) = refreshes.join_next().await {
            if let Ok((index, models)) = joined {
                self.store(&sources[index], models);
            }
        }
        sources
            .iter()
            .filter_map(|source| {
                Some(DiscoveredLocalModels {
                    provider: source.provider.as_str().into(),
                    base_url: source.base_url.clone(),
                    models: self.usable(source)?,
                })
            })
            .collect()
    }

    fn needs_refresh(&self, source: &LocalModelSource) -> bool {
        let entries = self.entries.lock().expect("discovery cache poisoned");
        entries.get(&key(source)).is_none_or(|entry| {
            let wait = if entry.refresh_failed {
                self.retry_after_failure
            } else {
                self.fresh_for
            };
            entry.checked_at.elapsed() >= wait
        })
    }

    fn store(&self, source: &LocalModelSource, models: Option<Vec<String>>) {
        let now = Instant::now();
        let mut entries = self.entries.lock().expect("discovery cache poisoned");
        let entry = entries.entry(key(source)).or_insert(Entry {
            checked_at: now,
            refresh_failed: false,
            last_good: None,
        });
        entry.checked_at = now;
        entry.refresh_failed = models.is_none();
        match models {
            Some(models) => entry.last_good = Some((now, models)),
            None => tracing::debug!(
                provider = source.provider.as_str(),
                base_url = %source.base_url,
                "local model discovery failed; serving the catalog without it"
            ),
        }
    }

    fn usable(&self, source: &LocalModelSource) -> Option<Vec<String>> {
        let entries = self.entries.lock().expect("discovery cache poisoned");
        let (listed_at, models) = entries.get(&key(source))?.last_good.as_ref()?;
        (listed_at.elapsed() <= self.keep_last_good_for).then(|| models.clone())
    }
}

fn key(source: &LocalModelSource) -> (String, String) {
    (
        source.provider.as_str().to_string(),
        source.base_url.trim().trim_end_matches('/').to_string(),
    )
}

#[derive(serde::Deserialize)]
struct ListedModels {
    data: Vec<ListedModel>,
}

#[derive(serde::Deserialize)]
struct ListedModel {
    id: String,
}

enum Capabilities {
    /// The backend reported this model's capabilities.
    Known(Vec<String>),
    /// The backend has no capability endpoint (llama.cpp, a hosted gateway).
    NoEndpoint,
    /// The lookup did not complete; nothing is known.
    Failed,
}

/// `GET {base}/models`, narrowed to models that can call tools when the
/// backend can say so. `None` means the endpoint could not be read.
async fn fetch_tool_capable_models(
    client: &reqwest::Client,
    source: &LocalModelSource,
) -> Option<Vec<String>> {
    let base = source.base_url.trim().trim_end_matches('/');
    let bearer = source.bearer.as_ref().map(|secret| secret.expose());
    let mut request = client.get(format!("{base}/models"));
    if let Some(bearer) = bearer {
        request = request.bearer_auth(bearer);
    }
    let listed = request
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<ListedModels>()
        .await
        .ok()?;
    let mut ids: Vec<String> = Vec::new();
    for model in listed.data {
        if ocean_providers::is_local_model_name(&model.id) && !ids.contains(&model.id) {
            ids.push(model.id);
        }
    }

    // Ollama serves its native API beside `/v1`; `/api/show` reports each
    // model's capabilities. llama.cpp has no such endpoint, and neither does
    // an arbitrary compatible gateway, so everything they list is offered.
    if source.provider == ProviderId::LlamaCpp {
        return Some(ids);
    }
    let Some(root) = base.strip_suffix("/v1") else {
        return Some(ids);
    };
    let Some(first) = ids.first() else {
        return Some(ids);
    };
    // One probe decides whether the backend has a capability endpoint at all,
    // so a gateway listing hundreds of models is asked exactly once.
    let mut capabilities = vec![None; ids.len()];
    match show_capabilities(client, root, bearer, first).await {
        Capabilities::NoEndpoint => return Some(ids),
        Capabilities::Failed => return None,
        Capabilities::Known(known) => capabilities[0] = Some(known),
    }
    let mut pending = (1..ids.len()).collect::<Vec<_>>().into_iter();
    loop {
        let mut lookups = tokio::task::JoinSet::new();
        for index in pending.by_ref().take(SHOW_CONCURRENCY) {
            let client = client.clone();
            let root = root.to_string();
            let bearer = bearer.map(str::to_string);
            let id = ids[index].clone();
            lookups.spawn(async move {
                (
                    index,
                    show_capabilities(&client, &root, bearer.as_deref(), &id).await,
                )
            });
        }
        if lookups.is_empty() {
            break;
        }
        while let Some(joined) = lookups.join_next().await {
            // A model whose lookup did not answer is left out: without its
            // capabilities it cannot be offered as tool-capable.
            if let Ok((index, Capabilities::Known(known))) = joined {
                capabilities[index] = Some(known);
            }
        }
    }
    Some(
        ids.into_iter()
            .zip(capabilities)
            .filter(|(_, capabilities)| {
                capabilities
                    .as_ref()
                    .is_some_and(|known| known.iter().any(|capability| capability == "tools"))
            })
            .map(|(id, _)| id)
            .collect(),
    )
}

async fn show_capabilities(
    client: &reqwest::Client,
    root: &str,
    bearer: Option<&str>,
    model: &str,
) -> Capabilities {
    let mut request = client
        .post(format!("{root}/api/show"))
        .json(&serde_json::json!({ "model": model }));
    if let Some(bearer) = bearer {
        request = request.bearer_auth(bearer);
    }
    let Ok(response) = request.send().await else {
        return Capabilities::Failed;
    };
    if response.status().is_server_error() {
        return Capabilities::Failed;
    }
    if !response.status().is_success() {
        return Capabilities::NoEndpoint;
    }
    let Ok(body) = response.json::<serde_json::Value>().await else {
        return Capabilities::NoEndpoint;
    };
    match body.get("capabilities").and_then(|value| value.as_array()) {
        Some(capabilities) => Capabilities::Known(
            capabilities
                .iter()
                .filter_map(|capability| capability.as_str().map(str::to_string))
                .collect(),
        ),
        None => Capabilities::NoEndpoint,
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use axum::{routing::get, routing::post, Json, Router};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A stub endpoint. `capabilities` maps a model id to what `/api/show`
    /// reports; `None` serves no `/api/show` at all (the llama.cpp shape).
    pub(crate) struct StubEndpoint {
        pub(crate) base_url: String,
        pub(crate) list_requests: Arc<AtomicUsize>,
        pub(crate) show_requests: Arc<AtomicUsize>,
        server: tokio::task::JoinHandle<()>,
    }

    impl StubEndpoint {
        pub(crate) fn stop(&self) {
            self.server.abort();
        }
    }

    pub(crate) async fn stub_endpoint(
        models: &[&str],
        capabilities: Option<&[(&str, &[&str])]>,
    ) -> StubEndpoint {
        let list_requests = Arc::new(AtomicUsize::new(0));
        let show_requests = Arc::new(AtomicUsize::new(0));
        let listing = json!({
            "object": "list",
            "data": models.iter().map(|id| json!({"id": id, "object": "model"})).collect::<Vec<_>>(),
        });
        let counter = list_requests.clone();
        let mut app = Router::new().route(
            "/v1/models",
            get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                let listing = listing.clone();
                async move { Json(listing) }
            }),
        );
        if let Some(capabilities) = capabilities {
            let capabilities: HashMap<String, Vec<String>> = capabilities
                .iter()
                .map(|(id, caps)| {
                    (
                        (*id).to_string(),
                        caps.iter().map(|cap| (*cap).to_string()).collect(),
                    )
                })
                .collect();
            let counter = show_requests.clone();
            app = app.route(
                "/api/show",
                post(move |Json(body): Json<serde_json::Value>| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    let found = body["model"]
                        .as_str()
                        .and_then(|model| capabilities.get(model))
                        .cloned();
                    async move {
                        match found {
                            Some(capabilities) => {
                                Ok(Json(json!({"capabilities": capabilities, "details": {}})))
                            }
                            None => Err(axum::http::StatusCode::NOT_FOUND),
                        }
                    }
                }),
            );
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub endpoint");
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        StubEndpoint {
            base_url,
            list_requests,
            show_requests,
            server,
        }
    }

    /// The Studio's Ollama shape: tool models, one without tools, one embedder.
    pub(crate) const OLLAMA_MODELS: [&str; 4] = [
        "qwen3.8:27b-nvfp4",
        "hf.co/Org/Model_GGUF:Q4_K_M",
        "embeddinggemma-2:740m",
        "mimo-v2.6:9b-nvfp4",
    ];
    pub(crate) const OLLAMA_CAPABILITIES: [(&str, &[&str]); 4] = [
        ("qwen3.8:27b-nvfp4", &["completion", "vision", "tools"]),
        ("hf.co/Org/Model_GGUF:Q4_K_M", &["completion", "vision"]),
        ("embeddinggemma-2:740m", &["embedding"]),
        ("mimo-v2.6:9b-nvfp4", &["completion", "tools", "thinking"]),
    ];

    fn source(provider: ProviderId, base_url: &str) -> LocalModelSource {
        LocalModelSource {
            provider,
            base_url: base_url.to_string(),
            bearer: None,
        }
    }

    fn cache(fresh_for: Duration) -> LocalModelDiscovery {
        LocalModelDiscovery::new(fresh_for, Duration::ZERO, Duration::from_secs(300))
    }

    #[tokio::test]
    async fn ollama_backend_lists_only_tool_capable_models() {
        let stub = stub_endpoint(&OLLAMA_MODELS, Some(&OLLAMA_CAPABILITIES)).await;
        let sources = [source(ProviderId::OpenAiCompatible, &stub.base_url)];

        let found = cache(Duration::from_secs(60)).discovered(&sources).await;

        assert_eq!(
            found,
            [DiscoveredLocalModels {
                provider: "openai-compatible".into(),
                base_url: stub.base_url.clone(),
                models: vec!["qwen3.8:27b-nvfp4".into(), "mimo-v2.6:9b-nvfp4".into()],
            }],
            "the embedding model and the model without tools are skipped"
        );
        assert_eq!(stub.show_requests.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn backend_without_a_capability_endpoint_lists_what_it_serves() {
        // llama.cpp's `/v1/models` with no `/api/show`: one probe, then all.
        let stub = stub_endpoint(&["seatC", "other"], None).await;
        let compatible = [source(ProviderId::OpenAiCompatible, &stub.base_url)];
        let found = cache(Duration::from_secs(60)).discovered(&compatible).await;
        assert_eq!(found[0].models, ["seatC", "other"]);

        // The llama-cpp provider is never asked for capabilities.
        let stub = stub_endpoint(&["seatC"], Some(&[("seatC", &["embedding"])])).await;
        let llama = [source(ProviderId::LlamaCpp, &stub.base_url)];
        let found = cache(Duration::from_secs(60)).discovered(&llama).await;
        assert_eq!(found[0].provider, "llama-cpp");
        assert_eq!(found[0].models, ["seatC"]);
        assert_eq!(stub.show_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn listing_is_cached_until_its_refresh_interval() {
        let stub = stub_endpoint(&["seatC"], None).await;
        let sources = [source(ProviderId::LlamaCpp, &stub.base_url)];

        let cached = cache(Duration::from_secs(60));
        for _ in 0..3 {
            assert_eq!(cached.discovered(&sources).await[0].models, ["seatC"]);
        }
        assert_eq!(stub.list_requests.load(Ordering::SeqCst), 1);

        let uncached = cache(Duration::ZERO);
        for _ in 0..3 {
            assert_eq!(uncached.discovered(&sources).await[0].models, ["seatC"]);
        }
        assert_eq!(stub.list_requests.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn endpoint_down_yields_nothing_then_keeps_the_last_good_listing() {
        // Never reachable: no entry at all, quickly.
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_url = format!("http://{}/v1", closed.local_addr().unwrap());
        drop(closed);
        let started = Instant::now();
        let found = cache(Duration::ZERO)
            .discovered(&[source(ProviderId::OpenAiCompatible, &dead_url)])
            .await;
        assert!(found.is_empty());
        assert!(started.elapsed() < REFRESH_TIMEOUT + Duration::from_secs(1));

        // Reachable, then gone: the last good listing is still served.
        let stub = stub_endpoint(&["seatC"], None).await;
        let sources = [source(ProviderId::LlamaCpp, &stub.base_url)];
        let discovery = cache(Duration::ZERO);
        assert_eq!(discovery.discovered(&sources).await[0].models, ["seatC"]);
        stub.stop();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(discovery.discovered(&sources).await[0].models, ["seatC"]);

        // ...but not once it is older than the keep window.
        let stub = stub_endpoint(&["seatC"], None).await;
        let sources = [source(ProviderId::LlamaCpp, &stub.base_url)];
        let forgetful =
            LocalModelDiscovery::new(Duration::ZERO, Duration::ZERO, Duration::from_millis(250));
        assert_eq!(forgetful.discovered(&sources).await.len(), 1);
        stub.stop();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(forgetful.discovered(&sources).await.is_empty());
    }
}
