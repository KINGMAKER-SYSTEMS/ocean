# Diagnostic branch only

This branch temporarily replaces release.yml with a manual, read-only native
Mac comparison. Application source must exactly match Accounts candidate
8d39b059; only this contract and the workflow may differ. Build the actual web
bundle, run the unchanged native suite, then repeat the existing watcher test.
No packaging, credentials, live service, signing or publication. Do not merge
this diagnostic workflow into a product branch.

Verification: actionlint .github/workflows/release.yml and the dispatched run.
