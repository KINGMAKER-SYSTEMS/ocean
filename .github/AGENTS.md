# Ocean builds

## Purpose

Root GitHub Actions builds for the public Ocean monorepo.

## Ownership

- `workflows/ci.yml` owns root PR, main-push and manual builds.
- `workflows/release.yml` owns manually dispatched release-candidate validation
  and stable-tag validation/publishing. Routine pull requests use the two build
  checks; manual validation never publishes. Package paths remain rooted under
  `platform/ocean-os/`, and release secrets stay confined to the tag-publish job.
  Registry authentication, publication and latest-tag reconciliation target
  `@kingmaker-systems/ocean`, linked to this canonical repository.
  Manual `windows_probe` builds daemon/TUI executables and runs Observatory
  credential, operator-key, guarded skill-file and TUI daemon-discovery tests on Windows. The probe
  boots the actual daemon in a disposable profile without provider credentials,
  has read-only permissions and never publishes. This probe is separate from routine CI and Mac package validation.
  The publish job has no source checkout: its first shell starts at the existing
  workspace root, verifies the artifact digest, then creates the component
  staging directory used by subsequent steps.
  Registry convergence reads the complete version list through the verified
  version, so a package without a `latest` tag can finish its first publication.
- `build-scope.py` selects changed components; `test-build-scope.py` checks scope decisions, including NUL-terminated Git output. Unknown or unavailable diffs build both.
- `test-daemon-startup.py` checks actual candidate startup, revision, isolated
  member identity and state-file creation; Unix also checks private file modes.
  It does not establish Windows ACLs, provider inference or installation.
- Root CI builds Ocean OS and the active Surface deliverables. Publishing and
  deployment remain owned by their component workflows and installers.

## Local Contracts

- Keep root CI simple: one inexpensive scope job and two conditional build jobs, without separate test, lint, dependency
  audit, MSRV, feature matrix or aggregate jobs.
- Build with the repository's Rust pin and locked dependencies from explicit
  component working directories; genuine compilation failures must remain visible.
- Routine CI uses development profiles and builds Ocean daemon, TUI, CLI, ACP and MCP with their transitive dependencies; release packaging retains its own release profiles.
- Documentation-only changes skip both build jobs while preserving their required status names. Scope script/workflow or shared root build-input changes build both; embedded source Markdown counts as code.
- Surface builds the real Trunk web bundle, proxy, Tauri executable and VSCode
  extension. The native shell consumes that generated bundle.
- Use read-only workflow permissions, bounded jobs and same-ref cancellation.
- Keep the abandoned `ocean-gui` client outside build commands.

## Work Guidance

- Native status checks are `Build Ocean` and `Build Surface`.
- Record history in the canonical monorepo `events.md`, not in this folder.

## Verification

- `actionlint .github/workflows/ci.yml`
- `actionlint .github/workflows/release.yml`
- `PYTHONDONTWRITEBYTECODE=1 python3 .github/test-build-scope.py`
- `PYTHONDONTWRITEBYTECODE=1 python3 .github/test-release-workflow.py`
- The release contract fixtures use local Node/npm and a loopback-only npm
  registry to exercise first publication without credentials or an existing tag.
- Actual hosted builds establish build success. Syntax checking alone does not.

## Child devlog Index

- No child devlogs; root workflow configuration is owned here.
