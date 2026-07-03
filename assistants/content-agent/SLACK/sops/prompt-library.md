# SOP — the prompt library (content-agent)

Your durable memory of **what to generate**. Where `generate.md` owns the call to the
content-lab API, this SOP owns the *prompt* you feed it: the crowned **master** prompt
per content format plus the top production-proven performers, and which prompt each
**page** is running. Backed by the deterministic harness — you never hand-invent a
format's prompt when a proven one exists.

Harness: `harness/prompt_library.py` (wrapper: `bin/content-agent-prompts`). Pure
stdlib, local SQLite (`data/prompt_library.db`), rebuildable from
`data/prompt_library.seed.json`. Granted in `tools.toml` under `[tool.prompt_library]`.

## The formats (crowned masters, 8-round bake-off, 4-judge council)

| Format slug | What it is | Master method | Council |
|---|---|---|---|
| `silhouette-truck` | Couple silhouette + F-150 at dusk (relationship/CountryTok) | i2v anchor | 8.88 |
| `truck-scenic` | Parked black F-150, golden hour, no people (TruckTok) | i2v anchor | 8.57 |
| `truck-ugc` | Handheld creator walkaround of a lifted F-150 (TruckTok) | hailuo t2v | 8.35 |
| `boat-lake` | Summer golden-hour lake-ride POV | i2v anchor | 8.47 |

**The core method finding:** batch consistency (same truck/couple/boat every clip) is
NOT promptable — the generator can't see its own other outputs. 3 of 4 masters are
**image-to-video**: upload a clean anchor frame, run a short motion prompt, and every
clip in the batch inherits the anchor. UGC is the exception — hailuo text-to-video with
a legible-FORD-grille lock. Anchor frames ship in `anchors/`.

## Intake → reason → act

**"What's our master prompt for X / give me the X format."**
- `bin/content-agent-prompts master <format>` → the exact prompt + copy-paste run
  settings (provider, method, resolution, anchor image, provider args). Hand it back
  verbatim; if they want it generated, pass those exact settings to `generate.md`
  (for i2v, upload the named anchor from `anchors/` first).

**"Assign the truck formats to <new page>." / "What is <page> running?"**
- `bin/content-agent-prompts assign <page> <format>` binds that format's master to the
  page (add `--prompt-id` to pin a specific performer, `--note` for context). This is
  how a new page gets its reusable kit.
- `bin/content-agent-prompts page <page>` prints the page's full kit — one prompt per
  format, run-ready.

**"Show me the other top prompts for X."**
- `bin/content-agent-prompts top <format> [--n 15]` → the production-proven performers
  (ranked by videos historically generated). Use when the master doesn't fit the ask or
  the page wants variety.

**"We found a better one / here's a new bake-off winner."**
- `bin/content-agent-prompts add-winner <format> --json '{...}'` promotes a new prompt;
  `tier:"master"` repoints the format's master. Then commit the regenerated
  `data/prompt_library.db` **and** update `data/prompt_library.seed.json` so the seed
  stays the source of truth (the DB is rebuilt from it).

## Rules

- **Never invent a format's prompt when a master exists** — read it from the library and
  use its exact settings. That's the whole point of the library.
- **i2v needs its anchor.** For any `method=i2v` master, the `anchor_image` must be
  uploaded to the generate call — the prompt alone won't reproduce the look.
- **The seed is the source of truth.** Edits go to `data/prompt_library.seed.json`, then
  `seed --force` rebuilds the DB; commit both together.
- Page assignments are append-only per (page, format) — reassigning overwrites that one
  slot, it never deletes history you weren't asked to remove.
