#!/usr/bin/env python3
"""
compose_profile.py — base-profile injection, made REAL in ocean-agents.

WHY THIS EXISTS (the base-profile pattern resolution):
The design spec §4 describes composing `_shared` + `_base/<SURFACE>/` + an agent's
`<SURFACE>/` overrides into the surface profile. But the Ocean daemon does NOT do
that composition. Verified in ocean-os (crates/ocean-agent/src/lib.rs):

  • build_system_prompt(cwd, client_type) = BASE_SYSTEM_PROMPT
      + load_project_prompt(cwd)         ← ancestor-walks AGENTS.md/CLAUDE.md ONLY
      + append_client_type(client_type)  ← reads ONE file: assistants/<DIR>/system.md

  • append_client_type → load_surface_profile_from(root, client_type) reads exactly
    `<root>/<surface_dir>/system.md` (a single file). It does NOT read `_shared/`,
    does NOT path-resolve `_base/<SURFACE>/`, does NOT concatenate multiple files.
    The daemon's own surface_dir doc comment confirms the symlink-vs-resolver
    composition is "not implemented runtime-side" — i.e. NOT implemented runtime-side.

So the daemon-composed layering is FLAT, not the rich _base/ composition the spec
wants. To keep the house rules DRY (spec §4: "common rules live once; per-agent
files hold only deltas") WITHOUT waiting on a Rust change, ocean-agents assembles
the base itself — this composer — into the file the daemon actually loads.

WHAT IT DOES:
Assemble, in order:
   1. assistants/_shared/system.md                  (cross-surface house core)
   2. assistants/_base/<SURFACE>/system.md          (per-surface house role)
   3. assistants/_base/<SURFACE>/{comms,canvas,limits,...}.md  (house SOP split)
   4. (optional) assistants/<agent>/<SURFACE>/system.md        (agent overrides)
into a single composed profile. With --write, publish it to the file the daemon
loads (assistants/<SURFACE>/system.md for the house profile, or under the agent).

This is option (b) from spec §4/§13 — PATH-RESOLVE + concatenate, no symlink
sprawl, base stays read-only — done in agents-space because the daemon doesn't do
it. When ocean-os later implements runtime composition, this composer is replaced
by that; until then it makes the DRY pattern real today.

Pure stdlib. Usage:
  compose_profile.py SLACK                          # print composed house profile
  compose_profile.py SLACK --agent <name>    # + agent overrides
  compose_profile.py SLACK --agent <name> --write   # publish the file
  compose_profile.py --check SLACK                  # verify ONE published file is current
  compose_profile.py --check                        # verify EVERY surface (CI drift gate)
"""
import sys
from pathlib import Path

ASSISTANTS_ROOT = Path(__file__).resolve().parents[1]
SHARED = ASSISTANTS_ROOT / "_shared" / "system.md"
BASE = ASSISTANTS_ROOT / "_base"

# House SOP files for a surface, in composition order. system.md first (the role),
# then the focused SOP files. Extra .md files in the surface dir are appended
# alphabetically after these named ones.
#
# The named set covers the section vocabulary used across the authored surfaces:
#   system  — the surface role + the surface's defining rule (always first)
#   comms   — how to talk / answer / format on this surface (style, brevity)
#   canvas  — rich-rendering / spatial / artifact SOPs (Slack canvas, tldraw board)
#   tools   — surface-specific shape of the tool/action SOPs (browser tools, etc.)
#   limits  — format constraints, rate limits, the don't-do-on-this-surface list
#   safety  — the surface-specific shape of the safety invariants
#   vibe    — the closing "what a great teammate on this surface feels like"
# Any other .md in the surface dir is appended alphabetically after these.
BASE_ORDER = [
    "system.md",
    "comms.md",
    "canvas.md",
    "tools.md",
    "limits.md",
    "safety.md",
    "vibe.md",
]

BANNER = (
    "<!-- COMPOSED by assistants/tools/compose_profile.py — do not edit by hand.\n"
    "     Edit the sources: _shared/system.md, _base/{surface}/*.md, "
    "and (if any) {agent}/{surface}/system.md, then re-run the composer. -->\n"
)


def _read(p: Path) -> str:
    try:
        return p.read_text().strip()
    except FileNotFoundError:
        return ""


def _base_files(surface: str) -> list[Path]:
    sdir = BASE / surface
    if not sdir.is_dir():
        return []
    named = [sdir / n for n in BASE_ORDER if (sdir / n).exists()]
    extras = sorted(
        p for p in sdir.glob("*.md")
        if p.name not in BASE_ORDER
    )
    return named + extras


def compose(surface: str, agent: str | None = None) -> str:
    """Assemble the composed profile string for a surface (+ optional agent)."""
    parts: list[str] = [BANNER.format(surface=surface, agent=agent or "<agent>")]
    shared = _read(SHARED)
    if shared:
        parts.append(shared)
    for f in _base_files(surface):
        body = _read(f)
        if body:
            parts.append(body)
    if agent:
        override = ASSISTANTS_ROOT / agent / surface / "system.md"
        body = _read(override)
        if body:
            parts.append(body)
    # Join with blank-line separators; strip stray edges.
    return "\n\n".join(p.strip() for p in parts if p.strip()) + "\n"


def _target(surface: str, agent: str | None) -> Path:
    if agent:
        return ASSISTANTS_ROOT / agent / surface / "_composed.system.md"
    return ASSISTANTS_ROOT / surface / "system.md"


def _all_surfaces() -> list[str]:
    """Every surface that has a `_base/<SURFACE>/` source dir, sorted."""
    if not BASE.is_dir():
        return []
    return sorted(p.name for p in BASE.iterdir() if p.is_dir())


def _check_one(surface: str, agent: str | None) -> bool:
    """Return True if the published file matches its composed sources.

    A non-empty published file is REQUIRED — the daemon loads it. An empty or
    missing target fails the check even if (degenerately) the composed string were
    also empty, so CI can never green-light a surface the daemon would read blank.
    """
    composed = compose(surface, agent)
    tgt = _target(surface, agent)
    current = _read(tgt)
    ok = bool(current.strip()) and current.strip() == composed.strip()
    print(f"{'OK' if ok else 'STALE'}: {tgt}")
    return ok


def main(argv: list[str]) -> int:
    args = list(argv)
    check = "--check" in args
    write = "--write" in args
    args = [a for a in args if a not in ("--check", "--write")]
    agent = None
    if "--agent" in args:
        i = args.index("--agent")
        agent = args[i + 1] if i + 1 < len(args) else None
        del args[i:i + 2]

    # `--check` with no surface = check EVERY authored surface (CI drift gate).
    if check and not args:
        surfaces = _all_surfaces()
        if not surfaces:
            print("no surfaces found under _base/")
            return 1
        all_ok = True
        for s in surfaces:
            if not _check_one(s, agent):
                all_ok = False
        return 0 if all_ok else 2

    if not args:
        print(__doc__)
        return 0
    surface = args[0]

    if check:
        return 0 if _check_one(surface, agent) else 2

    composed = compose(surface, agent)

    if write:
        tgt = _target(surface, agent)
        tgt.parent.mkdir(parents=True, exist_ok=True)
        tgt.write_text(composed)
        print(f"wrote {tgt} ({len(composed)} bytes)")
        return 0

    sys.stdout.write(composed)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
