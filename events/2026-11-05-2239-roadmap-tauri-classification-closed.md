time:  [22:39] [05-11-26]
agent: [zcode cli] [glm-5.3]
worktree: [kmf/fq-k23] [~/dev/kingmaker-ocean-worktrees/kmf-fq-k23-r2238]
type:  [docs]
area:  [writing]

Removed the resolved "Decide the Tauri effective harness classification" item
from `platform/ocean-os/ROADMAP.md` (factory FQ-K23, follow-on to #179). The
item self-instructed removal: it proposed mapping `surface-tauri` to the Web
harness profile "pending the operator's yes on KINGMAKER-SYSTEMS/ocean#179 ...
Merging it is the decision; remove this item then." #179 squash-merged as
e6dc81c6 on 2026-11-05 under the operator's "merge and proceed", so the
decision is taken. The implemented behavior is already documented in
`platform/ocean-os/docs/ARCHITECTURE.md` (updated in that merge) and the
ROADMAP header keeps the file to open work only. The immutable
`events/2026-10-24-0042-port-tauri-web-harness.md` record is untouched.

Validation: `cargo xtask docs-check` passed from `platform/ocean-os/`.
