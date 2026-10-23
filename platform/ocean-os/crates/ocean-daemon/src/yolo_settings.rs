use axum::Json;
use ocean_core::{PermissionMode, PermissionSettingsRequest, PermissionSettingsResponse};
use serde_json::json;
use std::env;
use std::path::PathBuf;

/// Parse the legacy `OCEAN_YOLO` switch. The three-state setting keeps this
/// compatibility layer: true forces `skip_all`; false prohibits `skip_all`
/// while preserving a saved `manual` versus `automatic` choice. Unset, empty,
/// or unrecognised spellings express no preference.
fn parse_yolo_env(raw: Option<&str>) -> Option<bool> {
    match raw?.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Focused env-layer assertion target retained for the legacy YOLO tests.
#[cfg(test)]
pub(super) fn yolo_env_enabled(raw: Option<&str>) -> bool {
    parse_yolo_env(raw).unwrap_or(false)
}

/// The two process inputs the global approval policy is resolved from.
///
/// Production captures them from the process environment once per request
/// (`from_process_env`). Tests build them explicitly so the policy resolution
/// and the settings endpoints can be exercised against a disposable directory
/// without mutating the process-global environment that every other test in
/// this binary shares.
#[derive(Debug, Clone)]
pub(super) struct PermissionPolicyInputs {
    /// Directory holding `permission_mode_pref` and the legacy `yolo_pref`.
    pub(super) config_dir: PathBuf,
    /// Raw `OCEAN_YOLO` value, `None` when unset.
    pub(super) yolo_env: Option<String>,
}

impl PermissionPolicyInputs {
    /// Resolve from `OCEAN_CONFIG_DIR` (via ocean-agent's config-dir chain) and
    /// `OCEAN_YOLO`, exactly as the daemon has always read them.
    pub(super) fn from_process_env() -> Self {
        Self {
            config_dir: ocean_agent::config_dir_from_env(),
            yolo_env: env::var("OCEAN_YOLO").ok(),
        }
    }

    #[cfg(test)]
    pub(super) fn explicit(config_dir: impl Into<PathBuf>, yolo_env: Option<&str>) -> Self {
        Self {
            config_dir: config_dir.into(),
            yolo_env: yolo_env.map(str::to_owned),
        }
    }

    fn yolo_env_pref(&self) -> Option<bool> {
        parse_yolo_env(self.yolo_env.as_deref())
    }

    fn persisted_mode(&self) -> Option<PermissionMode> {
        ocean_agent::load_permission_mode(&self.config_dir)
    }

    /// Resolve the daemon-owned approval policy for a newly-started turn.
    ///
    /// Precedence:
    /// 1. `OCEAN_YOLO=1` forces `skip_all`.
    /// 2. `OCEAN_YOLO=0` prevents `skip_all`, but does not collapse an explicit
    ///    `manual` choice into `automatic`.
    /// 3. The persisted three-state choice (with legacy `yolo_pref` migration).
    /// 4. `automatic`, the safe autonomous default: read-only tools run while
    ///    mutating/side-effecting tools keep their permission gate.
    pub(super) fn effective_mode(&self) -> PermissionMode {
        let persisted = self.persisted_mode().unwrap_or_default();
        match self.yolo_env_pref() {
            Some(true) => PermissionMode::SkipAll,
            Some(false) if persisted == PermissionMode::SkipAll => PermissionMode::Automatic,
            Some(false) | None => persisted,
        }
    }

    /// The effective mode forced by `OCEAN_YOLO`, when it differs from the
    /// saved/default mode.
    pub(super) fn env_override(&self) -> Option<PermissionMode> {
        let persisted = self.persisted_mode().unwrap_or_default();
        match self.yolo_env_pref() {
            Some(true) if persisted != PermissionMode::SkipAll => Some(PermissionMode::SkipAll),
            Some(false) if persisted == PermissionMode::SkipAll => Some(PermissionMode::Automatic),
            Some(true) | Some(false) | None => None,
        }
    }

    /// Legacy boolean view used by voice dead-end checks and compatibility APIs.
    pub(super) fn effective_yolo(&self) -> bool {
        self.effective_mode() == PermissionMode::SkipAll
    }

    /// Resolve an inert legacy request `yolo` flag into the operator's
    /// three-state policy. A client-supplied flag can never escalate daemon
    /// authority.
    pub(super) fn resolve_request_mode(&self, wire_yolo: bool) -> PermissionMode {
        let _ = wire_yolo;
        self.effective_mode()
    }

    /// `GET /v1/settings/permissions` body: both the saved choice and the mode a
    /// newly-started turn will actually use after the legacy env override.
    pub(super) fn settings_snapshot(&self) -> PermissionSettingsResponse {
        PermissionSettingsResponse {
            ok: true,
            error: None,
            persisted: self.persisted_mode(),
            effective: self.effective_mode(),
            env_override: self.env_override(),
        }
    }

    /// `POST /v1/settings/permissions` body: persist the operator's three-state
    /// global approval policy. The daemon resolves it at turn start, so
    /// in-flight turns keep their captured policy and subsequent turns use the
    /// new choice. A write failure is reported, never a false success.
    pub(super) fn persist_mode(&self, mode: PermissionMode) -> PermissionSettingsResponse {
        if let Err(error) = ocean_agent::persist_permission_mode(&self.config_dir, mode) {
            tracing::warn!(%error, requested = ?mode, "failed to persist permission mode");
            return PermissionSettingsResponse {
                ok: false,
                error: Some(error.to_string()),
                persisted: self.persisted_mode(),
                effective: self.effective_mode(),
                env_override: self.env_override(),
            };
        }
        let effective = self.effective_mode();
        let env_override = self.env_override();
        tracing::info!(persisted = ?mode, ?effective, ?env_override, "permission mode persisted");
        PermissionSettingsResponse {
            ok: true,
            error: None,
            persisted: Some(mode),
            effective,
            env_override,
        }
    }

    /// `GET /v1/settings/yolo` body — retained compatibility shape.
    pub(super) fn yolo_snapshot(&self) -> serde_json::Value {
        json!({
            "ok": true,
            "persisted": ocean_agent::load_yolo_pref(&self.config_dir),
            "effective": self.effective_yolo(),
            "env_override": self.yolo_env_pref(),
        })
    }

    /// `POST /v1/settings/yolo` body — retained compatibility adapter. Writing
    /// the bool also updates the new three-state preference so old and new
    /// clients cannot leave conflicting settings behind.
    pub(super) fn persist_yolo(&self, enabled: bool) -> serde_json::Value {
        let mode = if enabled {
            PermissionMode::SkipAll
        } else {
            PermissionMode::Automatic
        };
        if let Err(error) = ocean_agent::persist_permission_mode(&self.config_dir, mode) {
            tracing::warn!(%error, "failed to persist yolo default");
            return json!({
                "ok": false,
                "persisted": ocean_agent::load_yolo_pref(&self.config_dir),
                "effective": self.effective_yolo(),
                "env_override": self.yolo_env_pref(),
                "error": error.to_string(),
            });
        }
        let env_override = self.yolo_env_pref();
        tracing::info!(persisted = enabled, ?env_override, "yolo default persisted");
        json!({
            "ok": true,
            "persisted": enabled,
            "effective": self.effective_yolo(),
            "env_override": env_override,
        })
    }
}

/// Resolve the daemon-owned approval policy for a newly-started turn from the
/// process environment. See [`PermissionPolicyInputs::effective_mode`].
pub(super) fn effective_permission_mode() -> PermissionMode {
    PermissionPolicyInputs::from_process_env().effective_mode()
}

pub(super) fn permission_env_override() -> Option<PermissionMode> {
    PermissionPolicyInputs::from_process_env().env_override()
}

/// Legacy boolean view used by voice dead-end checks and compatibility APIs.
pub(super) fn effective_yolo() -> bool {
    PermissionPolicyInputs::from_process_env().effective_yolo()
}

/// Resolve an inert legacy request `yolo` flag into the operator's three-state
/// policy. A client-supplied flag can never escalate daemon authority.
pub(super) fn resolve_request_permission_mode(wire_yolo: bool) -> PermissionMode {
    PermissionPolicyInputs::from_process_env().resolve_request_mode(wire_yolo)
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct YoloSetRequest {
    /// `true` maps to `skip_all`; `false` maps to `automatic`.
    pub(super) enabled: bool,
}

/// `GET /v1/settings/yolo` — retained compatibility shape.
pub(super) async fn yolo_setting_get() -> Json<serde_json::Value> {
    Json(PermissionPolicyInputs::from_process_env().yolo_snapshot())
}

/// `POST /v1/settings/yolo` — retained compatibility adapter.
pub(super) async fn yolo_setting_set(Json(req): Json<YoloSetRequest>) -> Json<serde_json::Value> {
    Json(PermissionPolicyInputs::from_process_env().persist_yolo(req.enabled))
}

/// `GET /v1/settings/permissions` — report both the saved choice and the mode a
/// newly-started turn will actually use after the legacy env override.
pub(super) async fn permission_settings_get() -> Json<PermissionSettingsResponse> {
    Json(PermissionPolicyInputs::from_process_env().settings_snapshot())
}

/// `POST /v1/settings/permissions` — persist the operator's three-state global
/// approval policy.
pub(super) async fn permission_settings_set(
    Json(req): Json<PermissionSettingsRequest>,
) -> Json<PermissionSettingsResponse> {
    Json(PermissionPolicyInputs::from_process_env().persist_mode(req.mode))
}
