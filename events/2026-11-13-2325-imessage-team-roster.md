time: [23:25] [11-13-26] [America/New_York]
agent: [claude] [fable 5.2]
worktree: [claude/imessage-team-roster-20261113]
type: [feature-request]
area: [backend] [testing] [docs]

Operator ruling 2026-11-13: the Ocean agent gets its own number that the whole
team texts, each teammate recognized individually, with what they may do
layered by role. This slice turns the July fixed-pair iMessage adapter
(platform/ocean-os/integrations/ocean-imessage) into a team-roster adapter.

The installed July bridge on the operator's Mac had been exiting with
"Messages database is unavailable" since 2026-07-27. Evidence points at the
ad-hoc signed rebuild that day invalidating its Full Disk Access grant; that
cause is inferred from timestamps, not confirmed.

Changed: admission is now enrolled-member-to-agent-identity, one-to-one, from an
operator-owned mode-0600 roster; no phone number is compiled in, and the literal
handles were removed from the package and from the July manifest (they remain in
git history and in other components). Each member continues their own daemon
session bound to their role's named folder-as-agent; the bridge refuses an agent
that is missing or declares no tool allowlist. Replies are owner-bound through
the ledger, limited per member, split into text bubbles, and carry only the
agent's answer after its last tool call. Members are served concurrently, a slow
turn is acknowledged first, per-message failures are contained, and an
unreadable Messages database now idles and retries instead of crash-looping
under launchd. Added an operator-run installer that refuses ad-hoc signing, a
`check` preflight, and a `run --print-replies` rehearsal mode. New design record
docs/specs/2026-11-13-ocean-imessage-team-roster-implementation-manifest.md;
package, integrations, and docs indexes refreshed.

Validation: swift test passed 31 tests; swift build -c release passed; sh -n on
the installer passed; cargo xtask docs-check passed (30 packages, 165 active
Markdown files, 194 local links); git diff --check clean. A
rehearsal against the live local daemon with a fixture Messages database and
fictional handles showed: a stranger never admitted, a member's second message
answered from their first, a second member unable to see it, /new clearing it, a
fourth message in a three-per-hour role refused before any turn, a slow turn
acknowledged first, and no text or handle in the state file.

Not verified: reading the real Messages database and sending a real reply. Both
need operator-granted Full Disk Access and Automation on the host, and sending
texts a real person. Nothing was installed, loaded, or granted; the operator's
installed July app and LaunchAgent are untouched.

Open gates recorded in the manifest: the daemon keeps the full toolset when a
named agent's allowlist matches nothing, so role enforcement must fail closed in
the daemon before any write-capable role; per-member authority and the
operator's role control surface need a daemon-owned roster and grant model;
bodies stored only in attributedBody are not admitted.
