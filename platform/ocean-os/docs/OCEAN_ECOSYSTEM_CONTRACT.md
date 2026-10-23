# Ocean Ecosystem Contract

Status: active contract for runtime and first-party surfaces.

## Terms

- `Project`: repo/product identity. A project is the durable named thing Ocean recognizes, configures, and caretakes.
- `Workspace`: one concrete checkout/worktree/local directory on disk. A workspace has a path and may belong to a project.
- `Session`: one daemon-owned agent/human work thread with a persisted workspace binding that an explicit later request may rebind under current workspace-resolution rules.
- `Surface`: a UI/client attached to a session, such as the Tauri desktop app, Chrome extension, web/PWA, TUI, ACP, CLI, or voice.
- `Canvas`: a tldraw/CRDT document visible from a surface. Concurrent
  operator+agent edits converge via the per-component version-vector merge
  (OCEAN-258); see `docs/OCEAN_CANVAS_CONVERGENT_MERGE.md`.
- `LiveKit room`: the real-time audio/video/data collaboration container.
- `Longhouse`: the reasoning/federation plane.

## Invariants

1. A product surface chooses or creates a session before it submits a turn.
2. A session stores its workspace root, but every turn request still carries the required `cwd` field. A caller may send an explicit cwd, or an empty cwd together with `project_id`; an empty cwd without a project is rejected. A valid new binding may intentionally rebind and persist the resumed session.
3. Product surfaces subscribe with `GET /v1/agent/events?session_id=<id>`.
4. Product surfaces submit turns with `session_id=<id>`.
5. SSE events never change a surface's active session, and never cross between
   sessions. A subscriber on `?session_id=<id>` receives only that session's
   events. Only user attach/select or explicit session creation can change a
   surface's active session.
   - **Extension-event exception.** `AgentTurnEvent::Extension` events (e.g.
     Longhouse council events) carry an optional `scope`. A council spans many
     agents/sessions, so it often has no single owning session. Delivery rules:
     - `scope = Some(session_id)` — treated exactly like any session-bearing
       event: delivered only to that session's subscribers (and `?all=1`).
     - `scope = None` (council-wide / global-by-design) — delivered **only** to
       subscribers who opt into the global stream via `?all=1`. It is **never**
       delivered to a session-scoped subscriber. This keeps the no-crossing
       guarantee intact: a council never leaks into an unrelated session's
       transcript; it reaches the deck (which subscribes globally) by design.
   - **Verified against `crates/ocean-daemon/src/main.rs:3453` (`should_emit_agent_event`)
     on 2026-06-06.** The live SSE filter matches this contract exactly:
     `(Some(want), Some(sid)) => sid == want` (a scoped subscriber sees only its
     own session), `(Some(_), None) => false` (a council-wide `scope = None`
     event is never delivered to a scoped subscriber), and `(None, _) => all`
     (any session-bearing or council-wide event requires the explicit `?all=1`
     firehose opt-in). `Extension` scope routing is implemented by
     `AgentTurnEvent::session_id()` returning `*scope` for `Extension` events
     (`crates/ocean-agent-sdk/src/lib.rs:487`).
6. The global `/v1/agent/events` stream (no `session_id`, opt in with `?all=1`)
   is for debug, the Longhouse deck, and legacy clients only. Session-bearing
   events and council-wide extension events are delivered there only when
   `?all=1` is set.
7. `client_type` describes render/communication medium. It is not a session id, workspace id, LiveKit participant id, or canvas id.

## Runtime Flow

```text
Project -> Workspace -> Session -> Turns -> Events
Surface -> Session
```

New first-party surface flow:

```text
choose/create Project or local directory
choose/create Workspace
POST /v1/agent/sessions
GET /v1/agent/events?session_id=<id>
POST /v1/agent/turns { session_id, prompt, cwd, project_id?, client_type }
```

`POST /v1/agent/sessions` is **live** in the daemon today. It explicitly
allocates and persists a session before the first turn instead of relying on the
implicit create-on-turn path. Shape:

