# Ocean builds

## Purpose

Root GitHub Actions builds for the public Ocean monorepo.

## Ownership

- `workflows/ci.yml` owns root PR, main-push and manual builds.
- `workflows/release.yml` owns manually dispatched release-candidate validation
  and stable-tag validation/publishing. Routine pull requests use the two build
  checks; manual validation never publishes. Package paths remain rooted under
  `platform/ocean-os/`, and release secrets stay confined to the tag-publish job.
- `build-scope.py` selects changed components; `test-build-scope.py` checks scope decisions, including NUL-terminated Git output. Unknown or unavailable diffs build both.
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
- Actual hosted builds establish build success. Syntax checking alone does not.
- Review gate read-back: `gh api repos/KINGMAKER-SYSTEMS/ocean/branches/main/protection --jq '.required_pull_request_reviews'` must be non-null with `required_approving_review_count` >= 1, `dismiss_stale_reviews` and `require_last_push_approval` true; a stored count with null review protection is not enforced. Confirm with `gh api repos/KINGMAKER-SYSTEMS/ocean/rules/branches/main` that no ruleset adds bypass actors.

## Child devlog Index

- No child devlogs; root workflow configuration is owned here.
