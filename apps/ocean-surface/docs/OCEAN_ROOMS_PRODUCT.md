# Ocean Rooms — Product Contract

Ocean Rooms is a persistent, multi-human + multi-agent collaboration surface.
One room = one named space with a participant roster, an append-only transcript,
and durable state owned by the daemon. The web and desktop surfaces render the
same room data through the same API contract.

The active ordered rebuild and acceptance gates are defined in
`specs/2026-08-13-ocean-rooms-post-g3-product-depth.md`. This document describes
the enduring product contract; it does not imply that every capability is
already delivered.

**Reference only, never dependencies:** Buzz's VISION_SOVEREIGN + VISION_AGENT
docs, Stitchpad's daemon-side room loop, Slack/Discord interaction patterns.

**Design direction:** `OCEAN_ROOMS_DESIGN_DIRECTION.md` (2026-09-09) is the
accepted-for-review direction for the Rooms surface — identity, transcript vs
activity, the room details drawer, agent authoring, and the slice plan. It
supersedes the rendering bullets in this document; the API contract below
stays authoritative for the wire.

**Wire truth:** the routes, bodies and states below are what
`crates/ocean-surface-ui/src/rooms.rs` sends and the daemon's `room_routes()`
(`platform/ocean-os/crates/ocean-daemon/src/main.rs`) serves.
`crates/ocean-surface-ui/tests/rooms_product_doc_truth.rs` holds this document
to that code.

---

## Room Lifecycle

### 1. Creating a Room

A human creates a room from the surface:

```
POST /v1/rooms/persistent
{ "key": "my-room", "name": "My Room", "trigger_policy": { ... }, "workspace_root": "/path/to/project" }
-> 201 { "ok": true, "room": { ... } }
```

That is `CreateRoomBody` in `rooms.rs`; `trigger_policy` and `workspace_root`
are omitted when unset. The `key` is derived from the name (`slugify`:
lower-kebab); the daemon keys rooms by it and refuses a duplicate.
`workspace_root` is optional — when set, agent turns in this room resolve their
project and `cwd` from it. It is resolved on the machine running the daemon
(an absolute existing directory, else 400 `invalid_workspace_root`). The daemon
can rebind or unbind it later through `PATCH /v1/rooms/persistent/{key}`
(`workspace_root`; `null` unbinds), which this surface does not call. This
surface sends no `trigger_policy` (see "Agent Wake Policy").

Creating does not join: the roster starts empty, the surface opens the new
room, and the header's Join action is the next step.

The daemon responds with a `Room` entity including the participant roster,
timestamps, trigger policy, `muted` and `workspace_root`.

### 2. Joining a Room

A human joins the open room (`join_open` in `rooms.rs`):

```
POST /v1/rooms/persistent/{key}/participants
{ "kind": "human" }
-> { "ok": true, "room": { ... } }
```

That is `JoinBody`: a human join carries no id or display name. Identity is
daemon-owned — the daemon joins its owner, the `participant_id` and
`display_name` from `GET /v1/me`, and ignores any id a client sends. The join
appends a `participant_joined` row to the transcript, which reaches every
subscriber as a `room_message` frame; on that row (and on `participant_left`)
the surface re-reads `GET /v1/rooms/persistent/{key}` for the roster.

Leaving is `DELETE /v1/rooms/persistent/{key}/participants/{id}` with the
owner's `participant_id` (`leave_open`); it appends the matching
`participant_left` row.

Federated membership is invitation-gated. Owners mint expiring invites through
the daemon-owned outbound route; recipients open a strict native or universal
link, review the pending room invitation, and redeem the code through a local
daemon POST (see "Inviting Another Human"). Invite codes are bearer material
and never belong in logs, status text, daemon URL paths, analytics, or ordinary
room projections.

### 3. Opening a Room

A surface opens a room by key (`open_room` in `rooms.rs`):

```
GET /v1/rooms/persistent/{key}
-> { ok, room, transcript, has_more, next_seq, access, agent_owners, aliases }
```

`transcript` is the room's first transcript page from seq 0 (the store's
1000-row cap; `has_more`/`next_seq` say when the room is longer). The surface
reads it once and never pages `/transcript` or `/snapshot` itself: the live
tail below resumes from the last row it holds and the daemon replays every
newer row on it. A soft-closed room answers 404 here.