```text
POST /v1/agent/sessions
  { "workspace_root": "<path>",       # required; resolved to git toplevel if inside a repo
    "project_id": "<uuid>",           # optional; falls back to the project's workspace_root
    "client_type": "surface-tauri" }  # optional render/communication medium

-> { "session_id": "<id>",
     "cwd": "<resolved working dir>",
     "client_type": "surface-tauri" }
```

The returned `session_id` is then carried on `GET /v1/agent/events?session_id=<id>` and on every `POST /v1/agent/turns`. Resuming does not make `cwd` optional: each turn must provide it, or provide an empty value with `project_id` so the daemon can resolve the project workspace. A different valid binding intentionally rebinds the persisted session.

Two surfaces intentionally sharing one session both subscribe to the same `session_id`.
Two surfaces on different sessions cannot receive each other's session-bearing events.

### Turn request shape

The full `AgentTurnRequest` accepted by `POST /v1/agent/turns` (source of truth:
`crates/ocean-agent-sdk/src/lib.rs`, `struct AgentTurnRequest`):

```text
POST /v1/agent/turns
  { "prompt": "<operator instruction>",   # required
    "cwd": "<working directory>",         # required (may be empty when project_id is set)
    "session_id": "<id>",                 # optional; a new session is created if omitted
    "client_type": "surface-tauri",       # optional render/communication medium
    "project_id": "<uuid>",               # optional; with empty cwd the daemon binds the
                                          #   turn to the project's workspace_root
    "guidance": ["focus on tests"],       # optional list of guidance hints
    "thinking_level": "high",             # optional ThinkingLevel; per-turn reasoning-effort
                                          #   override, applied to this turn only (does not
                                          #   mutate the runtime's global thinking_level)
    "model_id": "claude-opus-4-7" }       # optional; per-turn / per-session model override
                                          #   (OCEAN-36). Drives this turn only, leaving the
                                          #   runtime's global model selection untouched, so
                                          #   independent client windows can each pin a model
                                          #   without racing through POST /v1/model.
```

Every field except `prompt` and `cwd` is optional. `thinking_level` and
`model_id` are per-turn overrides: they never mutate global runtime state, which
is what lets two ACP/surface windows run different models or reasoning efforts
against the same daemon concurrently. (`model_id` is OCEAN-36; source of truth
`crates/ocean-agent-sdk/src/lib.rs`, `AgentTurnRequest::model_id`.)

### Events of note

Beyond turn lifecycle deltas (`TurnStarted`, `AssistantTextDelta`,
`ToolCall*`, `TurnFinished`), two event families have surface-facing contracts:

- **`BrowserActivity { session_id, active }`** — emitted so the Chrome
  extension side panel can auto-focus while Ocean drives the browser and release
  afterward. Per OCEAN-77 the emission rule is contract-honest: every browser
  tool that performs a **live browser action** (a CDP round-trip to the running
  Chrome) emits `BrowserActivity { active: true }`. This includes the
  read-only-but-live tools `browser_list_tabs` (enumerates tabs over CDP) and
  `browser_response_body` (issues `Network.getResponseBody`). Two tools are
  **exempt** because they read a purely in-memory buffer with no CDP round-trip
  (the live action that populated the buffer already flagged activity):
  `browser_captured_requests` (netcap snapshot) and `browser_downloads`
  (download-tracking snapshot). Source of truth:
  `crates/ocean-runtime/src/tools/browser/mod.rs` (module-level contract comment
  + `active_result`). `BrowserActivity` is session-bearing and obeys Invariant 5.
- **`Extension { extension, payload, scope }`** — the catch-all for council /
  extension events; its `scope`-based delivery is the Invariant 5 exception
  documented above.

## Persistent Rooms (OCEAN-65)

The **persistent `Room`** is Ocean's durable collaboration entity. A `Room`
owns a free-form id (`RoomKey`), a name, a participant roster, created/updated
timestamps, an append-only transcript, and an optional trigger policy. Its
lifecycle is namespaced under `/v1/rooms/persistent`. Source of truth:
`crates/ocean-core/src/lib.rs` (`Room`, `RoomKey`, `RoomParticipant`,
`RoomMessage`, `RoomTriggerPolicy`, `RoomTriggerEvent`, `evaluate_trigger_policy`)
and `crates/ocean-store` (`RoomStore` trait for lifecycle, inherent access-projection and outbox APIs, `RoomStoreError`).

