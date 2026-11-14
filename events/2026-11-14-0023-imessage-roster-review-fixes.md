time: [00:23] [11-14-26] [America/New_York]
agent: [claude] [fable 5.2]
worktree: [claude/imessage-team-roster-20261113]
type: [review] [bug-report]
area: [backend] [testing] [docs]

Review follow-up for PR #199 (team-roster iMessage adapter). This entry replaces
follow-up text that was wrongly appended to the 2026-11-13-2325 event; that file
is restored to its original bytes. The times quoted inside the removed text were
estimates and were wrong.

Three reviews ran against this branch: my delegated independent reviewer (two
passes), the automated Codex review, and the factory's independent reviewer
(holds at 7a34efca and d8d4918c, issues #200, #202, #203).

Blockers found and fixed. (1) Reply text reached osascript as options: a bubble
beginning with `-e` was compiled as script, which gave any enrolled member code
execution inside the app holding Full Disk Access. Every invocation now ends
option parsing with `--`, and a test feeds hostile strings through the real
interpreter. (2) On the operator's host 1,316 of the last 1,361 plain inbound
iMessages, and all 8 from the enrolled owner handle, store their text only in
`attributedBody`; the reader (July's too) admitted nothing. It now extracts that
text with a narrow parser that never unarchives the blob, checked against 126
real rows carrying both columns (126 exact matches).

Factory and Codex findings, all addressed in source: the roster is re-read when
the file changes and enrollment is re-checked at delivery; a waiting message is
never evicted from the ledger (answered history is trimmed per member, a member
past 512 waiting messages is refused explicitly, every claim attempt closes its
message, and a restart closes orphans); a session is forgotten only on a
definite answer; a full scan batch stops the cursor at its last row and both
reader queries share one read transaction (#202 item 1); every failure after
the daemon accepts a turn cancels it, including a ledger write failure (#202
item 3); and turns now send `require_named_agent: true` (#200/#201). Older
daemons ignore that field, so the role boundary is enforced only where the
daemon includes #201; the operator's running daemon has not been confirmed to.

Also changed: a 15 minute message-age bound so a backlog is never answered;
one-to-one enforced through chat.style and the join table; a private URL session
(no proxy, no redirects, 128 connections); LF-only stream splitting with an
oversized line dropped rather than fatal; only tool names declared in agent.toml
count as an allowlist; timestamped logs; a rehearsal cannot use the live ledger;
the installer refuses to replace the app under a loaded LaunchAgent without
--load; and without Messages access the process exits once every five minutes
so a newly granted permission reaches a fresh process.

Validation: swift test passed 49 tests, including five that drive the daemon
client against a stand-in daemon (strict flag and session resume, timeout then
cancel, failed stream then cancel, ledger write failure then cancel, unusable
agent never submitted) and the two 129-message regressions from #202. Release
build, installer shell syntax, cargo xtask docs-check, and git diff --check
passed. Three further rehearsals against the live local daemon with fixture
databases are recorded in the manifest. The real reader query was also run once
over the last 50,000 rows of the operator's Messages database with a scratch
ledger: schema accepted, nothing admitted, cursor caught up.

Not verified: the installed app reading Messages under its own permissions, a
real send, and the Automation prompt. Nothing has been installed, loaded, or
granted by this work. A role agent folder named ocean-text was added to the
operator's live agents root for the owner role; it is not yet in a repository.
The factory's live-messaging and deployment holds are its own; this entry does
not claim they are lifted.
