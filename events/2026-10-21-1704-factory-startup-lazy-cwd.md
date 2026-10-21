time: [01:04PM EDT] [21-10-26]
agent: [Codex] [factory startup repair builder]
worktree: [codex/factory-desktop-startup-env]
type: [bug report] [refactor]
area: [backend] [testing]

Addressed PR #143 review comment 4339419561: eager inherited current-directory
resolution rejected otherwise valid absolute/PATH daemon startup when that
directory had been removed or could not be resolved. The launch directory now
resolves lazily, at most once, only for a relative multi-component executable
or supported nonempty relative filesystem override. Required relative paths
still fail on lookup error; no home/temp substitute changes their meaning.

Added deterministic injected-failure coverage for absolute/PATH executables,
absolute overrides, blank/Claude sentinels and SQLite memory/absolute URIs;
relative executable and each path override fail closed. Multiple relative
locations share one lookup. A Unix disposable subprocess removes only its own
inherited cwd and proves a real absolute executable still starts from the
absolute neutral directory, without changing the parent runner cwd/environment.
Existing URI admission and startup model precedence are unchanged.

Devlog pass updates Surface's launch-directory contract; root/child indexes
remain unchanged because no ownership boundary moved. New immutable entry only;
legacy events.md and earlier event files are untouched. At commit, rustfmt and
scoped diff whitespace checks passed. Focused daemon_child and startup_model
tests are scheduled on this committed head using the existing real PR143 web
bundle (UI source unchanged); results are reported separately. No branch push,
canonical merge, installation, deployment, or live credential/provider use.