Routes (all registered by `room_routes()` in `crates/ocean-daemon/src/main.rs`; JSON routes use typed `{ ok, error }` bodies with 400 on a bad key and 404 on an unknown or soft-closed room; SSE is streaming; retry returns strict status codes). The table below is the COMPLETE room surface — one row per registered method and path, fifty-eight of them, in the order `room_routes()` registers them — and it is executable, not descriptive: `router_contract_room_route_table_and_architecture_counts_are_in_parity` in `crates/ocean-daemon/src/main.rs` fails if the router registers a `/v1/rooms` route this table does not carry, if this table carries a row the router does not register (a wrong method is such a row), or if the rows fall out of registration order. `GET`/`PUT /v1/me`, the daemon owner identity, share the router group but are not room routes; `OCEAN_RUNTIME_OPERATOR_GUIDE.md` documents them beside the full daemon quick reference.

The **Auth** column names what a caller must present BEYOND reaching the daemon. `Loopback trust only` is the honest reading of most rows: the daemon is a local trust boundary with no HTTP authentication of its own (see `OCEAN_BIND` and the browser trust set in `OCEAN_RUNTIME_OPERATOR_GUIDE.md`), so on those routes reaching the socket IS the grant. Three global layers apply to every row and are not repeated per row: every request needs an allowlisted `Host` (421 `host_not_allowed`, `crates/ocean-daemon/src/host_guard.rs`); every non-GET/HEAD/OPTIONS request is refused when it carries a `Cookie` or a present `Origin`/`Referer` outside that trust set (403 `ambient_credential_rejected` / `foreign_origin_rejected`, `crates/ocean-daemon/src/cross_site_write.rs`), so a browser page cannot drive a roster-asserted write directly; and an absent `Origin` passes, so a write laundered through a surface proxy is that proxy's gate, not this table's. `Roster-asserted <field>` means the caller NAMES an identity and the daemon checks that name against the live roster inside the same store guard as the write, refusing an Agent's or System's identity with a typed 403 — an assertion the room can refuse, not a credential the client holds. `X-Ocean-Operator` is the header-only operator key (`crates/ocean-daemon/src/room_operator.rs`): a cookie-bearing or foreign-origin request is refused on its shape before the value is compared, a wrong key is 403 `operator_credential_invalid`, and where the header is the only authority a missing header or an unconfigured key file is 503 (`operator_credential_missing` / `operator_identity_unavailable`) rather than a fall back to loopback trust. `+ decision_id` marks the operator mutations that also demand a replay-safe decision id (a replay with different content is a 409). `Daemon-held room bearer` marks the federation lanes: the room's Bedrock credential stays in this process and no client-supplied bearer is ever read.

