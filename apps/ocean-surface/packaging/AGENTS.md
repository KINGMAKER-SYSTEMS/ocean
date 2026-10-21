# Desktop distribution

## Purpose

Build native Ocean desktop candidates with their matching daemon, MCP bridge and legal payload.

## Ownership

- `build_desktop.py` owns candidate builds, tool pins, license inventories,
  bundle verification and revision-addressed output.
- `sign_macos.py` owns explicit local Developer ID preparation from a verified
  unsigned candidate; `SIGNING.md` owns its operator handoff and remaining
  notarization/installation gates. It never changes Keychain permissions.
- The monorepo release workflow owns opt-in hosted builds and artifact upload.
- `windows-hooks.nsh` replaces the pinned NSIS template's process-kill macro
  with refusal. Install/uninstall must not force-terminate Ocean or touch files
  while its app or installed sidecars are in use. Explicit app Quit owns shutdown.
- Existing proxy/daemon service installers retain live-service ownership.

## Local Contracts

- Build only clean committed canonical source, with locked Cargo graphs and
  repository Rust pins. App, UI, daemon and MCP bridge must come from the same revision.
- Preserve the native manifest's tracked LF checkout attribute; Tauri must not
  change its bytes during a Windows build. Never reset source to pass the gate.
- Native targets are Apple Silicon Mac, Intel Mac and Windows x64. Refuse a
  different host architecture rather than label a cross-build as acceptance.
- Bundle the daemon and MCP bridge as sibling executables through Tauri `externalBin`;
  no caller-controlled executable path or private configuration enters a bundle.
- `ASSET-NOTICES.txt` and the font-adjacent OFL file retain bundled asset notices.
- Include full dependency licenses for the native shell, WASM UI, daemon and MCP bridge,
  plus applicable upstream NOTICE files and project legal files.
- Inventory the native shell with Tauri's release `custom-protocol` feature.
  Mac app and both sidecar imports must resolve only to macOS system libraries;
  Homebrew, local-development and unresolved rpath dependencies fail packaging.
- Build both Windows sidecars with static CRT linkage as well as the Tauri app;
  inspect all three extracted executables and refuse Visual C++ redistributable DLL
  imports. This does not establish clean-machine WebView or installer acceptance.
- Candidate builds are unsigned and do not publish, install, start live services,
  change credentials, or establish update/recovery or coworker acceptance.
  A separate opt-in workflow check runs the actual Windows installer only on a
  disposable hosted runner, with no existing Ocean installation or profile.
- Signing uses a private verified snapshot, fixed app/sidecar identities, the
  expected Apple team, hardened runtime and secure timestamps. Original inputs
  and existing outputs are immutable; only a fully verified signed copy is
  published locally. Its receipt must explicitly say notarization is pending.
- Generated tools, staging and outputs stay under ignored component `target/`.
  Completed candidate directories are never overwritten.

## Work Guidance

- Keep packaging separate from provider login and first-run UI.
- Signing, update/recovery and two-machine acceptance remain release gates.

## Verification

- `python3 -m unittest discover -s packaging -p 'test_*.py'`
- `python3 packaging/build_desktop.py --target <native Rust target>`
- Hosted native builds must inspect actual bundle contents and boot the bundled
  daemon in disposable state, then verify the bundled MCP bridge's stdio handshake,
  daemon revision and room projection. Record both sidecar checksums in the receipt.
  This does not configure an external MCP host or establish provider inference.
- The Windows candidate workflow then checks actual silent install, refusal
  without file changes or process termination, idle reinstall/uninstall, and
  preservation of fixture markers in the real default profile directories.
  It does not establish provider sign-in, user-driven GUI installation, older
  uninstaller behavior, atomic rollback or clean-machine acceptance.

## Child devlog Index

- No child boundaries.
