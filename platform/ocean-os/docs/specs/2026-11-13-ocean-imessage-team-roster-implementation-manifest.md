# Ocean iMessage Bridge — Team Roster Implementation Manifest

**Status:** implementation slice 2 landed in source, independently reviewed, and rehearsed against a live daemon with fixture Messages databases. Reading the real Messages database through the installed app and sending a real reply are NOT yet verified (they need operator-granted Full Disk Access and Automation on the host). Operator-managed only.
**Date:** 2026-11-13
**Supersedes:** the fixed-pair admission and sender of [Ocean iMessage Bridge — Local Adapter Implementation Manifest](2026-07-25-ocean-imessage-bridge-implementation-manifest.md). That manifest's process, privilege, and extension-boundary rulings remain in force.
**Operator ruling (2026-11-13):** the agent gets its own number that the whole team texts; each teammate is recognized individually; what a teammate may do is layered by their role.

## Decision

`integrations/ocean-imessage/` admits one-to-one iMessage conversations between
the agent's own identity and an operator-enrolled **roster** of teammates. The
compiled single pair is gone; no phone number is compiled into the package.

```text
inbound:  enrolled member handle -> the agent's own identity   (one-to-one only)
outbound: only to the member whose admitted message is being answered
```

The roster lives in the operator's mode-0600 config beside the daemon URL:

```json
{
  "daemonURL": "http://127.0.0.1:4780/",
  "cwd": "/absolute/workspace",
  "messagesAccountID": "operator-verified-iMessage-account-id",
  "localIdentity": "+15550100100",
  "roles": {
    "admin":  { "agent": "text-admin",  "repliesPerHour": 60 },
    "member": { "agent": "text-member" }
  },
  "members": [
    { "id": "ada", "phone": "+15550100101", "role": "admin", "name": "Ada" },
    { "id": "ben", "phone": "+15550100102", "role": "member" }
  ]
}
```

## Landed slice

- **Roster admission.** A message is admitted only when the sender handle is an
  enrolled member, the destination is the agent's identity, the message belongs
  to exactly one conversation that Messages marks one-to-one (`chat.style` 45,
  no room name), and that conversation's only participant address is the
  sender. SQL pre-filters; Swift repeats the check. Group chats, strangers, SMS,
  attachments, reactions, and other identities signed in on the host stay
  rejected and never leave the reader.
- **Recent messages only.** A message older than 15 minutes is never admitted.
  The cursor moves past everything old enough to be complete, so a stale or
  missing cursor catches up in one poll without answering the backlog, and a
  first run starts from the present.
- **Text from `attributedBody`.** Current macOS stores most message text only in
  the `attributedBody` column (on the operator's host, 1,316 of the last 1,361
  plain inbound iMessages had `text` NULL). The reader takes `text` when present
  and otherwise extracts the string with a narrow typedstream parser that never
  unarchives the blob. A blob that is not exactly the expected layout is not
  admitted.
- **Member identity, not phone numbers.** An admitted message carries the roster
  member id. Phone numbers never enter the prompt, the state file, or logs.
- **Role → agent.** Each role names a daemon folder-as-agent. The turn runs as
  that agent, so the agent's instructions and tool allowlist are what the role
  may do. The bridge refuses a turn when the agent is missing or declares no
  tool names in `agent.toml`. Names the daemon discovers from an agent's
  `tools/` folder do not count, because it lists every file there.
- **One ongoing session per member.** Each member continues their own daemon
  session, bound to the agent that owns it; a role change that selects another
  agent starts clean. A member texting exactly `/new` drops their session.
- **Owner-bound replies.** The reply ledger records which member each admitted
  message belongs to. The sender resolves the recipient from that ledger and the
  roster; no caller supplies a handle.
- **Text-message replies.** Only what the agent says after its last tool call is
  sent, split into at most eight bubbles of at most 1,000 characters. A turn
  that outlasts the acknowledgement delay gets one "working on it" bubble first.