| Method | Path | Purpose | Auth |
| --- | --- | --- | --- |
| `GET` | `/v1/rooms/persistent` | List open rooms as one bounded page (`?limit=&cursor=`, with `next_cursor` and `has_more`), most recently updated first, plus per-room `read_states` and the additive owner-local `muted`. | Loopback trust only; the read-cursor principal is daemon-derived, never caller-supplied |
| `POST` | `/v1/rooms/persistent` | Create a room from `{ key, name, trigger_policy?, workspace_root? }`; a policy enabling `on_ci_failure` is refused (400 `trigger_policy_unwired`) and a `workspace_root` that does not resolve to an existing absolute directory is 400 `invalid_workspace_root`. | Loopback trust only |
| `GET` | `/v1/rooms/persistent/{key}` | Fetch one OPEN room: its bounded first transcript page with `has_more`/`next_seq`, access projection, retirement aliases and `agent_owners`; a soft-closed room is 404. | Loopback trust only |
| `PATCH` | `/v1/rooms/persistent/{key}` | Update `{ name?, trigger_policy?, workspace_root? }` on the absent-vs-`null` contract (absent is unchanged, `null` clears the policy or unbinds the workspace); an unknown field is 400 `invalid_request`; open rooms only; create's CI refusal applies. | Loopback trust only |
| `GET` | `/v1/rooms/persistent/{key}/inspect` | Bounded identity read: room id and name, `closed`, the local owner member id and the capped alias projection; no transcript, workspace or credential data. | Loopback trust only |
| `POST` | `/v1/rooms/persistent/{key}/close` | Soft-close the room: append its durable close marker, cancel its active turns and stop its federation task; the frozen room stays readable through `/snapshot` and `/transcript`. | `X-Ocean-Operator` when the header is present (a presented credential is never downgraded to the member lane; refusal is 403, an unconfigured key 503), otherwise roster-asserted `?actor_id=` (400 when absent, 403 `forged_closer` for an Agent or System identity) |
| `GET` | `/v1/rooms/persistent/{key}/agents` | List the room's durable agent bindings with the owner member and its eligibility. | Loopback trust only |
| `POST` | `/v1/rooms/persistent/{key}/agents` | Authorize one exact room member / package / digest binding with its activation, context, memory and capability-grant policies. | `X-Ocean-Operator` + `decision_id`; a missing header or unconfigured key is 503 |
| `POST` | `/v1/rooms/persistent/{key}/agents/bootstrap` | Atomically establish or verify the Local room owner plus the package-derived Agent roster tuple, authorizing no execution. | `X-Ocean-Operator`; a missing header or unconfigured key is 503 |
| `GET` | `/v1/rooms/persistent/{key}/agents/preview/{package_id}` | Non-authorizing preview of a package's digest, grantable and unavailable capabilities, and any existing member or binding. | Loopback trust only |
| `GET` | `/v1/rooms/persistent/{key}/agents/{agent_member_id}` | Inspect one binding (404 `agent_binding_not_found`). | Loopback trust only |
| `POST` | `/v1/rooms/persistent/{key}/agents/{agent_member_id}/reauthorize` | Approve the current full package digest and bump the authority generation; older-generation turns are cancelled. | `X-Ocean-Operator` + `decision_id` |
| `POST` | `/v1/rooms/persistent/{key}/agents/{agent_member_id}/suspend` | Suspend a binding and cancel its older-generation in-flight turns. | `X-Ocean-Operator` + `decision_id` |
| `POST` | `/v1/rooms/persistent/{key}/agents/{agent_member_id}/resume` | Resume a binding under a new authority generation. | `X-Ocean-Operator` + `decision_id` |
| `POST` | `/v1/rooms/persistent/{key}/agents/{agent_member_id}/revoke` | Terminally revoke a binding and cancel its in-flight turns. | `X-Ocean-Operator` + `decision_id` |
| `POST` | `/v1/rooms/persistent/{key}/agents/{agent_member_id}/invoke` | Queue one explicit turn for a bound agent from a durable same-room message (`{ invoked_by, message_seq, decision_token? }`); 202 with the admission, request and session ids. | Roster-asserted `invoked_by` — a local non-Agent, non-System participant or a federated `User` member — who authored `message_seq` (403 `invoke_membership_required` / `invoke_author_mismatch` / `invoke_message_not_found`); the binding must be admissible (409 `agent_binding_required`) |
| `POST` | `/v1/rooms/maintenance/run` | Run the store-wide retention and attachment-orphan sweep now and answer with its report. | `X-Ocean-Operator`; there is no room to be a member of, so a missing header or unconfigured key is 503 |
| `POST` | `/v1/rooms/persistent/{key}/participants` | Join: add one participant to the roster and append the `ParticipantJoined` marker. | Loopback trust only; a `human` join is always the daemon owner (body `id`/`display_name` ignored), `system` is refused (400 `forged_participant_kind`), and an `agent` must resolve to a folder agent whose `owner_id` is a Human already on the roster |
| `DELETE` | `/v1/rooms/persistent/{key}/participants/{participant_id}` | Leave: remove one participant and cancel the room requests the roster no longer admits. | Loopback trust only |
| `POST` | `/v1/rooms/persistent/{key}/participants/{participant_id}/retire` | Retire a placeholder human (`surface-operator`, `web-<16 hex>`) into a live member: moves the owner role and agent ownership, records an alias, appends an audit row. | `X-Ocean-Operator` + `decision_id` |
| `POST` | `/v1/rooms/persistent/{key}/attachments` | Upload one file as raw bytes (8 MiB cap, 413 `attachment_too_large`) with its metadata in the query, and a transcript marker in the same transaction. | Roster-asserted `?uploader_id=` (403 `forged_attachment_author` for an Agent or System identity; an off-roster id is refused by the store) |
| `GET` | `/v1/rooms/persistent/{key}/attachments` | List the room's attachments, metadata only; a soft-closed room is served as an audit read with `closed: true`. | Loopback trust only |
| `GET` | `/v1/rooms/persistent/{key}/attachments/{attachment_id}` | Download the verified bytes under a content type DERIVED from the bytes, never the declared one, with `nosniff` and `Content-Disposition: attachment`. | Loopback trust only |
| `DELETE` | `/v1/rooms/persistent/{key}/attachments/{attachment_id}` | Remove the row and then its bytes, recording in the transcript who removed it. | Roster-asserted `?actor_id=` (403 `forged_attachment_author`) |
| `GET` | `/v1/rooms/persistent/{key}/resources` | List safe contributed-folder grant projections without local roots. | Loopback trust only |
| `POST` | `/v1/rooms/persistent/{key}/resources` | Grant one validated local folder to room agents with generation-bound access. | `X-Ocean-Operator` + `decision_id` |
| `GET` | `/v1/rooms/persistent/{key}/resources/{resource_id}` | Read one safe grant projection with its effective expiry status (404 `resource_not_found`). | Loopback trust only |
| `DELETE` | `/v1/rooms/persistent/{key}/resources/{resource_id}` | Revoke one grant permanently and advance its generation. | `X-Ocean-Operator` + `decision_id` |
| `POST` | `/v1/rooms/persistent/{key}/resources/{resource_id}/suspend` | Suspend one grant. | `X-Ocean-Operator` + `decision_id` |
| `POST` | `/v1/rooms/persistent/{key}/resources/{resource_id}/resume` | Resume one suspended grant. | `X-Ocean-Operator` + `decision_id` |
| `POST` | `/v1/rooms/persistent/{key}/resources/{resource_id}/list` | Operator preview: a bounded confined listing through the named agent's current binding and per-call resource authority, audited. | `X-Ocean-Operator`; the body's `agent_member_id` must hold an active binding |
| `POST` | `/v1/rooms/persistent/{key}/resources/{resource_id}/read` | Operator preview: one bounded UTF-8 chunk through the same admitted read tool and resource authority, audited. | `X-Ocean-Operator`; the body's `agent_member_id` must hold an active binding |
| `POST` | `/v1/rooms/persistent/{key}/summarize` | Run ONE bounded model turn over the newest transcript rows and fold it into the room's single `room-summary` artifact (create, or compare-and-swap amend in place). | Roster-asserted `requested_by` (403 `forged_artifact_author` for an Agent or System identity, 403 off-roster); 429 `at_capacity` at the turn limit |
| `GET` | `/v1/rooms/persistent/{key}/profile` | Read the references-only room profile and this node's credential-slot status, never a credential value. | Loopback trust only |
| `PUT` | `/v1/rooms/persistent/{key}/profile` | Replace the room profile references with a replay-safe decision and a content-minimal audit row. | `X-Ocean-Operator` + `decision_id` |
| `POST` | `/v1/rooms/persistent/{key}/messages` | Append one chat message (`{ author_kind?, author_id?, body, thread_parent_seq? }`), then evaluate the trigger policy against its @-mentions; in a federated room the daemon enqueues the post to its outbox and answers 202. | Roster-asserted author: a `human` post is authored as the daemon owner (who must be on the roster), a `bot` post names `author_id` (403 `author_not_in_roster`); `agent` and `system` are daemon-only kinds (403 `forged_author_kind`); federated posts ride the daemon-held room bearer |
| `POST` | `/v1/rooms/persistent/{key}/invites` | Mint a federated invite `{ recipient_name?, ttl_minutes? }`, registering a Local room with Bedrock first. | Daemon-held room bearer; a Local room without a credential registers with `OCEAN_FEDERATION_OWNER_TOKEN` (503 `federation_unavailable` when unset), which then becomes that room's stored bearer; neither is ever surface-submitted |
| `POST` | `/v1/rooms/persistent/invites/redeem` | Redeem `{ code }` restart-safely and self-join the room its scope resolves to; raw access projection plus `room_key`. | The invite `code` IS the grant; the minted room bearer stays in the daemon |
| `POST` | `/v1/rooms/persistent/{key}/members/agents` | Register this daemon's safe local agent descriptors `{ agent_names }` with the room's federation peer. | Daemon-held room bearer (409 `federation_conflict` for a room without a credential, 403 `federation_forbidden` when revoked) |
| `GET` | `/v1/rooms/persistent/{key}/transcript` | Read one bounded transcript page (`?after_seq=N&limit=M`) with `next_seq`/`has_more`; a soft-closed room is served as an audit read. | Loopback trust only |
| `POST` | `/v1/rooms/persistent/{key}/artifacts` | Record what the room produced as a task, decision or note, with a System transcript line in the same transaction. | Roster-asserted `author_id` (403 `forged_artifact_author` for an Agent or System identity, 403 off-roster) |
| `GET` | `/v1/rooms/persistent/{key}/artifacts` | List the room's artifacts, most recently changed first. | Loopback trust only |
| `GET` | `/v1/rooms/persistent/{key}/artifacts/{artifact_id}` | Read one artifact: the cheap half of the compare-and-swap loop (404 `unknown_artifact`). | Loopback trust only |
| `POST` | `/v1/rooms/persistent/{key}/artifacts/{artifact_id}/amend` | Rewrite one artifact in place under `expected_version` compare-and-swap (409 `artifact_version_conflict`, never a silent merge). | Roster-asserted `author_id` (403 `forged_artifact_author`, 403 off-roster) |
| `GET` | `/v1/rooms/persistent/{key}/snapshot` | Hydrate the room: entity, roster, one bounded transcript page, `last_seq`/`next_seq`/`has_more`, access projection, aliases and `closed`; a soft-closed room is served as an audit read. | Loopback trust only |
| `GET` | `/v1/rooms/persistent/{key}/runs` | The room's recent agent work cards, oldest first, bounded. | Loopback trust only |
| `POST` | `/v1/rooms/persistent/{key}/runs/{run_id}/permission` | The owner's in-room approve or deny for a run's pending tool permission (`{ permission_id, tool?, decision }`); anything but the run's current pending request is a 409. | `X-Ocean-Operator`; a signed-in first-party Surface host may present the key on the owner's behalf (`room_operator.rs` module doc) |
| `GET` | `/v1/rooms/persistent/{key}/agents/{agent_id}/settings` | Per-room agent overrides (instructions overlay, model); local only, never federated. | Loopback trust only |
| `PUT` | `/v1/rooms/persistent/{key}/agents/{agent_id}/settings` | Replace the per-room agent overrides; empty fields clear them. | `X-Ocean-Operator`; a signed-in first-party Surface host may present the key on the owner's behalf |
| `GET` | `/v1/rooms/persistent/{key}/search` | Case-insensitive chat-message body search (`?q=&limit=`), newest first. | Loopback trust only |
| `GET` | `/v1/rooms/inbox` | Mentions of, and thread replies to, the local owner across the most recently active open rooms (`?limit=`). | Loopback trust only; "me" is daemon-derived |
| `GET` | `/v1/rooms/persistent/{key}/prefs` | The owner's per-room prefs `{ muted }`; local only, never federated. | Loopback trust only |
| `PUT` | `/v1/rooms/persistent/{key}/prefs` | Replace the owner's per-room prefs. | Loopback trust only (no operator gate in source) |
| `GET` | `/v1/rooms/persistent/{key}/events` | Merged SSE: the initial full `room_access` projection, then id-bearing `room_message` replay from `?after_seq=` or `Last-Event-ID`, plus the access, read-cursor and agent-run tails; open non-`call:` rooms only. | Loopback trust only |
| `GET` | `/v1/rooms/persistent/{key}/read-cursor` | The daemon-owned read-cursor projection for a Local room, or the mirrored upstream cursor for a Live room; other access states are 409 `room_read_cursor_unsupported`. | Loopback trust only; the Live mirror rides the daemon-held room bearer (403 `membership_revoked` from upstream) |
| `PATCH` | `/v1/rooms/persistent/{key}/read-cursor` | Advance the read cursor `{ read_seq }` monotonically and publish the `room_read_cursor` wake. | Loopback trust only; same Live-room bearer rule as the read |
| `POST` | `/v1/rooms/persistent/{key}/outbox/retry` | Durably requeue one locally-authored federated event `{ client_event_id }` still awaiting Bedrock confirmation; 202, and no network call of its own. | Loopback trust only (403 `room_access_revoked` for a revoked room) |
| `POST` | `/v1/rooms/{room_id}/livekit-token` | Mint a LiveKit join token for web in-room voice and video. A `call:` id must name a server-authored room that still EXISTS and is open (404 otherwise); any other id is admitted with no store check, which is what lets a fresh surface room get a token before it is durable. | Subscribe by default; PUBLISH requires `X-Ocean-Publish-Token` or `Authorization: Bearer` matching `OCEAN_LIVEKIT_PUBLISH_TOKEN`, and fails closed when that secret is unset |

