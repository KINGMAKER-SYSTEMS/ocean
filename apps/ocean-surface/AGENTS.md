# Ocean Surface Agent Guide

Ocean Surface is the product-surface repo. The canonical UI is one Leptos/WASM
app (`crates/ocean-surface-ui`) built once via Trunk and shipped to two hosts:
the browser PWA (served by `crates/ocean-surface-proxy`) and the Tauri native
shell (`crates/ocean-tauri`, macOS first). A Chrome extension wraps the same
bundle. The GPUI desktop app (`crates/ocean-gui`) is ABANDONED (2026-07-21) —
DO NOT work on it: no fixes, no features, no scouting for bugs, no tickets. It
is dead source, frozen for reference only; touching it is wasted effort.

The monorepo component `../../platform/ocean-os` owns runtime authority: daemon, agent loop,
tools, providers, permissions, projects, workspaces, sessions, and events.
`ocean-bedrock` is a private, optional authenticated service for authorized team
deployments; Surface must not require it or expose Bedrock credentials to
browser state. Cross-repo routing and ownership map:
`docs/OCEAN_PROJECT_MAP.md`.

Do not put provider calls, agent reasoning, session storage, permission policy,
or tool execution authority in this repo. Surface code should render state,
collect intent, attach to sessions, and call the daemon.

## Platform Contract (binding — web + desktop + mobile)

`docs/OCEAN_PLATFORM_CONTRACT.md` is the cross-team alignment layer; the
desktop feature split lives in `docs/OCEAN_DESKTOP_NORTH_STAR.md` (Surface
Capability Matrix). The load-bearing rules:

- One core, many shells: `crates/ocean-surface-ui` is the product everywhere;
  shells (proxy/PWA, extension, Tauri desktop, future Tauri mobile) stay thin.
- `src/host.rs` is the ONLY seam for platform capability; every fn no-ops off
  its platform, and UI mounts platform features conditionally — absence, not
  errors.
- Sorting rule: "does the phone version need this?" Yes → shared core (daemon
  HTTP/SSE only, compact/touch rendering in the same slice). No, it's about
  the machine → ocean-tauri command + host.rs wrapper + conditional mount.
- The web core is the bones of the mobile app (Tauri 2 iOS/Android; PWA is
  mobile v0; `compact.css` is the mobile stylesheet; no hover-only
  affordances).
- One shell on every host (team-platform amendment P1,
  `docs/specs/2026-10-02-ocean-rooms-team-platform-amendment.md`): full-bleed
  frame, header spanning it, reading surfaces on the `--content-rail` token;
  the Dynamic Island is the Sessions entry everywhere; `WorkspacePane` is
  the only Files/Repo/Browser surface, docked at ≥900px and overlaying below
  (width-driven, never host-driven). `in_tauri` may gate native titlebar chrome
  (`is-titlebar` inset, drag region) and keyboard bindings that conflict with
  browser commands (Cmd/Ctrl+P and Cmd/Ctrl+Shift+F remain native-only; their hints are absent on web/PWA) — never
  layout, panels, or navigation. Do not reintroduce a web-only deck or per-host
  panel markup.
- Shared-file discipline (`app.rs`): smallest hunks, committed promptly;
  NEVER reference an uncommitted module from a shared file — `mod x;` +
  usage lands only when `x` compiles with passing tests.
- Main must build standalone: verify the pushed tree in a detached worktree
  before pushing (GitButler lanes split hunks across sessions).

## Current Build Focus

```sh
# Web/PWA (canonical UI; builds Trunk bundle + release proxy):
./run-surface.sh
# Native desktop (Tauri; builds and loads the same dist/ bundle):
./run-tauri.sh
```

`run-surface.sh` requires `OCEAN_SURFACE_USER` and `OCEAN_SURFACE_PASS` for its
default LAN/tailnet bind. For trusted localhost diagnostics only, bind
`127.0.0.1` and set `OCEAN_SURFACE_AUTH=off`. Direct `cargo tauri dev` does not
rebuild `dist/`; use `run-tauri.sh` whenever freshness matters.

The public proxy login contract is username/password to an HttpOnly,
SameSite=Strict session cookie. Ordinary browsers and devices must not be
rejected by Origin, Host, forwarded-header, Cloudflare Access, Tailscale, or
device-posture gates. Public HTTPS deployments set
`OCEAN_SURFACE_COOKIE_SECURE=on`; that setting controls only the cookie's
Secure attribute.

Native surface direction:

- Tauri 2.x shell (`crates/ocean-tauri`) loads `dist/` as `frontendDist` — the
  same Trunk-built Leptos WASM bundle the browser PWA ships.
  Windows release builds use the GUI subsystem so app launch opens no console;
  the native probe checks that setting in the built PE executable.
- Rust commands in the Tauri backend replace the GPUI crate's `rfd` (folder
  dialogs) and `notify` (path watcher) native bits.
- Native LiveKit Rust client is a later phase behind a feature flag, not part
  of this batch.
- Ocean daemon stays the session/runtime authority.
- Native Git inspection uses vendored libgit2 without SSH/HTTPS transports.
  It reads local repository state; desktop startup must not require Homebrew
  OpenSSL or other developer-installed libraries.
- The native `.gitattributes` pins `Cargo.toml` to LF. Tauri serializes it with
  LF on every host; Windows checkout conversion must not dirty packaged source.
- Native first-run setup is collected through `host.rs` and the main-window-only
  `daemon_setup`/`daemon_start` commands. `startup_model.rs` shares only the
  runtime provider catalog's metadata; no credential or provider call enters
  Surface. A user-selected catalog route is stored under the desktop app's
  own config directory and supplied only as `OCEAN_DEFAULT_MODEL` to its child.
  Existing environment defaults and daemon-persisted model choices take
  precedence. Blank `OCEAN_MODEL` is removed from the child even without a
  desktop selection, allowing the daemon's persisted/default model to resolve.
  Nonblank explicit choices and existing defaults remain authoritative.
  The desktop never writes daemon configuration or picks a model
  silently. Corrupt/nonregular desktop settings fail visibly without replacement.
  The startup form exists only while native supervision is offline. Initial
  native status must resolve before mounting Rooms; a later disconnect preserves
  the conversation and unsent draft, refuses direct-chat submission, and shows
  a compact restart action. Offline headers expose
  no ready indicators. Ordinary connected model/effort controls remain unchanged.
  Successful startup refreshes model/project state and reconnects the focused
  session with a fresh authoritative projection on the replacement SSE generation.
  Retire prior sync polling without switching focus or clearing the unsent draft;
  an old restoration/recovery reply must not replace the new projection. Full
  session reads claim a separate projection epoch before fetching; only the
  latest read may commit or trigger caller retry/status/poll cleanup, even within
  one SSE generation without a new activity frame. Permission-only reconciliation
  retains its independent epoch and cannot retire a transcript read. Every admitted
  session metadata, transcript, tool or pinned-component mutation advances the
  activity fence before changing snapshot-owned state; wrong-session and suppressed
  frames do not. Admitted steering deliveries bypass the ordinary reducer and must
  advance the same fence immediately before appending their user transcript row;
  pending steer removal does not permit an older snapshot to erase delivery.
  Initial hydration and SSE recovery share four immediate activity-race
  retries, then one epoch/focus-guarded recovery waiting for 250ms of quiet before
  another read. Streaming stays visible, recovery stays pending visibly, and a pause
  or terminal frame permits convergence without replaying ambiguous deltas. Verify
  live-content preservation/coalescing with UI `cargo test -p ocean-surface-ui projection_content`,
  ordering with UI `cargo test -p ocean-surface-ui projection_epoch`, and recovery
  with UI `cargo test -p ocean-surface-ui native_reconnect`, native
  `cargo test --lib startup_model`, native daemon tests, and the real WASM build.
- Native daemon start resolves the operator's `OCEAN_DAEMON_BIN`, then the
  installed executable sibling, then the platform binary on PATH. No IPC
  argument may select an executable. Explicit start/restart remains unchanged.
  Children run from the absolute home/profile (or OS temporary directory),
  never the app's inherited workspace; Windows children have no console window.
  Explicit nonempty relative runtime state/auth/input path overrides are anchored
  to the app's launch directory before changing child cwd. Resolve that directory
  lazily, once, only for relative executable/path overrides; unavailable inherited
  cwd must not reject absolute or PATH launches, and required relative paths fail
  without a substitute base. Absolute paths and
  blank runtime sentinels retain their meaning. The native host environment and
  operating-system home remain unchanged. The allowlist in `lib.rs` covers Ocean
  and XDG state, Ocean/Codex/Claude auth, room/title databases, agent/plugin/
  assistant/skill roots and prompt capture. SQLite `:memory:`, in-memory `file:`
  URIs (including parsed `mode=memory`) and absolute `file:` URIs retain their
  original bytes; relative disk `file:` URIs fail before spawn and require an
  explicit absolute URI. Verify with `cargo test --lib daemon_child`.
  Home/profile also owns native daemon logs. Verify native launch with
  `cargo test --lib daemon_` from `crates/ocean-tauri`.
  A native-process `OCEAN_DAEMON_LOG` may select an absolute log file for
  diagnostics; no webview argument controls that path. Isolated startup
  acceptance keeps config, auth, Claude fallback and logs in disposable paths
  while preserving the operating-system home and the live daemon.
- Explicit app Quit reaps only its owned daemon before exit, releasing the
  bundled executable for updates. Committed exit closes process admission under
  the child lock so queued starts/restarts cannot orphan a replacement. Closing
  the window still hides to the tray; external daemons and shell-crash survival
  are unchanged. Verify with native `cargo test --lib daemon_exit` and an isolated
  native close/reopen/Quit check; never use the operator's live daemon as a fixture.
- Canvas ledger/state injected into turns as surface context; the Leptos canvas
  already lives at `crates/ocean-surface-ui/src/canvas.rs`.
- `docs/OCEAN_GPUI_CANVAS_LIVEKIT_SPEC.md` is retained as a historical reference
  for the canvas+LiveKit design intent; the implementation surface moved to
  Leptos+Tauri.

## Web Surface Design System

All visual work on the Leptos web surface follows
`docs/OCEAN_WEB_SURFACE_DESIGN.md`. Non-negotiables:

- Identity is the OCEAN depth ramp (the TUI splash banner, deep indigo →
  bright aqua) and the accepted circular neumorphic wave mark in
  `public/brand/master-1024.png`. No Rising Tides
  magenta/purple (rejected 2026-07-04), no legacy teal-mint, no gradient-clip
  text, and no prompt/cursor/code-glyph logo direction. The ramp paints solid
  colors on discrete elements or vector fills inside the circular mark.
- Stylesheets are split per domain under `styles/` (tokens → base → chrome →
  transcript → components → composer → panels → call → canvas → compact →
  float, in cascade order). Colors live ONLY in `styles/tokens.css`.
  `extension/sidepanel.html` and `scripts/build-extension.sh` enumerate the
  same files — adding a stylesheet touches all three places.
- Control density is a design defect: conditional rendering over permanent
  chrome, one header overflow (`⋯`) for secondary actions, ghost triggers for
  idle features (dialer, join call), reveal-on-intent for power knobs.

Ocean Floor is the read-only Observatory mode. Its isometric pixel scene,
semantic list, inspector, and replay rail consume only the typed reducer over
metadata-safe `/v1/observatory/{snapshot,events,replay}` responses; no mock
executions, transcript content, inferred activity, or observer write controls.
Every reducer execution owns one constant-footprint cubicle with a stable slot:
snapshot rows preserve server admission order, and a live admission appends the
next slot without resizing or repacking existing modules. Modules pack
grid-adjacent into one connected facility — corridors, doorway partitions, and
the tall boundary envelope derive from present slots only. Furnishings are
static architecture; actor, tool, attention, status, room lighting, and
topology treatments come only from real reducer state. The replay rail never
requests `snapshot?at=<past cursor>`: the daemon snapshots only its current
watermark and answers an earlier `at` with `409 snapshot_not_historical`. A
scrub rebuilds the past client-side — empty state with the slot registry kept,
`/v1/observatory/replay` paged from the earliest retained cursor through the
target, folded by the same reducer (`observatory/replay.rs`). Pruned history
or a skipped event reads as incomplete; a replay the daemon cannot serve reads
as "replay unavailable", never as offline. Animation is truthful:
typing/blink/wave and screen activity render recorded phases, the one-shot
walk-in visualizes a live admission event, and reduced motion stops the loop.
The web proxy reads the daemon-minted private observer token immediately
before each upstream request and injects it server-side. Never expose that
token to browser code, bundle it, or cache it in client storage.

## Windows proxy files

