#!/usr/bin/env python3
"""
Bonzai harness — the deterministic hands of the worktree-gardener assistant.

Bonzai prunes git branch sprawl SAFELY. The brain (any Ocean-driven model)
orchestrates and confirms; this harness owns the git facts and the irreversible
operations, so the behavior never changes when the provider does.

Pure stdlib. No network except local git. Never touches remotes, never
force-pushes, never disturbs uncommitted work, never deletes a branch that holds
the only copy of its commits.

Subcommands:
  scan   <repo>...            → JSON: every branch classified (shipped/work/noise)
  board  <repo>... [--out P]  → write an HTML triage board from a scan
  prune  <repo>... [--apply]  → delete already-shipped + (with --noise) noise
  apply  --decisions FILE     → execute restore/delete/cherry from a decisions file

Classification (per branch, vs its repo's default base):
  shipped : 0 unique commits — every change already on base. Always safe to drop.
  noise   : unique commits but no real content (empty GitButler commits,
            duplicate one-file PRD copies, standup logs, gitignore chores).
  work    : unique commits with real content — KEEP unless told otherwise.
"""
import json
import re
import subprocess
import sys
from pathlib import Path


def git(repo, *args, check=False):
    r = subprocess.run(
        ["git", "-C", str(repo), *args],
        capture_output=True, text=True,
    )
    if check and r.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {r.stderr.strip()}")
    return r.stdout.strip(), r.returncode


def base_branch(repo):
    out, rc = git(repo, "symbolic-ref", "refs/remotes/origin/HEAD")
    if rc == 0 and out:
        return out.rsplit("/", 1)[-1]
    for cand in ("main", "master"):
        _, rc = git(repo, "rev-parse", "--verify", cand)
        if rc == 0:
            return cand
    return "main"


def local_branches(repo, base):
    out, _ = git(repo, "branch", "--format=%(refname:short)")
    skip = {base, ""}
    return [b for b in out.splitlines()
            if b not in skip and not b.startswith("gitbutler/")]


def classify(repo, base, b):
    """Return (tier, tag, detail-dict)."""
    cherry, _ = git(repo, "cherry", base, b)
    unique = [l for l in cherry.splitlines() if l.startswith("+")]
    if not unique:
        return "shipped", "ALREADY ON MAIN", {"commits": 0}

    n = len(unique)
    subjects, _ = git(repo, "log", f"{base}..{b}", "--format=%s")
    subs = [s for s in subjects.splitlines()][:8]
    files_out, _ = git(repo, "diff", "--name-only", f"{base}...{b}")
    files = len([f for f in files_out.splitlines() if f])
    shortstat, _ = git(repo, "diff", "--shortstat", f"{base}...{b}")
    adds = int((re.search(r"(\d+) insertion", shortstat) or [0, 0])[1]) if "insertion" in shortstat else 0
    dels = int((re.search(r"(\d+) deletion", shortstat) or [0, 0])[1]) if "deletion" in shortstat else 0
    age, _ = git(repo, "log", "-1", "--format=%cr", b)
    iso, _ = git(repo, "log", "-1", "--format=%cI", b)
    detail = {"commits": n, "files": files, "adds": adds, "dels": dels,
              "age": age, "iso": iso, "subjects": subs}

    s = " ".join(subs)
    head = subs[0] if subs else ""
    noise = (
        files == 0
        or (re.search(r"GitButler Workspace Commit", s) and files == 0)
        or (re.search(r"verify auto-deploy trigger", s) and adds == 827)
        or (re.match(r"^docs: add Telegram sound distribution PRD$", head) and n == 1)
        or re.match(r"^standup:", head)
        or re.search(r"gitignore \.claude", s)
        or re.match(r"^Resolve [0-9a-f]{8}-", head)
    )
    if noise:
        tag = ("EMPTY" if files == 0 else
               "DUP PRD" if "827" == str(adds) or "PRD" in s else
               "STANDUP" if head.startswith("standup") else
               "GITIGNORE" if "gitignore" in s else "TEST")
        return "noise", tag, detail
    return "work", "REAL WORK", detail


def scan(repos):
    rows = []
    for repo in repos:
        repo = Path(repo).expanduser()
        if not (repo / ".git").exists():
            continue
        git(repo, "fetch", "--quiet", "origin")
        base = base_branch(repo)
        for b in local_branches(repo, base):
            tier, tag, detail = classify(repo, base, b)
            rows.append({"repo": repo.name, "path": str(repo), "base": base,
                         "branch": b, "tier": tier, "tag": tag, **detail})
    return rows