**Transcript** is a flat, append-only event log of `RoomMessage` entries, each
carrying author attribution (`author_id`, `author_kind`), a `kind`
(`Message` / `ParticipantJoined` / `ParticipantLeft` / `System`), a body, and a
store-assigned monotonic `seq` so clients can request `after_seq` tails.

**Trigger-policy evaluation** is the pure, I/O-free
`evaluate_trigger_policy(policy, event) -> TriggerDecision` in `ocean-core`. The
optional `RoomTriggerPolicy` gates each `RoomTriggerEvent` variant one-for-one:
`on_mention` ↔ `Mention`, `on_thread_reply` ↔ `ThreadReply`,
`on_component_event` ↔ `ComponentEvent`, `on_schedule` (cron) ↔ `Schedule`. An
absent policy (`None`) never convenes. A positive `TriggerDecision`
(`should_convene: true`, optional `target_participant`, human-readable `reason`)
is the seed for auto-convene; the daemon evaluates it at the transcript/
component-event wiring point.

**Access projection** is a typed, store-owned `RoomAccessProjection` (a struct, not a tagged variant) returned on room detail (`GET /v1/rooms/persistent/{key}`) and snapshot (`GET /v1/rooms/persistent/{key}/snapshot`). Access states are exact `local`, `connecting`, `live`, `recovering`, and `revoked`. `members` and `outbox` are skip-when-empty struct fields, not variant-confined fields. Outbox stays separate and never enters the confirmed transcript before Bedrock confirmation. Rooms without an access row (including the frozen soft-closed fixture) default exact `local`.

