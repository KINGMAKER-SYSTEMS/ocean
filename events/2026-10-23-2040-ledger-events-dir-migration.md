time: [20:40] [23-10-26]
agent: [claude code] [claude-fable-5-1]
worktree: [chore/ledger-events-dir-migration] [~/dev/oceanwork/wt-ledger]
type: [refactor]
area: [docs]

Brought the ledger back under the contract #147 set this afternoon: root AGENTS.md now records work as one `events/YYYY-MM-DD-HHMM-<branch-slug>.md` file per entry and freezes legacy `events.md`. Twenty-eight entries were appended to the frozen file after that merge (the triage-sweep ports and fixes #150–#176, the `kmf/fq-k1`, `fq-k2`, `fq-k7` and `kmf/reconcile-129` entries, and five 2026-10-22 port entries that #147's branch predated and that later "keep both sides" conflict resolutions re-added). Each of those 28 entries now lives verbatim in its own `events/` file, named from its own time line and worktree branch, and `events.md` is byte-identical to its content at ec64d223 (207 entries). Verified by splitting both versions on their `time:` headers: every frozen entry is still present, every moved entry appears in exactly one new file, nothing was edited. Hosted CI does not check the ledger; agents working after #147 followed the per-session instruction to append, so the instruction, not the contract, was stale.
