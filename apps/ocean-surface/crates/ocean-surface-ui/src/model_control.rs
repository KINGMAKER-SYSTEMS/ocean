//! One composer disclosure for the next turn's model and reasoning effort.
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::daemon::{Daemon, ModelInfo};

fn matches_model(model: &ModelInfo, id: &str) -> bool {
    model.id == id || model.aliases.iter().any(|alias| alias == id)
}

fn choice_id(models: &[ModelInfo], id: &str) -> String {
    models
        .iter()
        .find(|model| matches_model(model, id))
        .map(|model| model.id.clone())
        .unwrap_or_else(|| id.into())
}

fn model_label(models: &[ModelInfo], id: &str) -> String {
    models
        .iter()
        .find(|model| matches_model(model, id))
        .map(|model| {
            if model.label.is_empty() {
                id.to_owned()
            } else {
                model.label.clone()
            }
        })
        .unwrap_or_else(|| {
            if id.is_empty() {
                "Default model".into()
            } else {
                id.into()
            }
        })
}

fn effort_levels(id: &str) -> &'static [&'static str] {
    let id = id.split_once('/').map_or(id, |(_, model)| model);
    if matches!(
        id,
        "gpt-6-astra" | "gpt-6.1-sol" | "claude-fable-5-1" | "claude-opus-5-5"
    ) {
        &["low", "medium", "high", "xhigh"]
    } else if id.starts_with("gpt-6")
        || id.starts_with("gpt-5.6")
        || id.starts_with("claude-opus-5")
        || id.starts_with("claude-sonnet-5")
    {
        &["off", "low", "medium", "high", "xhigh"]
    } else {
        &["off", "minimal", "low", "medium", "high", "xhigh"]
    }
}

fn available_efforts(models: &[ModelInfo], id: &str) -> Vec<String> {
    models
        .iter()
        .find(|model| matches_model(model, id))
        .and_then(|model| model.effort_levels.clone())
        .unwrap_or_else(|| {
            effort_levels(id)
                .iter()
                .map(|level| (*level).into())
                .collect()
        })
}

fn model_choices(mut models: Vec<ModelInfo>) -> Vec<ModelInfo> {
    models.sort_by(|a, b| {
        (a.ready == Some(false), &a.provider).cmp(&(b.ready == Some(false), &b.provider))
    });
    models
}

