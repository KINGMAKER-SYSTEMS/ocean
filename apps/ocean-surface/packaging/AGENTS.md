# Desktop distribution

## Purpose

Build native Ocean desktop candidates with their matching daemon and legal payload.

## Ownership

- `build_desktop.py` owns candidate builds, tool pins, license inventories,
  bundle verification and revision-addressed output.
- The monorepo release workflow owns opt-in hosted builds and artifact upload.
- Existing proxy/daemon service installers retain live-service ownership.

## Local Contracts

- Build only clean committed canonical source, with locked Cargo graphs and
  repository Rust pins. App, UI and daemon must come from the same revision.
- Preserve the native manifest's tracked LF checkout attribute; Tauri must not
  change its bytes during a Windows build. Never reset source to pass the gate.
- Native targets are Apple Silicon Mac, Intel Mac and Windows x64. Refuse a
  different host architecture rather than label a cross-build as acceptance.
- Bundle the daemon as a sibling executable through Tauri `externalBin`;
  no caller-controlled executable path or private configuration enters a bundle.
- `ASSET-NOTICES.txt` and the font-adjacent OFL file retain bundled asset notices.
- Include full dependency licenses for the native shell, WASM UI and daemon,
  plus applicable upstream NOTICE files and project legal files.
- Inventory the native shell with Tauri's release `custom-protocol` feature.
  Mac app and daemon imports must resolve only to macOS system libraries;
  Homebrew, local-development and unresolved rpath dependencies fail packaging.
- Build the Windows daemon with static CRT linkage as well as the Tauri app;
  inspect both extracted executables and refuse Visual C++ redistributable DLL
  imports. This does not establish clean-machine WebView or installer acceptance.
- Candidate builds are unsigned and do not publish, install, start live services,
  change credentials, or establish update/recovery or coworker acceptance.
- Generated tools, staging and outputs stay under ignored component `target/`.
  Completed candidate directories are never overwritten.

## Work Guidance

- Keep packaging separate from provider login and first-run UI.
- Signing, update/recovery and two-machine acceptance remain release gates.

## Verification

- `python3 -m unittest discover -s packaging -p 'test_*.py'`
- `python3 packaging/build_desktop.py --target <native Rust target>`
- Hosted native builds must inspect actual bundle contents and boot the bundled
  daemon in disposable state before a candidate is considered packaged.

## Child devlog Index

- No child boundaries.
