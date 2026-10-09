# Ocean Rooms — Team Platform Amendment

**Date:** 2026-10-02
**Status:** Operator-authorized amendment to `2026-08-13-ocean-rooms-post-g3-product-depth.md`. The operator authorized repair, review and merge preparation of P1–P6 on 2026-10-04. Required builds and the production acceptance gates below still apply.
**Amends:** post-G3 slices R2, R3, R5; `superpowers/specs/2026-07-10-ocean-federated-rooms-design.md` non-goals (narrowly, below)
**Owners:** Surface (rendering, intent), Ocean OS (identity derivation, run projection, room tools), Bedrock (shared record, member display names)

## Decision

Rooms become the team's multi-user agentic coding platform: Slack-style channels where coworkers chat and `@mention` agents that do real work, show live progress, ask follow-up questions, and request approvals inline.

Topology is fixed and not reopened by this amendment:

- Every coworker runs their own Ocean daemon. Their agents act only on that machine's repositories and credentials.
- Shared rooms federate through ocean-bedrock (invite → redeem → join, per-person scoped contributor tokens). No shared host daemon, no creator-hosted room, no daemon-to-daemon mesh.
- A coworker's "login" is their Bedrock membership plus their own Ocean. A daemon's web proxy login protects that one person's daemon from other devices; it is not a team account system and never multiplexes people.

## Narrow non-goal amendments

The federated-rooms design lists "a person/account entity in Bedrock" and "new identity/profile machinery inside ocean-os or ocean-surface" as non-goals. This amendment relaxes them only as follows:

