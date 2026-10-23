time: [12:08] [21-10-26]
agent: [Codex] [factory coordinator]
worktree: [codex/factory-dmg-attach-recovery]
type: [bug report] [refactor]
area: [review] [testing]

Independent review of PR123 at19e16c8b found an ambiguous disk-image attach
failure outside cleanup protection; the same defect remained in candidate
PR129 at63d8c027. Issue139 was created and read back before implementation.
This branch reconciles the canonical image and unique owned mountpoint before
bounded cleanup, preserves the original attachment failure, and leaves other
mounts alone. Packaging's owning AGENTS contract was updated; parent structure
and indexes did not change. Legacy events.md and existing event files remain
untouched; the root event-policy correction is already owned by frozen PR138.

Validation before final-head review: Python3.11 packaging suite19/19 passed,
including timeout/nonzero attach, absent or unrelated mounts, cleanup failures,
and normal/body-failure cleanup; git diff --check passed. No real disk image,
provider credentials, expensive build or live service was used. Independent
review and required hosted checks remain pending at this record's creation.
The branch targets the existing desktop candidate; maintainer approval,
integration, signing and product acceptance remain separate gates.
