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
- Admit only one-to-one iMessage text from an enrolled member to the agent identity. Filter in the native reader before any content reaches Ocean. Never emit, log, persist, or forward rejected Messages rows.
- The reader is the sole component that may receive Full Disk Access; the daemon, shell wrappers, and agent process must not receive it.
- A reply goes only to the member whose message was admitted. The sender takes a roster member id resolved from the reply ledger, never a handle; it must reject unknown or removed members and never use Accessibility/UI scripting or interpolate a handle or text into script source.
- A role's named agent is its permission boundary. Never run a member turn on the surface profile or on an agent that is missing or declares no tool allowlist.
- Each member's session is their own and is bound to the agent that owns it. Never share or reuse a session across members or across agents.
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

`run --print-replies` rehearses the full pipeline against a live daemon without handing anything to Messages; pair it with `--database` pointing at a fixture.

## Child devlog Index

- `Sources/OceanIMessage/Bridge.swift` — roster, admission, state ledger, Messages reader, roster-bound sender.
- `Sources/OceanIMessage/Config.swift` — operator config parsing and daemon URL policy.
- `Sources/OceanIMessage/Daemon.swift` — turn prompt, SSE reader, final-answer assembly, daemon client.
- `Sources/OceanIMessage/Reply.swift` — reply sanitizing and bubble splitting.
- `Sources/OceanIMessage/OceanIMessageMain.swift` — CLI and the concurrent run loop.
- `Tests/` — roster, admission, ledger, reader, sender, reply, protocol, and config safety tests.
- `scripts/` — operator-run signed installer and build wrapper.