1. **Daemon owner profile (ocean-os).** Each daemon holds one local owner profile: `{ display_name }`, operator-set. It is display data, not an authentication principal, and authorizes nothing.
2. **Bedrock member display name.** `longhouse.room_members.display_name` (already present) is the authoritative cross-machine name. It stays derived from the authenticated principal's token name, which the inviter sets through the invite's `recipient_name` (the owner's comes from their Bedrock admin token). Members cannot choose their own federated name: Bedrock's self-join deliberately accepts no caller identity fields, and letting a member pick a name would reopen impersonation. No new Bedrock table, column, API field, or account entity.
3. **Avatars are derived, not stored.** Avatar = initials of the display name + a stable color index derived from the opaque member/participant id. No upload, no blob, no Bedrock schema migration.

Everything else in those non-goal lists stands.

## Phase map

| Phase | Outcome | Post-G3 slice | Layers |
|---|---|---|---|
| P1 | One shell for web and desktop | (shell, precondition for R2 visual parity) | Surface |
| P2 | Real identity, no impersonation | R2 identity | ocean-os, Surface |
| P3 | Agent work cards | R5 agent state + R2 rendering parity | ocean-os, Surface, Bedrock (event type only) |
| P4 | Agents act inside the conversation | R5 permissions, per-room config, wake modes | ocean-os runtime + daemon, Surface |
| P5 | Rooms visual revamp + module split | R2 decomposition and visual bar | Surface |
| P6 | Attention: inbox, typing, mute, search | R3 | ocean-os, Bedrock, Surface |

R1 (admission rail) is already landed except the HTTPS landing page, which stays deferred until a canonical origin exists. R4 (attachments), R6 (roles/admin), R7 (presence/finish) keep their post-G3 definitions and follow P6.

Rollout order is P1 → P2 → P3 → P4 → P5 → P6. P5 follows P3/P4 deliberately: the work card and in-room approvals define the message anatomy the visual system must serve. Each phase has its own branch and PR, passes the post-G3 verification baseline plus the owning daemon/Bedrock tests, and updates the Surface Rooms Contract before landing.

## P1 — One shell for web and desktop

Both hosts already load one bundle and one stylesheet set. Divergence comes from host-conditional mounts in `app.rs` and host-scoped root classes.

Contract:

- **One shell geometry.** The shell is full-bleed on both hosts and the header spans it. Reading surfaces (transcript, permissions, composer) ride one centered `--content-rail` (tokens.css; replaces `--shell-max`). Tauri's only geometric difference is `--titlebar-inset` padding on the header.
- **One Sessions entry point.** The header Dynamic Island is the Sessions entry on every host. Native Tauri opens its Sessions stage with Cmd/Ctrl+P; web/PWA leaves Cmd/Ctrl+P to browser Print and opens Sessions by selecting the Island. The native-only shortcut hint is hidden on web. The web-only "Sessions" text button is removed; the `/sessions` registry command remains the deep-browse fallback.
- **One panel system.** Files, Repo, and Browser render only through `WorkspacePane` on every host. The web deck rail, its Files panel, and the action-log Browser cockpit are retired; the pane's live screencast is the Browser surface everywhere. Posture is width-driven (docked split ≥900px, overlay below; default open only when the viewport can dock it), never host-driven.
- **Tauri-only chrome** is limited to the traffic-light inset and the drag region. Cmd/Ctrl+P is also native-only because the browser owns Print; this must not split the shared Sessions entry point. Every other host difference is a capability mounted through `host.rs` (native folder pick, path watch, daemon lifecycle), absent rather than erroring off-host.

Gate P1: web (`run-surface.sh`) and Tauri (`run-tauri.sh`) screenshots at the same window width read as the same product on the Rooms and session views.

## P2 — Real identity

Interfaces:

- **ocean-os** `GET /v1/me` → `{ participant_id, display_name }` (ocean-store `daemon_owner`). `participant_id` is derived once from the first display name (`OCEAN_OWNER_NAME`, else `USER`) and never changes; `PUT /v1/me { display_name }` renames the owner and its Local roster rows. The avatar seed is the participant/member id itself.
- **ocean-core** `RoomAccessProjection.local_member_id` (additive, optional): the credential's local human member id, so a surface knows "me" in a federated room without minting identity.
- **ocean-os** room writes derive authorship server-side:
  - Local rooms: human posts are authored by the daemon owner participant. `RoomMessageRequest.author_id/author_kind` become ignored-for-humans (accepted for wire compatibility, never trusted). Agent and system authorship remain daemon-internal only.
  - Federated rooms: human posts are authored by the daemon's `local_human_member_id`; display name comes from the Bedrock member row registered from the owner profile.
  - Participant join for humans likewise derives `{ id, display_name }` from `/v1/me`; self-asserted human joins are rejected.
- **Surface**: delete `RoomIdentity` (`rooms.rs`, localStorage `ocean.room_identity`). Hydrate `/v1/me` at boot; render display names and derived avatars in roster, messages, mentions, threads, and the composer. Mention completion keys on participant/member id, displays the name.

Gate P2: two machines, two humans. Neither can author as the other through Surface or direct daemon calls; names and avatars are correct on both machines, across restart.

## P3 — Agent work cards

An agent turn renders as one live card, threaded under the triggering message.

Interfaces:

- **ocean-core** `RoomAgentRun { run_id, room, agent_id, session_id, trigger_seq, thread_root_seq, state, started_at, updated_at, summary: Option<String>, files_changed: Vec<String>, reply_seq: Option<u64> }` with `RoomAgentRunState = Queued | Thinking | RunningTool { label } | AwaitingPermission | AwaitingReply | Done | Failed { reason } | Cancelled`.
- **ocean-store**: a mutable `room_agent_runs` projection table keyed by `run_id`. It is a projection over the session's events, not a second transcript; the transcript remains append-only.
- **ocean-daemon**: `spawn_room_agent_turn` creates the run at convene, advances `state` from the session's own agent events (tool start/end, permission request/resolution, terminal result), and on completion appends the reply message with `run_id` linkage. A new room SSE frame `room_agent_run` (projection replace, no sequence, same generation guard as `room_access`) carries state changes. `GET /v1/rooms/persistent/{key}/runs` hydrates active and recent runs.
- **Owner detail is local-only.** Tool steps, thinking, diffs, and outputs are read by the owner's Surface from the existing session endpoints for `session_id` (`/v1/agent/sessions/{id}` + scoped events). They never enter the room record.
- **Federation**: the bridge publishes ledger events of type `room.agent_run` with payload `{ run_id, agent_member_id, state, summary?, files_changed_count }`, only. No tool names, arguments, paths, prompts, or outputs. Ledger `event_type` is free-form; no Bedrock schema migration.
- **Surface**: `RoomMessage` deserializes `session_id` and `run_id`. The card reuses transcript components (`Block`, `ToolGroup`, `ThinkingGroup`, `LiveActivityRow`, diff component) and the chat markdown renderer for the summary/reply. Steps are collapsed by default; the header row carries agent, state, elapsed time, and changed-file count. Remote viewers see state + summary only.
- Room replies render with the chat-grade markdown renderer; `room_markdown` remains only for mention resolution inside it.

Gate P3: `@agent fix X` in a room shows live state progression and a changed-files/diff card to the owner, a truthful state-only card to a remote member, and the final reply threaded under the request.

## P4 — Agents act inside the conversation

Interfaces:

- **ocean-runtime tools**, registered only for turns whose session carries a room binding (`client_type: "room"` + `room_key`):
  - `room_post_update { text }` — appends an agent-authored message into the run's thread and sets the run summary line. Rate-limited per run.
  - `room_ask { question }` — appends the question into the run's thread, sets the run to `AwaitingReply`, and ends the turn. The existing `on_thread_reply` trigger (enabled by default for rooms with agents) convenes the same deterministic `(room, agent)` session on the next human thread reply, so context continues. Restart-safe because no in-memory wait is held.
- **In-room approvals**: room turns get a per-run `decision_token` minted by the daemon. Permission requests set the run to `AwaitingPermission` and project `{ tool_label, summary }` to local Surfaces. The owning human approves or denies from the card through `POST /v1/rooms/persistent/{key}/runs/{run_id}/permission { decision }`, which the daemon resolves against the session's pending request. Remote members see only `AwaitingPermission`. A remote member can never approve.
- **Per-room agent settings** (instructions overlay, model, trigger policy) behind the room header's single overflow menu: `GET/PUT /v1/rooms/persistent/{key}/agents/{agent_id}/settings`. Local-only; never federated.

Gate P4: an agent asks a clarifying question, the operator answers in the thread, the agent continues and finishes; a permission request is approved from the room card.

## P5 — Visual revamp

- Dark-first, dense, premium treatment on the OCEAN depth ramp. Colors only in `styles/tokens.css`. Refined type scale, 4px spacing rhythm, hover/focus/active states, reduced-motion-safe motion, `compact.css` parity.
- Split `rooms_workspace.rs` into tested modules: room list, header, timeline, message, work card, composer, thread, members, invite, agent settings.
- UI rules (binding): no emojis, no sparkle/AI ornament, no button sprawl (secondary actions in one overflow), no marketing copy, no instructional labels, icon controls carry `aria-label` and tooltip, reveal-on-intent over permanent chrome.

## P6 — Attention

Per post-G3 R3: activity/mentions inbox across rooms, typing as ephemeral presence, per-room mute, bounded server-side room search.

## Stop conditions (in addition to post-G3)

Stop and ask the operator before:

- any change to federation trust boundaries (what crosses Bedrock, who may author or approve);
- any Bedrock schema migration;
- any change that breaks the Surface Session Contract.
