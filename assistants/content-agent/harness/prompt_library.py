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
  publish-bedrock [--dry-run] [--dest /context/content-prompts]
                            → publish per-format cards + index + library.json to
                              ocean-bedrock (/context) for org-wide reuse. Env:
                              OCEAN_BEDROCK_URL + OCEAN_BEDROCK_TOKEN.
"""
import argparse
import json
import os
import sqlite3
import sys
import urllib.error
import urllib.parse
import urllib.request
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
-- Modular prompting framework (CANON). A winning prompt split into an immutable SPINE
-- (subject + quality guards, verbatim every time) and rotating SCENE SLOTS, so the truck
-- stays solid while every clip is a new scene (law 0: rotate or TikTok flags unoriginal).
CREATE TABLE IF NOT EXISTS modular_formats (
    format_slug     TEXT PRIMARY KEY,
    method          TEXT NOT NULL,           -- 'i2v' | 't2v'
    provider        TEXT NOT NULL,           -- e.g. 'hailuo' (MiniMax, crispier + 5-way crop)
    fixed_subject   TEXT NOT NULL,           -- the immutable subject/brand identity
    quality_guards  TEXT NOT NULL,           -- the immutable quality/consistency guards
    template        TEXT NOT NULL,           -- prompt/anchor template with {slot} placeholders
    motion_prompt   TEXT,                    -- i2v only: fixed motion prompt for the anchor
    base_anchor     TEXT,                    -- i2v only: subject/style reference frame
    extra_json      TEXT,                    -- provider extras (crop_mode 'both' = 5-way, ...)
    clips_per_gen   INTEGER DEFAULT 1,       -- 5 when crop_mode='both' (the cost multiplier)
    note            TEXT,
    created_at      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS modular_slots (
    format_slug  TEXT NOT NULL,
    slot         TEXT NOT NULL,              -- 'angle' | 'setting' | 'light' | 'composition' | ...
    values_json  TEXT NOT NULL,             -- JSON array of on-brand menu values
    PRIMARY KEY (format_slug, slot)
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
    for slug, m in (data.get("modular") or {}).items():
        con.execute(
            """INSERT INTO modular_formats(format_slug,method,provider,fixed_subject,quality_guards,
               template,motion_prompt,base_anchor,extra_json,clips_per_gen,note,created_at)
               VALUES(?,?,?,?,?,?,?,?,?,?,?,?)""",
            (slug, m["method"], m["provider"], m["fixed_subject"], m["quality_guards"],
             m["template"], m.get("motion_prompt"), m.get("base_anchor"),
             json.dumps(m["extra"]) if m.get("extra") else None,
             m.get("clips_per_gen", 1), m.get("note"), now()))
        for slot, values in m["slots"].items():
            con.execute("INSERT INTO modular_slots(format_slug,slot,values_json) VALUES(?,?,?)",
                        (slug, slot, json.dumps(values)))
    con.commit()
    nf = con.execute("SELECT count(*) FROM formats").fetchone()[0]
    npr = con.execute("SELECT count(*) FROM prompts").fetchone()[0]
    nm = con.execute("SELECT count(*) FROM modular_formats").fetchone()[0]
    con.close()
    print(f"seeded {DB} — {nf} formats, {npr} prompts, {nm} modular formats")


def _rotate_combos(slots, n):
    """Evenly-spread, non-repeating slot combinations (each slot strided by a distinct step)."""
    names = list(slots)
    sizes = [len(slots[k]) for k in names]
    total = 1
    for s in sizes:
        total *= s
    steps = [1, 3, 5, 7, 11, 13][: len(names)] or [1]
    out, seen, i = [], set(), 0
    while len(out) < n and len(seen) < total:
        pick = tuple((i * steps[j]) % sizes[j] for j in range(len(names)))
        if pick not in seen:
            seen.add(pick)
            out.append({names[j]: slots[names[j]][pick[j]] for j in range(len(names))})
        i += 1
    while len(out) < n and out:
        out.append(out[len(out) % len(seen)])
    return out[:n]


def cmd_compose(args):
    """Compose N modular scenes for a format: fixed spine + rotated slots, no repeats."""
    con = connect()
    m = con.execute("SELECT * FROM modular_formats WHERE format_slug=?", (args.format,)).fetchone()
    if not m:
        con.close()
        sys.exit(f"no modular template for '{args.format}' — try: prompt_library.py compose <format>")
    slots = {r["slot"]: json.loads(r["values_json"])
             for r in con.execute("SELECT * FROM modular_slots WHERE format_slug=?", (args.format,))}
    con.close()
    fields = dict(subject=m["fixed_subject"], guards=m["quality_guards"], subject_state=m["quality_guards"])
    recs = []
    for combo in _rotate_combos(slots, args.n):
        text = m["template"].format(**fields, **combo)
        rec = {"format": args.format, "method": m["method"], "provider": m["provider"], "combo": combo}
        if m["method"] == "i2v":
            rec.update(anchor_prompt=text, motion_prompt=m["motion_prompt"], base_anchor=m["base_anchor"])
        else:
            rec.update(prompt=text, extra=json.loads(m["extra_json"]) if m["extra_json"] else {})
        recs.append(rec)
    if args.json:
        print(json.dumps(recs, indent=1))
        return
    cpg = m["clips_per_gen"] or 1
    print(f"# {args.format} — {args.n} modular scenes  ({m['provider']} {m['method']}"
          + (f", {cpg}-way crop → {args.n * cpg} clips" if cpg > 1 else "") + ")")
    print(f"# SPINE (in every scene): {m['fixed_subject']}")
    for i, r in enumerate(recs, 1):
        print(f"\n[{i}] " + " · ".join(f"{k}={v}" for k, v in r["combo"].items()))
        print("   " + (r.get("anchor_prompt") or r.get("prompt")))


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


def _library():
    con = connect()
    formats = [dict(r) for r in con.execute("SELECT * FROM formats ORDER BY slug")]
    prompts = [dict(r) for r in con.execute("SELECT * FROM prompts ORDER BY format_slug, rank")]
    con.close()
    return formats, prompts


def _bedrock_cards(formats, prompts):
    """Build {api_path: (content_bytes, mime)} for the bedrock publish set."""
    files = {}
    # one markdown card per format (semantic-search-friendly)
    for f in formats:
        ps = [p for p in prompts if p["format_slug"] == f["slug"]]
        m = next((p for p in ps if p["tier"] == "master"), None)
        lines = [f"# {f['title']}  (`{f['slug']}`)",
                 f"*{f['niche']}*\n", f"{f['description']}\n"]
        if m:
            lines += ["## ★ Master prompt", "",
                      "```", m["prompt"], "```", "",
                      f"- **method**: {m['method']}   **provider**: {m['provider']}   "
                      f"**resolution**: {m['resolution']}   **aspect**: {m['aspect_ratio']}   "
                      f"**duration**: {m['duration']}s",
                      f"- **council score**: {m['council_score']}   **source**: {m['source']}"]
            if m["anchor_image"]:
                lines.append(f"- **anchor image (i2v)**: `{m['anchor_image']}` — upload this to the "
                             f"generate call; the motion prompt alone won't reproduce the look")
            if m["extra_json"]:
                lines.append(f"- **provider args**: `{m['extra_json']}`")
            if m["notes"]:
                lines += ["", f"> {m['notes']}"]
        lines += ["", "## Top production-proven performers", ""]
        for p in [x for x in ps if x["tier"] != "master"]:
            sig = f"council {p['council_score']}" if p["council_score"] else f"{p['videos_generated']} vids"
            lines.append(f"{p['rank']}. ({sig}, `{p['provider']}`) {p['prompt']}")
        body = ("\n".join(lines) + "\n").encode("utf-8")
        files[f"{f['slug']}.md"] = (body, "text/markdown")
    # modular bible (CANON) — per-format spine + slots, published for org-wide reuse
    con = connect()
    mfs = [dict(r) for r in con.execute("SELECT * FROM modular_formats ORDER BY format_slug")]
    mslots = [dict(r) for r in con.execute("SELECT * FROM modular_slots ORDER BY format_slug, slot")]
    con.close()
    if mfs:
        mb = ["# Modular Prompt Bible (CANON)", "",
              "Never run one prompt N times (TikTok flags near-identical uploads). Compose "
              "rotated scenes off a fixed spine: subject stays solid, scene changes every clip.", ""]
        for m in mfs:
            cpg = m.get("clips_per_gen") or 1
            mb.append(f"## `{m['format_slug']}` — {m['provider']} {m['method']}"
                      + (f" · {cpg}-way crop ({cpg} clips/gen)" if cpg > 1 else ""))
            mb.append(f"- **spine (immutable):** {m['fixed_subject']}")
            mb.append(f"- **guards:** {m['quality_guards']}")
            if m.get("extra_json"):
                mb.append(f"- **provider args:** `{m['extra_json']}`")
            mb.append(f"- **template:** {m['template']}")
            for s in [x for x in mslots if x["format_slug"] == m["format_slug"]]:
                vals = json.loads(s["values_json"])
                mb.append(f"- **slot `{s['slot']}`** ({len(vals)}): {' · '.join(vals)}")
            mb.append("")
        files["modular-bible.md"] = (("\n".join(mb) + "\n").encode("utf-8"), "text/markdown")
    # machine-readable library
    files["library.json"] = (
        (json.dumps({"generated_at": now(), "formats": formats, "prompts": prompts,
                     "modular_formats": mfs, "modular_slots": mslots}, indent=1) + "\n").encode("utf-8"),
        "application/json")
    # index README
    idx = ["# Rising Tides — Content Prompt Library",
           "", "Reusable, proven video prompts per content format — the crowned **master** "
           "plus top production performers. Source of truth: the `content-agent` package in "
           "ocean-agents (local SQLite); this is the org-wide-readable mirror, refreshed on publish.",
           "", "| Format | Master method | Council | Card |", "|---|---|---|---|"]
    for f in formats:
        m = next((p for p in prompts if p["id"] == f["master_prompt_id"]), None)
        idx.append(f"| **{f['title']}** (`{f['slug']}`) | {m['method'] if m else '?'} | "
                   f"{m['council_score'] if m else '—'} | [`{f['slug']}.md`]({f['slug']}.md) |")
    idx += ["", f"_Published {now()} · machine-readable: `library.json`_"]
    files["README.md"] = (("\n".join(idx) + "\n").encode("utf-8"), "text/markdown")
    return files


def cmd_publish_bedrock(args):
    formats, prompts = _library()
    files = _bedrock_cards(formats, prompts)
    dest = args.dest.rstrip("/")
    if args.dry_run:
        print(f"# DRY RUN — would publish {len(files)} files to {dest}/ on ocean-bedrock:\n")
        for name, (body, mime) in sorted(files.items()):
            print(f"  {dest}/{name}   ({len(body)} bytes, {mime})")
        return
    url = os.environ.get("OCEAN_BEDROCK_URL")
    tok = os.environ.get("OCEAN_BEDROCK_TOKEN")
    if not url or not tok:
        sys.exit("set OCEAN_BEDROCK_URL and OCEAN_BEDROCK_TOKEN to publish (see content-agent.env.example)")
    url = url.rstrip("/")
    # ensure the destination folder exists (idempotent)
    try:
        req = urllib.request.Request(
            f"{url}/api/v1/mkdir", method="POST",
            data=json.dumps({"path": dest}).encode(),
            headers={"Authorization": f"Bearer {tok}", "Content-Type": "application/json"})
        urllib.request.urlopen(req, timeout=30)
    except urllib.error.HTTPError as e:
        if e.code not in (200, 201, 409):   # 409 = already exists
            sys.exit(f"mkdir {dest} failed: {e.code} {e.read().decode()[:200]}")
    ok = 0
    for name, (body, mime) in sorted(files.items()):
        path = f"{dest}/{name}"
        req = urllib.request.Request(
            f"{url}/api/v1/file?path={urllib.parse.quote(path)}", method="PUT",
            data=body, headers={"Authorization": f"Bearer {tok}", "Content-Type": mime})
        try:
            urllib.request.urlopen(req, timeout=60)
            ok += 1
            print(f"  published {path}  ({len(body)} bytes)")
        except urllib.error.HTTPError as e:
            print(f"  FAILED {path}: {e.code} {e.read().decode()[:200]}")
    print(f"\npublished {ok}/{len(files)} files to {dest}/ on ocean-bedrock")


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
    s = sub.add_parser("compose"); s.add_argument("format"); s.add_argument("--n", type=int, default=10)
    s.add_argument("--json", action="store_true"); s.set_defaults(fn=cmd_compose)
    s = sub.add_parser("export"); s.add_argument("--format", choices=["md", "json"], default="md"); s.set_defaults(fn=cmd_export)
    s = sub.add_parser("publish-bedrock")
    s.add_argument("--dry-run", action="store_true")
    s.add_argument("--dest", default="/context/content-prompts")
    s.set_defaults(fn=cmd_publish_bedrock)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
