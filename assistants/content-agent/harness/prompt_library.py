#!/usr/bin/env python3
"""
Prompt-library harness — the deterministic hands of content-agent's prompt memory.

content-agent reasons + confirms; this harness owns the facts: the crowned MASTER
prompt per content format plus the top production-proven performers, and which
prompt is assigned to which page. The brain (any Ocean-driven model) reads a master,
copies its exact run settings into a content-lab generate call, or assigns a format's
master to a new page. The behavior never changes when the provider does.

Pure stdlib (sqlite3, json, argparse). No network. The DB is the source of truth;
`data/prompt_library.seed.json` is the version-controlled seed it is rebuilt from.

The library came from an 8-round evolutionary bake-off (4-judge council: GPT-4.1
frames + Gemini full-video + two Fable binary-checklist judges) against the live
content-posting-lab. Masters are i2v-anchored (3 of 4) or hailuo t2v (UGC); the
top performers are the historically most-produced prompts per format.

Subcommands:
  seed [--force]            → (re)build the DB from the seed JSON
  formats                   → list every format + its crowned master + score
  master  <format>          → print the master prompt with copy-paste run settings
  top     <format> [--n N]  → list the top performers for a format (default 15)
  show    <prompt_id>       → full record for one prompt (JSON)
  assign  <page> <format> [--prompt-id ID] [--note ...]
                            → assign a format's master (or a specific prompt) to a page
  page    <page>            → show every prompt assigned to a page (the page's kit)
  add-winner <format> --json '{...}'
                            → add/promote a new winning prompt (e.g. a fresh bake-off master)
  export  [--format md|json]→ dump the whole library (md = human-readable cards)
"""
import argparse
import json
import sqlite3
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent          # assistants/content-agent/
DB = ROOT / "data" / "prompt_library.db"
SEED = ROOT / "data" / "prompt_library.seed.json"

SCHEMA = """
CREATE TABLE IF NOT EXISTS formats (
    slug             TEXT PRIMARY KEY,
    title            TEXT NOT NULL,
    niche            TEXT,
    description      TEXT,
    master_prompt_id TEXT,
    created_at       TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS prompts (
    id               TEXT PRIMARY KEY,
    format_slug      TEXT NOT NULL,
    rank             INTEGER,
    tier             TEXT NOT NULL,          -- 'master' | 'top-performer'
    method           TEXT NOT NULL,          -- 'i2v' | 't2v'
    provider         TEXT NOT NULL,
    resolution       TEXT,
    aspect_ratio     TEXT,
    duration         INTEGER,
    prompt           TEXT NOT NULL,
    anchor_image     TEXT,                   -- i2v only: the hero frame to animate
    extra_json       TEXT,                   -- provider extras (crop_mode, optimize_prompt, ...)
    council_score    REAL,                   -- bake-off 4-judge weighted mean
    videos_generated INTEGER,                -- production usage (proven signal)
    notes            TEXT,
    source           TEXT NOT NULL,          -- 'bakeoff-2026-07' | 'production-history'
    created_at       TEXT NOT NULL,
    FOREIGN KEY (format_slug) REFERENCES formats(slug)
);
CREATE TABLE IF NOT EXISTS page_assignments (
    page         TEXT NOT NULL,              -- the page/account the prompt is assigned to
    format_slug  TEXT NOT NULL,
    prompt_id    TEXT NOT NULL,
    assigned_at  TEXT NOT NULL,
    note         TEXT,
    PRIMARY KEY (page, format_slug)          -- one prompt per (page, format)
);
"""


def now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def connect():
    if not DB.exists():
        sys.exit(f"no DB at {DB} — run: prompt_library.py seed")
    con = sqlite3.connect(DB)
    con.row_factory = sqlite3.Row
    return con


