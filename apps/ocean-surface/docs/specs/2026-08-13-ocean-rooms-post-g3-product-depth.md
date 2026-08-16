# Ocean Rooms — Post-G3 Product-Depth Program

**Date:** 2026-08-13  
**Status:** Operator-authorized successor to landed TASK-9–12, Gate 2, TASK-49, and G3; Slice R1 in implementation  
**Product owner:** Ocean Surface  
**Runtime owner:** Ocean OS  
**Shared collaboration owner:** Ocean Bedrock

## Decision

Rooms must become a first-class multi-human, multi-agent collaboration product at the same quality bar as Ocean's direct chat. A share link is the admission rail, not the finished product. Pasture is the interaction reference for legible identity, presence, mentions, wake behavior, and sovereign agents; it is not a runtime dependency.

The landed implementation is the accepted architectural baseline, not a prototype to replace: TASK-9–12 established named-agent resolution, durable room SSE, shared web/Tauri UI, and native acceptance; Gate 2 established Bedrock membership/fanout, restart-safe federation, invites, outbox, presence, revocation, and sovereign dispatch; TASK-49 established the current three-rail workspace; G3 hardened exact-roster authorship, agent attribution, deduplicated dispatch, and thread roots. This successor preserves those contracts while closing onboarding, conversation depth, identity, administration, unread, search, attachment, and agent-control gaps required for sustained daily use.

## Non-negotiable product test

Two people on different machines must be able to:

1. create one room and share one ordinary link;
2. review and redeem the invitation in an installed Ocean client;
3. add agents owned by their respective local Ocean installations;
4. exchange durable human and agent messages, threads, attachments, reactions, and mentions;
5. wake exactly the intended agent once;
6. understand who owns each agent and whether it is available, thinking, blocked, awaiting permission, or offline;
7. disconnect, restart, and recover transcript, membership, unread position, pending delivery, and permissions without losing or duplicating work.

The program is not complete until this test passes on two real machines.

## Ownership boundary

### Surface

Owns room creation/join/share flows, confirmation, message rendering, composer interactions, roster and agent controls, notifications, unread projections, search UX, accessibility, and responsive behavior. Surface never invents membership, presence, delivery, agent state, or permission success.

### Local Ocean daemon

Owns local durable room projection, local identity, locally owned agent inventory, room-agent bindings, permission-gated turns, ordered local delivery, reconnect/replay, and Bedrock bridging. Core remains a generic execution authority; it does not become a remote fleet scheduler.

### Bedrock

Owns shared room identity, invitations, membership/roles, ordered shared record, presence leases, revocation, and cross-machine fanout. It never receives model credentials, local tool authority, or private agent state beyond the reviewed public projection.

### Sovereign-agent rule

A participant may contribute only an agent owned by that participant's Ocean. Remote participants may request or mention that agent through the shared room, but cannot spawn, configure, authorize, or directly execute it. Tool permissions remain local and visible to the owning human.

## Accepted baseline and remaining truth

Already implemented:

- persistent room records, rosters, transcripts, and room sequence numbers;
- room-scoped snapshot plus SSE replay/live delivery;
- Bedrock invite creation/redemption primitives;
- federated membership, presence projection, outbox, reconnect, and revocation states;
- local agent inventory and safe-agent registration;
- daemon-owned mention/thread trigger evaluation and agent reply attribution;
- a three-rail Surface with room list, transcript, members, composer, threads, and basic agent picker.

Material gaps:

- no finished share/deep-link/redemption journey;
- raw technical identities and weak onboarding;
- no durable unread/read-marker authority;
- no edits, deletes, reactions, pins, attachments, or room search;
- no complete notification and mention center;
- no room administration or invite management;
- no explicit agent capabilities/configuration/state UX;
- no two-machine product acceptance harness;
- oversized UI modules and stale contracts that obscure ownership and behavior.

## Ordered delivery program

Each slice must compile, pass focused tests, receive defect-first review, and preserve compatibility before the next slice starts.

### R1 — Admission rail: create, share, review, redeem

- Add a first-class Share action to an open room.
- Mint an expiring single-use invite through the local daemon.
- Produce an installed-client `ocean://room/<key>/join?code=<code>` link.
- Parse a strict native-link shape without decoding untrusted path structure.
- Stage, display, and require explicit confirmation before redemption.
- Redeem through the local daemon; never place the code in daemon URLs, logs, status text, analytics, or crash output. The successful daemon response additively includes `room_key`; Surface opens that daemon-authoritative room and treats the link path as untrusted routing decoration.
- Open the joined room and recover failures without consuming UI state incorrectly.
- Follow with an HTTPS landing/universal-link service once a canonical public Ocean origin and app-association files are assigned. Do not invent a production domain in Surface.