- The Windows proxy uses `ocean_observatory::auth::windows_file` from the public
  runtime component for session-secret creation and observer/operator reads.
  This target-only dependency shares file validation, not execution authority.
  It requires a protected current-user-only ACL and refuses reparse points,
  nonregular files and extra hard links without repairing existing files.
  Unix mode-0600/no-follow behavior remains unchanged.
- Default state and session-secret paths use absolute HOME or USERPROFILE on
  Windows, preserving explicit config/key/secret overrides and Unix HOME rules.
- The session gate and exact owner-mutation route allowlist apply on both
  platforms. No operator credential is exposed to the browser or attached
  when proxy login is disabled. Native Room consent remains Tauri-only.
- Verify the full proxy suite on Windows with `cargo test --locked -p
  ocean-surface-proxy`; the manual Windows probe owns this check. Compilation
  and synthetic transport fixtures do not establish packaged/client acceptance.

## Desktop Accounts

- `src/accounts.rs` owns the native Accounts dialog, reached through the existing
  header overflow. Native dialog focus trapping, Escape and return focus apply.
  Provider rows keep stable identity; pending actions retain keyboard focus with
  `aria-disabled` plus handler admission guards. Restore menu focus after unmount.
  No credential forms, provider calls, login URLs or tokens enter the webview.
- `src/host.rs` calls the main-window-only `provider_accounts` command.
  `crates/ocean-tauri/src/provider_accounts.rs` admits only List, Login and Cancel
  for Claude, Codex and ChatGPT. Cancel carries the exact daemon attempt id;
  no caller chooses a URL, method, path, credential or executable.
- The existing shared native transport attaches the owner-only operator key to
  numeric loopback only. Room owner-mutation routes remain unchanged. Responses
  project only provider/status/attempt fields. Login opens only that provider's
  fixed HTTPS authorization endpoint in the system browser, never in the app.
- The daemon owns callback listeners, credential publication and cancellation.
  Closing Accounts retires reads, not the browser login; reopening reads its
  current state. New intent retires older list replies. Reads poll only while a
  login is pending; mutations never retry automatically. Credential availability
  means Connected, not verified provider inference. Model readiness refreshes
  after projected account changes and when the model disclosure opens, including
  after browser login completes with Accounts closed. Opening does not select a
  provider or continue the closed dialog's poll. Latest model requests share a
  generation guard and require the same resolved endpoint before publication.
- Verify native `cargo test --lib provider_accounts`, UI `cargo test -p
  ocean-surface-ui accounts`, the real WASM bundle and rendered native dialog.

## Desktop Team Connection

- `src/team_connection.rs` owns the native Team dialog in the existing header
  overflow. Use the Accounts dialog vocabulary, native focus trap and Escape;
  return focus to the menu on close. No permanent header setup controls.
- `host.rs` calls main-window-only `team_connection` with Read or Save intent.
  The native broker uses the shared operator transport for exactly GET/PUT
  `/v1/rooms/connection`; URL validation and first-write custody stay in the
  daemon. No credentials, arbitrary routes or raw errors reach the page.
- Saving only selects the initial server. Show Configured only from the daemon's
  startup projection; it proves neither reachability nor membership. Never
  replace an existing team destination or retry a save automatically. Lost
  acknowledgements require a read before another save; drafts survive failures.
- Restart is explicit and offered only for a shell-owned child. Recheck ownership
  before invoking restart and read back configuration afterward. External
  services retain their restart requirement. Close/new intent retires reads and
  delayed actions; committed saves are not undone by closing the dialog.
- Verify native `cargo test --lib team_connection`, UI `cargo test -p
  ocean-surface-ui team_connection`, the real WASM bundle and rendered native
  setup. Hosted Windows checks remain separate from local Mac acceptance.

## Empty Session Surface

- Empty transcripts show the existing static circular Ocean mark and an
  accessible title. Keep the composer primary: no full-pane animated hero,
  repeated Sessions action, marketing copy or idle rendering loop.
- Sessions navigation stays in the existing header/native navigation. Hide
  unavailable voice controls instead of showing disabled setup placeholders
  or local credential paths. Existing voice transport readiness owns visibility.

## Model and Effort Control

- `crates/ocean-surface-ui/src/model_control.rs` owns the composer's single model/effort disclosure.
  Main and floating Leptos chat use this same component and daemon selection state;
  do not add a second Leptos model/effort picker for an alternate chat host.
  It consumes the daemon's shared model catalog and current selection; choices
  remain per-turn overrides rather than global model mutations. Prefer additive
  `/v1/models.routes` and `current.route` when present, with legacy `models`
  fallback. Provider-qualified ids distinguish API and subscription auth;
  aliases render older saved selections without rewriting their stored value.
