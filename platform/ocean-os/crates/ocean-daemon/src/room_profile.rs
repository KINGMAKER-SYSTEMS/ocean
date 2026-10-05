//! Read-only required-slot admission status; profile writes/routes remain deferred.
use chrono::Utc;
use ocean_core::RoomKey;
use ocean_store::{CredentialSlot, RoomStoreError};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path as FsPath};

/// A slot's status on THIS node. Never carries a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct SlotStatus {
    pub(super) name: String,
    pub(super) required: bool,
    /// `resolved` | `missing` | `expired` | `resolver_not_open`
    pub(super) status: &'static str,
    /// The resolver that satisfied the slot, when `resolved`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) resolver: Option<String>,
}

impl SlotStatus {
    pub(super) fn blocks_admission(&self) -> bool {
        self.required && self.status != "resolved"
    }
}

/// What one resolver found. Ordered so the worst of several can be picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Probe {
    Missing,
    NotOpen,
    Expired,
    Resolved,
}

/// The daemon's own `auth.json`, read as an opaque map so this module never
/// touches a token field. Only the top-level provider keys and each block's
/// `expires` are consulted.
fn auth_blocks(config_dir: &FsPath) -> BTreeMap<String, Option<i64>> {
    let path = config_dir.join("auth.json");
    let Ok(raw) = std::fs::read(&path) else {
        return BTreeMap::new();
    };
    let Ok(Value::Object(root)) = serde_json::from_slice::<Value>(&raw) else {
        return BTreeMap::new();
    };
    root.into_iter()
        .filter_map(|(key, block)| match block {
            Value::Object(block) => {
                let expires = block.get("expires").and_then(Value::as_i64);
                Some((key, expires))
            }
            _ => None,
        })
        .collect()
}

fn probe(resolver: &str, auth: &BTreeMap<String, Option<i64>>, now_secs: i64) -> Probe {
    let Some((scheme, target)) = resolver.split_once(':') else {
        return Probe::Missing;
    };
    match scheme {
        "env" => match std::env::var_os(target) {
            Some(v) if !v.is_empty() => Probe::Resolved,
            _ => Probe::Missing,
        },
        "oauth" => match auth.get(target) {
            None => Probe::Missing,
            Some(None) => Probe::Resolved,
            Some(Some(expires)) => {
                // `oauth_refresh.rs` stores ms; older blocks stored seconds.
                let expires_secs = if *expires >= 1_000_000_000_000 {
                    expires / 1_000
                } else {
                    *expires
                };
                if expires_secs > now_secs {
                    Probe::Resolved
                } else {
                    Probe::Expired
                }
            }
        },
        "keychain" => Probe::NotOpen,
        _ => Probe::Missing,
    }
}

/// Resolve every slot's status on this node. The first resolver that resolves
/// wins; otherwise the slot reports the most informative failure it saw
/// (`expired` over `resolver_not_open` over `missing`), so an operator learns
/// "your token lapsed" before "keychain isn't wired yet".
pub(super) fn resolve_slots(slots: &[CredentialSlot], config_dir: &FsPath) -> Vec<SlotStatus> {
    let auth = auth_blocks(config_dir);
    let now_secs = Utc::now().timestamp();
    slots
        .iter()
        .map(|slot| {
            let mut worst = Probe::Missing;
            let mut resolved_by = None;
            for resolver in &slot.resolvers {
                match probe(resolver, &auth, now_secs) {
                    Probe::Resolved => {
                        resolved_by = Some(resolver.clone());
                        break;
                    }
                    other => worst = worst.max(other),
                }
            }
            let status = match (resolved_by.is_some(), worst) {
                (true, _) => "resolved",
                (false, Probe::Expired) => "expired",
                (false, Probe::NotOpen) => "resolver_not_open",
                (false, _) => "missing",
            };
            SlotStatus {
                name: slot.name.clone(),
                required: slot.required,
                status,
                resolver: resolved_by,
            }
        })
        .collect()
}