**Pending outbox** is durable per-room storage of locally-authored federated events awaiting Bedrock confirmation. `POST /v1/rooms/persistent/{key}/outbox/retry` accepts `{ "client_event_id": "<id>" }` and returns 202 on successful durable requeue, 403 revoked, 404 for an unknown room or item, 409 pending/local, 400 for malformed/non-object body, or sanitized store 500 on internal error. Outbox durability lives in `ocean-store`; the retry route still owns only HTTP validation, store lookup, and wake publication — it performs no network call itself.

**Federation bridge (S2 P2-B/P2-C).** `ocean-daemon::room_federation` owns one AppState-held supervisor with one cancellable task tree per credentialed room. It subscribes to Bedrock's header-bearer room SSE from the store cursor, commits a safe roster before first ingest, maps authenticated message rows through the store's atomic `ingest_confirmed_event`, and publishes transcript/access hints only after commit. A duplicate publishes nothing; non-message rows advance only the access cursor. The sender periodically scans the durable Pending outbox and posts exact one-at-a-time producer tuples; a `201` is only remote-persistence acknowledgement and leaves transcript/outbox unchanged until ordered SSE confirms the full tuple. Transport/429/5xx preserve Pending; row/schema conflicts fail that row; auth/scope failures revoke the room with Pending cleanup before Revoked persists. No store guard crosses network I/O. Missing/invalid `OCEAN_FEDERATION_URL` moves credentialed rooms to Recovering.