- Honor additive `ready` metadata: disable explicitly disconnected provider
  entries, retain compatibility when an older daemon omits readiness, and
  preserve pinned ids while the catalog loads. Readiness proves credentials,
  not provider entitlement or successful inference. Option rows re-read their
  entry (readiness, label) from the live `models` signal inside the row body —
  a keyed `For` never re-runs same-key children, so capturing at mount freezes
  the row — keeping `For` keys as model ids (#144): a same-id catalog refresh
  must flip the rendered disabled state and label without an identity change,
  and refreshes leave drafts, model overrides and reasoning effort intact.
- Use additive daemon `effort_levels` metadata for model-specific choices,
  including max on current models. Older daemons retain the existing fallback
  vocabulary based on the wire model even for qualified saved ids. Explicit
  empty effort metadata remains authoritative; do not synthesize extra choices.
  Model changes clear incompatible overrides; stored max survives
  reload. The default selection comes only from `/v1/models.current`, never
  the most recent executed turn. Escape closes an open disclosure and restores
  focus; a closed disclosure lets Escape reach the global reveal handler.
- Product intent lives in `PRODUCT.md`; existing web design and token files
  remain the visual authority.

- Floating chat keeps drafts editable during streaming. Send admission requires
  a resolved endpoint, an idle turn and nonblank input, in both the button and
  submit handler. Rejected submissions preserve the draft; Halt remains the
  active-turn action. Its model panel is anchored to the compact composer.
  Browser float mode uses Ocean's background; native overlay transparency is
  gated through the existing `host::running_in_tauri` seam.
- Enter while one of this surface's turns is streaming steers it: the text
  goes to `POST /v1/agent/sessions/{id}/steer` and shows as a
  `turn--steering` row ("Steering · next step", no remove control) until the
  daemon's `steering_delivered` frame, when it becomes a user turn at the
  point it landed and the next round renders below it. A refusal (409/429/
  network) sends it to the queue; an accepted steer still pending when the
  active turn's `turn_finished` arrives goes to the FRONT of the queue and runs
  as the next turn. Delivery matches by `steer_id`, or by text for a row whose
  POST answer has not arrived yet. Messages with staged images, voice
  prompts, and sends with no acknowledged turn take the queue instead. Stop,
  New Session and a session switch clear pending steers with the queue.
- The main composer never refuses a send because a turn is running. A prompt
  that cannot steer (see above), sent while earlier prompts are queued, or
  while the head of the queue is waiting out a busy daemon goes into
  `Daemon::queued_prompts` (oldest first) and renders as a dimmed, dashed
  `turn--queued` row below the live-activity row with a remove control; it
  becomes an ordinary user turn when `drain_queued_prompt` sends it, which the
  `install_queue_drain` effect does each time `streaming` returns to false.
  Staged images leave `pending_images` with the prompt they were staged for.
  A daemon `409 session has an active operation` answer to a send re-queues
  that prompt at the head (taking back its echoed user row) and retries every
  1.5 s, up to 40 times, before the old error block appears. Stop, New Session
  and a session switch clear the queue; a queued follow-up never starts the
  moment a turn is cancelled. The live-activity label for "turn accepted, no
  block yet" reads "Thinking…".

## Session Contract

The ecosystem invariant is:

```text
Project -> Workspace -> Session -> Turns -> Events
Surface -> Session
```

First-party surfaces must create or choose a session before posting a turn:

```text
POST /v1/agent/sessions
GET  /v1/agent/events?session_id=<id>
POST /v1/agent/turns { session_id, prompt, cwd, project_id?, client_type }
```

Rules:

- Never open a product transcript on the global `/v1/agent/events` stream.
- Never adopt the active session from `SessionCreated` or `TurnStarted` on a
  global stream.
- Cross-surface sharing is explicit: two surfaces attach to the same
  `session_id`.
- Different sessions on different surfaces must not blend, switch each other,
  or race to become the active transcript.
- `client_type` only describes the surface medium (`surface-web`,
  `surface-extension`, `surface-tauri`; legacy: `surface-gpui`). It is not a
  session id or workspace id.

Web surface session UI:

- A `?session=<id>` link names the chat to open. The TUI's `/web` and `/beam`
  hand a session over with exactly that URL, so on boot it wins over the
  session this browser last used; only a well-formed session id is honoured,
  and a link to a session this daemon does not have falls back to the
  persisted one untouched (the miss is logged; the status chip is rewritten
  by whatever loads next, so there is nowhere lasting to say so). The link is
  single-use: the parameter is dropped from the address bar as soon as it is
  read, so a reload never jumps back. A boot restore (linked or persisted)
  stands down when the user starts or opens a session while the daemon is
  being asked, and boot never connects afresh over a session the user already
  has. The URL shape is a cross-client contract with
  `ocean-tui`; change it only together. Known limit, owned by
  `ocean-surface-proxy`: an unauthenticated navigation is redirected to
  `/login` and then to `/`, which drops the parameter, so the link only works
  once this browser is signed in to the origin it names.
- The composer splits a `/` line once (`palette::SlashLine`) and both the
  popover (`CommandRegistry::slash_rows`) and dispatch
  (`CommandRegistry::resolve_slash`) read that split, so the highlighted row is
  always the command that runs; Send uses the highlighted row too. While the
  name is being typed the popover ranks an exact alias, then prefixes, then
  scattered matches, and Enter, Tab or a click run the chosen row; for a
  command that reads arguments (`/model`, `/thinking`) they complete the name
  into the composer instead (`/th` becomes `/thinking `). Once whitespace
  follows the name, only the command that name spells exactly is listed or
  run: an abbreviation followed by words (`/s what do you think`) is not a
  command and shows no menu, and with a single row shown the arrow keys move
  the caret, not the highlight. A command that takes no arguments is not run
  with words after it. In each of those cases, and for an unknown or
  unavailable command, the draft stays in the composer at its height with a
  hint. A line whose first word is a path (`/etc/hosts`, `/notes.md`) or
  comment punctuation with text after it (`// broken`, `/// doc`, `/* note`)
  is sent as a message; a leading space sends any other line that starts with
  a slash. `/model` and `/thinking` read the first word after their name, and
  the `/thinking` hints list `THINKING_LEVELS`. A bare `/thinking ` shows the
  choices and never changes the level. Selection resets when the name
  changes, not while arguments are typed.
- The sessions panel is project-first: prefer daemon-provided `owning_project`,
  fall back to exact `workspace_root`/`cwd` matches against the project catalog,
  and put everything else in an explicit `Other` bucket.
- Opening the sessions panel refreshes both the session list and project
  catalog. Project-section render identity includes the visible label so a
  daemon-side project rename cannot leave stale client chrome behind.
- `New Session` is lazy on the web surface. It resets local transcript/turn
  state and waits for the first prompt to POST a daemon session; do not re-add
  eager session creation that litters 0-turn drafts. The bounded exception is
  confirmed Voice Planner: the exact `Create draft` or `Create & start` click
  is itself the explicit product action that creates one session.
- The pre-session Voice Planner remains non-executing: its Realtime tool set is
  limited to bounded `list_workspace` / `read_workspace_file` reads whose
  normalized and daemon-canonicalized targets stay under the validated workspace
  plus `propose_handoff`; only the exact `Create draft` or `Create & start` click
  creates a session or starts a turn.
- Ordinary realtime Voice chat receives workspace inspection only when the
  daemon's secret response includes the canonical root resolved from the active
  session. Freeze that root into the connection config and fulfill only bounded
  relative `list_workspace` / `read_workspace_file` calls whose daemon-resolved
  targets remain under it. Never derive authority from model arguments or a
  browser-selected path; older/project-less responses keep render + handoff only.
- Ordinary realtime Voice chat streams OpenAI `response.output_audio_transcript.*`
  deltas into one local assistant turn keyed by output item, then replaces the
  accumulated text with the authoritative done transcript. Keep this projection
  conversation-only (Planner has no chat transcript), and do not represent it as
  daemon-persisted history: a later session refresh remains authoritative.
- Mount the Rooms workspace only after `daemon.endpoint_ready` confirms proxy
  config or the host fallback has resolved. Its mount fetch must never race
  bootstrap against an initial fallback daemon URL.
- Idle web/extension headers stay single-bar: project/session context may stay
  visible, but call/join affordances live behind overflow until intentionally
  opened or actively connected.
- Sessions-panel live state (OCEAN A1 / v7): abortable poll rail — every
  async fetch is wrapped in `abortable()` with the `AbortHandle` stored in
  `RwSignal<Option<AbortHandle>>`. Only `fetch_all_sessions().await` lives
  inside abortable. `poll_guard_write(current_gen, my_gen, panel_open)` is
  called before every `session_list.set()` — gen match + panel open required,
  so a settled stale fetch after close/reopen can never write. After matching
  all three outcomes, ONE unified `poll_release_in_flight` gen-gated guard
  releases `in_flight` (reads the actual `poll_in_flight.get_untracked()`, not
  a literal). Interval uses `leptos::prelude::set_interval_with_handle(_, Duration::from_secs(2))`
  → `RwSignal<Option<IntervalHandle>>` (thin i32 Copy+Send+Sync wrapper, no raw
  Closure/i32/forget leak). `handle.clear()` in `stop_polling`. The three
  production deciders (`poll_guard_write`, `poll_should_skip`,
  `poll_release_in_flight`) are file-scope fns; the Abortable test feeds old
  `Err(Aborted)` through the unified cleanup after the new task sets
  `in_flight=true` and proves the stale task does not clear it. The session
  dot renders `data-state` (permission → cancelling → running → recent → idle)
  with colour-keyed breathing/fade animations, `role="img"`, and
  `aria-label`; CSS tokens are `--fg-4` (idle), `--fg-3` (cancelling),
  `--accent`/`--warn`; `prefers-reduced-motion` sits
  after the state selectors so it wins. Unknown daemon variants are
  `#[serde(other)]` and render idle/recent.
- Transcript live-follow is intent-aware: follow streams only while the reader
  is at (or near) the bottom; scrolled-up history reading is never yanked, a
  quiet zero-height sticky `↓ latest` affordance returns and re-pins, and a
  session switch always re-pins so the new transcript opens at its latest
  turn. Do not re-add unconditional scroll-to-bottom on stream deltas.
- Session switches and reconnects commit the daemon's complete persisted
  transcript immediately, including while a turn is live, while preserving the
  daemon-owned running/Stop projection and continuing the scoped SSE tail.
  Never quarantine a live session behind client-side detail polling,
  `detail syncing…`, or a manual refresh control.
- Snapshot hydration reconciles pending steering rows and the Stop target by
  identity (#148). SSE is a live tail, so `steering_delivered` /
  `turn_finished` frames missed in a gap never re-arrive: the projection
  commit matches each accepted row against the daemon's `steering` outcome
  records (`steer_id` + originating turn — never text), removes confirmed
  deliveries without duplicating the transcript, queues confirmed drops once
  ahead of the queue, keeps rows whose turn is still live pending, and pins
  everything else `unresolved` (visible, never auto-requeued). A steer POST
  that never returned an answer stays unresolved; an explicit HTTP refusal
  still queues. A terminal snapshot naming the local turn in
  `finished_requests` clears the preserved Stop target (the registration race
  and older daemons keep the old preserve behavior).
  (PR #129 reconciliation: the desktop candidate's #148 variant — client-minted
  steer ids, bounded receipt hydration, and typed 503 snapshot-churn retries —
  was an independent solution of #148 and is superseded by this #156 contract;
  do not reintroduce or blend the two designs.)
- The daemon's `event: error` reset frame on `GET /v1/agent/events`
  (published `agent_events_error`: `live_lag`, `anchor_unavailable`,
  `malformed_anchor`, `reset_required`) shares its name with the DOM
  connection-error event, so gloo reports it as a `ConnectionError`.
  `src/agent_events_reset.rs` tells the two apart (a reset frame is a
  `MessageEvent` with data; a transport error is a plain `Event`), and
  `Daemon::connect` polls that subscription first, leaves the connection once
  on a reset and takes the existing reconnect path (fresh projection, fresh
  `EventSource`, no stale anchor). A plain transport error stays with gloo's
  handling. `tests/unheld_agent_stream_reset.rs` pins that wiring; its table
  records which mutations the compiler holds and which only the guard does.
- Token telemetry consumes the daemon's additive `TurnFinished` provider
  footprint and cache-read/cache-write breakdowns. The provider total is
  authoritative; cache buckets may already be included in input and total, so
  never add them to reconstruct the footprint. Keep unknown totals unavailable.
  The session subtotal counts observed terminal events since the current
  session binding only; label it accordingly because transcript hydration does
  not reconstruct historical token usage.
- Every complete session-list request—thin daemon spawner or A1 panel
  poll—claims the same daemon-owned generation ticket. Only the latest claimant
  may replace `session_list`; panel-local generation/open guards still protect
  lifecycle writes and cleanup. Pagination cursors are RFC 3986-encoded as one
  query-component value before daemon requests.
- A project-section `+ new session` always passes an explicit cwd: catalogue
  `workspace_root` first, otherwise the section's newest concrete session root,
  and the action stays absent when neither root exists. It must never inherit
  the previously active session's cwd or silently fall back to `/tmp`.
  Project-form focus refs belong on the actual first inputs (existing path and
  new-project name), not container elements.
- Reveal lifecycle is deterministic: opening Council closes every competing
  reveal; opening the Island closes every non-Island reveal; every peer reveal
  open (Council, Rooms, Sessions, Floor, phone dialer, LiveKit controls)
  closes the Island. Window Escape closes exactly one topmost surface in visual
  z-order: Council → Island → Rooms → Sessions → Floor → phone dialer →
  LiveKit. The workspace pane is permanent furniture, not a reveal.

## Rooms Contract

Design direction for the Rooms workspace (identity rule, transcript vs
activity, the room details drawer, agent authoring, the S0-S8 slice plan):
`docs/OCEAN_ROOMS_DESIGN_DIRECTION.md`. It supersedes the rendering bullets of
`docs/OCEAN_ROOMS_PRODUCT.md`; the contract bullets below describe main as it
ships today and are updated slice by slice.
- `docs/OCEAN_ROOMS_PRODUCT.md` is held to the code by
  `tests/rooms_product_doc_truth.rs` (Rooms DoD 5.4): the access states it
  names must be exactly `RoomAccessState`'s variants (read from
  `src/rooms.rs`), every `.rooms-*` class it names must be defined in
  `styles/` and emitted from production Rust (page or Tauri shell), and every
  route in its API table must be sent by the page or shell and, in the
  monorepo, registered by `ocean-daemon`'s router with that method; its
  create body must show exactly the wake policy `CreateRoomBody::new`
  attaches. Edit the doc and this guard together; a backticked non-state word
  on a line that mentions access goes in `ACCESS_LINE_NON_STATES`.
- The operator-authorized post-G3 product-depth program is governed by
  `docs/specs/2026-08-13-ocean-rooms-post-g3-product-depth.md`. A share link is
  only the admission rail; do not describe Rooms as complete before its
  two-human/two-machine/two-sovereign-agent acceptance gate passes.
- The standalone `risingtides-dev/ocean-surface` history through `f91195f`
  (2026-08-14) was restored into the authoritative monorepo on 2026-08-16.
  Its durable Slack-style density, unread recovery, one-level threads,
  markdown/action rail, deterministic identity, mention completion, and compact
  Tauri navigation are accepted Rooms baseline behavior. Do not regress or
  replace them with the stale 2026-08-04 subtree snapshot; new work lands only
  in this monorepo.

- Every successful `GET /v1/rooms/persistent/{key}` carries a required
  `RoomAccessProjection`, including explicit `Local` for G1 rooms. Surface
  `None` means loading or no open room; it is never a local-room discriminator.
- A selected room without a loaded record renders its existing loading or failure
  status in the center pane with a polite status announcement. An unselected
  canvas stays empty; rendering adds no requests or automatic retries.
- Room transcripts hydrate once, then tail only the room-scoped SSE endpoint
  `GET /v1/rooms/persistent/{key}/events` with sequence resume. Subscribe to
  both `room_message` and `room_access` immediately: messages alone advance the
  room sequence cursor; access projections replace state without a sequence.
  Do not restore transcript polling or consume the global agent-event stream.
  The tail is the sole forward-ingest path: join, leave, agent add and post
  responses never re-read `/transcript` (`tests/room_history_follow.rs`).
- The open response serves the OLDEST rows capped at the daemon's
  `MAX_TRANSCRIPT_LIMIT` with `has_more` / `next_seq` (OCEAN-249). `open_room`
  follows `/transcript?after_seq=&limit=` pages (`rooms::OpenCatchUp`) until
  the daemon reports no more rows, then publishes the whole transcript once
  and starts the tail from its real frontier, so a long room opens on its
  newest, contiguous rows with follow pinned, one read advance, and the unread
  divider computed over the full history. This bounded one-time catch-up on
  open is explicitly not transcript polling: it runs once per open, stops at
  the first exhausted page, a non-advancing page, or 50 pages, is cancelled by
  the room generation on switch, and is the only production `/transcript`
  reader. Do not add "load older" or any backward paging here (the rail's
  forward room-list paging is a separate user control; see the room-list
  paging bullet below); a failed page lands the open on the rows held and the
  tail's durable replay delivers the rest in order.
- Room live-follow consumes scroll intent captured by the transcript's own
  scroll events (`transcript_follow_pinned`), not post-render geometry, so a
  tall append still follows a pinned reader and never yanks a scrolled one.
  Only the pin and the "New messages" jump write `scroll_top`.
- Decoded room-tail frames mutate state only after one shared generation+room
  admission check. Opening and closing a room share one synchronous reset path
  so stale transcript, access, and tail state cannot leak across room identity.
- Read PATCH dedupe uses confirmed cursors and the current request ticket. A
  clamped acknowledgement remains retryable; a completion clears only its own
  in-flight request.
- The browser PWA proxy must forward `/v1/agents` as JSON and stream room SSE
  unbuffered while preserving `Last-Event-ID`; Tauri reaches the same daemon
  endpoints directly. Persistent-room PATCH routes (including `read-cursor`)
  must forward the method, JSON body and query behind the existing session gate;
  browser cookies and forged operator headers never pass upstream. Verify with
  `cargo test -p ocean-surface-proxy room_read_cursor_patch`.
- Canonical room keys use alphanumeric, `-`, `_`, and `.`; native invite
  parsing keeps that alphabet separate from the narrower session-id and
  invite-code alphabets.
- One-level thread replies are writable only in Local rooms. Live federated
  rooms may render threads, but keep the reply composer disabled until the
  Bedrock message contract preserves `thread_parent_seq` end-to-end.
- Identity is daemon-owned (team-platform P2). The surface never mints or
  stores a room identity: it reads the owner from `GET /v1/me` (proxied),
  human joins send only `{kind: human}`, and human posts send no `author_id` —
  the daemon authors both as its owner. "Me" is the owner `participant_id` in
  Local rooms and the daemon-projected `access.local_member_id` in federated
  rooms (`rooms::local_speaker_id`). Names render through
  `rooms::author_display_name` (roster, then federated member projection,
  then raw id) and initials through `rooms::name_initials`, with the
  deterministic `avatar_identity_class` hue on every avatar.
- Agent work cards (team-platform P3) are daemon projections, not
  transcript rows. The room SSE also carries `room_agent_run` frames (projection
  replace by `run_id`, no sequence, same generation+room admission as
  `room_access`); the daemon replays recent runs on connect, so the surface
  never polls `/runs`. `room_work_card.rs` renders one card per run under the
  message whose thread root convened it. Steps load only when opened, from the
  owner's agent session (`daemon::fetch_session_turns`, sliced by the
  `[#seq] ... «— mention` prompt marker) and render through the
  own `AssistantTurn` (`detached`). Card colors come only from
  tokens (`styles/rooms-cards.css`). Live projections retain the daemon's 50 most
  recent runs, ordered by start time and run id. Card-step fetches publish
  success or error only for the latest request while open, with matching
  daemon origin and unchanged run projection (including session and trigger);
  closing or disposal invalidates pending work.
