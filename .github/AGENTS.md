# Diagnostic branch only

This branch temporarily replaces release.yml with a manual, read-only Windows
manifest probe. It uses the pinned Tauri CLI and stops before native compilation
because no web bundle exists. It prints only public manifest differences and
Git newline configuration. No package, artifact, credential, install or publish
step runs. Do not merge this diagnostic workflow into a product branch.

Verification: actionlint .github/workflows/release.yml and the dispatched run.
