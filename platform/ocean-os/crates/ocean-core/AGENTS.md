# ocean-core — Shared Protocol Types

## Purpose

This crate owns shared protocol types used across Ocean clients, daemon, runtime, and SDK surfaces: requests, responses, events, sessions, and common data structures.

## Ownership

- **Scope:** `crates/ocean-core/`
- **Parent contracts:** `../../AGENTS.md` and `../AGENTS.md`
- **Primary responsibilities:** stable shared types, serialization contracts, cross-crate API compatibility

## Local Contracts

- Treat public type changes as cross-crate contract changes.
- Preserve serde compatibility unless the breaking change is intentional and documented.
- Keep protocol types free of daemon/runtime implementation details.
- Session synchronization uses a bounded `SessionSyncSnapshot` plus opaque
  boot-local `SessionEventFence`; it carries the persisted monotonic
  legacy-default-zero model `config_revision`, excludes raw messages, tool
  rows/payloads, thinking, and image metadata, caps visible user/assistant text
  at 512 rows and 1 MiB, and reports truncation counts. `AgentReplayGap` is reset-required and
  never claims ordering/range semantics for UUID event ids.
- `PermissionMode` wire names are stable (`manual`, `automatic`, `skip_all`);
  clients display daemon-reported saved/effective settings rather than deriving
  policy locally.
- `TokenUsage.context_is_floor` is additive and legacy-default false. It marks
  a `context_tokens` reading taken before a failure; consumers must not present
  a marked reading as the final request.
- The closed Track-0 `RoomId` projection family is retired. Durable room contracts use the open `RoomKey` model; do not recreate projection DTOs without a new audited API design.

### Federation types

- `RoomMessage.federated` and optional `attachment_id` remain additive,
  legacy-default absent. Attachment links omit the key when absent;
  `RoomAttachment` is metadata only and never defines filesystem authority.
- `FederatedMessageMeta`, `FederatedRoomMemberProjection`, `FederatedActorType`,
  `FederatedRoomRole`, `MemberPresence` (`live` | `unavailable`; no `stale`),
  `PublicAgentDescriptor`, `RoomOutboxItem`, `OutboxItemState`,
  `RoomAccessProjection`, `RoomAccessState`, `CreateInviteRequest` (`Serialize`
  only), `InviteResponse`, `RedeemInviteRequest` are owned here.
- Every new struct field is required unless individually `#[serde(default)]` or
  `skip_serializing_if`. Do not remove `Serialize`/`Deserialize` from types
  the daemon routes use. Trigger flags `on_build_failure` and `on_ci_failure`
  default off independently; evaluator support does not imply daemon admission
  or an available event source.
- `PublicAgentDescriptor` explicitly transforms live `GET /v1/agents` fields;
  local paths, provider credentials, tool config, and permission posture are
  NEVER included. Forbidden keys (`owner_principal_token_id`,
  `provider_api_key`, `execution_role`, `local_paths`, `tool_config`,
  `permission_posture`) must not survive serde roundtrip on any type.
- `ocean-store` and `ocean-daemon` own their implementation surfaces; this
  crate owns the type definitions + exhaustive serde round-trip/enum/backward
  compat + required-field + forbidden-key tests.
- `RoomAccessProjection.caller_member_id` is optional, legacy-default absent,
  and derived by the store from the daemon's room credential. It contains no
  bearer and participates in projection equality so caller changes survive SSE
  dedup. Browser participant ids and roster roles do not identify the caller.


### Rooms persistence parity

- The persisted Rooms foundation follows running source
  `0abb558179af3ff848d3bd19ac67aff246299ce1`. Store APIs and wire additions
  alone do not restore daemon execution authority or lift the migration hold.
- `bounded_quotable` bounds characters after dropping controls;
  `bounded_prose` additionally strips brackets before the bound so marker names
  cannot author a Markdown destination. Store and daemon marker composers share
  this rule; URL validation must compare against the primitive, not prose.

## Work Guidance

- Prefer explicit fields and stable enums over implicit client-specific conventions.
- Update downstream crates when shared types change.
- Document any migration or compatibility risk in the root `events.md` entry for the work.

## Verification

- `cargo test -p ocean-core`
- `cargo check --workspace`

## Child devlog Index

No child boundaries defined within `ocean-core/` at this time.
