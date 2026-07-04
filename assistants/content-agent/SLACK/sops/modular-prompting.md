# SOP — modular prompting (CANON)

**This is how we generate content now.** A master prompt is a *recipe*, not a deliverable.
You NEVER run one prompt N times — that produces near-identical clips and TikTok's
unoriginal-content detection suppresses the whole batch (law 0, the highest rule). Instead
every batch is composed from a **modular template**: an immutable SPINE plus rotating SCENE
SLOTS. The subject stays rock-solid (same truck, brand-consistent); the scene changes every
clip. Consistency = same brand/format; Variety = different scenes. Both, always.

Backed by the prompt-library harness (`bin/content-agent-prompts`, SQLite tables
`modular_formats` + `modular_slots`). Modular formats: `truck`, `scenic`, `silhouette`,
`ugc`, `boat`.

## The framework

- **SPINE (immutable, in every prompt verbatim):** the subject identity (exact truck) + all
  quality/consistency guards (no lettering, wheels stationary, nothing else moving,
  photorealistic, format). This is what keeps the truck solid batch after batch.
- **SLOTS (rotated, never the subject or a guard):** `angle`, `setting`, `light`,
  `composition`, `atmosphere` — each a menu of on-brand values. One value per slot = one
  scene. The composer rotates them so no two clips share a scene combo.
- A slot may only change the *scene*, never the *subject or a guard*. "red barn setting" ✓ ·
  "white truck" ✗ (identity) · "wheels spinning" ✗ (guard). Reject any variable that touches
  the subject or removes a guard.

## The truck bible — MiniMax + 5-way crop (Eric's canon, cost-efficient)

The `truck` format is the flagship of the modular approach:
- **Provider: `hailuo` (MiniMax)** — crispier, more consistent truck than i2v.
- **`crop_mode: both` = 5-way crop.** One wide side-profile generation → **5 distinct 9:16
  clips** (grille/front-wheel · front-cab · cab · bed · tailgate). ~$0.28/gen ≈ **$0.056/clip**
  (~6× cheaper than i2v).
- **The spine forces an edge-to-edge full side profile** (front bumper at the far-left edge,
  tailgate at the far-right) — that's what makes the 5 crops slice into *distinct* sections
  instead of near-duplicates. A centered truck yields overlapping crops; edge-to-edge yields
  five real clips.
- So **10 rotated scenes × 5 crops = 50 varied clips for ~$3.** Ten different backdrops ×
  five framings each = genuine variety, TikTok-safe.

## How to run it

1. **Compose the scenes** (spine locked, slots rotated, no repeats):
   `bin/content-agent-prompts compose truck --n 10`  (add `--json` for the machine list)
2. **Generate each scene** via the content-lab (`sops/generate.md`), passing the composed
   prompt + the format's provider/extras. For `truck`: `provider=hailuo`, `crop_mode=both`,
   `optimize_prompt=false` — then download **all crops** of each generation (5 clips/gen), not
   just the first.
3. **Rotate before you post.** A page's batch must span multiple scenes; never hand a page 5
   near-identical clips from one generation to one account.

`harness/modular_batch.py` does the whole loop for a t2v-crop format (compose → generate →
download every crop) — `python3 harness/modular_batch.py truck 10`.

## QA filter — MANDATORY before posting (never skip)

A modular batch is not postable until it's filtered. Generators drift within a format:
- **truck (hailuo):** renders a garbled flank decal on ~25-30% of side-shot clips no matter
  the wording ("murdered-out / blacked-out" positive framing beats "no badges" but only
  gets ~73% clean — law 21). So **generate ~1.5× the target and drop the gibberish ones.**
- **i2v (silhouette/scenic/boat):** the anchor can occasionally break a guard (faces leak in,
  couple not fully featureless, floating artifact). Filter the clips (and eyeball anchors).

Run a cheap Gemini pass per clip for the format's failure modes (garbled lettering / wheels
rotating / people / featureless-broken), keep only clean, top up to target. Yield ≈ 65-75%,
so plan the generation count accordingly. A page never receives an unfiltered batch.

## Adding / editing a modular format
Edit `data/prompt_library.seed.json` under `"modular"` (spine, template, slot menus), then
`harness/prompt_library.py seed --force` and commit the seed + rebuilt DB together. Keep slot
values on-brand; never let one touch the subject or a guard.