- **Per-member limits and concurrency.** Reply limits are per member, per hour,
  claimed before any model work. Members are served concurrently; one member's
  messages stay in order. A message waiting for its turn is never evicted from
  the ledger; a member with 512 messages already waiting is refused, with a log
  line, until their queue drains. Every claim attempt closes its message, and a
  restart closes messages the previous process left waiting.
- **Strict role agent.** Every turn sends `require_named_agent: true`, so a
  daemon that supports it refuses the turn rather than widening it when the role
  agent is missing, invalid, or matches no tool.
- **Live roster.** The bridge re-reads the roster whenever the file changes.
  Removing or demoting a member takes effect on the next poll without a restart,
  and delivery re-checks enrollment, so a reply to a member removed mid-turn is
  dropped. A missing, non-private, or invalid roster file means no roster.
- **Contained failure.** A failed turn answers that member with a fixed notice
  and never stops the bridge. When the bridge stops waiting for a turn (timeout
  or stream failure) it cancels that turn in the daemon. An unreadable Messages
  database or a missing roster idles and retries with one log line per state
  change. After five minutes without Messages access the process exits once so
  its supervisor starts a fresh one, since a newly granted permission may reach
  only a new process; that is a slow retry, not a crash loop. Log lines are
  timestamped.
- **Stable signing.** `scripts/install-ocean-imessage` signs the app bundle with
  an identity-based signature and refuses ad-hoc signing, so permission grants
  survive rebuilds.
- **Rehearsal mode.** `run --print-replies --state <path>` runs the full
  pipeline and prints replies instead of handing them to Messages. It requires
  its own state file so a rehearsal cannot write into the live ledger.

## Invariants

1. Rejected Messages records never reach Ocean, stdout, bridge state, or logs.
2. Full Disk Access is granted only to the signed bridge app, never to Ocean, a
   shell, or a general interpreter. That one app is reader, daemon client, and
   sender; to send it runs `/usr/bin/osascript` with a constant script, and the
   reply text and handle are positional arguments placed after `--`, so neither
   can be parsed as an option or compiled as script.
3. Only the operator-owned mode-0600 config can change who is enrolled, which
   role they hold, or which agent a role runs. Message content cannot.
4. A reply is deliverable only to the member whose message was admitted. The
   sender exposes no handle parameter, passes the handle and text as positional
   AppleScript arguments, never uses Accessibility or GUI scripting, and never
   chooses a Messages account implicitly.
5. A member removed from the roster is no longer admitted from the next poll
   and can no longer be sent to, including for a turn already running.
5a. A message older than 15 minutes is never admitted, whatever the cursor says.
6. Unknown private-schema fields, missing identity proof, read failure, roster
   anomaly, missing or allowlist-less role agent, oversized input, or rate-limit
   exhaustion fail closed.
7. Message body is untrusted input. It is fenced in the turn prompt behind a
   per-message nonce below a bridge-authored header naming the verified member.
8. State holds only opaque message ids, roster member ids, daemon session ids,
   and timestamps. No text, no phone numbers, no credentials.
9. Admitted content may be delivered only to `http://127.0.0.1:4780`.
10. Admission and reply claims hold a cross-process file lock and an in-process
    lock; duplicate watchers and concurrent member turns cannot double-submit or
    double-send.

## Known boundaries and next gates

- **Role enforcement is only as strong as the daemon's tool narrowing.** The
  daemon keeps the full toolset when a named agent's allowlist is empty or
  matches no available tool. The bridge rejects an empty allowlist but cannot
  prove an allowlist matches real tools. Before any role is given a
  write-capable or secret-bearing agent, the daemon must fail closed for a named
  agent whose allowlist matches nothing, and per-member authority must be
  enforced by the daemon rather than implied by a prompt header.
- **No per-member identity at the daemon yet.** The daemon sees a named agent
  and a session, not an authenticated member. The control surface the operator
  asked for (set what each person may do by org level and role) needs a
  daemon-owned roster, role, and grant model with an authenticated,
  package-attributed inbound seam; the config file here is its stand-in.
