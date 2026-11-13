time: [17:45] [11-13-26] [America/New_York]
agent: [Codex desktop] [factory release owner]
worktree: [codex/issue197-codex-pin-limits]
type: [bug report] [refactor]
area: [backend] [testing]

Issue: https://github.com/KINGMAKER-SYSTEMS/ocean/issues/197

PR196 merged externally as 8a081d920e53d45162963495246a4d5bf49a23f2 with
both builds passing. Its automated review finding remained reproducible in
source: explicit openai-codex/codex pins assigned GPT-6.1 Sol/Terra 400K context
while the bare and qualified catalog routes assigned 500K.

The isolated follow-up assigns those two exact model ids the existing 500K
catalog context limit, retaining 128K output and older/unknown explicit Codex
fallbacks. Regression coverage compares bare, qualified and both pinned
provider spellings and checks OpenAI/ChatGPT and older-model controls.
The provider devlog records this invariant. Parent ownership and indexes
remain unchanged.

Validation: cargo test -p ocean-providers --locked passed all76 unit tests
and0 doctests on Rust1.97.0. Tests used disposable absolute Ocean/Codex auth
and config paths, preserved HOME, and made no live provider requests.
rustfmt and git diff --check passed. Validation ran on the available desktop;
the Mac mini remained saturated and no new build was launched there.

Root owns the branch after this event and final commit. Independent final-head
review and hosted builds are subsequent gates. This entry does not claim merge,
deployment or native/client acceptance. Existing approval and release holds
under55/90 remain. No live service, credential, database or permission change.
