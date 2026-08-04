# CoffeeTok Anti-Slop Doctrine (mindfulsources format)

A self-prompt: the rules every coffee anchor AND every generated clip must obey. Anything that
breaks a REJECT rule is slop — it never ships, no matter how good the rest looks. The council
(gemini-video watches the actual motion) is the enforcer; these rules are what it rejects on.

## The reference aesthetic (assessed from the real @mindfulsources page, 2026-07-03)
The look is ONE thing on repeat, not variety of vessels:
- **Vintage white/cream porcelain teacup** — fine bone china, not a modern mug.
- **Thin delicate GOLD rim line** (sometimes a double gold band) and an **ornate gold handle**.
- **Sitting on a matching white porcelain saucer** (a visible white ring around the cup).
- Occasional subtle detail: a tiny blue floral sprig, or a scalloped/fluted rim — same family.
- **Dark near-black coffee** with a **natural marbled tan crema swirl** (real espresso crema,
  fine bubbles, organic — never latte art, never milky).
- **Pure solid black background.** Perfect **top-down flat-lay**. Cup centered.
The ONLY thing that changes clip-to-clip is the **crema/foam swirl pattern**. Everything else
is identical on purpose — that repetition IS the brand.

## VARIATION — the only knobs allowed
1. Crema pattern: bold center swirl · off-center swirl · thin rim ring (mostly dark) · full even
   crema · lightly marbled · almost none (nearly clean black) · bubble bloom near rim.
2. Subtle cup detail: plain gold rim · gold double-rim · tiny blue floral sprig · scalloped rim.
NEVER vary: vessel type (always the vintage white porcelain teacup + saucer), background (always
pure black), angle (always dead top-down), color of the cup (always white/cream porcelain).

## REJECT rules (auto-fail — council cuts these before Eric ever sees them)
R1. **No humans and nothing entering, ever.** No hand, finger, arm, or person; and NO pour —
    no pitcher, jug, kettle, pour stream, or liquid poured in, no object entering from any edge.
    The scene is closed and complete from first to last frame; only the coffee surface moves.
    (wan-i2v's #1 hallucination — law 17.) A clip where anything enters is dead on arrival.
R2. **One cup only.** No second cup, no duplicate, no cup splitting/cloning mid-clip.
R3. **No props/utensils.** No spoon, straw, napkin, book, phone, plate-of-food — nothing but
    the cup + saucer.
R4. **No steam, no smoke.**
R5. **Cup, saucer, rim, handle stay PERFECTLY fixed.** No whole-cup rotation, no spin, no drift,
    no zoom, no camera move. ONLY the coffee surface moves.
R6. **Crema moves like real liquid** — a slow gentle continuous drift. REJECT morphing, melting,
    fabric-like texture flow, boiling, or fast spinning. Real surface tension only.
R7. **Pure solid black background.** REJECT gradients, vignettes, table surfaces, reflections,
    or any grey wash.
R8. **No text, letters, numbers, or logos** anywhere (captions are added later in post).
R9. **Photographic, not CGI.** REJECT plastic/glossy sheen, video-game render look, or an
    obviously-fake "digital" crema.
R10. **Consistent framing.** Cup centered, top-down, generous black caption space. REJECT a cup
    cropped off-frame, tilted off top-down, or shoved into a corner.

## Framing spec (Eric, 2026-07-03) — guaranteed in post, not left to the generator
- **Pure #000000 backdrop.** Crush the dark/charcoal background to true black so the cup pops.
- **Saucer edge-to-edge.** Scale the cup+saucer so the plate spans the full 9:16 width.
- **Dead center.** Cup centered with equal black bands above/below — a stable frame for viral
  on-screen text to land (top, bottom, or over the coffee).
These are applied to EVERY clip by `frame_clip()` in modular_i2v_batch.py (crush → crop-to-cup →
scale-to-width → symmetric black pad). The prompt asks for them too, but the post-step guarantees
them regardless of generator drift.

## How it's enforced
- Anchor-gen guards (in the modular `quality_guards`) bake R2–R10 into every minted anchor.
- The fixed `motion_prompt` bakes R1–R6 into every clip ("only the coffee surface moves… empty
  frame, no people, no hands, no arms, cup and handle fixed").
- **End-of-clip intruder trim (auto).** wan often hallucinates a hand/pitcher creeping in during
  the final ~0.5s — and 4-frame contact sheets miss it. `frame_clip()` scans the clip: on the
  pure-black bg, any bright pixels appearing OUTSIDE the stable cup = an intruder, and the clip is
  trimmed to just before it enters (min length 2s). This runs on every clip regardless of judging.
  **Min length ≥5s:** if trimming drops a clip under 5s, it's slowed ~10% (setpts) back over 5
  seconds — imperceptible on a gentle loop — so every delivered clip is ≥5s.
- Every clip is council-judged; gemini-video (watches the mp4) catches R1/R5/R6 motion slop that
  a still frame misses. Any clip tripping a REJECT rule is discarded. Only clean clips ship.
- Over-generate (2+ clips/anchor), keep the clean ones, toss the rest. Slop is expected from the
  generator; the filter is the product.
