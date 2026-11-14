# Ocean iMessage — team roster local adapter

This macOS-only adapter lets an operator-enrolled team text one Ocean agent
identity. Before Ocean receives a single byte of message content, a message
must be:

- a one-to-one iMessage conversation (one that Messages itself marks one-to-one),
- **from** a handle on the operator's roster,
- **to** the agent's own identity,
- sent within the last 15 minutes.

It rejects outgoing rows, SMS, group conversations, strangers, attachments,
reactions, other identities signed in on the host, non-E.164 identifiers,
overlong text, anything older than 15 minutes, and unknown Messages schemas.
The bridge answers what was just sent; after an outage or a stale cursor it
skips the backlog instead of acting on it. Rejected rows are never printed,
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
A turn that takes a while is preceded by a short "working on it" bubble. A turn
that outlasts `turnTimeoutSeconds` gets a failure notice and is cancelled in the
daemon, so it does not keep working behind the notice.

Current macOS stores most message text only in the `attributedBody` column. The
reader takes `text` when present and otherwise extracts the string from
`attributedBody` with a narrow parser. It never unarchives the blob, and a blob
that is not exactly the expected layout is not admitted.

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
  an agent that is missing or declares no tool names under `tools` in its
  `agent.toml`; files in the agent's `tools/` folder do not count. Every turn
  also sends `require_named_agent: true`, which makes a supporting daemon refuse
  the turn when the agent is missing, invalid, or matches no tool. A daemon that
  predates that field ignores it and keeps every tool in those cases, and the
  bridge cannot tell the difference, so do not enroll anyone in a restricted
  role until the running daemon supports it.
- `repliesPerHour` defaults to 30 and is capped at 120. Optional
  `turnTimeoutSeconds` (30–1800, default 600) and `acknowledgeAfterSeconds`
  (5–300, default 25, `0` disables) tune long turns.
- Up to 64 members. Ids, roles, and agent names are lowercase
  `[a-z0-9][a-z0-9_-]*`.

The running bridge re-reads the roster whenever the file changes; no restart
is needed. Removing a member stops admitting them on the next poll (about two
seconds) and drops any reply still on its way to them. A roster file that is
missing, not mode 0600, or invalid means no roster: the bridge admits nothing
until it is fixed.

Never reuse a member id for a different person: sessions are keyed by member id,
so the new person would continue the previous person's conversation.

## Security boundary

`chat.db` requires Full Disk Access. Grant FDA **only to the signed app** this
package builds; never to `ocean-daemon`, a shell, Python, Node, or a general
agent runtime. Prefer running it under a macOS user whose Messages account is
the agent's own Apple ID, so FDA exposes only conversations with the agent.

That one app is the reader, the daemon client, and the sender. To send, it runs
`/usr/bin/osascript` with a constant script. The reply text and the recipient
handle are passed as positional arguments after `--`, so neither can be read as
an `osascript` option or become script source. The recipient comes from the
reply ledger and the roster. The sender selects only the explicit,
operator-enrolled Messages account id; it never falls back to the first
account, uses Accessibility, or uses GUI scripting. Sending needs Automation
access to Messages, granted to the same app.

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
`run --print-replies --state <path>` runs the whole pipeline against the daemon
and prints the replies instead of handing them to Messages, so a roster and its
agents can be rehearsed without texting anyone. It requires its own `--state`
file so a rehearsal can never write into the live ledger; pair it with
`--database` pointing at a fixture.

## Install

Installation is operator-run. Sign with a stable identity: macOS ties FDA and
Automation grants to the app's designated requirement, and an ad-hoc signature
changes on every rebuild, silently voiding the grants.

```bash
security find-identity -v -p codesigning
integrations/ocean-imessage/scripts/install-ocean-imessage --sign-identity "<identity>" --load
```

The script builds, signs, and installs `~/Applications/Ocean iMessage.app`. If
the LaunchAgent is already loaded it refuses to replace the app unless `--load`
is passed; with `--load` it stops the agent, replaces the app, and starts it
again. It never grants a permission and never writes the roster. A bridge with
no roster or no Full Disk Access idles and says so once in its log instead of
crash-looping, and it starts from the present, never from old messages. Without
Messages access it exits once every five minutes so a fresh process can pick up
a newly granted permission. `--load` moves the previous logs aside.

Afterwards, by hand: enable the app under Full Disk Access, write the roster,
and approve the Messages Automation prompt on the first reply. The service log
(`~/Library/Application Support/Ocean/iMessage/service.err.log`) is the
authority on whether the installed app can read Messages: `check` run from a
terminal tests the terminal's permissions, not the app's. Restarting the bridge
drops messages that are mid-turn, so restart only to upgrade. Enroll the Messages account id from
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