**Gate R1:** two installed clients complete create → copy → open → confirm → redeem → room open; expired, reused, revoked, offline, and unavailable-service paths are legible.

### R2 — Identity and conversation parity

- Replace raw IDs with authoritative display identity and stable avatar projection.
- Group consecutive messages and add day/unread separators.
- Render the same safe Markdown/code/link quality as direct chat.
- Add optimistic pending, delivered, failed, and retry states tied to durable authority.
- Add edit/delete with explicit authored-message and moderation rules.
- Add reactions and thread reply affordances without opening a parallel event log.
- Decompose the monolithic workspace into tested room list, header, timeline, composer, thread, member, invite, and agent-control modules.

**Gate R2:** a room conversation is visually and behaviorally at least equal to direct chat for text and threads.

### R3 — Durable attention

- Add daemon/Bedrock read-marker authority per human and room.
- Project durable unread and mention counts into room list, native badge, and notification center.
- Preserve notification ordering and deduplicate across reconnect/replay.
- Add typing as ephemeral presence, never durable transcript truth.
- Add per-room mute and notification policy.
- Add room-scoped transcript search with bounded server-side pagination.

**Gate R3:** unread, mentions, badges, and search remain correct across two clients, restart, reconnect, and replay.

### R4 — Rich collaboration

- Add attachments with bounded upload/download, metadata, progress, cancellation, retry, and safe preview.
- Add pinned messages and room bookmarks.
- Add drag/drop and paste through the existing safe attachment pipeline where possible.
- Define retention and deletion semantics before adding shared blobs.

**Gate R4:** two clients exchange text, image, and file attachments with durable recovery and no credential leakage.

### R5 — Sovereign agent depth

- Present locally owned agents separately from humans and remote agents.
- Show public capability summary, owner, model label where policy allows, and live state: available, thinking, blocked, awaiting permission, offline.
- Support explicit room binding and removal.
- Support reviewed wake modes: mention, thread participation, component interaction, and schedule. No hidden always-listening default.
- Add per-room instructions, workspace/project binding, model selection, context policy, budget, and local permission policy.
- Surface local permission requests to the owning human; remote members see only a sanitized blocked/waiting projection.
- Ensure one confirmed shared trigger produces at most one locally owned agent turn and one attributed reply.

**Gate R5:** each of two humans contributes one locally owned agent; cross-owner mentions work while configuration, credentials, and permissions remain local.

### R6 — Administration and trust

- Add owner, moderator, member, guest, and agent roles with explicit capability matrices.
- Add invite listing, expiration, revocation, reusable-link policy, and audit history.
- Add remove, ban, leave, archive, reopen, and delete journeys.
- Add room name, topic, icon, project association, and retention settings.
- Make every destructive or authority-changing operation confirmed and auditable.

**Gate R6:** role changes, revocation, removal, archive, reconnect, and stale-client behavior fail closed.

### R7 — Pasture-grade presence and cross-device finish

- Make presence lease-backed and visually legible without pretending that historical membership is online presence.
- Add last-seen and reconnect states with privacy controls.
- Complete responsive phone/tablet layouts and accessible keyboard/focus behavior.
- Add canonical HTTPS invitation landing, app install/open handoff, and association files.
- Exercise Mac, web/PWA, and supported remote/tailnet paths.

**Gate R7:** the full non-negotiable product test passes on real machines and the room remains understandable under offline/recovering/revoked states.

## Stop conditions

Pause a slice when it would:

- expose invite codes, bearer tokens, private model credentials, prompts, tool arguments, or local filesystem details to Bedrock or logs;
- let a remote participant execute/configure a locally owned agent or approve its tools;
- introduce a parallel room event authority beside the durable transcript/access contracts;
- report presence, delivery, unread, membership, or agent state that no authority can prove;
- require an invented production domain, unratified storage service, or hidden fleet scheduler;
- weaken replay, idempotency, revocation, generation, or permission invariants.

## Verification baseline

For every Surface slice:

```bash
cargo fmt --all -- --check
cargo test -p ocean-surface-ui
cargo check -p ocean-surface-ui --target wasm32-unknown-unknown
cargo clippy -p ocean-surface-ui --target wasm32-unknown-unknown -- -D warnings
```

Daemon/store/Bedrock slices add their owning tests, compatibility checks, migrations, restart tests, and cross-machine acceptance harness. A visual pass must compare Rooms against the current direct-chat quality bar at desktop and compact widths; unit tests alone do not establish product quality.