- In-room approvals (P4): a card in `awaiting_permission` with a
  `pending_permission` shows the tool label plus Approve/Deny icon buttons,
  which call `POST /v1/rooms/persistent/{key}/runs/{run_id}/permission` with
  the rendered request's `permission_id` and `tool` (`run_permission_body`), so
  a stale click is a daemon 409 ("Already decided."), never a decision on a
  later request. Never apply a decision optimistically; the next
  `room_agent_run` frame clears it.
  Permission/settings mutations and the owner rename keep daemon header-only
  Room operator authority. Operator decision (2026-10-08): any first-party
  Ocean surface the operator is signed into may act as the Room operator for
  exactly `PUT /v1/me`, `POST .../runs/{run_id}/permission` and
  `PUT .../agents/{agent_id}/settings`; every other operator-gated route is
  unchanged, and a client-supplied operator header is never forwarded.
  - Web: the proxy attaches the daemon's mode-0600 `operator.key`
    (`OCEAN_OPERATOR_KEY_FILE` overrides the path), read just before
    forwarding, to exactly those routes, and only when its operator login is
    on and the session gate has admitted the request; an unreadable key is a
    503 before forwarding, and with `OCEAN_SURFACE_AUTH=off` it attaches
    nothing, so the daemon keeps refusing.
  - Tauri desktop: the page calls the `room_owner_mutation` command (main
    window only) with a fixed `kind` (`rename_owner`, `run_permission`,
    `agent_settings`) and the ids (`rooms.rs` `OwnerRoute` →
    `host::room_owner_mutation`). The shell
    (`crates/ocean-tauri/src/owner_mutation.rs`) builds the fixed method and
    path itself and sends it through the shared native transport
    (`daemon_transport.rs`, see "Shared native daemon transport" below): the
    key file is read just before each request and goes only to a
    numeric-loopback daemon with no `Origin`/`Cookie`, is never cached,
    logged or returned, and only the daemon's status and JSON object body are
    relayed.
  - Chrome extension: not covered. The side panel talks to the daemon
    directly from `chrome-extension://`, cannot read the key file, has no
    proxy session, and its origin is refused by the daemon's operator origin
    check by design; doing this safely needs a native-messaging host or an
    extension proxy login, so these actions keep the daemon's authority error
    there.
- The room header has exactly one primary action (Join when not joined), the
  members pill (`.rooms-workspace__members-chip`, design §7: the Groups glyph
  plus `members_pill_count` — the Local roster or the access projection's
  members — toggling the room details drawer at every width), the P6 search
  icon, and one overflow (`room_overflow.rs`): Members, Files, Share, Agents,
  Mute/Unmute, Leave, Close room…. Agents opens the
  per-room agent settings panel (instructions overlay + model) over
  `GET/PUT .../agents/{agent_id}/settings`; these settings are local to this
  Ocean. New secondary room actions go into this overflow, not the header.
- Closing a room (DoD 4.3, `room_close.rs`; ported from
  Risingtides-dev/ocean-surface#229): the overflow's `Close room…` item renders
  only for a viewer the daemon's member lane would accept — `identity_id` (the
  `/v1/me` owner, non-empty after trimming) on `Room.participants` as a
  non-`Agent`/`System` row (`close_offered`; `Human`, `Bot`, `Tool` pass) — and
  only ARMS a modal (`RoomCloseDialog`) whose focused default is `Keep room
  open`; the modal's own button fires, re-checking the rule at dispatch
  (`dispatch_check`). `RoomCloseState` and the dialog live at `RoomsWorkspace`
  scope, outside the header closure that every `open_room` write rebuilds, so a
  roster frame cannot re-enable the button or unmount a close in flight
  (`tests/unheld_room_close.rs`). The fire sends
  `POST /v1/rooms/persistent/{key}/close?actor_id=<me>`, the daemon's MEMBER
  lane only, on every host: the operator lane (`X-Ocean-Operator`) is
  deliberately not roster-checked, so close is not an owner-mutation route and
  the proxy injects nothing (`cargo test -p ocean-surface-proxy room_close`);
  the Tauri operator transport is not used for the same reason. Reply contract
  (`classify_close`): `200 {ok:true, closed:true}` is closed; this daemon's
  close route answers a not-open room with the store's `UnknownRoom` as a JSON
  `404 {ok:false}` WITHOUT the `room_not_open` marker its read routes carry, so
  a 404 that decodes (marker or `ok:false`) means the room already closed,
  while an EMPTY 404 or a 405 is a daemon or proxy without the route; 502/504
  and a transport error say the close MAY have landed (the daemon commits
  before it answers; a retry of a landed close reads as already closed);
  `403 forged_closer`, the roster 403 (no code), 400 and 5xx each get their own
  sentence. Main has no soft-closed audit view, so both closed outcomes land on
  the rail: `Rooms::close_room` retires the tail through the shared reset path
  and `fetch_rooms` drops the row; focus is handed to the rail listbox's tab
  stop when it fell to `<body>`. The modal traps Tab (`tab_trap`), holds focus
  on itself while both buttons are disabled in flight, dismisses on Escape
  (`prevent_default`, so the workspace and window handlers stand down) or the
  scrim, and returns focus to the overflow trigger. For any non-`Local` access
  state the copy says the close is local and other nodes keep their copy.