P2-C splits Local and federated intent authority at credential installation. Local messages retain immediate 201 append/trigger behavior; federated human posts ignore browser identity, and bound-agent replies use the opaque bound member id. Both allocate only a durable Pending outbox row, return/use 202, and await ordered SSE confirmation before transcript append or trigger claim. Mention ids resolve only against the current safe member projection. Confirmed claims require positive current User roster evidence and local mention policy; dispatch then revalidates a current safe locally-owned Agent roster member plus its private binding, while fallback-Human and Agent authors claim nothing. The claim journal commits before an unbounded nonblocking daemon dispatch channel, giving at-most-once local execution without P2-B-era backfill. Federated agent replies re-enter the outbox and local-only audit/failure rows are not appended.

The daemon-only control plane adds owner invite creation, idempotent `{code, redemption_id, token}` recovery plus bodyless self-join, and ordered safe-agent registration. `OCEAN_FEDERATION_OWNER_TOKEN` is used only to register a Local room and is never surface-submitted. Every pending redemption survives restart; existing rooms start first, then a concurrency-four worker attempts every listed triple once. Registration keys are domain-separated SHA-256 over stable instance/room/agent identity and cross only in the daemon-to-Bedrock agent batch alongside the allowlisted public description/model/skill-count/subagent-name fields. Bearers cross only in daemon-authenticated Bedrock requests. Neither bearer nor registration key enters a surface request, projection, transcript, log, or error; local paths/tools/provider credentials and permission posture never cross Bedrock.