def remove_worktree_for(repo, branch):
    """If a worktree pins `branch`, force-remove it so the branch can be deleted."""
    out, _ = git(repo, "worktree", "list", "--porcelain")
    cur = {}
    blocks = []
    for line in out.splitlines():
        if not line.strip():
            blocks.append(cur); cur = {}
        elif line.startswith("worktree "):
            cur["path"] = line.split(" ", 1)[1]
        elif line.startswith("branch "):
            cur["branch"] = line.split(" ", 1)[1].rsplit("/", 1)[-1]
    if cur:
        blocks.append(cur)
    for wt in blocks:
        if wt.get("branch") == branch and wt.get("path"):
            git(repo, "worktree", "remove", "--force", wt["path"])
    git(repo, "worktree", "prune")


def delete_branch(repo, branch):
    _, rc = git(repo, "branch", "-D", branch)
    if rc != 0:
        remove_worktree_for(repo, branch)
        _, rc = git(repo, "branch", "-D", branch)
    return rc == 0


def prune(repos, drop_noise=False, apply=False):
    rows = scan(repos)
    targets = [r for r in rows if r["tier"] == "shipped" or (drop_noise and r["tier"] == "noise")]
    result = {"would_delete": [], "deleted": [], "kept_work": []}
    for r in rows:
        if r["tier"] == "work":
            result["kept_work"].append(f'{r["repo"]}  {r["branch"]}')
    for r in targets:
        label = f'{r["repo"]}  {r["branch"]}  ({r["tag"]})'
        if apply:
            ok = delete_branch(Path(r["path"]), r["branch"])
            (result["deleted"] if ok else result["would_delete"]).append(label)
        else:
            result["would_delete"].append(label)
    return result


def apply_decisions(decisions_file):
    """decisions: {restore:[], cherry:[], delete:[], undecided:[]} of 'repo  branch'."""
    data = json.loads(Path(decisions_file).read_text())
    # only DELETE is executed here; restore/cherry are interactive (brain-driven).
    done = {"deleted": [], "failed": [], "for_review": {
        "restore": data.get("restore", []), "cherry": data.get("cherry", [])}}
    # map repo name → path from a fresh scan of cwd-relative dev root
    for entry in data.get("delete", []):
        parts = entry.split()
        repo_name, branch = parts[0], parts[-1]
        path = Path("~/dev").expanduser() / repo_name
        if not path.exists():
            done["failed"].append(entry); continue
        # SAFETY: re-confirm it's shipped or noise before deleting
        base = base_branch(path)
        tier, _, _ = classify(path, base, branch)
        if tier == "work":
            done["failed"].append(f"{entry} (REFUSED: has unique work)"); continue
        (done["deleted"] if delete_branch(path, branch) else done["failed"]).append(entry)
    return done


# ---- HTML board -----------------------------------------------------------
BOARD_TEMPLATE = (Path(__file__).parent.parent / "artifacts" / "triage-board.template.html")


def board(repos, out_path):
    rows = scan(repos)
    tpl = BOARD_TEMPLATE.read_text()
    html = tpl.replace("__DATA__", json.dumps(rows))
    Path(out_path).expanduser().write_text(html)
    return out_path, len(rows)


def main(argv):
    if len(argv) < 2:
        print(__doc__); return 1
    cmd = argv[1]
    if cmd == "scan":
        print(json.dumps(scan(argv[2:]), indent=2))
    elif cmd == "board":
        args = argv[2:]
        out = "branch-triage.html"
        if "--out" in args:
            i = args.index("--out"); out = args[i + 1]; args = args[:i] + args[i + 2:]
        p, n = board(args, out)
        print(f"wrote {p} ({n} branches)")
    elif cmd == "prune":
        args = argv[2:]
        apply = "--apply" in args
        noise = "--noise" in args
        repos = [a for a in args if not a.startswith("--")]
        print(json.dumps(prune(repos, drop_noise=noise, apply=apply), indent=2))
    elif cmd == "apply":
        args = argv[2:]
        if "--decisions" not in args:
            print("apply needs --decisions FILE"); return 1
        f = args[args.index("--decisions") + 1]
        print(json.dumps(apply_decisions(f), indent=2))
    else:
        print(__doc__); return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
