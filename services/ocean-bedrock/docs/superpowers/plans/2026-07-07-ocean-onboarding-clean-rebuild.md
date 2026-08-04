# Ocean Onboarding — Clean Rebuild

Supersedes the Leptos/WASM ceremony approach (`ui/onboarding-ceremony/`).
That build diverged from the spec: a garish globe bolted to a 12-dropdown
button wall, two terminals required. This plan rebuilds the coworker
onboarding as one agent-steered, animated surface in the aesthetic the
owner already liked.

## Goal

A non-technical coworker opens one prefilled link. An onboarding agent
steers the whole experience. The human confirms once. No button wall, no
second terminal, no WASM.

## Stack

- One Node process (`scripts/ocean-local-app.mjs`, trimmed).
- One HTML page served at `/`, vanilla JS + **anime.js v4** (ESM) for
  choreography + **three.js** (ESM) for the centerpiece and particles.
- No build step. No Trunk, no cargo, no Leptos, no Svelte, no bundler.
- Vendored ESM copies of anime.js + three.js under `assets/vendor/`
  (offline-safe; CDN as fallback).

## Aesthetic (anchor on existing tokens)

Deep ocean dark (`#07111f → #050914` radial), glass cards
(`backdrop-filter: blur`, translucent `rgba(8,20,36,.82)`), Inter,
cyan accent (`#7fd1ff` / `#57b5ff`), pill buttons (`border-radius:999px`),
24px rounded cards. Particle/3D layer is monochrome cyan-on-black,
depth-fogged, slow. **No emoji, anywhere.** Restraint is the whole point.

## Centerpiece — the Ocean (three.js)

Reference technique (owner-supplied): high-detail `IcosahedronGeometry`
displaced along normals by periodic Perlin noise (`pnoise`, Gustavson),
scaled by an audio-frequency uniform; monochrome wireframe; camera
mouse-parallax.

Rebuilt clean:
- Port shader to plain three.js ESM. Keep `pnoise` vertex displacement.
- Recolor white → cyan with additive glow; sit on the ocean-dark radial;
  depth fog.
- `u_frequency` ← **ceremony intensity** (sources/skills/projects
  detected) + optional ambient ocean audio when idle. Constellation nodes
  ignite on the surface per detected source. Secrets-skipped dims a node
  to ember-red and disperses nearby particles.

## Particle physics, refined

Soft attractors + mild repulsion + drag around constellation nodes, so the
field breathes instead of spasming. Particles are data (detected items),
never decorative sparkle.

## Spine — kept as-is

The ceremony event protocol (schema, REST `/api/ceremony/*`, WebSocket
`/api/ceremony/ws`, replay, visibility/redaction) is clean and stays. The
agent emits events; the new page is a better renderer for them. Real scan
/ preview / sync still run through the existing endpoints (`/api/preview`,
`/api/preview/ack`, `/api/sync`, source/sync lineage). Timeline narrates,
pipeline works — one unified thing.

## Flow (agent-steered, one confirm)

1. Open prefilled link (`?server=&invite=&name=`). Ocean boots ambient.
2. Agent scans configs, streams events → constellations ignite, items flow.
3. Agent builds import plan, runs dry-run → one glass card
   (changed / skipped / sensitive excluded).
4. One anime.js-focused **review & unlock** beat (the safety gate,
   surfaced as a moment, not a panel).
5. Agent syncs → completion ripple.
6. Schedule = one toggle set in-flow, not a dropdown panel.

## Dies

- `ui/onboarding-ceremony/` (entire Leptos/WASM/Trunk app).
- The `/admin` button wall and inline `INDEX_HTML` config panel.
- Fixture-seeded fake project/skill names from the demo driver.
- The `/ceremony` static-serve route and the favicon-emoji path.

## Build lanes (parallel)

1. **three.js centerpiece** — port noise-globe to ESM, recolor, drive from
   ceremony intensity, constellation nodes. Vendor three.js.
2. **anime.js UI layer** — phase scenes, source/skill/project stream
   cards, preview glass card, the one unlock beat, transitions. Vendor
   anime.js.
3. **Page + protocol wiring** — single HTML page at `/`, WS client,
   reconnect/replay, reduced-motion fallback, the `/admin` deletion.
4. **Skill → real data** — wire the onboarding skill to emit real scan
   events and call real preview/sync (replaces fixture replay).
5. **Cleanup** — delete `ui/onboarding-ceremony/`, demo fixture names,
   stale ceremony-serve code; update `events.md`.

Lanes 1–3 are co-dependent on a shared page shell and event contract;
land the shell + contract first, then 1/2/3/4 fan out.

## Acceptance

- Open `http://127.0.0.1:8765/` → one page, no terminal, no button wall.
- Ocean centerpiece renders, noise-displaced, cyan, depth-fogged.
- Real onboarding scan drives it (actual `~/.claude`/`~/.codex`/`~/.pi`),
  not fixture names.
- One review/unlock beat gates sync; secrets excluded visibly.
- `node --check scripts/ocean-local-app.mjs`, `npm run smoke`,
  `npm run secret:scan`, `npm run secret:globs:check` all pass.
- Zero emojis in output or served UI.

## Open decisions (owner)

- Audio: ambient ocean drone on by default (mutable), or off entirely?
- Particle density: minimal / moderate / rich? (default: minimal)
