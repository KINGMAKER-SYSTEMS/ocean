use super::AppState;
use axum::{extract::State, Json};
use serde_json::json;
use std::collections::HashSet;

#[derive(Debug, serde::Deserialize)]
pub(super) struct ModelSetRequest {
    pub(super) model: String,
}

pub(super) async fn model_get(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (provider, model) = state.runtime.current_model();
    Json(json!({"ok": true, "provider": provider, "model": model}))
}

/// List the models the daemon can route to, plus the currently selected one,
/// for a client model picker.
pub(super) async fn models_list(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (provider, model) = state.runtime.current_model();
    // Per-model readiness (credential visible to THIS daemon process) so a
    // picker can tell the menu apart from what's actually selectable. Additive:
    // entries keep id/provider/label top-level and gain ready/credential_source.
    // Auth-file reads are blocking I/O, so they ride spawn_blocking.
    let (mut models, mut routes, auth_file) = tokio::task::spawn_blocking(|| {
        let env = ocean_agent::ProviderEnv::from_process();
        (
            ocean_agent::known_models_with_readiness(&env),
            ocean_providers::model_routes_with_readiness(&env),
            env.auth_file,
        )
    })
    .await
    .unwrap_or_default();
    if let Some(auth_file) = auth_file {
        // The normal turn path refreshes OAuth before resolving a credential.
        // Model discovery has its own request path, so refresh this block first
        // or an expired token would hide the model choices that could renew it.
        ocean_agent::ensure_oauth_provider_fresh(&auth_file, "openai-chatgpt").await;
    }
    // ChatGPT-plan availability is account-specific. A configured OAuth
    // credential alone must not make every bundled GPT route appear usable.
    // Refresh this list whenever the picker is opened, including after account
    // switches, and only mark routes present in that account's public catalog.
    let listed = chatgpt_plan_models().await;
    if let Some(account_models) = listed.as_ref() {
        // The supported catalog is account-scoped and can change without an
        // Ocean release. Replace the bundled ChatGPT entries with exactly the
        // slugs and display names returned for this signed-in account.
        routes.retain(|route| route.provider != "openai-chatgpt");
        routes.extend(account_model_routes(account_models));
    }
    let listed_ids = listed
        .as_ref()
        .map(|models| listed_model_ids(models))
        .unwrap_or_default();
    for model in models
        .iter_mut()
        .filter(|model| model.model.provider == "openai-chatgpt")
    {
        model.ready = listed_ids.contains(&model.model.id);
        if !model.ready {
            model.credential_source = None;
        }
    }
    for route in routes
        .iter_mut()
        .filter(|route| route.provider == "openai-chatgpt")
    {
        route.ready = listed_ids.contains(&route.model_id);
    }
    Json(json!({
        "ok": true,
        "current": { "provider": provider, "model": model, "route": format!("{provider}/{model}") },
        "models": models,
        "routes": routes,
    }))
}

#[derive(serde::Deserialize)]
struct OpenAiModelsResponse {
    models: Vec<OpenAiListedModel>,
}

#[derive(serde::Deserialize)]
struct OpenAiListedModel {
    slug: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
}

async fn chatgpt_plan_models() -> Option<Vec<OpenAiListedModel>> {
    let credential = tokio::task::spawn_blocking(|| {
        ocean_providers::resolve_credential_from_env(&ocean_providers::ProviderId::OpenAiChatGpt)
    })
    .await
    .ok()?
    .ok()??;
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?
        .get("https://api.openai.com/v1/models")
        .bearer_auth(credential.secret.expose())
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<OpenAiModelsResponse>()
        .await
        .ok()?;
    Some(
        response
            .models
            .into_iter()
            .filter(|model| model.visibility.as_deref() == Some("list"))
            .filter(|model| valid_chatgpt_slug(&model.slug))
            .collect(),
    )
}

fn valid_chatgpt_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn listed_model_ids(models: &[OpenAiListedModel]) -> HashSet<String> {
    models.iter().map(|model| model.slug.clone()).collect()
}

fn account_model_routes(models: &[OpenAiListedModel]) -> Vec<ocean_providers::ModelRoute> {
    models
        .iter()
        .map(|model| ocean_providers::ModelRoute {
            id: format!("openai-chatgpt/{}", model.slug),
            model_id: model.slug.clone(),
            provider: "openai-chatgpt".into(),
            label: model
                .display_name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(&model.slug)
                .to_string(),
            ready: true,
            effort_levels: ocean_providers::model_effort_levels(&model.slug)
                .iter()
                .map(|level| (*level).into())
                .collect(),
            aliases: Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatgpt_picker_only_marks_models_listed_for_the_account() {
        let response: OpenAiModelsResponse = serde_json::from_value(json!({
            "models": [
                {"slug":"gpt-6-astra","display_name":"GPT-6 Astra","visibility":"list"},
                {"slug":"internal-preview","visibility":"hidden"},
                {"slug":"legacy-without-visibility"}
            ]
        }))
        .unwrap();
        let listed: Vec<_> = response
            .models
            .into_iter()
            .filter(|model| model.visibility.as_deref() == Some("list"))
            .collect();
        let listed = listed_model_ids(&listed);
        assert!(listed.contains("gpt-6-astra"));
        assert!(!listed.contains("internal-preview"));
        assert!(!listed.contains("legacy-without-visibility"));
    }

    #[test]
    fn account_catalog_builds_routable_dynamic_models_with_server_labels() {
        let models = vec![OpenAiListedModel {
            slug: "gpt-next-preview".into(),
            display_name: Some("GPT Next Preview".into()),
            visibility: Some("list".into()),
        }];
        let routes = account_model_routes(&models);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].id, "openai-chatgpt/gpt-next-preview");
        assert_eq!(routes[0].label, "GPT Next Preview");
        assert!(routes[0].ready);
    }

    #[test]
    fn invalid_catalog_slugs_are_not_selectable_routes() {
        assert!(valid_chatgpt_slug("gpt-6.1-sol"));
        assert!(!valid_chatgpt_slug("../other-provider"));
        assert!(!valid_chatgpt_slug(""));
    }
}

pub(super) async fn model_set(
    State(state): State<AppState>,
    Json(req): Json<ModelSetRequest>,
) -> Json<serde_json::Value> {
    match state.runtime.set_model(&req.model) {
        Ok((provider, model)) => {
            tracing::info!(provider, model, "model swapped");
            Json(json!({"ok": true, "provider": provider, "model": model}))
        }
        Err(e) => Json(json!({"ok": false, "error": e.to_string()})),
    }
}
