# ocean-imessage — Local macOS iMessage Adapter

## Purpose

This package is the local-only macOS adapter that lets an operator-enrolled team roster text one Ocean agent identity. It is a client adapter, not a daemon plugin or alternate agent runtime.

## Ownership

- **Scope:** `integrations/ocean-imessage/`
- **Parent:** `../AGENTS.md`
- **Runtime authority:** Ocean daemon, `ocean-agent`, and `ocean-runtime` retain session, model, tool, permission, and persistence authority.
- **Design record:** `../../docs/specs/2026-11-13-ocean-imessage-team-roster-implementation-manifest.md` (supersedes the fixed pair of the 2026-07-25 manifest; that manifest's privilege and extension-boundary rulings still hold).

## Local Contracts

- No phone number is compiled into this package. The agent identity, the enrolled members, and their roles come only from the operator's mode-0600 config. Tests and docs use fictional `+1555010xxxx` handles.
- Admit only one-to-one iMessage text from an enrolled member to the agent identity, sent within the last 15 minutes. Filter in the native reader before any content reaches Ocean. Never emit, log, persist, or forward rejected Messages rows. Never act on a backlog.
- Message text comes from `message.text` or, when that is NULL, from `attributedBody` through the narrow extractor in `AttributedBody.swift`. Never unarchive that blob and never widen the extractor to guess at unknown layouts.
- The reader is the sole component that may receive Full Disk Access; the daemon, shell wrappers, and agent process must not receive it.
- A reply goes only to the member whose message was admitted. The sender takes a roster member id resolved from the reply ledger, never a handle; it must reject unknown or removed members and never use Accessibility/UI scripting or interpolate a handle or text into script source.
- Every `osascript` invocation goes through `osascriptArguments`, which ends option parsing with `--` before any caller-supplied argument. Reply text is model output; without the separator a bubble beginning with `-e` runs as AppleScript.
- A role's named agent is its permission boundary. Never run a member turn on the surface profile or on an agent that is missing or declares no tool names in `agent.toml`, and always send `require_named_agent: true`.
- A message waiting for its turn is never evicted from the ledger. Every claim attempt closes its message. `run` takes `RunnerLock` custody of its ledger before the startup sweep that closes what a previous process left waiting; never sweep without it.
- A ledger that has never scanned (cursor zero) is primed to the newest row before the first scan. Housekeeping must not create the ledger file.
- Never return a reply assembled across a lost stream frame. An oversized frame may be skipped only when `lostFrameIsIrrelevant` says it cannot shape the reply.
- Each member's session is their own and is bound to the agent that owns it. Never share or reuse a session across members or across agents.
- The roster file is re-read whenever it changes; a missing, non-private, or invalid file means no roster and nothing is admitted or sent. Do not cache roster decisions across polls or deliver a reply without re-checking enrollment.
- When the bridge stops waiting for a turn, it cancels that turn in the daemon.
- Read Messages `chat.db` read-only. Its schema is private and unsupported; an unknown/ambiguous schema or identity is a fail-closed condition.
- Do not claim App Store compatibility. This is a signed, local direct-distribution adapter and requires explicit FDA/Automation consent. Install only with an identity-based signature; ad-hoc signing voids permission grants on every rebuild.
- `run` may deliver content only to `http://127.0.0.1:4780` through documented daemon HTTP/SSE APIs. The extension manifest intentionally declares no executable resource: current supervised-service contracts cannot express this FDA/Automation boundary, so autonomous extension integration requires a separately ratified privileged-service protocol. Do not reuse `ocean-hooks` or Ocean Buddy.

## Work Guidance

- Keep message payloads and phone numbers out of diagnostics and state. State may retain only opaque message ids, roster member ids, daemon session ids, cursors, and timestamps. Logs may name a member id or role, never a handle or text.
- Preserve the one-to-one iMessage-only, no-attachment/no-reaction, exact-E.164 checks and the per-member reply limits and bubble bounds.
- Per-message failures stay contained to that member; only invalid state or configuration may stop the process.
- Stop-and-consult changes: admitting group chats, attachments, or non-roster senders; any direct database write; a caller-supplied recipient; a GUI scripting fallback; delivering content to anything but the local daemon; or a new source of credentials.

## Verification

```bash
swift test --package-path integrations/ocean-imessage
swift build --package-path integrations/ocean-imessage
sh -n integrations/ocean-imessage/scripts/install-ocean-imessage
```

`run --print-replies --state <path>` rehearses the full pipeline against a live daemon without handing anything to Messages; pair it with `--database` pointing at a fixture. The sender test runs the real `/usr/bin/osascript` with an echo script.

## Child devlog Index

- `Sources/OceanIMessage/Bridge.swift` — roster, admission, state ledger, Messages reader, roster-bound sender.
- `Sources/OceanIMessage/AttributedBody.swift` — narrow text extractor for the `attributedBody` column.
- `Sources/OceanIMessage/Config.swift` — operator config parsing, daemon URL policy, live roster reload.
- `Sources/OceanIMessage/Daemon.swift` — turn prompt, stream line and SSE readers, final-answer assembly, daemon client and turn cancellation.
- `Sources/OceanIMessage/Reply.swift` — reply sanitizing and bubble splitting.
- `Sources/OceanIMessage/OceanIMessageMain.swift` — CLI and the concurrent run loop.
- `Tests/` — roster, admission, ledger, reader, sender, reply, protocol, and config safety tests; `DaemonClientTests.swift` drives the daemon client against a stand-in daemon.
- `scripts/` — operator-run signed installer and build wrapper.