The transcript hydrates once from that response, then the surface subscribes
to the room-scoped SSE endpoint (`start_live_tail`):

```
GET /v1/rooms/persistent/{key}/events?after_seq=<last held seq>
```

Replay starts strictly after `after_seq`; the daemon also honours a
`Last-Event-ID` header, which wins when both are present, so a browser
auto-reconnect resumes correctly too. Four frames arrive on this stream:
`room_message` (`id` = the row's `seq`), `room_access` (the current projection
is the first frame on every connection, then every change), `room_read_cursor`
(the durable read cursor) and `room_agent_run` (agent work-card projections;
recent runs are replayed on connect). Messages advance the resume cursor; the
other three replace state without advancing it. The tail is the room's sole
forward-ingest path: join, leave, agent add and post responses never re-read
`/transcript`. The surface must **never** consume the global agent-event stream
for room data — rooms have their own event namespace.

### Access States

Every successful open carries a required `access` projection
(`RoomAccessProjection` in `rooms.rs`). Its `state` is exactly one of the five
values of `RoomAccessState`:

- `local` — the room lives on this daemon only, with no federation. Writable.
- `connecting` — federated, link coming up. Banner "Connecting to federated
  room…"; the composer is held.
- `live` — federated and connected. Writable.
- `recovering` — federated, link dropped and coming back. Banner "Recovering
  connection…"; the composer is held.
- `revoked` — this daemon's member was removed from the federated room. Banner
  "Access revoked"; the composer is held and agents cannot be added.

The gates live in `crates/ocean-surface-ui/src/rooms_workspace/access.rs`:
`access_allows_writes` (composer, join, leave) holds for `local` and `live`;
`access_allows_thread_writes` (thread replies) for `local` only;
`access_allows_sharing` for `local`, or `live` when the daemon's
`caller_member_id` names a projected User with the Owner role. The banner
strings come from `AccessStateBanner` in `banners.rs`. Before a projection
arrives the surface holds no access state at all — that absence means
"loading or no open room" and is never a stand-in for a local room.

### 4. Sending Messages

A participant sends a chat message into an open room (`post_message` →
`PostMessageBody` in `rooms.rs`):

```
POST /v1/rooms/persistent/{key}/messages
{ "author_kind": "human", "body": "@builder fix the map component", "thread_parent_seq": 42 }
```

A human post carries no `author_id`: the daemon authors it as its owner, who
must be on the roster. `thread_parent_seq` is omitted for a root message, and
thread replies are composed only in `local` rooms. There is no mention list on
the wire: a mention is the `@id` text in `body` (inserted by the composer's
mention picker) and the daemon parses mentions itself. In a `local` room the
daemon appends the row, broadcasts the `room_message` frame and evaluates the
trigger policy; the tail delivers the row, so the surface starts no re-read
after the POST.

In a federated room the same POST is queued to Bedrock instead and answers
`202 { ok, access }`. Items waiting for confirmation ride on the access
projection's `outbox` and render outside the confirmed transcript
(`.rooms-workspace__outbox-item`, "Pending" or "Failed"); only a failed item
offers "Retry", which posts
`POST /v1/rooms/persistent/{key}/outbox/retry { "client_event_id": … }` and
applies the returned access projection behind the room-generation guard.

### 5. Leaving / Closing

Closing a room on the surface (`close_room`) bumps the room generation and
clears transcript, access and tail state in one synchronous reset; the daemon
keeps the room and its transcript. Leaving the roster is the participants
DELETE above. The daemon's `POST /v1/rooms/persistent/{key}/close` (soft
close) is not called by this surface.

---

## Agent Participation

### Binding an Agent to a Room

Agents are selected from the daemon-owned `/v1/agents` identity catalog — never
created as free-text. Which route adds one depends on the room's access state
and the host (`rooms::agent_add_route`):

- `local` room, native consent host (Tauri): the shell's operator-gated
  bootstrap, `POST /v1/rooms/persistent/{key}/agents/bootstrap`
  `{ owner_member_id, agent_package_id }`, records this daemon's owner and
  returns a frozen consent preview; the consent sheet then authorizes the
  binding through `POST /v1/rooms/persistent/{key}/agents`.
