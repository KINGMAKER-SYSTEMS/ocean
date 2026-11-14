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

Follow-up, same branch, after independent review (01:10 on 11-14-26): the review
found one blocker and the operator's real Messages database exposed a second.
Blocker: a reply bubble beginning with `-e` was parsed by osascript as an option
and compiled as script; every invocation now ends option parsing with `--`, and
a test feeds hostile strings through the real interpreter. Second: on this host
1,316 of the last 1,361 plain inbound iMessages, and all 8 from the enrolled
owner handle, store their text only in `attributedBody`, so the reader (July's
too) would have admitted nothing; it now extracts that text with a narrow
parser that never unarchives the blob, validated against 126 real rows carrying
both columns (126 exact matches). Also fixed from the review: a 15 minute
message-age bound and a settling cursor so a stale or missing cursor never
replays a backlog; live roster reload with enrollment re-checked at delivery;
cancelling the daemon turn when the bridge stops waiting; a private URL session
(64 concurrent streams, no proxy, no redirects); LF-only stream line splitting;
one-to-one enforced through chat.style and a single chat; per-member ledger
retention; definite-answer-only session expiry; corrupt state failing closed;
rehearsals requiring their own state file; and an installer that refuses to
replace the app under a loaded LaunchAgent without --load. Validation: swift
test 41 passed; release build passed; cargo xtask docs-check passed; two further
live-daemon rehearsals recorded in the manifest. Still not verified: the
installed app reading the real Messages database, and a real send.