def cmd_seed(args):
    data = json.loads(SEED.read_text())
    if DB.exists() and not args.force:
        sys.exit(f"{DB} exists — pass --force to rebuild")
    DB.parent.mkdir(parents=True, exist_ok=True)
    if DB.exists():
        DB.unlink()
    con = sqlite3.connect(DB)
    con.executescript(SCHEMA)
    for f in data["formats"]:
        con.execute(
            "INSERT INTO formats(slug,title,niche,description,master_prompt_id,created_at) VALUES(?,?,?,?,?,?)",
            (f["slug"], f["title"], f.get("niche"), f.get("description"),
             f.get("master_prompt_id"), f.get("created_at", now())))
    for p in data["prompts"]:
        con.execute(
            """INSERT INTO prompts(id,format_slug,rank,tier,method,provider,resolution,aspect_ratio,
               duration,prompt,anchor_image,extra_json,council_score,videos_generated,notes,source,created_at)
               VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
            (p["id"], p["format_slug"], p.get("rank"), p["tier"], p["method"], p["provider"],
             p.get("resolution"), p.get("aspect_ratio"), p.get("duration"), p["prompt"],
             p.get("anchor_image"), p.get("extra_json"), p.get("council_score"),
             p.get("videos_generated"), p.get("notes"), p["source"], p.get("created_at", now())))
    con.commit()
    nf = con.execute("SELECT count(*) FROM formats").fetchone()[0]
    npr = con.execute("SELECT count(*) FROM prompts").fetchone()[0]
    con.close()
    print(f"seeded {DB} — {nf} formats, {npr} prompts")


def _run_block(p):
    """Copy-paste run settings for a prompt row."""
    lines = [f"  provider     : {p['provider']}",
             f"  method       : {p['method']}",
             f"  resolution   : {p['resolution']}    aspect: {p['aspect_ratio']}    duration: {p['duration']}s"]
    if p["anchor_image"]:
        lines.append(f"  anchor image : {p['anchor_image']}   (upload this, then run the prompt)")
    if p["extra_json"]:
        lines.append(f"  provider args: {p['extra_json']}")
    return "\n".join(lines)


def cmd_formats(args):
    con = connect()
    for f in con.execute("SELECT * FROM formats ORDER BY slug"):
        m = con.execute("SELECT * FROM prompts WHERE id=?", (f["master_prompt_id"],)).fetchone()
        score = f"council {m['council_score']}" if m and m["council_score"] else "—"
        print(f"\n● {f['slug']}  —  {f['title']}  ({f['niche']})")
        print(f"  master: {f['master_prompt_id']}  [{score}]  method={m['method'] if m else '?'}")
    con.close()


def cmd_master(args):
    con = connect()
    f = con.execute("SELECT * FROM formats WHERE slug=?", (args.format,)).fetchone()
    if not f:
        sys.exit(f"unknown format '{args.format}' — try: formats")
    m = con.execute("SELECT * FROM prompts WHERE id=?", (f["master_prompt_id"],)).fetchone()
    print(f"# MASTER — {f['title']}  ({f['slug']})")
    print(f"# council score {m['council_score']}   source {m['source']}\n")
    print("PROMPT:")
    print(m["prompt"] + "\n")
    print("RUN SETTINGS:")
    print(_run_block(m))
    if m["notes"]:
        print(f"\nNOTE: {m['notes']}")
    con.close()


def cmd_top(args):
    con = connect()
    rows = con.execute(
        "SELECT * FROM prompts WHERE format_slug=? ORDER BY rank LIMIT ?",
        (args.format, args.n + 1)).fetchall()
    if not rows:
        sys.exit(f"unknown format '{args.format}' — try: formats")
    for p in rows:
        tag = "★ MASTER" if p["tier"] == "master" else f"#{p['rank']}"
        sig = f"council {p['council_score']}" if p["council_score"] else f"{p['videos_generated']} vids"
        print(f"\n{tag}  [{p['id']}]  ({sig}, {p['provider']})")
        print(f"  {p['prompt']}")
    con.close()


def cmd_show(args):
    con = connect()
    p = con.execute("SELECT * FROM prompts WHERE id=?", (args.prompt_id,)).fetchone()
    con.close()
    if not p:
        sys.exit(f"no prompt '{args.prompt_id}'")
    print(json.dumps(dict(p), indent=1))


def cmd_assign(args):
    con = connect()
    f = con.execute("SELECT * FROM formats WHERE slug=?", (args.format,)).fetchone()
    if not f:
        sys.exit(f"unknown format '{args.format}'")
    pid = args.prompt_id or f["master_prompt_id"]
    p = con.execute("SELECT * FROM prompts WHERE id=? AND format_slug=?", (pid, args.format)).fetchone()
    if not p:
        sys.exit(f"prompt '{pid}' is not in format '{args.format}'")
    con.execute(
        """INSERT INTO page_assignments(page,format_slug,prompt_id,assigned_at,note)
           VALUES(?,?,?,?,?)
           ON CONFLICT(page,format_slug) DO UPDATE SET
             prompt_id=excluded.prompt_id, assigned_at=excluded.assigned_at, note=excluded.note""",
        (args.page, args.format, pid, now(), args.note))
    con.commit()
    con.close()
    print(f"assigned {pid} → page '{args.page}' for format '{args.format}'")


def cmd_page(args):
    con = connect()
    rows = con.execute(
        """SELECT a.format_slug, a.prompt_id, a.note, p.prompt, p.provider, p.method,
                  p.anchor_image, p.resolution, p.aspect_ratio, p.duration, p.extra_json
           FROM page_assignments a JOIN prompts p ON p.id=a.prompt_id
           WHERE a.page=? ORDER BY a.format_slug""", (args.page,)).fetchall()
    con.close()
    if not rows:
        print(f"page '{args.page}' has no assignments yet")
        return
    print(f"# Prompt kit for page: {args.page}\n")
    for r in rows:
        print(f"● {r['format_slug']}  [{r['prompt_id']}]")
        print(f"  {r['prompt']}")
        print(_run_block(r))
        if r["note"]:
            print(f"  note: {r['note']}")
        print()


def cmd_add_winner(args):
    p = json.loads(args.json)
    p.setdefault("format_slug", args.format)
    for req in ("id", "tier", "method", "provider", "prompt", "source"):
        if req not in p:
            sys.exit(f"add-winner json missing required field: {req}")
    con = connect()
    con.execute(
        """INSERT OR REPLACE INTO prompts(id,format_slug,rank,tier,method,provider,resolution,
           aspect_ratio,duration,prompt,anchor_image,extra_json,council_score,videos_generated,
           notes,source,created_at)
           VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
        (p["id"], p["format_slug"], p.get("rank"), p["tier"], p["method"], p["provider"],
         p.get("resolution", "720p"), p.get("aspect_ratio", "9:16"), p.get("duration", 6),
         p["prompt"], p.get("anchor_image"), p.get("extra_json"), p.get("council_score"),
         p.get("videos_generated"), p.get("notes"), p["source"], now()))
    if p["tier"] == "master":
        con.execute("UPDATE formats SET master_prompt_id=? WHERE slug=?", (p["id"], args.format))
    con.commit()
    con.close()
    print(f"added {p['id']} to {args.format}" + ("  (now the master)" if p["tier"] == "master" else ""))