- `local` room, browser or extension: the plain participant route,

  ```
  POST /v1/rooms/persistent/{key}/participants
  { "id": "builder", "display_name": "builder", "kind": "agent" }
  ```

  The daemon validates that `id` resolves to a known agent (else 400
  `agent_unresolved`). This creates no binding.
- `connecting`, `live` or `recovering` room:
  `POST /v1/rooms/persistent/{key}/members/agents { "agent_names": [...] }`,
  which registers the agent with Bedrock and answers the refreshed access
  projection.
- `revoked` room: refused without a request.

Roster membership alone does not let an agent run. Every wake — auto-convene
or explicit invoke — is admitted against the room's agent bindings
(`GET /v1/rooms/persistent/{key}/agents`): without an active binding the daemon
refuses with `agent_binding_required`, and the binding's activation policy
must permit the trigger kind. Bindings are minted only through the native
consent ceremony in `crates/ocean-tauri/src/room_consent.rs` (preview,
authorize, reauthorize, revoke, each under the operator key), so browser and
extension hosts render binding status but cannot authorize.

### Agent Wake Policy

Each room carries an optional `RoomTriggerPolicy` (`ocean_core`), all flags
off by default:

```json
{
  "on_mention": true,
  "on_thread_reply": true,
  "on_component_event": false,
  "on_build_failure": false,
  "on_ci_failure": false,
  "on_schedule": null
}
```

A room with no policy at all wakes nobody ("no trigger policy configured").
`on_mention` and `on_thread_reply` are evaluated on every non-agent-authored
post in a `local` room: each `@id` in the body, then the thread root's author
when the post is a reply; a positive decision that resolves to an agent
participant is admitted against its binding and queued as a turn, once per
agent per row. `on_build_failure` fires from workspace build-failure ledger rows
(`room_federation.rs`). `on_ci_failure` is a marker only: a create with it set
is refused with 400 `trigger_policy_unwired`. `on_component_event` and
`on_schedule` are stored and `ocean_core` would honour them, but nothing in the
daemon produces those events. In a federated room, mentions arrive as claimed
trigger targets on the Bedrock ledger and are admitted against the binding's
activation policy rather than this policy.

The policy is set in the create body or edited through
`PATCH /v1/rooms/persistent/{key} { "trigger_policy": … }` (`null` clears it;
`on_ci_failure` is refused the same way). This surface sends neither, so rooms
created here carry no policy and their agents run only through the explicit
invoke below. The surface's `RoomTriggerPolicy` mirror in `rooms.rs` still
lacks the two failure flags.

### Agent Turns in Rooms

The surface posts no agent turns for rooms, and room messages never go through
the session turn route (`POST /v1/agent/turns`). Posting a message is the whole
client-side path: the daemon evaluates the trigger policy against the new row
and, for an admitted agent, runs the turn itself (`spawn_room_agent_turn`) with
the turn folder resolved from the agent's folder, then the room's folder grant,
then `workspace_root`; an unbound room records a `workspace_unavailable`
outcome instead of running. The reply lands like any other row, as a
`room_message` frame authored `agent`, and agent-authored rows never trigger
anyone (no ping-pong).

A human can also wake a bound agent on a specific message from the message
action rail (`rooms::invoke`):

```
POST /v1/rooms/persistent/{key}/agents/{agent_member_id}/invoke
{ "invoked_by": "<owner participant id>", "message_seq": 42 }
```

`invoked_by` must be a non-agent roster member who authored that row; the
daemon checks it, not the client. The surface sends one invoke per
(agent, message) and leaves a lost acknowledgement `unknown`, never retried.
Each run is projected on the tail as a `room_agent_run` frame and rendered as a
work card (`room_work_card.rs`); a card waiting on a tool decision takes it
through `POST /v1/rooms/persistent/{key}/runs/{run_id}/permission`, and a run
this page started can be cancelled through
`POST /v1/requests/{request_id}/cancel`.

---

## Surface Contract

### Browsing Rooms

The rooms rail lists the rooms the daemon serves:

```
GET /v1/rooms/persistent
-> { ok, rooms, read_states, next_cursor }
```

