//! Read-only Room cwd/catalog and per-call contributed-folder authority.
//! No grant/profile mutation, preview, writer or route is activated here.
use crate::persistent_rooms::persisted_room_workspace;
use chrono::Utc;
use ocean_core::RoomKey;
use ocean_store::{ResourceAccessMode, ResourceStatus, RoomStore, RoomStoreError};
use std::path::{Path as FsPath, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TurnCwd {
    /// A live grant that authorizes the agent; the root stays private.
    ResourceGrant {
        resource_id: String,
        generation: u64,
        cwd: String,
    },
    /// `Room.workspace_root`, already public on the room record.
    RoomWorkspaceRoot { cwd: String },
    /// Nothing usable: the turn is refused with `workspace_unavailable`.
    Unbound,
}

impl TurnCwd {
    pub(super) fn cwd(&self) -> Option<&str> {
        match self {
            Self::ResourceGrant { cwd, .. } | Self::RoomWorkspaceRoot { cwd } => Some(cwd),
            Self::Unbound => None,
        }
    }
}

/// Manifest §5, as ruled in §11.4. A grant is usable as a cwd only if it is
/// `available`, authorizes the agent, and its root still canonicalizes to
/// itself as a directory — the same liveness bar `Room.workspace_root` meets.
pub(super) fn resolve_turn_cwd(
    store: &mut ocean_store::SqliteRoomStore,
    room: &RoomKey,
    agent_member_id: &str,
) -> Result<TurnCwd, RoomStoreError> {
    let now = Utc::now();
    let profile = store.room_profile(room)?;
    let candidates = profile
        .as_ref()
        .map(|p| {
            p.agent_defaults
                .get(agent_member_id)
                .cloned()
                .into_iter()
                .chain(p.default_resource_id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for resource_id in candidates {
        let Some(grant) = store.room_resource_grant(room, &resource_id)? else {
            continue;
        };
        if !grant.admits(agent_member_id, ResourceAccessMode::List, now) {
            continue;
        }
        if let Some(cwd) =
            persisted_room_workspace(&grant.local_root).filter(|cwd| cwd == &grant.local_root)
        {
            return Ok(TurnCwd::ResourceGrant {
                resource_id: grant.resource_id,
                generation: grant.generation,
                cwd,
            });
        }
    }
    let workspace = store
        .get(room)?
        .and_then(|record| record.room.workspace_root)
        .as_deref()
        .and_then(persisted_room_workspace);
    Ok(match workspace {
        Some(cwd) => TurnCwd::RoomWorkspaceRoot { cwd },
        None => TurnCwd::Unbound,
    })
}

pub(super) struct DurableRoomResourceAuthority {
    pub(super) authority: crate::room_agent_authority::RoomOperationAuthority,
    /// Fixed `agent` classification for the admitted turn's audit row.
    pub(super) actor: &'static str,
}

fn needed_mode(op: ocean_agent::RoomResourceOp) -> ResourceAccessMode {
    match op {
        ocean_agent::RoomResourceOp::List => ResourceAccessMode::List,
        ocean_agent::RoomResourceOp::Read => ResourceAccessMode::Read,
    }
}

#[async_trait::async_trait]
impl ocean_agent::RoomResourceAuthority for DurableRoomResourceAuthority {
    async fn resolve(
        &self,
        scope: &ocean_agent::RoomResourceScope,
        resource_id: &str,
        op: ocean_agent::RoomResourceOp,
    ) -> Result<ocean_agent::ResolvedResource, ocean_agent::RoomResourceError> {
        use ocean_agent::RoomResourceError as E;
        if scope.room_key() != self.authority.room.as_str()
            || scope.agent_member_id() != self.authority.member
            || scope.binding_generation() != self.authority.generation
        {
            return Err(E::StaleGeneration);
        }
        let room = RoomKey::new(scope.room_key());
        crate::persistent_rooms::with_rooms_handle(&self.authority.rooms, |store| {
            if self.authority.cancel.is_cancelled() {
                return Err(E::StaleGeneration);
            }
            let live = crate::room_agent_authority::current_binding_on(
                store,
                &room,
                scope.agent_member_id(),
                scope.binding_generation(),
            )
            .map_err(|_| E::Unavailable("room_store_unavailable".into()))?
            .is_some_and(|binding| binding.agent_definition_digest == self.authority.digest);
            if !live {
                return Err(E::StaleGeneration);
            }
            let grant = store
                .room_resource_grant(&room, resource_id)
                .map_err(|_| E::Unavailable("room_store_unavailable".into()))?
                .ok_or(E::NotFound)?;
            let now = Utc::now();
            if grant.effective_status(now) != ResourceStatus::Available {
                return Err(E::NotAvailable);
            }
            if !grant.authorizes_agent(scope.agent_member_id()) {
                return Err(E::AgentNotAuthorized);
            }
            if !grant.access_mode.allows(needed_mode(op)) {
                return Err(E::ModeNotGranted);
            }
            if !FsPath::new(&grant.local_root).is_absolute() {
                return Err(E::NotAvailable);
            }
            let root = grant.local_root;
            if self.authority.cancel.is_cancelled() {
                return Err(E::StaleGeneration);
            }
            Ok(ocean_agent::ResolvedResource {
                local_root: PathBuf::from(root),
                grant_generation: grant.generation,
            })
        })
    }

    async fn record(
        &self,
        scope: &ocean_agent::RoomResourceScope,
        fact: ocean_agent::RoomResourceAuditFact,
    ) {
        let room = RoomKey::new(scope.room_key());
        let result = crate::persistent_rooms::with_rooms_handle(&self.authority.rooms, |store| {
            store.append_room_resource_audit(
                &room,
                ocean_store::RoomResourceAuditInput {
                    resource_id: fact.resource_id,
                    agent_member_id: scope.agent_member_id().to_string(),
                    binding_generation: scope.binding_generation(),
                    grant_generation: fact.grant_generation,
                    op: fact.op.as_str().to_string(),
                    relative_path_digest: fact.relative_path_digest,
                    bytes: fact.bytes,
                    entries: fact.entries,
                    outcome: fact.outcome,
                    actor: self.actor.to_string(),
                },
                Utc::now(),
            )
        });
        if result.is_err() {
            tracing::warn!(room = %room, error_code = "resource_audit_failed", "room resource audit row not recorded");
        }
    }
}

/// Every grant that admits `agent` for at least `list`, as the catalog the
/// tool description shows. Empty means the turn gets no resource tools.
pub(super) fn admitted_resource_catalog(
    store: &mut ocean_store::SqliteRoomStore,
    room: &RoomKey,
    agent_member_id: &str,
) -> Result<Vec<ocean_agent::RoomResourceCatalogEntry>, RoomStoreError> {
    let now = Utc::now();
    Ok(store
        .room_resource_grants(room)?
        .into_iter()
        .filter(|grant| grant.admits(agent_member_id, ResourceAccessMode::List, now))
        .map(|grant| ocean_agent::RoomResourceCatalogEntry {
            resource_id: grant.resource_id,
            display_name: grant.display_name,
            // Phase 2d exposes list/read only; report what the tools can do,
            // not the recorded intent.
            access_mode: if grant.access_mode.allows(ResourceAccessMode::Read) {
                "read".into()
            } else {
                "list".into()
            },
        })
        .collect())
}

/// Rooms Phase 2 Stage 2b: the profile PUT names repo/tool references that
/// become live grants in Stage 2c; refuse a profile that references a grant
/// which does not exist or is revoked.
pub(super) fn check_profile_resource_refs(
    store: &mut ocean_store::SqliteRoomStore,
    room: &RoomKey,
    refs: impl IntoIterator<Item = String>,
) -> Result<(), crate::room_agent_authority::ApiError> {
    let now = Utc::now();
    for resource_id in refs {
        let grant = store
            .room_resource_grant(room, &resource_id)
            .map_err(crate::room_agent_authority::ApiError::from)?;
        match grant {
            Some(grant) if grant.effective_status(now) != ResourceStatus::Revoked => {}
            _ => {
                return Err(crate::room_agent_authority::ApiError::bad_request(
                    "resource_not_found",
                ))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocean_store::{GrantRoomResourceInput, PutRoomProfileInput, SetResourceStatusInput};
    use std::collections::BTreeMap;

    fn grant(
        store: &mut ocean_store::SqliteRoomStore,
        room: &RoomKey,
        root: &FsPath,
        decision: &str,
    ) -> ocean_store::RoomResourceGrant {
        store
            .grant_room_resource(
                room,
                GrantRoomResourceInput {
                    display_name: decision.into(),
                    local_root: std::fs::canonicalize(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    access_mode: ResourceAccessMode::Read,
                    authorized_agent_member_ids: vec!["helper".into()],
                    expires_at: None,
                    granted_by: "fixture-operator".into(),
                    decision_id: decision.into(),
                    request_digest: decision.into(),
                },
                Utc::now(),
            )
            .unwrap()
            .0
    }

    #[test]
    fn cwd_prefers_authorized_agent_default_then_room_default_then_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["agent", "room", "workspace"] {
            std::fs::create_dir(tmp.path().join(name)).unwrap();
        }
        let mut store = ocean_store::SqliteRoomStore::open_in_memory().unwrap();
        let room = RoomKey::new("cwd-precedence");
        let workspace = std::fs::canonicalize(tmp.path().join("workspace"))
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        store
            .create_in_workspace(
                room.clone(),
                "Cwd",
                Some(workspace.clone()),
                None,
                Utc::now(),
            )
            .unwrap();
        let agent_grant = grant(
            &mut store,
            &room,
            &tmp.path().join("agent"),
            "agent-default",
        );
        let room_grant = grant(&mut store, &room, &tmp.path().join("room"), "room-default");
        store
            .put_room_profile(
                &room,
                PutRoomProfileInput {
                    repos: vec![],
                    tools: vec![],
                    credential_slots: vec![],
                    default_resource_id: Some(room_grant.resource_id.clone()),
                    agent_defaults: BTreeMap::from([(
                        "helper".into(),
                        agent_grant.resource_id.clone(),
                    )]),
                    updated_by: "fixture-operator".into(),
                    decision_id: "profile-defaults".into(),
                    request_digest: "profile-defaults".into(),
                },
                Utc::now(),
            )
            .unwrap();
        assert!(
            matches!(resolve_turn_cwd(&mut store, &room, "helper").unwrap(), TurnCwd::ResourceGrant { resource_id, .. } if resource_id == agent_grant.resource_id)
        );
        store
            .set_room_resource_status(
                &room,
                &agent_grant.resource_id,
                SetResourceStatusInput {
                    status: ResourceStatus::Suspended,
                    actor: "fixture-operator".into(),
                    decision_id: "suspend-agent".into(),
                    request_digest: "suspend-agent".into(),
                },
                Utc::now(),
            )
            .unwrap();
        assert!(
            matches!(resolve_turn_cwd(&mut store, &room, "helper").unwrap(), TurnCwd::ResourceGrant { resource_id, .. } if resource_id == room_grant.resource_id)
        );
        store
            .set_room_resource_status(
                &room,
                &room_grant.resource_id,
                SetResourceStatusInput {
                    status: ResourceStatus::Suspended,
                    actor: "fixture-operator".into(),
                    decision_id: "suspend-room".into(),
                    request_digest: "suspend-room".into(),
                },
                Utc::now(),
            )
            .unwrap();
        assert_eq!(
            resolve_turn_cwd(&mut store, &room, "helper").unwrap(),
            TurnCwd::RoomWorkspaceRoot {
                cwd: workspace.clone()
            }
        );
        assert_eq!(
            resolve_turn_cwd(&mut store, &room, "different-agent").unwrap(),
            TurnCwd::RoomWorkspaceRoot { cwd: workspace }
        );
        std::fs::remove_dir(tmp.path().join("workspace")).unwrap();
        assert_eq!(
            resolve_turn_cwd(&mut store, &room, "helper").unwrap(),
            TurnCwd::Unbound
        );
    }

    #[cfg(unix)]
    #[test]
    fn readonly_catalog_omits_roots_and_cwd_refuses_root_symlink_substitution() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let mut store = ocean_store::SqliteRoomStore::open_in_memory().unwrap();
        let room = RoomKey::new("cwd-substitution");
        store.create(room.clone(), "Cwd", None, Utc::now()).unwrap();
        let resource = grant(&mut store, &room, &root, "default");
        store
            .put_room_profile(
                &room,
                PutRoomProfileInput {
                    repos: vec![],
                    tools: vec![],
                    credential_slots: vec![],
                    default_resource_id: Some(resource.resource_id.clone()),
                    agent_defaults: BTreeMap::new(),
                    updated_by: "fixture-operator".into(),
                    decision_id: "profile".into(),
                    request_digest: "profile".into(),
                },
                Utc::now(),
            )
            .unwrap();
        let catalog = admitted_resource_catalog(&mut store, &room, "helper").unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].access_mode, "read");
        assert!(admitted_resource_catalog(&mut store, &room, "other-agent")
            .unwrap()
            .is_empty());
        assert!(matches!(
            resolve_turn_cwd(&mut store, &room, "helper").unwrap(),
            TurnCwd::ResourceGrant { .. }
        ));
        std::fs::rename(&root, tmp.path().join("moved-root")).unwrap();
        symlink(&outside, &root).unwrap();
        assert_eq!(
            resolve_turn_cwd(&mut store, &room, "helper").unwrap(),
            TurnCwd::Unbound
        );
    }
}