**Merged SSE** (`GET /v1/rooms/persistent/{key}/events`) delivers both `room_access` and `room_message` frames on a single stream. The initial frame is a full `room_access` projection (no `event.id`) reflecting the committed access state; `room_message` replay follows via `Last-Event-ID` (numeric or 400, wins over `after_seq`). Post-commit, the tail sends access-update frames on a dedicated `RoomAccessWakeBus` and unchanged id-bearing message frames on `RoomWakeBus` — separate buses so heavy transcript traffic never back-pressures access subscribers. Both bus wake hints are payload-free; relevant and lagged hints reread from SQLite. Message gap recovery pages ascending; access dedup compares the full projection. Unknown/closed rooms return 404; `call:` rooms return the typed unsupported rejection.

## Implementation Anchors

Runtime side:

- `crates/ocean-agent-sdk/src/lib.rs`: `AgentSessionCreateRequest`,
  `AgentSessionCreateResponse`, `AgentTurnEvent::session_id`.
- `crates/ocean-agent/src/lib.rs`: session creation and workspace binding.
- `crates/ocean-daemon/src/main.rs`: `POST /v1/agent/sessions`, surface turn
  guard, and scoped SSE filtering.

Surface side:

- `../ocean-surface/crates/ocean-surface-ui/src/daemon.rs`: shared
  web/extension/Tauri daemon client, scoped event URL, and
  create-session-before-turn flow.
- `../ocean-surface/crates/ocean-tauri/src/lib.rs`: native shell composition
  around the shared Surface UI.
- `../ocean-surface/crates/ocean-surface-proxy/src/main.rs`: forwards
  `POST /v1/agent/sessions` and preserves `?session_id=...` on SSE.

## Legacy Notes

Older docs may discuss TUI rooms, tmux layouts, or room guidance. Treat those as
TUI/runtime reference material unless they explicitly point back to this
contract. Archived client implementation plans are not current architecture.