- **The strict role agent needs a supporting daemon.** A daemon without
  `require_named_agent` (added in KINGMAKER-SYSTEMS/ocean#201) ignores the field
  and still falls back to the surface profile, with every tool, when a named
  agent fails to resolve at turn time or its allowlist matches nothing. The
  bridge cannot tell which daemon it is talking to. Do not enroll anyone in a
  restricted role until the running daemon includes that change.
- **Reader and sender share one process.** The `osascript` child runs inside the
  app that holds Full Disk Access. Splitting the sender into a helper with
  Automation only is the stronger shape and is not done here.
- **`attributedBody` extraction is validated, not specified.** The parser
  matched `text` exactly on all 126 recent rows on the operator's host that
  carry both columns and decoded all 1,316 text-NULL inbound rows. Apple does
  not document the layout; an OS update that changes it stops admission (fail
  closed) until the parser is updated.
- **The cursor can pass a row that completes after it was inserted.** A row
  younger than ten seconds stays ahead of the cursor, but an admitted row moves
  the cursor to itself, and the ten seconds are measured from the send date.
  If Messages ever writes a message row before its chat joins, a late-completed
  row below an admitted one, or one delivered late, is skipped. Whether Messages
  does that is unverified.
- **Restart drops in-flight messages.** There is no drain on termination: a
  message mid-turn when the process stops is claimed and gets no reply, and its
  daemon turn is not cancelled. Restart only to upgrade.
- **Member ids must not be reused.** Sessions are keyed by member id; a
  different person given an old id continues that conversation.
- **Dedicated account recommended.** Run the bridge under a macOS user whose
  Messages account is the agent's own Apple ID, so Full Disk Access exposes only
  conversations with the agent.
- **Privileged-service contract unchanged.** `ocean-extension.toml` still
  declares metadata only; autonomous extension integration still requires the
  separately ratified privileged-service protocol described in the 2026-07-25
  manifest.
- Not in this slice: attachments, group chats, tapbacks, typing indicators,
  proactive outbound, voice calls, and per-member connected accounts.

## Validation

```bash
swift test --package-path integrations/ocean-imessage
swift build -c release --package-path integrations/ocean-imessage
sh -n integrations/ocean-imessage/scripts/install-ocean-imessage
cargo xtask docs-check
```

Rehearsal evidence (2026-11-13, live daemon, fixture database, fictional
handles, replies printed). First run: a stranger's message was never admitted; a
member's second message was answered from their first within one session; a
second member could not see it; `/new` cleared it; a fourth message inside a
three-per-hour role was refused before any turn ran; a slow turn was preceded by
the acknowledgement bubble; the state file contained no text or handle. Second
run, after independent review, on a fixture shaped like the real database (text
only in `attributedBody`, `chat.style`, nanosecond dates): attributedBody-only
messages were admitted; a months-old message behind a cursor of zero was skipped
and the cursor caught up; a member removed from the roster while the bridge ran
was no longer admitted and the log showed the roster reload. Third run: a turn
that outlasted a 30 second limit was acknowledged at 5 seconds and answered with
the failure notice at about 32 seconds. Cancelling a running turn by its turn id
directly against the daemon returned `ok` with state `cancelling`, and the turn
finished within seconds; the daemon's session view labels a cancelled turn
`completed`, so the bridge cannot distinguish the two from that view.

Independent review (2026-11-13) found one blocker, since fixed and covered by a
test that runs the real `osascript`: reply text beginning with `-e` was parsed
as an `osascript` option and compiled as script. It also prompted the age bound,
live roster reload, turn cancellation, the private URL session, LF-only stream
splitting, and the installer's loaded-agent guard. A second pass over those
fixes found no blocker and confirmed the first; its findings led to counting
only declared tool names, batch-safe cursor movement inside one read
transaction, counting a message's conversations from the join table itself,
dropping rather than failing on an oversized stream line, and timestamped logs.
The code added after that second pass has not been independently reviewed.