#[component]
pub fn ModelControl(daemon: Daemon) -> impl IntoView {
    let models = daemon.models;
    let current = daemon.default_model;
    let selected = daemon.model_override;
    let effort = daemon.thinking_level;
    let daemon = StoredValue::new(daemon);
    let effective_id = move || {
        selected
            .get()
            .unwrap_or_else(|| current.get().unwrap_or_default())
    };
    view! {
        <details class="ocean-model-control" on:keydown=move |event| {
            if event.key() == "Escape" {
                if let Some(target) = event.target().and_then(|target| target.dyn_into::<web_sys::Element>().ok()) {
                    if let Ok(Some(details)) = target.closest("details") {
                        if !details.has_attribute("open") { return; }
                        event.stop_propagation();
                        let _ = details.remove_attribute("open");
                        if let Ok(Some(summary)) = details.query_selector("summary") {
                            if let Ok(summary) = summary.dyn_into::<web_sys::HtmlElement>() { let _ = summary.focus(); }
                        }
                    }
                }
            }
        }>
            <summary class="ocean-model-control__trigger" aria-label="Model and effort">
                <span class="ocean-model-control__name">{move || model_label(&models.get(), &effective_id())}</span>
                <span class="ocean-model-control__effort">{move || effort.get().unwrap_or_else(|| "default".into())}</span>
                <span class="ocean-model-control__chevron" aria-hidden="true">"⌄"</span>
            </summary>
            <div class="ocean-model-control__panel">
                <label class="ocean-model-control__field">
                    <span>"Model"</span>
                    <select aria-label="Model for next turn" prop:value=move || choice_id(&models.get(), &selected.get().unwrap_or_default())
                        on:change=move |event| {
                            let id = event_target_value(&event);
                            let effective = if id.is_empty() { current.get_untracked().unwrap_or_default() } else { id.clone() };
                            daemon.with_value(|daemon| {
                                if effort.get_untracked().is_some_and(|level| !available_efforts(&models.get_untracked(), &effective).contains(&level)) {
                                    daemon.set_thinking_level(None);
                                }
                                daemon.set_model_override((!id.is_empty()).then_some(id));
                            });
                        }>
                        <option value="" prop:selected=move || selected.get().is_none()>"Default model"</option>
                        <Show when=move || selected.get().is_some_and(|id| !models.get().iter().any(|model| matches_model(model, &id)))>
                            <option prop:value=move || selected.get().unwrap_or_default() prop:selected=true>
                                {move || selected.get().unwrap_or_default()}
                            </option>
                        </Show>
                        <For each=move || model_choices(models.get()) key=|model| model.id.clone() children=move |model| {
                            let id = model.id.clone();
                            let label = model_label(std::slice::from_ref(&model), &id);
                            let unavailable = model.ready == Some(false);
                            view! {
                                <option value=id.clone() disabled=unavailable prop:selected=move || selected.get().is_some_and(|selected| choice_id(&models.get(), &selected) == id)>
                                    {if unavailable { format!("{label} · connect {}", model.provider) } else { label }}
                                </option>
                            }
                        } />
                    </select>
                </label>
                <label class="ocean-model-control__field">
                    <span>"Effort"</span>
                    <select aria-label="Reasoning effort" prop:value=move || effort.get().unwrap_or_default()
                        on:change=move |event| {
                            let value = event_target_value(&event);
                            daemon.with_value(|daemon| daemon.set_thinking_level((!value.is_empty()).then_some(value)));
                        }>
                        <option value="" prop:selected=move || effort.get().is_none()>"Default"</option>
                        <Show when=move || effort.get().is_some_and(|level| !available_efforts(&models.get(), &effective_id()).contains(&level))>
                            <option prop:value=move || effort.get().unwrap_or_default() prop:selected=true>
                                {move || effort.get().unwrap_or_default()}
                            </option>
                        </Show>
                        <For each=move || available_efforts(&models.get(), &effective_id()) key=|level| level.clone() children=move |level| {
                            let value = level.clone();
                            let selected_level = level.clone();
                            view! { <option value=value prop:selected=move || effort.get().as_deref() == Some(selected_level.as_str())>{level}</option> }
                        } />
                    </select>
                </label>
            </div>
        </details>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_always_thinking_models_do_not_offer_off_or_minimal() {
        for id in [
            "gpt-6-astra",
            "gpt-6.1-sol",
            "claude-fable-5-1",
            "claude-opus-5-5",
        ] {
            assert!(!effort_levels(id).contains(&"off"));
            assert!(!effort_levels(id).contains(&"minimal"));
            assert!(effort_levels(id).contains(&"xhigh"));
        }
        assert!(effort_levels("gpt-6-luna").contains(&"off"));
    }

    #[test]
    fn readiness_orders_connected_routes_first_and_retains_catalog_order() {
        let models: Vec<ModelInfo> = serde_json::from_value(serde_json::json!([
            {"id":"disconnected", "provider":"anthropic", "label":"A", "ready":false},
            {"id":"latest", "provider":"openai-codex", "label":"Z", "ready":true},
            {"id":"older", "provider":"openai-codex", "label":"A", "ready":true},
            {"id":"legacy-daemon", "provider":"deepseek", "label":"D"}
        ]))
        .unwrap();
        let ids: Vec<_> = model_choices(models)
            .into_iter()
            .map(|model| model.id)
            .collect();
        assert_eq!(ids, ["legacy-daemon", "latest", "older", "disconnected"]);
    }

    #[test]
    fn daemon_effort_metadata_controls_max_and_old_daemons_remain_compatible() {
        let models: Vec<ModelInfo> = serde_json::from_value(serde_json::json!([
            {"id":"gpt-6.1-sol", "effort_levels":["low","medium","high","xhigh","max"]},
            {"id":"custom", "effort_levels":["low","high"]}
        ]))
        .unwrap();
        assert!(available_efforts(&models, "gpt-6.1-sol").contains(&"max".into()));
        assert_eq!(available_efforts(&models, "custom"), ["low", "high"]);
        assert!(!available_efforts(&[], "gpt-6.1-sol").contains(&"max".into()));
    }

    #[test]
    fn missing_metadata_uses_wire_model_for_qualified_saved_routes() {
        for id in [
            "openai/gpt-6.1-sol",
            "openai-codex/gpt-6-astra",
            "claude-code/claude-fable-5-1",
            "anthropic/claude-opus-5-5",
        ] {
            let levels = available_efforts(&[], id);
            assert!(!levels.contains(&"off".into()), "{id}");
            assert!(!levels.contains(&"minimal".into()), "{id}");
            assert!(
                !levels.contains(&"max".into()),
                "legacy metadata cannot establish max support"
            );
            assert!(levels.contains(&"high".into()));
        }
        assert!(available_efforts(&[], "openai-codex/gpt-6-luna").contains(&"off".into()));
    }

    #[test]
    fn explicit_empty_effort_capability_does_not_restore_legacy_options() {
        let models: Vec<ModelInfo> = serde_json::from_value(serde_json::json!([
            {"id":"openai/gpt-6.1-sol", "effort_levels":[]},
            {"id":"openai-codex/gpt-6.1-sol"}
        ]))
        .unwrap();
        assert!(available_efforts(&models, "openai/gpt-6.1-sol").is_empty());
        assert_eq!(
            available_efforts(&models, "openai-codex/gpt-6.1-sol"),
            ["low", "medium", "high", "xhigh"]
        );
    }

    #[test]
    fn auth_choices_preserve_legacy_alias_and_api_identity() {
        let models: Vec<ModelInfo> = serde_json::from_value(serde_json::json!([
            {"id":"openai-codex/gpt-6.1-sol", "aliases":["gpt-6.1-sol"], "label":"GPT-6.1 Sol (Codex)", "effort_levels":["low","max"]},
            {"id":"openai/gpt-6.1-sol", "label":"GPT-6.1 Sol (API)", "effort_levels":["low","max"]}
        ])).unwrap();
        assert_eq!(
            choice_id(&models, "gpt-6.1-sol"),
            "openai-codex/gpt-6.1-sol"
        );
        assert_eq!(model_label(&models, "gpt-6.1-sol"), "GPT-6.1 Sol (Codex)");
        assert_eq!(
            model_label(&models, "openai/gpt-6.1-sol"),
            "GPT-6.1 Sol (API)"
        );
        assert_eq!(
            available_efforts(&models, "openai/gpt-6.1-sol"),
            ["low", "max"]
        );
        assert_eq!(choice_id(&models, ""), "");
        assert_eq!(choice_id(&models, "custom-pinned"), "custom-pinned");
    }

    #[test]
    fn unknown_pinned_model_keeps_its_identity() {
        assert_eq!(
            model_label(&[], "custom-pinned-model"),
            "custom-pinned-model"
        );
        assert_eq!(model_label(&[], ""), "Default model");
    }
}