The daemon pages this route (`limit`, default 100, and `cursor` = the previous
page's `next_cursor`); the surface sends no query and renders the first page
only. `read_states` carries each room's latest and read sequence, which the
rail turns into unread dots.

Each room row (`rail_room_option` in `room_list.rs`) shows the `#` name and an
unread dot (`.rooms-workspace__room--unread`; muted rooms never show one).
Rows are grouped by `workspace_root`, unbound rooms last under "Other rooms".
The "Add room" disclosure offers Create room and Join room (invitation-link
entry). Participant counts and last-activity timestamps are not rendered in
the rail.

The rail is the left column of `rooms_workspace/`. Its list is
`.rooms-workspace__left-list` (`styles/rooms-workspace.css`), which keeps
`min-height: 0` with `overflow-y: auto` — long room lists scroll instead of
pushing the create field and status line outside the viewport.

### Transcript Rendering

Messages render as a scrolling transcript (`rooms_workspace/mod.rs`) with the
density rules in `crates/ocean-surface-ui/src/room_messages.rs`:
- Author display name (`rooms::author_display_name`) and an initials avatar
  with a deterministic hue per author id.
- Timestamp as the wire clock, `HH:MM` taken from the daemon's RFC 3339 UTC
  value (`canonical_wire_clock_time`); the full value sits in the `<time>`
  element's `datetime` and `title`. A value that is not canonical RFC 3339
  shows unchanged. Not relative, and not converted to the viewer's zone.
- Day separators (`.rooms-workspace__day-separator`) open the transcript and
  mark each change of UTC day, labelled "Today", "Yesterday" or the
  `YYYY-MM-DD` date; a silence over 15 minutes earns a time header.
- Consecutive messages from one author within 5 minutes on the same day group
  under one header (`.rooms-workspace__msg--grouped`).
- Join, leave and other system rows are compact single lines
  (`.rooms-workspace__msg--system`); routine convene, bootstrap and admission
  audit rows are hidden, refusals and access changes stay visible.
- Message bodies render as markdown (`room_markdown.rs`); `@id` mentions of
  roster members render as `.rooms-md__mention` pills.
- Agent work cards render under the message whose thread convened them; they
  are run projections, not transcript rows.
- Federated outbox items render below the confirmed transcript (see "Sending
  Messages").

**Live-follow is intent-aware:** the transcript follows new messages while the
reader is at/near the bottom (`transcript_follow_pinned`, written by the
transcript's own scroll events). Scrolled-up history reading is never yanked;
a `↓ New messages` button (`.rooms-workspace__jump-new`) returns and re-pins,
and a "New" divider (`.rooms-workspace__unread-divider`) marks the reader's
left-off point.

### Composer

The message composer (`rooms_workspace/composer.rs`) is enabled only while
`access_allows_writes` holds — `local` or `live` access. It is a single-line
input in a form: Enter (or Send) submits, and there is no multi-line entry.
It supports:
- Plain text, rendered as markdown
- @-mention autocomplete (`mentions.rs`) from the daemon-authoritative roster —
  `Room.participants` in `local` rooms, the access projection's members
  elsewhere — accepted with Enter or Tab
- One-level thread replies (`thread_parent_seq`), composed in `local` rooms only
- The federated outbox's Pending/Failed state with retry on failure

### Federated Rooms

A room becomes federated when an invite is minted on it or redeemed into it
(see "Inviting Another Human"); from then on its access state is
`connecting`, `live`, `recovering` or `revoked`, the members rail shows the
access projection's member list instead of the local roster, and posts go
through the outbox above. The access projection returned by an outbox retry
applies immediately behind the room-generation guard.

---

## Onboarding Flow (Human)

### First Join — Web Surface

1. Operator navigates to the Ocean surface PWA (or desktop app).
2. Operator authenticates (username/password → HttpOnly session cookie).
3. Surface opens to the default chat view.
4. Operator opens the Rooms workspace once the daemon endpoint has resolved.
5. The rail calls `GET /v1/rooms/persistent` and `GET /v1/me` — the room list
   and the owner identity render.
6. Operator clicks a room → `GET /v1/rooms/persistent/{key}` hydrates the room,
   its transcript page and its access projection; `GET .../agents` loads the
   bindings.
7. The live tail opens on `GET /v1/rooms/persistent/{key}/events?after_seq=…`.
8. Transcript renders; composer enables if access is `local` or `live`.
9. Operator types a message, hits Enter → `POST .../messages`; the row arrives
   on the tail (in a federated room, as a pending outbox item until Bedrock
   confirms).

### First Join — Desktop (Tauri)

Same flow, but the Tauri shell loads the identical `dist/` bundle and reaches
the daemon's room endpoints directly rather than through the PWA proxy.

### Inviting Another Human

Invites and redemption are live. The room overflow's Share action mints one
(`create_invite` in `rooms.rs`):

```
POST /v1/rooms/persistent/{key}/invites
{}                                   # the daemon also accepts recipient_name and ttl_minutes (default 1440, max 10080)
-> 201 { "code", "expires_at", "room_key", "room_name" }   # the invite raw, no {ok} envelope
```

A daemon without federation configured answers 503, shown as "Room sharing is
not configured on this Ocean." Minting on a `local` room registers it with
Bedrock, which federates it. The code is a bearer grant: it lives in one signal
and the open panel, which shows the link, its expiry and a copy control — never
a log line or the rail.

The other human redeems it from the rail's Join room entry or an
`ocean://room/{key}/join?code=…` link, either of which is staged for explicit
confirmation first (`redeem_invite`):

```
POST /v1/rooms/persistent/invites/redeem
{ "code": "…" }
-> 200 { "state", …, "room_key" }    # a RoomAccessProjection plus the joined room's key
```

The surface opens the room named by `room_key` — the key in the link is
routing decoration, not authority. 403 means the invitation is invalid or
expired, 409 that this room is already linked differently, 503 that sharing is
not configured; the code never enters status text.

---

## Onboarding Flow (Agent)

### Porting an Existing Agent

An agent identity already registered with the daemon (visible in
`GET /v1/agents`) is added from the room's Members rail:

1. Human opens Members from the room overflow.
2. Human picks the agent in the `+ agent` picker (daemon identities only;
   unresolved entries stay disabled).
3. The surface takes the route for the room's access state and host (see
   "Binding an Agent to a Room"): bootstrap + consent on Tauri, the
   participants POST with `kind: "agent"` on browser and extension hosts, or
   the members/agents registration in a federated room.
4. The daemon validates the identity and appends the join row; the roster and
   mention autocomplete update from the tail. On Tauri the consent sheet then
   authorizes the binding; elsewhere the agent is on the roster but unbound,
   and its row says so.

### Configuring Agent Wake Behavior

There is no wake-policy control in this surface. The daemon takes a policy in
the create body or on `PATCH /v1/rooms/persistent/{key}`; this surface sends
neither, so agents in rooms created here are woken by the explicit invoke on a
message, not by mentions.

### Agent's First Turn in a Room

1. Human writes a message in a room whose agent is bound (authorized), and
   either mentions it in a room that carries `on_mention` or picks it in the
   message's invoke menu.
2. The daemon admits the agent against its binding, resolves the turn folder
   from the room's `workspace_root` (an unbound room records
   `workspace_unavailable` instead) and runs the turn itself.
3. A `room_agent_run` frame projects the run as a work card while it runs; a
   tool that needs approval surfaces Approve/Deny on the card.
4. The reply appears as a room message with the agent's participant id, on the
   same stream as human messages.

### Agent Session Identity

The daemon owns the agent session a room turn runs in: each `room_agent_run`
projection carries its `session_id`, `request_id` and the triggering message
`seq`, and the surface reads a card's steps from `GET /v1/sessions/{id}` only
when the card is opened. Session attribution is daemon-derived: a room post
carries no `session_id`, so a client can never attribute its post to a session.

---

## What Rooms Are NOT (G1)

- **Not a replacement for the chat/PTY session surface.** Rooms are additive —
  a collaboration layer for teams. The solo agent chat session remains the
  primary coding surface.
- **Not a real-time voice/video space.** LiveKit controls stay outside the room
  lifecycle until explicitly reintroduced behind a reviewed platform contract.
- **Not federated by default.** A room starts `local` to one daemon. It
  federates through Bedrock only when an invite is minted on it or redeemed
  into it.
- **Not a file-sharing surface yet.** Room messages carry text. The daemon
  serves room attachments (`/v1/rooms/persistent/{key}/attachments`), but this
  surface does not call those routes.
- **Not a project management tool.** No kanban, no issue tracker, no sprints.
  Rooms carry conversation and agent turns. Workflows that need structured
  tracking belong in the Longhouse or the agent session surface.

---

## Daemon API Summary

The routes this surface calls, with the module that calls them. Rows marked
"Tauri shell" are sent by `crates/ocean-tauri/src/room_consent.rs` under the
operator key; the page only asks the shell for them.

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/v1/me` | Daemon owner identity (`rooms.rs`) |
| `GET` | `/v1/rooms/persistent` | List rooms, first page (`rooms.rs`) |
| `POST` | `/v1/rooms/persistent` | Create a room (`rooms.rs`) |
| `GET` | `/v1/rooms/persistent/{key}` | Open: room + first transcript page + access projection; roster re-read (`rooms.rs`) |
| `GET` | `/v1/rooms/persistent/{key}/events?after_seq=` | SSE: `room_message`, `room_access`, `room_read_cursor`, `room_agent_run` (`rooms.rs`) |
| `POST` | `/v1/rooms/persistent/{key}/messages` | Send a message (`rooms.rs`) |
| `POST` | `/v1/rooms/persistent/{key}/outbox/retry` | Retry a failed federated send (`rooms.rs`) |
| `PATCH` | `/v1/rooms/persistent/{key}/read-cursor` | Advance the reader's durable read cursor (`rooms.rs`) |
| `POST` | `/v1/rooms/persistent/{key}/participants` | Join as the owner; add an agent on browser hosts (`rooms.rs`) |
| `DELETE` | `/v1/rooms/persistent/{key}/participants/{id}` | Leave (`rooms.rs`) |
| `POST` | `/v1/rooms/persistent/{key}/members/agents` | Register an agent into a federated room (`rooms.rs`) |
| `GET` | `/v1/rooms/persistent/{key}/agents` | Agent bindings (`rooms.rs`) |
| `POST` | `/v1/rooms/persistent/{key}/agents/{agent_member_id}/invoke` | Explicit invocation (`rooms.rs`) |
| `GET`/`PUT` | `/v1/rooms/persistent/{key}/agents/{agent_id}/settings` | Per-room agent settings (`rooms.rs`; operator key added by the proxy or shell) |
| `POST` | `/v1/rooms/persistent/{key}/runs/{run_id}/permission` | Approve or deny a card's tool request (`rooms.rs`; operator key added by the proxy or shell) |
| `POST` | `/v1/requests/{request_id}/cancel` | Cancel this page's own run (`rooms.rs`) |
| `POST` | `/v1/rooms/persistent/{key}/invites` | Mint an invite (`rooms.rs`) |
| `POST` | `/v1/rooms/persistent/invites/redeem` | Redeem an invite (`rooms.rs`) |
| `GET` | `/v1/rooms/inbox` | Mentions inbox (`room_attention.rs`) |
| `GET` | `/v1/rooms/persistent/{key}/search` | Room message search (`room_attention.rs`) |
| `PUT` | `/v1/rooms/persistent/{key}/prefs` | Per-room mute (`room_attention.rs`) |
| `GET` | `/v1/rooms/persistent/{key}/agents/preview/{package_id}` | Consent preview (Tauri shell) |
| `POST` | `/v1/rooms/persistent/{key}/agents/bootstrap` | First local agent bootstrap (Tauri shell) |
| `POST` | `/v1/rooms/persistent/{key}/agents` | Authorize a binding (Tauri shell) |
| `POST` | `/v1/rooms/persistent/{key}/agents/{agent_member_id}/reauthorize` | Re-authorize a binding (Tauri shell) |
| `POST` | `/v1/rooms/persistent/{key}/agents/{agent_member_id}/revoke` | Revoke a binding (Tauri shell) |
| `GET` | `/v1/agents` | List known agent identities (`rooms.rs`) |
| `GET` | `/v1/sessions/{id}` | Work-card steps for an opened run (`daemon.rs`) |