pub(super) fn blocking_slot(
    store: &mut ocean_store::SqliteRoomStore,
    room: &RoomKey,
    config_dir: &FsPath,
) -> Result<Option<String>, RoomStoreError> {
    let Some(profile) = store.room_profile(room)? else {
        return Ok(None);
    };
    Ok(resolve_slots(&profile.credential_slots, config_dir)
        .into_iter()
        .find(SlotStatus::blocks_admission)
        .map(|slot| slot.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn slot(name: &str, required: bool, resolvers: &[&str]) -> CredentialSlot {
        CredentialSlot {
            name: name.into(),
            purpose: String::new(),
            required,
            resolvers: resolvers.iter().map(|r| r.to_string()).collect(),
        }
    }

    #[test]
    fn env_resolver_reports_presence_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("OCEAN_TEST_SLOT_PRESENT", "not-a-real-secret");
        std::env::remove_var("OCEAN_TEST_SLOT_ABSENT");
        let statuses = resolve_slots(
            &[
                slot("A", true, &["env:OCEAN_TEST_SLOT_PRESENT"]),
                slot("B", true, &["env:OCEAN_TEST_SLOT_ABSENT"]),
                slot(
                    "C",
                    false,
                    &["env:OCEAN_TEST_SLOT_ABSENT", "env:OCEAN_TEST_SLOT_PRESENT"],
                ),
            ],
            tmp.path(),
        );
        assert_eq!(statuses[0].status, "resolved");
        assert_eq!(
            statuses[0].resolver.as_deref(),
            Some("env:OCEAN_TEST_SLOT_PRESENT")
        );
        assert!(!statuses[0].blocks_admission());
        assert_eq!(statuses[1].status, "missing");
        assert!(statuses[1].blocks_admission());
        assert_eq!(
            statuses[2].status, "resolved",
            "second resolver may satisfy"
        );
        assert!(!statuses[2].blocks_admission(), "optional never blocks");
        let rendered = serde_json::to_string(&statuses).unwrap();
        assert!(!rendered.contains("not-a-real-secret"));
        std::env::remove_var("OCEAN_TEST_SLOT_PRESENT");
    }

    #[test]
    fn oauth_resolver_reads_block_presence_and_expiry_never_the_token() {
        let tmp = tempfile::TempDir::new().unwrap();
        let future_ms = (Utc::now().timestamp() + 3_600) * 1_000;
        let past_secs = Utc::now().timestamp() - 3_600;
        std::fs::write(
            tmp.path().join("auth.json"),
            serde_json::to_vec(&json!({
                "claude-code": { "access": "sk-live-SECRET", "expires": future_ms },
                "openai-codex": { "access": "sk-old-SECRET", "expires": past_secs },
                "deepseek": { "api_key": "dk-SECRET" },
                "not-a-block": "string"
            }))
            .unwrap(),
        )
        .unwrap();
        let statuses = resolve_slots(
            &[
                slot("FRESH", true, &["oauth:claude-code"]),
                slot("LAPSED", true, &["oauth:openai-codex"]),
                slot("KEYED", true, &["oauth:deepseek"]),
                slot("NONE", true, &["oauth:kimi"]),
                slot("STRING", true, &["oauth:not-a-block"]),
                slot(
                    "FALLBACK",
                    true,
                    &["oauth:openai-codex", "oauth:claude-code"],
                ),
                slot("KC", true, &["keychain:ocean/X"]),
                slot(
                    "KC_THEN_LAPSED",
                    true,
                    &["keychain:ocean/X", "oauth:openai-codex"],
                ),
            ],
            tmp.path(),
        );
        let by_name: BTreeMap<_, _> = statuses.iter().map(|s| (s.name.as_str(), s)).collect();
        assert_eq!(by_name["FRESH"].status, "resolved");
        assert_eq!(by_name["LAPSED"].status, "expired");
        assert_eq!(
            by_name["KEYED"].status, "resolved",
            "no expiry means a static key"
        );
        assert_eq!(by_name["NONE"].status, "missing");
        assert_eq!(by_name["STRING"].status, "missing");
        assert_eq!(by_name["FALLBACK"].status, "resolved");
        assert_eq!(
            by_name["FALLBACK"].resolver.as_deref(),
            Some("oauth:claude-code")
        );
        assert_eq!(by_name["KC"].status, "resolver_not_open");
        assert_eq!(
            by_name["KC_THEN_LAPSED"].status, "expired",
            "expired outranks not-open"
        );
        let rendered = serde_json::to_string(&statuses).unwrap();
        assert!(!rendered.contains("SECRET"), "{rendered}");
    }
}