- Room create (`RoomListRail`) carries an optional default folder (PR F,
  re-implemented through the shared picker ported from
  Risingtides-dev/ocean-surface#240): `WorkingDirectoryPicker`
  (`rooms_workspace/working_directory.rs`, mounted exactly twice — the create
  form with `allow_empty=true` and the drawer's Default folder row with
  `allow_empty=false`) is a native `<select>` whose rows are the daemon
  project catalogue's `ProjectInfo.workspace_root`s AND their `worktrees`
  (passed down from `RoomsWorkspace` as the one `daemon.projects` signal,
  never fetched again, deduped by path, first wins, offered verbatim), plus
  "Choose folder…" via `host::pick_folder` on Tauri only, plus "Other
  absolute path…", which reveals a mono text field. `directory_choice` is a
  pure decision rendered through per-option `prop:selected`: an empty value
  is the explicit "Unbound" row on create and the text field on bind, a
  catalogue path is its row, anything else (a stored binding the catalogue
  no longer lists, a picked folder, a typed path) is manual and stays
  visible and editable; typing keeps manual mode so a value that comes to
  equal a row never unmounts the field mid-keystroke, and a `reset` epoch
  bumped on successful create and on bind-draft reseed drops manual mode
  with its draft. Nothing in the picker judges a path (no absoluteness,
  existence or canonical-form check; `tests/directory_selection.rs` bans the
  tokens): the daemon's `400 invalid_workspace_root` is the only verdict.
  The choice is explicit and resets after a successful create; it is never
  seeded from a session cwd, the focused chat's project, or `/tmp`. What is
  sent is what is shown (`create_workspace_root`: the draft trimmed, or
  absent). `CreateRoomBody.workspace_root` is omitted when unset.
  `CreateRoomBody.trigger_policy` is never omitted: `CreateRoomBody::new`
  attaches `RoomTriggerPolicy::create_default()` (`on_mention` and
  `on_thread_reply` on; `on_component_event`, `on_build_failure`,
  `on_ci_failure` off; no `on_schedule`) to every create, so
  `Rooms::create_room` takes only the name and folder and no create path can
  send a policy-less body. That default is the "+ New room" policy in
  `docs/OCEAN_ROOMS_DESIGN_DIRECTION.md` (Sidebar) and the Gate 1 create
  flow; ocean-core never convenes on an absent policy, so the `None` the rail
  used to pass made every Surface room invoke-only against the documented
  "mention an agent to wake it" expectation. A mention still has to pass the
  agent binding's activation policy (`mention` / `task_and_thread`), which
  the consent sheet now offers with the daemon default `explicit_only`
  pre-selected (see the consent consumer section below). No room-level
  policy UI here: a later control writes through the daemon's
  `PATCH /v1/rooms/persistent/{key}`. Guarded by
  `create_body_always_carries_the_canonical_wake_policy` (`rooms.rs`) and
  `product_doc_create_body_carries_the_wake_policy_rooms_rs_sends`
  (`tests/rooms_product_doc_truth.rs`).
  The daemon binds it (absolute existing directory, canonicalized;
  otherwise 400 `invalid_workspace_root`, shown as "create failed: workspace
  folder not found on this Ocean" with the draft kept). `Room.workspace_root`
  decodes as absent for unbound rooms and older daemons; when set, the header
  shows its folder name read-only (`title` = full root). A room turn's folder
  is agent folder, then room folder grant, then `workspace_root`, else
  `workspace_unavailable`.
- Default folder (design §5.2; `rooms_workspace/workspace_binding.rs`): the
  drawer's Members mode ends with `Room.workspace_root` in mono (or "none"),
  the cwd rule in one sentence ("used only when an agent has no shared
  folder"), and a ghost Edit — rendered only when `local_store_write_gate`
  holds (the binding is this daemon's room record, never enqueued for a
  peer), absent otherwise — that reveals the shared picker with Bind and
  Unbind. Those two are the ONLY callers of `PATCH /v1/rooms/persistent/{key}`
  on this surface: `Rooms::set_open_room_workspace(Some(root))` sends
  `RoomWorkspacePatchBody { workspace_root: Some(root) }` (trimmed, otherwise
  verbatim) and `set_open_room_workspace(None)` sends an EXPLICIT
  `"workspace_root": null` (the daemon reads the field absent-vs-null; an
  omitted field would mean "unchanged", so the body type has no
  `skip_serializing_if`). No `trigger_policy` or name ever rides along; the
  daemon rejects unknown fields. One PATCH in flight at a time
  (`Rooms::workspace_patch_in_flight`); the reply lands only in the room
  admission that sent it (`room_is_current`): a `2xx {ok, room}` replaces
  `open_room` AND the rail row's `workspace_root`/`updated_at` (so the header
  folder and the workspace-root grouping follow the daemon), everything else
  is one sentence in `Rooms::workspace_patch_error` beside the control with
  the draft kept (`classify_workspace_patch` / `workspace_patch_status`:
  `invalid_workspace_root` → "workspace folder not found on this Ocean", 404
  → "This room is closed.", else the daemon's code). Bind is disabled for a
  blank draft, an unchanged root, or a PATCH in flight; Unbind only while
  bound. The draft reseeds from the stored root when the room id OR its root
  changes (`reseed_needed`), never on a roster frame, and reseeding closes
  edit mode. Design ruling followed over the legacy PR: no room-wide
  "agents cannot run" warning — the per-agent cannot-run line belongs to
  `inspect.agents[].execution` (S6/S7); the refusal still surfaces at the
  action through `invocation_error_text` (`workspace_unavailable`). The web
  proxy forwards this PATCH generically (`RoomsPersistentShape::Json`, no
  operator key); Tauri reaches the daemon directly. Guarded by the
  `workspace_patch_*` tests in `rooms.rs`, `workspace_binding.rs`'s unit
  tests, `tests/room_details_drawer.rs` and `tests/directory_selection.rs`.
- The Rooms rail groups rooms by `Room.workspace_root` (`group_rail_rooms` in
  `room_list.rs`; derived, never stored): first-seen root order, daemon order
  within a group, unbound rooms last under "Other rooms". Each group is a static
  `role="group"` labelled by a non-interactive header showing the full,
  case-preserved root (wrapping, never a basename or ellipsis) and a live count.
  Headers are not buttons and groups never collapse, so the listbox keeps one
  roving tab stop and Arrow/Home/End over the flattened grouped order
  (`rail_room_keys`). Header ids derive from the root, not list position. A
  shared root is a folder binding, not a daemon Project identity; do not call
  these project groups.
- Room-list paging (OCEAN-250): `GET /v1/rooms/persistent` serves one bounded
  page (`limit` default 100, max 1000; `next_cursor` / `has_more`). The rail
  renders the daemon-default first page with no query and, while the daemon
  reports `has_more`, ends with one "Load more rooms" button
  (`.rooms-workspace__left-more`) after the last workspace group and outside
  the listbox: a plain tab stop, never an option, so the roving tab stop and
  the `rail_room_keys` order are unchanged. `Rooms::fetch_rooms_more` sends
  `?cursor=` for one page at a time (disabled while in flight) and
  `rooms::RoomListPaging::absorb` appends it in daemon order, dropping ids the
  rail already holds and dropping the whole page when the list ticket or the
  pending cursor moved underneath it; an empty or non-advancing page ends
  paging. Every refetch (create, close, the 8 s silent refresh) sends `?limit=`
  for the depth the rail has been extended to (clamped to 1000), so a refresh
  never shrinks the rail back to one page. Forward only; the daemon's
  `has_more` is the only gate (never a local guess at page size), and older
  daemons that omit the pair decode as one complete page. Guarded by
  `load_more_rooms_follows_the_groups_outside_the_listbox` in `room_list.rs`
  and the `RoomListPaging` tests in `rooms.rs`.
- Transcript rows whose `author_kind` is `agent` carry an "agent" badge beside
  the name (`.rooms-workspace__msg-kind`, `format::author_kind_badge`) in the
  timeline, the thread root and thread replies. It is the mention suggestion's
  kind label reused: the class shares `.rooms-workspace__mention-kind`'s rule
  in `styles/rooms-workspace.css` (tokens only, no new colours). Humans and
  every other kind stay unlabelled, compact system rows never carry it, and
  grouped rows hide it with the name. `tests/rooms_agent_badge.rs` pins the
  three emit sites, the shared rule and the agent-only gate.
- Rooms UI lives in `src/rooms_workspace/` (team-platform P5): `mod.rs`
  owns `RoomsWorkspace`, its shared signals, the transcript and the details
  drawer; child components are `RoomListRail` (`room_list.rs`),
  `ChannelComposer` (`composer.rs`), `ThreadPanel` (`threads.rs`),
  `MembersPanel` (`members.rs`), `DefaultFolderSection`
  (`workspace_binding.rs`), the shared `WorkingDirectoryPicker`
  (`working_directory.rs`), the notice banners (`banners.rs`), and the agent
  consent and invoke components (`agents.rs`); pure helpers live
  in `read_cursor.rs`, `mentions.rs` (one `MentionState` drives both
  composers), `format.rs`, and `access.rs`, each with its own tests. Drafts,
  send latches, and mention state stay owned by `RoomsWorkspace` because the
  center rail and the drawer re-render on room state.
- Room details drawer (design §5, executed by the port of
  Risingtides-dev/ocean-surface#240): there is no permanent right rail.
  `.rooms-workspace__right` is one fixed slide-over at every width — 380px,
  closed by default (`transform: translateX(100%)` + `visibility: hidden`;
  `--open` slides it in over 280ms with the reduced-motion kill AFTER the
  state selectors), `--bg-raised` on `--shadow-lg` with no border, behind a
  `--scrim` backdrop (`.rooms-workspace__right-scrim`, rendered only while
  open, a click closes like Escape), a full-screen sheet at or below 650px;
  z-index 420/415, above the compact left drawer (400) and the mobile nav
  (410), below the close confirmation (440). The drawer hosts the former
  rail modes as content: Members (the roster, then the Default folder row,
  titled "Room details"), Files, Search, and Thread. Only the thread panel
  docks inline, at or above 1280px (§7), through the
  `rooms-workspace__right--thread` modifier (the scrim stands down there);
  Members/Files/Search never dock, and the 901-1080 docked-rail breakpoints
  are gone. The Room options overflow still offers Members (including an
  empty agent roster; it reveals the same MembersPanel and ConsentSheet, and
  native consent and browser/extension authority gates remain in those
  components) and Files; the header members pill toggles Members. Explicit
  picks select one drawer mode. Close, the scrim and unhandled Escape dismiss
  the drawer and restore its trigger focus (the pill, or Room options for
  Members); nested Escape handlers take precedence. Room admission/origin
  changes retire open drawer modes without moving focus or altering consent
  authority. Guarded by `details_drawer_css_is_a_slide_over_at_every_width`
  (`mod.rs`) and `tests/room_details_drawer.rs`. Not built here (later
  slices): the four plain-text tabs over `inspect`, `details_open`
  persistence, the pin-inline opt-in at 1440px, and the attention chip.
- Unread attention and the new-message jump exclude the same routine audit rows
  as the timeline. Local room-list summaries come from the daemon's conversation
  frontier; the open-room summary uses the same filter. Visible failures, access
  changes and human messages still count. Read cursor PATCH and SSE resume retain
  the full durable sequence; never mark unseen conversation read to silence a dot.
  Deploy the Surface and daemon attention changes together; older daemon summaries
  can still count hidden audits in unopened rooms.
- Unread rows also emit `rooms-workspace__room--unread` (bold name; muted rooms
  never). The transcript's `rooms-workspace__unread-divider` sits above the first
  visible root past `Rooms.unread_baseline`, which `open_room` locks from the
  room-list `read_summaries` before the open resets or advances the cursor and
  `close_room` clears; never derive it from `open_read_cursor` or keep it
  workspace-local. Compare in the room's read space (`read_cursor.rs`):
  transcript `seq` for `Local`, `global_sequence` for `Live` (rows without
  federated metadata are history, not unread), no divider in other states or
  without a baseline.
- The timeline hides routine convene, bootstrap, admission and output audit
  rows (`room_messages::is_convene_audit`), including exact labels when the
  daemon omits an agent name; durable records stay untouched.
  Refusals and access changes remain visible as plain language. Never hide
  unknown failures or rewrite human messages. System rows wrap legibly; grouped
  messages keep the avatar gutter with time revealed on intent.
- Agent binding status is visible on every host, including browser and extension.
  Active bindings read "Authorized": room access alone does not establish
  provider connectivity, workspace availability or successful inference.
  Fresh authority/bootstrap/refusal SSE messages refresh bindings through the
  existing room-generation and latest-request guards; duplicate replay rows do
  not dispatch refreshes. Authority/bootstrap records without an agent suffix
  still refresh bindings; known anonymous access/refusal notices remain readable,
  while unknown refusal codes retain their original text. Native consent controls remain native-only.
- Room creation reveals its optional project picker and Create action only after
  a name is entered. Enter and Create share the same guarded operation.
- Coworkers use both Macs and Windows. Installation, updates and acceptance
  must account for both; a macOS-only package or local smoke test is not team
  readiness. Keep Rooms conversation-first, with quiet sentence-case headings,
  one secondary menu, and no raw filesystem setup instructions in the main flow.
- Attention (P6, `room_attention.rs`, `styles/rooms-attention.css`): the
  left-rail @ icon swaps the room list for the mentions inbox
  (`GET /v1/rooms/inbox`, also forwarded by the proxy; render the daemon's
  `author_name`); room search input is capped at the daemon's 200 characters;
  inbox picks dismiss the compact room drawer; both inbox and search picks queue
  their thread via an origin- and room-key-scoped `Rooms.focus_thread`, consumed
  only after that room record and root load. Action rows retain native button
  semantics inside list-item wrappers. Inbox/search responses retire on newer reads, changed daemon origins, room admissions, query edits, and panel cleanup. Mute errors only publish into their originating room. The
  header search icon opens room search in the right rail (thread > search >
  members). Muted rooms (`GET/PUT .../prefs`) dim and drop the unread dot.
  The line above the composer shows agents with a live run. Coworker typing
  presence is not built: it would cross Bedrock and needs an operator
  trust-boundary decision. Empty states carry no copy.
- Room files (ported from Risingtides-dev/ocean-surface `attachments.rs`,
  legacy main `1b88f862`; now `src/rooms_workspace/attachments.rs`): the
  details drawer's Files mode (Room options → Files; same reveal, Escape and
  return-focus rules as Members) lists the open room's context files from
  `GET /v1/rooms/persistent/{key}/attachments` and links each at
  `GET .../attachments/{id}` with `<a download>` — the daemon serves a type
  DERIVED from the bytes with `nosniff` + `Content-Disposition: attachment`,
  so the declared type only picks a glyph and nothing is rendered inline. The
  composer's paperclip and the panel's `+ file` forward to one hidden
  `<input type=file>` mounted at `RoomsWorkspace` scope (`AttachmentInput`,
  outside the `open_room` closure a roster write rebuilds); the pick POSTs
  the file's raw bytes to `.../attachments?filename=&content_type=&uploader_id=`
  — metadata in the QUERY, never multipart or a custom header (daemon CORS
  allows only `content-type`/`authorization`) — with `uploader_id` = the
  `/v1/me` owner, after the daemon's own guards run client-side first (8 MiB
  cap, empty file, unusable name; an empty declared type is sent as
  `application/octet-stream`). `RoomAttachmentsState` lives at workspace
  scope: the admission Effect `reset()`s it (ticket bump retires in-flight
  lists, the upload flag goes with the room, the Files mode closes like the
  other rail modes), an upload completion re-validates `(generation, key)`
  through `room_is_current`, and the list is re-read after a success (the
  daemon's System marker row arrives on the tail but does not carry the
  attachment). Both controls take `access::local_store_write_gate`
  (Local/Connecting/Live/Recovering; Revoked and unknown held) — NOT
  `access_allows_writes` — because the write lands in this daemon's store
  and is never enqueued for a peer; identity is refused at the action, not
  the control. `DELETE .../attachments/{id}` exists on the daemon and is not
  called (the legacy surface designed no remove control). Web proxy
  (`rooms_persistent_shape`): the upload lane forwards the raw body up to
  `ATTACHMENT_UPLOAD_BODY_LIMIT` (8 MiB + 4096 slack, so a just-over-cap body
  still earns the daemon's typed 413) under the declared content type, the
  download lane relays Content-Type / Content-Disposition /
  X-Content-Type-Options verbatim (re-stamping `application/json` would have
  undone the daemon's XSS defence), every other shape keeps the 1 MiB JSON
  ceiling, and no operator key is injected
  (`cargo test -p ocean-surface-proxy room_attachments`). Guards:
  `tests/room_attachments_contract.rs` (route literals in the query form,
  CSS ↔ emitter parity, tokens-only CSS, workspace-scope mount, gate choice,
  daemon-constant parity) and `tests/rooms_product_doc_truth.rs`.
- The room wire contract is pinned in-tree, not vendored (supersedes the
  snapshots of legacy ocean-surface#225/#233): `src/rooms/wire_fixtures.rs`,
  plus `daemon_attachment_fixtures_decode_through_the_production_folds` in
  `rooms_workspace/attachments.rs`, decodes every file in
  `platform/ocean-os/crates/ocean-daemon/tests/fixtures/room-wire/` with the
  production decoders — `OwnerIdentity`, `RoomMutateResponse` (create, join),
  `RoomGetResponse` (room, transcript, `has_more`/`next_seq`, `access`),
  `RoomErrorResponse`, `RoomTranscriptPageResponse` (both halves of the cursor
  pair), `RoomsListResponse` (both pages: `read_states`, `next_cursor`,
  `muted`), `ReadCursorPatchEnvelope` through `parse_patch_read_cursor_response`,
  `RoomAgentSettingsEnvelope`, `parse_agent_bindings`, `RoomInvite`,
  `RedeemInviteResponse`, `RoomAccessProjection` for all five states,
  `fold_list_response`/`fold_upload_response`, and `decode_room_tail_frame` for
  the four SSE event names with one `room_agent_run` per `RoomAgentRunState` —
  and asserts the fields the panel reads round-trip; every fixture file must be
  claimed by one of those readers. The daemon writes the files from its real
  router (`OCEAN_UPDATE_WIRE_FIXTURES=1 cargo test -p ocean-daemon
  room_wire_fixtures` in `platform/ocean-os`), so a daemon serializer change
  goes red there first and here second. Never hand-edit a fixture; a decoder
  change here must still decode the committed files
  (`cargo test -p ocean-surface-ui fixtures`).
- In `styles/rooms-workspace.css` the "Layout & depth (canonical)" section is
  the final say on geometry, density, and depth; change values there rather
  than adding another override layer. Colors only from `tokens.css`.
- Owner responses publish only for the latest owner request at the current
  daemon origin. Origin changes clear the old owner and refresh `/v1/me`;
  an older response cannot overwrite a later owner read.
- Agent participants are selected from daemon-owned `/v1/agents` identities
  and remain subject to daemon join validation. Free-text agent creation does
  not belong in the surface. Picker keys, values, and labels use the canonical
  folder id `AgentSummary.name`; entries with resolution errors stay disabled.
- Local rosters and mention ids come from `Room.participants`; every non-Local
  roster and mention id comes only from the safe access member projection.
  Composer writes are enabled only for `Local` and `Live` access.
- Federated outbox items render outside the confirmed transcript. Pending items
  are informational; only failed items expose the daemon retry action, and the
  returned access projection applies immediately behind the room-generation
  guard before any duplicate SSE projection arrives.
- Invite creation and redemption use daemon-owned outbound routes. Native
  `ocean://room/{key}/join?code={code}` links are strict attacker-triggerable
  input: stage them for explicit confirmation, send codes only in POST bodies,
  and never log or persist codes. Codes appear only in explicit sharing or
  invitation-entry controls. The room list's single Add room disclosure offers
  Create room and Join room on every host. Browser invitation entry reuses the
  native-link parser, rejects session links, clears its draft on cancel/staging,
  and stages the existing confirmation without redeeming. Focus moves to the
  confirmation; Escape cancels entry before it can close Rooms. Joining still
  uses this client's own daemon identity, never the invitation sender's proxy.
- Redemption completion owns only the exact accepted invite revision and room
  navigation generation; stage, dismiss, or navigation invalidates it. The
  existing Join control blocks duplicate submission of that pending intent.
- Federated Share requires `Live` access and the daemon's `caller_member_id`
  matching a projected `User` with `Owner` role. Browser-local identity is not
  authority. Missing caller data disables federated Share; Local is unchanged.
  Accept the operated daemon's equivalent `self_member_id` as a read alias;
  serialize only `caller_member_id`. Duplicate caller fields fail decoding.
  Both names come from daemon credential projection, never browser identity.
- Rooms G1 is daemon-native text collaboration. LiveKit controls stay outside
  the room join, leave, roster, and transcript lifecycle until explicitly
  reintroduced behind a reviewed platform contract.
- The Rooms rail (`.rooms-workspace__left`, `styles/rooms-workspace.css`) is
  a flex column; its list `.rooms-workspace__left-list` (rendered from
  `rooms_workspace/room_list.rs`) keeps `min-height: 0` with `overflow-y:
  auto` so long room lists scroll instead of pushing the create field and
  status line outside the viewport. Guarded by
  `rooms_list_flex_child_can_shrink_and_scroll` in `app.rs`.

### Native Room agent consent consumer (ocean-private #65)

- Builds on the daemon digest guard and invited-Member own-agent consent
  (#65, on main). Owned targets: `crates/ocean-tauri/src/room_consent.rs`, its
  `lib.rs` registration (`ConsentState` managed after the plugins; the four
  consent commands and `room_agent_bootstrap` follow
  `owner_mutation::room_owner_mutation` in `generate_handler!`), its
  `Cargo.toml`/`Cargo.lock` (`libc`, `uuid` v4 only),
  `crates/ocean-surface-ui/src/{host.rs,rooms.rs,daemon.rs}`,
  `src/rooms_workspace/agents.rs` (mounted from `mod.rs` and `members.rs`),
  existing `styles/rooms-workspace.css` and this contract. No daemon, proxy,
  store, policy or provider change.
- Native operator custody lives in two shell modules: `owner_mutation.rs`
  (rename owner, run permission, agent settings; see above) and
  `room_consent.rs` (Room-agent consent and revoke). Both send through the
  one shared `daemon_transport.rs` (below). `room_consent.rs` fetches the
  preview itself and freezes room, package, agent, owner, digest, policies
  and a fresh UUIDv4 decision id; the
  page gets a projection and an opaque consent id, never a digest it can
  supply. Authorize and revoke share at most 4 pending decisions, with a
  120 s TTL; a newer preview supersedes an idle decision for the same agent.
  The TTL covers both the dialog and blocking-worker queue. Dispatch claims
  a still-owned, unexpired queued decision under the native mutex, then
  releases that mutex before I/O; only a dispatch-claimed send keeps its slot.
  Preview/bootstrap capture a native cancellation epoch before entering the
  blocking queue, and may insert a preview only if that epoch is still current.
- Authorize/reauthorize require a native dialog (`tauri-plugin-dialog`, Rust
  side; the webview gains no dialog permission) and always send the frozen
  non-null `expected_definition_digest`. The authorize dialog shows only
  natively frozen values: room, package id, agent member id, owner, the
  activation the binding will carry, requested capabilities and digest.
  Decline, expiry or cancellation received natively
  before the dispatch claim sends zero authorization/revoke requests.
- Activation choice (ocean#167 follow-up; ruling from
  `docs/OCEAN_ROOMS_DESIGN_DIRECTION.md` §5.1 step 3, "three radio groups
  with the daemon defaults selected and nothing else pre-ticked: activation
  explicit_only / mention / task_and_thread"): the consent sheet renders the
  activation radios from `rooms::ConsentActivation::ALL` with the design's
  labels ("only when invoked" / "when @mentioned" / "mentions, tasks and
  thread replies") and one consequence line each; `RoomConsentDraft` opens on
  the shell's frozen `activation_policy` (the daemon default `explicit_only`
  for a new binding, the binding's current policy for a reauthorization,
  `unwrap_or_default` otherwise) and `authorize_consent` passes
  `draft.activation.wire()` to `host::room_consent_authorize(consent_id,
  activation_policy)`. The shell command takes `activation_policy:
  Option<String>`, maps it onto `room_consent::Activation` (exact wire names;
  anything else is `invalid_request` before any dialog, and no page text ever
  reaches the body), `begin_confirm(consent_id, Some(activation))` adopts it
  under the native lock so the dialog and the body name the same choice, and
  `Frozen::intent` serializes `Activation::as_str()`. A binding whose
  activation the shell cannot name is `invalid_preview`, never narrowed to the
  default. `Entry::dispatched` is set at the dispatch claim: a decision that
  reached the daemon once (unknown outcome) replays only unchanged — the page
  locks the radios (`RoomConsentDraft::activation_locked`), and a different
  activation on that consent is `consent_invalidated` and drops it. Context
  policy and memory scope keep the daemon defaults; no control. Guarded by
  `preview_offers_the_daemon_default_activation_and_the_page_choice_is_what_is_sent`,
  `activation_is_a_closed_set_parsed_natively_and_refused_before_any_dialog`,
  `reauthorize_preview_carries_the_binding_s_activation_and_refuses_an_unknown_one`
  and `a_dispatched_decision_never_replays_under_a_changed_activation`
  (`room_consent.rs`), the `consent_activation_*` / `consent_draft_*` tests
  (`rooms.rs`) and `tests/rooms_consent_activation.rs` (page, shell and
  daemon name the same three policies and default; the sheet, bridge, command
  and body wiring; the deciding design text and `docs/OCEAN_ROOMS_PRODUCT.md`
  sentences).
  `room_consent_cancel(None)` clears all tracked decisions and advances the
  epoch, including while a revoke dialog or preview/bootstrap read is pending.
  This boundary is native receipt, not a page navigation click or IPC enqueue:
  after the dispatch claim, a request may finish, but cancellation prevents its
  late completion from restoring retry state. Bootstrap side effects are not
  rolled back. A definitive reply consumes the decision; a lost acknowledgement
  is `unknown`, never retried automatically, and an explicit re-confirmed retry
  replays the same decision id. Revoke registers native custody before its
  dialog, invalidates older authorization for that target, and uses a fresh
  decision id unless explicitly retrying a still-live unknown decision; its
  dialog names the exact room and `agent_member_id` being revoked.
- Native dialog text: member ids and capabilities containing controls,
  line/paragraph separators, bidi or invisible formatting characters are
  refused (preview `owner_not_eligible`/`invalid_preview`, revoke
  `invalid_request`) so they are shown verbatim; display names are labels
  only, stripped of those characters and capped at 128 characters.
- Consent transport: the shared `daemon_transport.rs` (below) with fixed
  typed intents (`Intent::{Preview, Authorize, Reauthorize, Revoke, Me,
  Bootstrap}`; only the preview and `Me` are unprivileged). A transport bad reply (hostile chunk size,
  redirect, non-JSON, oversize) on a decision is `unknown` and returns an
  uncancelled entry to Ready for a manual same-decision retry, never a panic;
  an untrusted `OCEAN_DAEMON_URL` makes every consent command
  `consent_unavailable`. Consent commands refuse any webview other than
  `main` (`consent_window_refused`).
- Local bootstrap (PR G): a plain Local `POST .../participants` never
  records a Room owner, so its preview is `owner_not_eligible`. On native
  consent hosts a Local add (`rooms::agent_add_route` → `Bootstrap`) calls
  `room_agent_bootstrap` (main window only, `consent_window_refused`). Rust
  reads the owner from `GET /v1/me` `participant_id` (unprivileged; the page
  never supplies an owner), then sends the operator-gated
  `POST .../agents/bootstrap` with exactly `{owner_member_id,
  agent_package_id}`. 200/201, or a `room_owner_conflict` 409, proceed to
  the normal preview freeze, which is kept only when its owner equals the
  `/v1/me` participant (else `room_owner_conflict`); the page gets the same
  projection as `room_consent_preview` and lands in the consent sheet.
  Bootstrap creates no binding: authorize still needs the native dialog.
  Fixed codes: `room_owner_required` ("Join the room before adding an
  agent."), `local_room_required`, `room_owner_conflict`,
  `bootstrap_target_conflict`, `room_not_found`, `agent_package_not_found`,
  `owner_unavailable`, else `bootstrap_refused`; a lost or unparseable
  bootstrap reply is `outcome_unknown`, fetches no preview and is never
  retried (the page re-reads the roster). Text lives in
  `rooms::bootstrap_error_text`. Web/extension hosts keep the plain
  participant add and get no bootstrap control.
- Binding GETs publish only for the newest request in the current Room generation; reset retires pending reads, and failed refreshes keep the last projection without reviving older responses.
- Surface: `host.rs` wrappers are absent off-Tauri, so browser/extension hosts
  render bindings but never emulate consent. `rooms.rs` owns the bindings
  client (`GET .../agents`), the consent draft (room generation + ticket
  guarded; committed or unknown outcomes refresh authoritative bindings even
  when navigation raced; post-send command errors `consent_task_failed` and
  `invalid_reply` are treated as `unknown`) and explicit invocation.
  `rooms::invoke_caller(access, open_room, identity_id)` names the invoker
  from daemon projections only: in Local rooms (PR H) the `/v1/me` owner
  `participant_id` (`Rooms.identity_id`, since #68) when non-empty and a
  Human participant on the open room's roster; in Live/Recovering rooms the
  daemon-projected `caller_member_id` naming a User (unchanged); every other
  state has no invoker. Browser-supplied identity is never authority. The
  daemon's Local invoke check matches (non-agent roster member who authored
  the message; not operator-gated). Invocation then needs a committed Human
  message by that caller and an active binding, runs once per (agent,
  message) and leaves a lost acknowledgement `unknown`. A refusal shows
  `rooms::invocation_error_text` in the composer status line (and as the
  invoke-menu badge's title), e.g. `workspace_unavailable` → "This room has
  no workspace folder for the agent.", `agent_binding_required` → "This
  agent is not authorized in this room.", author/membership refusals, and
  `room_not_found` (a soft-closed room) → "This room is closed."; unknown
  codes read "Ocean could not invoke the agent."
- Invocation sends no decision token (removed in PR C, together with
  `RoomInvocation::token_for` and #88's `try_mint_decision_token` split in
  `daemon.rs`): since #68 the daemon binds Room-turn permission waiters to a
  daemon-minted per-run token, and Room approvals go only through the
  operator-gated `POST .../runs/{run_id}/permission` route above. The daemon's
  invoke body keeps `decision_token` optional, so omitting it is accepted. Do
  not reintroduce a token-gated Room permission path.
- Own-run matching (`rooms::own_invocation_for_run`, pure): each
  `RoomInvocation` records its room key, page room generation and the bound
  `agent_package_id`. A work card is "yours" only for an `Acknowledged`
  invocation in the open room at the current generation; pending, unknown and
  refused invocations never match. A run carrying the daemon `request_id`
  (#93) matches exactly the invocation acknowledged with that id and session;
  a legacy run without one falls back to session id == `run.session_id`,
  message seq == `run.trigger_seq` and package id == `run.agent_id` (federated
  runs record the package id; Local runs record the participant id, so a
  Local invocation's run matches only by `request_id`, tested with its Cancel
  control, and a legacy Local run fails closed), and any other run sharing
  that session and seq makes it ambiguous, so neither matches. The card shows a
  display-only "yours" tag (`Rooms::run_is_own`); approve/deny stays on the
  operator route for every card, and the match never reads or writes
  `Daemon::permission_view`, decision authority or `Daemon::cancel_request`
  (focused direct-chat state is covered by a test).
- Own-run status and exact cancel (PR E): the invoke menu shows, for an
  `Acknowledged` invocation, the live state label of the single run
  `rooms::run_for_invocation` attributes to it (via `own_invocation_for_run`);
  no run yet, or several (a legacy double run on one session+seq), keeps the
  acknowledged label ("queued"). The work card shows one Cancel icon
  (`.room-card__cancel`, `aria-label` "Cancel this run", tokens only in
  `styles/rooms-cards.css`) as a sibling of the disclosure head (never nested
  in it), only while `rooms::own_run_cancel_target` holds: the page's own
  acknowledged invocation in the current room generation, not ambiguous, and
  the run not terminal. A click re-derives the target and calls
  `rooms::cancel_room_request(base, request_id)` →
  `POST /v1/requests/{request_id}/cancel` with the invocation's exact
  acknowledged request id (equal to the run's `request_id` when projected),
  no body and no operator key (the daemon route takes none; the web proxy
  already forwards it, see #49). `RunCancels` latches one request in flight
  per id; the reply settles only in its own room admission; success changes
  nothing (the card ends Cancelled only when the daemon's `room_agent_run`
  frame says so); transport failure, non-2xx or `ok: false` shows one inline
  "Ocean could not cancel this run." and is never retried automatically (an
  explicit click may try again). It never calls `Daemon::cancel_request`, a
  session-level cancel, or touches focused direct-chat state (tested).
- UI (`rooms_workspace/agents.rs`): one consent sheet (name, short digest,
  requested capabilities, the activation radios, one primary action, dismiss)
  above the right-rail list (styles `.rooms-workspace__consent-activation*`,
  tokens only); bound agent rows get a single overflow holding Review/Revoke; the
  message action rail gets one invoke trigger after the thread button listing
  active agents with their invocation state. Menus reuse the header's
  `.room-overflow__*` styles; their in-row geometry lives in the "Layout &
  depth" section. The `+ agent` picker hides agents already in the room
  (Local participant ids, plus any non-revoked binding's package id, since
  federated rosters project member ids) and keeps unresolved ones disabled; on federated rooms it is mounted only on
  native consent hosts in Live/Recovering access.
- Verification: `cd crates/ocean-tauri && cargo test room_consent` (disposable
  loopback daemons and key fixtures), `cargo test -p ocean-surface-ui`,
  WASM check/Clippy. Remaining: served native acceptance.

### Shared native daemon transport

- Owned targets: `crates/ocean-tauri/src/daemon_transport.rs` (the only
  place the shell attaches `X-Ocean-Operator`), its callers
  `owner_mutation.rs` (`OwnerMutation` routes + the `room_owner_mutation`
  command and its `OwnerMutationState`) and `room_consent.rs` (`Intent`s),
  and their `lib.rs` wiring (one `DaemonTransport` built from
  `daemon_url_from_env()` + `operator_key_path()`, cloned into
  `OwnerMutationState`, `ConsentBroker`, `provider_accounts::AccountState` and
  `team_connection::ConnectionState`).
  Account intent is governed by Desktop Accounts above. Callers keep typed intent enums
  on top; JS never supplies a method, path or header.
- Rules (strictest of the two former transports): daemon address from
  `OCEAN_DAEMON_URL` restricted to `http://<numeric loopback>[:port][/]`
  (hostnames incl. `localhost`, userinfo, TLS, paths/queries refused);
  request paths must be `/v1/…` visible ASCII; request bodies ≤64 KiB; the
  operator key is read per privileged request via `O_NOFOLLOW` (symlink
  refused, opened dev/ino must equal the inspected file, owned by the
  effective uid, no group/other bits, one 16–512 byte base64/base64url
  token) and never reaches JS, errors or logs; headers are exactly `Host`,
  `Accept`, `Connection: close`, `Content-Type`/`Content-Length` with a body
  and the operator header; 3xx replies are refused; replies must be JSON
  objects ≤256 KiB with checked chunked framing.
- Outcomes: `NotSent(code)` (`daemon_not_loopback`, `invalid_request`,
  `request_too_large`, `operator_credential_*`, `daemon_unreachable`),
  `Unknown` (request written, acknowledgement lost; code `outcome_unknown`),
  `BadReply(code)` (`redirect_refused`, `invalid_reply`, `reply_too_large`).
  Nothing retries automatically. `room_owner_mutation` returns these codes
  as its error string; the page already maps every error to its
  unreachable text.
- Intentional behavior changes for owner mutations (consent is unchanged):
  `OCEAN_DAEMON_URL=http://localhost:…` (or any hostname, `https://`, or a
  URL with a path) now disables owner mutations instead of sending to
  `localhost`; any numeric loopback address (`127.0.0.0/8`, `[::1]`) is
  accepted. An owner-only `0400` key is now accepted (formerly exactly
  `0600`), while a key owned by another user, a key opened through a
  swapped-in symlink, or a key that is not a header-safe token is refused.
  Redirects, non-JSON or non-object replies and replies over 256 KiB
  (formerly 1 MiB of any text) are errors instead of being relayed; the
  daemon's owner routes always answer JSON objects. Error strings are now
  fixed codes. The command refuses any webview other than `main`
  (`owner_mutation_window_refused`).
- Windows native transport uses the daemon's `ocean_observatory::auth::windows_file`
  reader: protected current-user-only ACL, regular single-link file, no reparse
  points, bounded read and the same header-safe token predicate. It never repairs
  unsafe files. Default config uses absolute HOME or USERPROFILE; explicit config
  and key paths retain precedence. Unix ownership/mode behavior is unchanged.
  The target-only dependency shares file validation, not runtime policy.
- The transport, owner-mutation and consent fixtures run on Windows and Unix;
  Windows fixtures create protected files or deliberately inherited ACLs. The
  manual native workflow builds the real UI and shell; no test bypasses the
  existing native confirmation, frozen preview or main-window gates.
- Verification: `cd crates/ocean-tauri && cargo test daemon_transport`
  (loopback refusal, key-file refusal incl. foreign owner, redirect,
  oversize/invalid reply, lost-ack → unknown), `cargo test owner_mutation`,
  `cargo test room_consent`, `cargo clippy --all-targets -- -D warnings`.

### Coding plans (web identity M3)

- `src/coding_plans.rs` is the panel for this daemon's Claude/Codex/ChatGPT
  plan logins over the daemon's five operator-gated routes
  (`platform/ocean-os/crates/ocean-daemon/src/provider_auth.rs`:
  `GET /v1/auth/providers`, `POST …/{provider}/login`,
  `GET|DELETE …/{provider}/login/{attempt_id}`, `POST …/{provider}/logout`).
  It renders what the daemon reports (status, `source` hint, expiry only
  once passed, a pending `login` adopted on open) and collects intent; it
  never holds a token. The monorepo proxy fronts one daemon, so sentences
  name "the machine running this Ocean" rather than a picked device. Reached
  from the header `⋯` overflow row and the ⌘K `coding-plans` command, both
  calling `CodingPlansState::show`; `tests/unheld_coding_plans_controls.rs`
  pins the overflow row (the phone's only door) and the `supported` gate.
- Transport is the one seam `call`, mirroring the Room owner mutations:
  web/PWA sends a same-origin fetch and the proxy attaches the operator key
  (`crates/ocean-surface-proxy/src/coding_plans.rs`: exact five shapes,
  dot-segment refusal, 4 KiB body cap, key via `owner_operator_key` so
  `OCEAN_SURFACE_AUTH=off` attaches nothing and the daemon keeps refusing;
  upstream built fresh so Cookie/Origin/Referer/forged operator headers never
  pass); Tauri calls `host::coding_plan_request(kind, provider, attempt)` with
  a fixed `kind` (`list`, `login_start`, `login_poll`, `login_cancel`,
  `logout`) and the shell (`crates/ocean-tauri/src/coding_plans.rs`) builds
  the method and path itself over the shared `daemon_transport.rs`
  (`Method::Delete` added for cancel; ids restricted to `[A-Za-z0-9_-]{1,64}`).
  The Chrome extension has neither, so `coding_plans::supported()` hides the
  command, the overflow row and the panel there.
- Sign-in opens a blank tab synchronously in the click (popup-blocker safe,
  `opener` severed) and points it at `authorize_url` once the daemon
  answers; on Tauri `host::open_external_url` hands the URL to the OS
  browser. The panel then polls the attempt every 2 s, bounded by 300 polls
  or 5 consecutive transport failures; attempts retired on those bounds are
  never re-adopted from the list. Closing, cancelling or a newer intent
  retires the loop by generation; a list reply publishes only for the newest
  refresh ticket. Sign-out is confirmed in place and hidden for
  environment-sourced credentials. Every refusal code (`login_unavailable`,
  `operator_credential_unavailable`/`_missing`, `unknown_attempt`,
  `unknown_provider`, `logout_failed`, the proxy's 502) and every attempt
  failure classification renders as a sentence, never the code. The
  same-machine caveat is stated once per attempt: the provider redirects to
  the daemon machine's localhost, so a browser elsewhere cannot finish it.
- Verification: `cargo test -p ocean-surface-ui coding_plans`, the proxy's
  `cargo test coding_plan`, `cd crates/ocean-tauri && cargo test coding_plan`.

## Workspace Files lifecycle

- Native Open Externally is a document action. On Windows it refuses known
  executable, script-host, installer, registry, shortcut and application-link
  suffixes, case-insensitively and after trailing dot/space normalization.
  The canonical target is checked before the OS opener; caller-supplied root
  containment is not an execution permission. Unix executable-bit/Mac suffix
  behavior is unchanged. This is a known-launcher guard, not an audit of every
  user-customized file association. Verify with `cargo test --lib document_open`
  and `cargo test --lib open_file_refuses_targets_that_would_execute` in Tauri.


- Native empty Files exposes the host folder picker as pane-local browsing; picking never rewrites daemon session cwd. Session/cwd intent changes retire picked roots, previews, tree caches and asynchronous results synchronously, including A-B-A; disposal rejects late picker/watch/read completions.
- Files acquires one native root lease per generation and drops only that lease on change or cleanup. `watch_paths`/`unwatch_paths` accept an optional owner token scoped by the injected native window label; scoped canonical maps are independent of legacy Repo watchers and other consumers/windows. Page-load Started and window destruction retire only that window’s scoped leases; a native page generation rejects pending old-page admissions, while Finished leaves new watches intact. Events carry the lease owner and canonical paths map back to the requested alias. Never clear the legacy canonical map for Files cleanup.
- The release fixture `scoped_native_watch_release_keeps_alias_peer_and_legacy_watch_live` (`crates/ocean-tauri/src/lib.rs`) drives real OS watches. `watch()` returns once a stream is scheduled, not once the OS delivers for it (FSEvents registers asynchronously), so each stream must prove it is live on a probe write before the release is performed, and the survivors are judged on a second, distinct file by path; a report from the released stream fails the fixture. Never add a post-`watch()` sleep or judge liveness on a write that preceded the probe. Verify with `cd crates/ocean-tauri && cargo test --locked watch_admission` (needs the ignored `dist/index.html` stub).
- The private `host/native_events.mjs` adapter belongs to the `host.rs` seam and uses Tauri 2 IPC callback registration and `plugin:event|listen/unlisten`; legacy subscribers retain their app lifetime. Workspace path listeners belong to the component; cleanup unregisters even late native registration. Ignore events outside the root by path-component boundary. All transcript tool argument previews, including browser tools, are capped at 60 Unicode characters; no remaining consumer parses whole preview JSON.
- Verify the production event adapter with `node --test scripts/native-events.test.mjs`; its mocked pinned Tauri API exercises delivery, cancellation, late admission and failure cleanup.
- Pinned cards use the vertical gutter only with the workspace closed; open-workspace layouts retain the horizontal strip.
- Below 900px, the existing workspace toggle stays above the open pane in a safe-area-aware corner outside its scrolling tabs. It uses the same pane visibility state for dismissal and reopening; do not add a second permanent close control. Desktop docking and reveal/Escape policy stay unchanged.

## Workspace Map

| Path | Role |
|---|---|
| `crates/ocean-surface-ui/` | Leptos WASM web/PWA/extension UI |
| `crates/ocean-tauri/` | Tauri 2.x native desktop shell; loads the same dist/ bundle as the browser PWA |
| `crates/ocean-surface-proxy/` | axum proxy for web bundle, STT/TTS relay to the daemon, config, daemon reverse proxy |
| `extension/` | Chrome extension wrapper around the Leptos surface |
| `vscode-extension/` | Cursor/VS Code ACP client surface with sidebar, bottom-panel, editor-tab, and status-bar launch modes |
| `legacy-voice/` | reference voice code only; do not build new architecture here |
| `crates/ocean-gui/` | ABANDONED GPUI desktop app (2026-07-21) — dead, do NOT work on it |
| `crates/ocean-gui/canvas-web/` | ABANDONED (part of the dead GPUI app) — do NOT work on it |

The proxy holds no provider credentials. `/api/stt` and `/api/tts` forward to
the daemon's `/v1/voice/stt` and `/v1/voice/tts`, where `ocean-os` resolves the
xAI key per-request (env `XAI_API_KEY` / auth.json `xai` block). Do not add
provider credentials or direct provider calls to the proxy. Google Maps is
optional and enabled only by an explicit non-empty `GOOGLE_MAPS_API_KEY`; never
commit an organization-owned browser key or restore a compiled-in default.

### Request-status forwarding (#49)

- Base: canonical main `e18678472bfff45e2ba2adeb750fa284adcde16c`.
  Targets are only proxy `src/main.rs` (including registered-router fixtures)
  and this contract.
- Forward exact `GET /v1/requests` through the existing session-authenticated,
  timed JSON rail, preserving the raw query, upstream status and response bytes.
  Keep `POST /v1/requests/{id}/cancel` and existing login/public-asset behavior.
- Inject no operator key or client authorization headers. Add no wildcard,
  request state, Room metadata, permission authority, UI or dependencies.
  Request/session/state metadata does not establish automatic Room-agent state
  or Rooms R5 acceptance; rendered/live acceptance remains issue #48.
- Fixtures drive the production router against disposable loopback upstreams
  and assert successful/error passthrough, exact method/path/query/body and
  unauthenticated refusal before forwarding. Root owns locked full proxy tests,
  check, Clippy and independent exact-head review; no live service or owner auth
  is a fixture input.

### Browser pane forwarding (ocean-private #100)

- Expose only the exact daemon `GET /v1/browser/screencast` SSE and
  `POST /v1/browser/input` JSON endpoints through the session-authenticated
  PWA proxy. Tauri continues to use its direct daemon URL.
- Stream screencast bytes through the existing untimed SSE client and
  `sse_stream_response`; use the bounded JSON client for input and preserve
  only the upstream status and JSON response bytes. Do not add wildcard
  forwarding, client credential/header forwarding, CORS, or daemon authority.
- Drive the production router in fixtures: prove the screencast response
  arrives before the upstream stream closes, input method/body/status
  passthrough, and refusal of both routes before upstream access without a
  valid existing proxy session. Do not use live daemon/browser state.

### Upstream paths and the auth-off cross-site gate (ocean-surface#230 port)

What the client sends is what the daemon gets, or nothing. The daemon behind
the proxy has no auth of its own for most routes, so the proxy's route table
and these checks are the real allow-list of browser-reachable authority.

- Every forward builds its upstream URL with `upstream_url(daemon_url, path,
  query)`, which parses the URL and refuses (`400
  {"ok":false,"error":"upstream_path_rewritten"}`) unless the parsed path is
  byte-for-byte the path the proxy decided to forward. The `url` crate
  normalises on parse (dot segments collapse, `\` becomes `/`, tab/newline
  vanish), and each of those once let an approved path arrive upstream as a
  different route. Callers hand the returned `reqwest::Url` to the client, so
  the value checked is the value sent. Never add a forwarder that formats a
  `String` URL and passes `&url` to reqwest. In `proxy_send_json_as` and
  `proxy_rooms_persistent` the check runs before the operator key is
  attached, so a refused path never carries the credential.
- `has_dot_segment` refuses, per `/`-segment and after one percent-decode,
  `.`, `..`, and any backslash (raw `\`, `%5C`, `%5c`). Answer: `400` plain
  text `invalid path`. hyper accepts a raw `\` in the request target, and the
  segment split did not see it as a separator while the URL parser did:
  `GET /v1/rooms/persistent/team\..\..\..\agent\sessions` reached
  `/v1/agent/sessions`. It runs on the raw wildcard paths (rooms-persistent,
  longhouse), on every captured `{id}`, and on the GitHub template captures.
- A captured `{id}` is percent-DECODED by axum, so it is rebuilt with
  `daemon_path_with_segment(prefix, id, suffix)`: dot-checked, then
  re-encoded into exactly one segment (sessions, agent sessions, projects,
  session messages and steer, permission decision, request cancel, LiveKit
  room id). Formatting a capture raw let `%2F` become a second segment and
  `%3F` start a query.
- Wildcard families forward the RAW request path (`req.uri().path()`), never
  the decoded `{*rest}` capture.
- Both halves are needed and each is tested alone
  (`src/tests/path_csrf.rs`): `%5C` is not rewritten by the parser (only the
  dot guard stops it), and a future normalisation rule is stopped only by the
  parse-and-compare. The tests drive the production router over `oneshot`
  AND a real TCP socket against a recording upstream, so "refused" means an
  empty upstream log.
- Legitimate clients are unaffected: the Surface bundle percent-encodes ids
  and room keys with the unreserved alphabet (`rooms::encode`,
  `daemon::encode_path_segment`) and query values with `encodeURIComponent`,
  so its paths parse back to themselves. Tauri, the VS Code extension and the
  Chrome extension reach the daemon directly and never cross this proxy.
  Fail-closed side effects: a hand-built request target carrying bytes the
  parser re-encodes (non-ASCII, spaces) is refused rather than forwarded
  re-encoded, and a room key or id containing a backslash is unreachable.
- Auth-off (`OCEAN_SURFACE_AUTH=off`, loopback-only by contract):
  `auth_off_cross_site_gate`, layered inside the session gate, runs
  `auth_off_mutation_source_allowed` on EVERY non-GET/HEAD request under
  `/v1/` and `/api/`. Every supplied Origin/Referer must be `http(s)` and
  name the exact loopback Host that received the request (case-insensitive;
  another loopback spelling or another port is a different origin); a
  request with neither header is admitted (local scripts/CLIs). Refusal:
  `403 {"ok":false,"error":"cross_site_mutation_refused"}`. Why all of them:
  every proxied POST is reachable without a CORS preflight (a `<form>` or
  `no-cors` fetch), the forwarders stamp `application/json` on whatever body
  arrives, and close needs no body at all. `/csp-report`, `/login`, `/logout`
  are outside it. Reads are never gated.
- Auth-on: no Origin check. The session cookie is `SameSite=Strict`, so a
  cross-site request arrives unauthenticated and the auth gate answers 401;
  keep it Strict. A tunnel or tailnet name is how auth-on is legitimately
  reached, so Host is not a gate there.
- Not ported from ocean-surface#230: the session-actor binding and room-owner
  gate (a multi-device, roster-user proxy model this proxy does not have), the
  JSON content-type lane, and the auth-off every-request loopback-Host
  (DNS-rebinding) refusal, which is a reads-side defence separate from the
  CSRF gate and would turn any non-loopback auth-off reach into a 403 outage.
- Verify: `cargo test -p ocean-surface-proxy path_csrf`.

## Build / Check

```sh
cargo check -p ocean-surface-ui --target wasm32-unknown-unknown
cargo check -p ocean-surface-proxy
cd crates/ocean-tauri && cargo check   # native shell (standalone, not a workspace member)
```

The GPUI crate is ABANDONED — do not work on it, do not scout it for fixes.

For local web/proxy work:

```sh
OCEAN_SURFACE_BIND=127.0.0.1:18790 OCEAN_SURFACE_AUTH=off ./run-surface.sh
```

For LAN/tailnet access, keep the default bind and provide both operator-login
environment variables. `trunk serve` exercises the bundle alone, not the
release proxy or daemon reverse-proxy path.

`localhost:8790` is the operator's live surface and serves the immutable release
selected by `~/.config/ocean-surface/current`, not the repo's `dist/`. The
`dev.risingtides.ocean-surface-auto-deploy` LaunchAgent polls `origin/main`,
builds and gates it in a disposable detached worktree, then selects a paired
release: `releases/<revision>-proxy/{bin,dist}` plus source revision and proxy
checksum. `current` points to its `dist`; the launcher resolves that physical
pair once and verifies the executable checksum. Mutable build targets cannot
replace a selected executable or its bundle.
The rail separates the Git root from the Surface component:
`OCEAN_SURFACE_REPO` accepts the monorepo root, `apps/ocean-surface/`, or the
standalone Surface root. Builds and gates run at the same repository-relative
component path in the detached worktree; bundle/cache/extension sync stays
inside Surface, and native rebuild detection uses its prefixed Tauri path.
The installer renders both launchd templates for the active home and physical
Surface component; installed jobs always use the component path, even in the
monorepo. Credentials stay in the existing owner auth file. Installation remains
stage-only unless `--bootstrap` is explicit: it stages selection, generated
client assets and supervision files, but invokes no launchctl commands. Installer,
explicit promotion and scheduled deployment share one owner lock. Failed sync
or restart restores prior selection, marker, generated client assets and installed
launchers/plists; incomplete recovery retains an owner-only backup with its path
and original failure code. Shared
release custody lives in `deploy/surface-release.sh`.
Before any mutable build, legacy bundle-only selections capture the prior proxy
from the installed component path (or explicit `OCEAN_SURFACE_LEGACY_PROXY`);
missing recovery bytes block migration. Captured legacy source remains `unknown`,
never inferred from a bundle marker. The new launcher supports that retained
legacy selection, and rollback restores old launchers' executable bytes too.
Known releases require matching `/health` `release_revision` and `proxy_sha256` after
restart. Same-revision ticks repair absent/stale listeners without rebuilding;
explicit bootstrap starts enabled jobs even at the same revision. Immediately
before config mutation, it refuses named operator-disabled or malformed overrides
and captures each job's loaded/cold state. Failed bootstrap reloads only previously
loaded jobs; neither bootstrap nor recovery writes enable/disable overrides.
Installer revision stamps require successful clean-source reads and unchanged
HEAD/clean source after the mutable build. Health provenance and a staged
marker do not replace actual running-binary and served-WASM verification. Release
compatibility with the active original daemon remains a separate rollout gate.
`OCEAN_SURFACE_OWNER_HOME` scopes non-live fixtures without changing real `HOME`;
production defaults and owner-only auth custody remain unchanged.
Run `node scripts/surface-auto-deploy.test.mjs` from Surface for the atomic
promotion, rendered installer and both-layout deployment contracts. Its temporary Git fixtures
exercise real worktree and promotion operations with fixture Rust/Trunk tools;
the release rail still requires the actual build and check commands above.
Still verify the served WASM hash and UI contracts on `:8790` itself—never
substitute a private proxy when reporting the live app's state.
Loopback origins must not retain a service worker or `ocean-shell-*` cache;
offline PWA interception is reserved for deployed non-loopback origins.

Native desktop:

```sh
./run-tauri.sh                          # build dist/ + launch Tauri
```

The daemon must be running from `../ocean-os` for live agent behavior.

## Cursor / VS Code Extension UI Contract

- The editor composer uses one native model/effort disclosure. ACP mode metadata owns readiness and model-specific effort options; older bridges use the legacy vocabulary. Keep selected ids intact, retain missing/pinned choices, clear incompatible effort on explicit model changes, and preserve Escape focus restoration. Max must remain supported by settings, the command picker and prompt metadata.
- Keep `vscode-extension/` transcript-first: do not add command decks, fake
  logos, sparkle/AI ornament, or rows of location/action buttons unless the
  operator explicitly asks for that UI.
- The operator did not request a new control-heavy UI for the extension; when
  repairing it, preserve a minimal chat surface and expose extra actions
  through Cursor commands/status entry points instead of visible button sprawl.
- If applying Kami to the extension, use it as restraint only: warm neutrals,
  sparse ink-blue accents, clean type hierarchy, and no copied document or
  landing-page chrome.
- For richer agent-thread affordances, use the installed Codex/ChatGPT
  extension as a product reference for patterns such as compact composers,
  inline tool activity, and file/diff rows. Do not copy proprietary source or
  styling verbatim; adapt the interaction pattern with Ocean-owned code.

## File Preview Deep-Link (Lane D)

**Contract:** FileTreeView captures `daemon.cwd` at mount, creates an
`on_file_click` callback that calls `resolve_file_tree_path` (unified resolver
with explicit-vs-absent provenance branch), and sets `preview_file_intent`. The
app producer Effect reads the intent on every host, opens the workspace,
dispatches `WorkspaceFocus::Preview`, and clears the intent. The workspace
routes it through `open_file` → `open_or_focus` → Preview tab +
fetch-on-cache-miss.

**Resolver:** `resolve_file_tree_path(entry_path, ancestor_prefix, name,
workspace_root, cwd)` — explicit absolute/home-relative passthrough; explicit
relative cwd-authoritative (no starts_with guard); absent path assembles
resolved-root + ancestor + name where relative roots resolve against cwd.
`resolve_file_path` exists for non-file_tree callers and follows the same
cwd-authoritative rule.

**Files:** daemon.rs, app.rs, workspace.rs, components.rs, deck/files.rs
(shared tree helpers), styles/workspace.css.

**Frozen gates:** `cargo fmt --check`, `cargo clippy -p ocean-surface-ui
--target wasm32-unknown-unknown -- -D warnings`, `cargo check -p
ocean-surface-ui --target wasm32-unknown-unknown`, `cargo check -p
ocean-surface-proxy`, `cargo test -p ocean-surface-ui --target
wasm32-unknown-unknown --no-run`, `cargo test -p ocean-surface-ui`.


## Child devlog Index

- `packaging/` — native desktop candidates, bundled daemon/MCP bridge and legal payload →
  `packaging/AGENTS.md`. Existing proxy deployment remains owned by this parent.