def cmd_export(args):
    con = connect()
    formats = [dict(r) for r in con.execute("SELECT * FROM formats ORDER BY slug")]
    prompts = [dict(r) for r in con.execute("SELECT * FROM prompts ORDER BY format_slug, rank")]
    con.close()
    if args.format == "json":
        print(json.dumps({"formats": formats, "prompts": prompts}, indent=1))
        return
    print("# Rising Tides — Content Prompt Library\n")
    for f in formats:
        print(f"## {f['title']}  (`{f['slug']}`)")
        print(f"*{f['niche']}* — {f['description']}\n")
        for p in [x for x in prompts if x["format_slug"] == f["slug"]]:
            tag = "★ MASTER" if p["tier"] == "master" else f"#{p['rank']}"
            sig = f"council {p['council_score']}" if p["council_score"] else f"{p['videos_generated']} vids"
            print(f"- **{tag}** ({sig}, `{p['provider']}` {p['method']})"
                  + (f" · anchor `{p['anchor_image']}`" if p["anchor_image"] else "") + f": {p['prompt']}")
        print()


def main():
    ap = argparse.ArgumentParser(description="content-agent prompt library")
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("seed"); s.add_argument("--force", action="store_true"); s.set_defaults(fn=cmd_seed)
    sub.add_parser("formats").set_defaults(fn=cmd_formats)
    s = sub.add_parser("master"); s.add_argument("format"); s.set_defaults(fn=cmd_master)
    s = sub.add_parser("top"); s.add_argument("format"); s.add_argument("--n", type=int, default=15); s.set_defaults(fn=cmd_top)
    s = sub.add_parser("show"); s.add_argument("prompt_id"); s.set_defaults(fn=cmd_show)
    s = sub.add_parser("assign"); s.add_argument("page"); s.add_argument("format")
    s.add_argument("--prompt-id"); s.add_argument("--note"); s.set_defaults(fn=cmd_assign)
    s = sub.add_parser("page"); s.add_argument("page"); s.set_defaults(fn=cmd_page)
    s = sub.add_parser("add-winner"); s.add_argument("format"); s.add_argument("--json", required=True); s.set_defaults(fn=cmd_add_winner)
    s = sub.add_parser("export"); s.add_argument("--format", choices=["md", "json"], default="md"); s.set_defaults(fn=cmd_export)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
