# Ocean iMessage Bridge — Team Roster Implementation Manifest

**Status:** implementation slice 2 landed in source and rehearsed against a live daemon with a fixture Messages database; live Messages read and send are NOT yet verified (they need operator-granted Full Disk Access and Automation on the host). Operator-managed only.
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
  enrolled member, the destination is the agent's identity, and the conversation
  has exactly that one participant. SQL pre-filters; Swift repeats the check.
  Group chats, strangers, SMS, attachments, reactions, and other identities
  signed in on the host stay rejected and never leave the reader.
- **Member identity, not phone numbers.** An admitted message carries the roster
  member id. Phone numbers never enter the prompt, the state file, or logs.
- **Role → agent.** Each role names a daemon folder-as-agent. The turn runs as
  that agent, so the agent's instructions and tool allowlist are what the role
  may do. The bridge refuses a turn when the agent is missing or declares no
  tool allowlist.
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
  messages stay in order.
- **Contained failure.** A failed turn answers that member with a fixed notice
  and never stops the bridge. An unreadable Messages database idles and retries
  each minute with one log line per state change instead of exiting.
- **Stable signing.** `scripts/install-ocean-imessage` signs the app bundle with
  an identity-based signature and refuses ad-hoc signing, so permission grants
  survive rebuilds.
- **Rehearsal mode.** `run --print-replies` runs the full pipeline and prints
  replies instead of handing them to Messages.

## Invariants

1. Rejected Messages records never reach Ocean, stdout, bridge state, or logs.
2. The reader is the only component that may receive Full Disk Access. Ocean,
   generic shells/interpreters, and the sender do not receive it.
3. Only the operator-owned mode-0600 config can change who is enrolled, which
   role they hold, or which agent a role runs. Message content cannot.
4. A reply is deliverable only to the member whose message was admitted. The
   sender exposes no handle parameter, passes the handle and text as positional
   AppleScript arguments, never uses Accessibility or GUI scripting, and never
   chooses a Messages account implicitly.
5. A member removed from the roster is no longer admitted and can no longer be
   sent to, including for messages admitted before removal.
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
- **Bodies stored outside `message.text` are not admitted.** The reader selects
  only the plain `text` column. Messages macOS stores solely in `attributedBody`
  are skipped. How often that affects inbound text on the host OS is unmeasured.
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
handles, replies printed): a stranger's message was never admitted; a member's
second message was answered from their first within one session; a second
member could not see it; `/new` cleared it; a fourth message inside a
three-per-hour role was refused before any turn ran; a slow turn was preceded by
the acknowledgement bubble; the state file contained no text or handle.
