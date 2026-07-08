# Ocean — Design Principles

The canonical design language for every Ocean surface — onboarding, the web
surface (Leptos), and the Tauri desktop app. One source of truth so they all
look and feel like the same product. The onboarding + orb are the reference
implementation; everything here is derived from them.

> **Ethos.** The interface is an ocean — deep, dark, and alive. One cyan light
> moves through it.

Ocean is calm, not busy. Deep, not flat. Liquid, not rigid. If a screen breaks
any of those, it's wrong.

---

## Principles

**1. Depth over flatness.**
Never a flat fill. Backgrounds are the radial ocean-dark gradient. Elements sit
in depth — fog, Fresnel rims, layered translucency. The orb lives in a deep dark
space and so does everything around it. A screen that reads as a flat poster has
diverged.

**2. One focal element, vast negative space.**
Restraint *is* the aesthetic. A single hero per view — the orb, a card, a moment
— surrounded by deep space. No button walls, no grid-of-everything. Density is
earned, never the default. Empty space is the design.

**3. Water is the motif.**
The brand is Ocean. The orb is liquid. Motion is fluid — eased, breathing,
organic — never linear or mechanical. Forms are rounded. The motion verbs are
*flow, breathe, ripple, absorb*.

**4. Motion carries meaning, not decoration.**
Nothing moves for show. The orb's motion *is* its state (idle / listening /
thinking / speaking). Progress and state transitions animate; static things stay
still. All motion eases toward a target and never snaps. `prefers-reduced-motion`
always gets a clean static frame.

**5. One accent, used sparingly.**
Cyan (`#7fd1ff`) is the single signature color against the deep blue-black.
Monochromatic discipline. The accent marks the active, the focal, the important
— and almost nothing else. No second accent, no category rainbow.

**6. Typography is structural and quiet.**
One typeface — **Inter, self-hosted** so it is identical on every machine (never
rely on a system install). Hierarchy comes from size, weight, and uppercase
tracked labels — not from color or effects. Labels: uppercase, letter-spaced,
muted. Body text carries the meaning.

**7. Real data makes it alive.**
The UI reflects actual state — sources detected, counts, live status, the orb
intensifying from real input. It is never static decoration. Empty and idle
states are designed deliberately, not left as accidents.

**8. Agent-steered, human-acked.**
The experience is conducted by an agent, not presented as a wall of controls.
One human confirmation where it matters. The UI shows state and progress; it does
not surface every option at once.

**9. No emoji, ever.**
Not in the UI, not in copy, not in code or assets. The iconography is the orb,
glyphs, monograms, and real logos. An emoji appearing is the canary that the
build drifted from this spec — treat it as a bug.

**10. Portable by construction.**
Plain web tech — HTML, CSS, JS, three.js ESM. No build step, no framework
lock-in. The same language ships to browser, Leptos WASM, and Tauri unchanged.
Consistency is enforced by shared tokens and portability, never by re-implementing
the look per surface.

---

## Surface treatment (panels, cards, inputs)

For surfaces that need contained UI (the web surface will), use **glass**:
translucent layered panels, not opaque boxes on a flat field. Depth comes from
transparency and hairline borders, not drop shadows. This extends the
depth/translucency philosophy above — it is prescribed for panel surfaces and
consistent with the language, even though the minimal onboarding has no panels
yet.

---

## Tokens

### Live in the onboarding (proven, rendering now)
| Token | Value |
|---|---|
| Background (radial) | `radial-gradient(circle at 32% 22%, #123a5f 0%, #07111f 42%, #050914 100%)` |
| Canvas base | `#050914` |
| Primary text | `#edf6ff` |
| Accent cyan | `#7fd1ff` |
| Muted / label text | `#6f8aa3` |
| Type | Inter (currently system-fallback — **self-host per spec**) |
| Label treatment | `text-transform: uppercase; letter-spacing: .16em` (hint) / `.44em` (wordmark) |

### Prescribed (spec — implement on every surface; onboarding catches up in Phase 3)
| Token | Value |
|---|---|
| Glass fill | `rgba(8, 20, 36, .82)` |
| Glass border | `1px solid rgba(98, 157, 207, .22)` |
| Glass blur | `backdrop-filter: blur(...)` |
| Card radius | `24px` |
| Focus cyan | `#57b5ff` |
| Pill radius | `999px` |
| Motion | eased (no linear); honor `prefers-reduced-motion` |

---

## Applying it to the web surface

- Consume the shared tokens from the canonical source in `design/`; do not
  re-derive colors per component.
- **Self-host Inter.** Do not assume it is installed on the user's machine.
- Mount `OceanOrb` as the voice-mode element; drive it with `setState`.
- One focal element per screen. Default to negative space; add density only when
  it earns its place.
- Panels are glass with hairline cyan borders — never opaque boxes.
- Cyan is reserved for the active / focal / important. Everything else is the
  deep palette and muted text.
- No emoji. If you reach for one, the build has diverged — stop.
