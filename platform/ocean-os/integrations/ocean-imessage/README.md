# Ocean iMessage — team roster local adapter

This macOS-only adapter lets an operator-enrolled team text one Ocean agent
identity. Before Ocean receives a single byte of message content, a message
must be:

- a one-to-one iMessage conversation,
- **from** a handle on the operator's roster,
- **to** the agent's own identity.

It rejects outgoing rows, SMS, group conversations, strangers, attachments,
reactions, other identities signed in on the host, non-E.164 identifiers,
overlong text, and unknown Messages schemas. Rejected rows are never printed,
written to state, or sent to Ocean. The state file holds opaque message ids,
roster member ids, daemon session ids, a cursor, and reply timestamps — never
message text or phone numbers.

> The Messages database is a private macOS implementation detail, not a public
> API. This is signed/direct-distribution-only software, not an App Store
> integration. A schema or permission anomaly fails closed.

## How a text becomes a turn

1. The reader admits the message and resolves the sender to a roster member.
2. The member's role selects a daemon folder-as-agent. The turn runs as that
   agent in the member's own ongoing session.
3. The agent's final answer is split into text bubbles and sent back to that
   member only.

Each member has a separate session, so one teammate's conversation is not
visible to another. Texting exactly `/new` starts that member's session over.
A turn that takes a while is preceded by a short "working on it" bubble.

## Roster

`config.json` must be mode 0600:

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

- `localIdentity` is the agent's own number. `members[].phone` are the
  teammates allowed to text it. All are strict E.164.
- `roles[].agent` names a folder under the daemon's agents root. **That agent's
  tool allowlist is what the role may do.** The bridge refuses to run a turn for
  an agent that is missing or declares no `tools` allowlist. It cannot prove an
  allowlist matches real tool names, so check `GET /v1/agents/<name>` yourself
  before giving a role a powerful agent.
- `repliesPerHour` defaults to 30 and is capped at 120. Optional
  `turnTimeoutSeconds` (30–1800, default 600) and `acknowledgeAfterSeconds`
  (5–300, default 25, `0` disables) tune long turns.
- Up to 64 members. Ids, roles, and agent names are lowercase
  `[a-z0-9][a-z0-9_-]*`.

Removing a member stops admitting them and makes them unaddressable at once.

## Security boundary

`chat.db` requires Full Disk Access. Grant FDA **only to the signed app** this
package builds; never to `ocean-daemon`, a shell, Python, Node, or a general
agent runtime. Prefer running it under a macOS user whose Messages account is
the agent's own Apple ID, so FDA exposes only conversations with the agent.

The sender separately needs Automation access to Messages. It addresses only
roster members: the recipient comes from the reply ledger and the roster, and
travels to AppleScript as a positional argument. It selects only the explicit,
operator-enrolled Messages account id; it never falls back to the first
account, uses Accessibility, or uses GUI scripting.

Message text is untrusted agent input. It cannot select a recipient, alter the
roster, change bridge configuration, or gain Messages database access. It is
delivered only to `http://127.0.0.1:4780`.

## Build, test, rehearse

```bash
swift test --package-path integrations/ocean-imessage
swift build -c release --package-path integrations/ocean-imessage
```

`check` is the operator preflight. It prints only counts, role and agent names,
and pass/fail:

```bash
integrations/ocean-imessage/.build/release/ocean-imessage check
```

`poll` prints admitted messages as JSON lines and contacts nothing.
`run --print-replies` runs the whole pipeline against the daemon and prints the
replies instead of handing them to Messages, so a roster and its agents can be
rehearsed without texting anyone.

## Install

Installation is operator-run. Sign with a stable identity: macOS ties FDA and
Automation grants to the app's designated requirement, and an ad-hoc signature
changes on every rebuild, silently voiding the grants.

```bash
security find-identity -v -p codesigning
integrations/ocean-imessage/scripts/install-ocean-imessage --sign-identity "<identity>" --load
```

The script builds, signs, installs `~/Applications/Ocean iMessage.app`, and with
`--load` writes and loads the LaunchAgent. It never grants a permission and
never writes the roster. Afterwards, by hand: enable the app under Full Disk
Access, write the roster, run `check`, and approve the Messages Automation
prompt on the first reply. Enroll the Messages account id from
`ocean-imessage accounts` only after confirming in Messages that it is the
agent's identity; Apple provides no public account-id-to-number proof.

## Extension status

`ocean-extension.toml` contains package metadata only and declares no service.
Autonomous extension integration requires a separately ratified privileged
service protocol with authenticated package attribution, bounded lifetime,
minimal environment, scoped state, FDA/Automation ownership, and explicit
startup/restart/shutdown behavior. The current Stage-A stdio service contract
and capability policy do not provide that boundary; do not substitute
`ocean-hooks`, Ocean Buddy, or a raw daemon route.

Design record: `docs/specs/2026-11-13-ocean-imessage-team-roster-implementation-manifest.md`.
