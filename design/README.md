# Ocean Design System

The canonical source for how every Ocean surface looks and moves. Consumed by
the ocean-bedrock local app today; intended to be vendored or imported by
ocean-surface (Leptos/WASM) and the Tauri desktop app so all surfaces read as
one product.

Read `DESIGN-PRINCIPLES.md` first. It is the design contract; everything in
this directory implements it.

## Contents

- `DESIGN-PRINCIPLES.md` - the design language: ethos, ten principles, token
  table. The one document to hand another surface's agent.
- `tokens.css` - the canonical CSS custom properties (`--ocean-*`) plus the
  self-hosted InterVariable `@font-face`. Link this BEFORE any surface
  stylesheet; never re-derive colors, radii, blur, or type per component.
- `tokens.json` - the same values as flat keys for non-CSS consumers
  (native shells, build scripts, canvas renderers).
- `fonts/InterVariable.woff2` - Inter variable (weights 100-900), self-hosted
  so typography is identical on every machine. `fonts/LICENSE.txt` carries the
  SIL OFL 1.1 notice.
- `orb/ocean-orb.js` - the OceanOrb class: Ocean's voice-mode presence
  element. A noise-displaced water-surface sphere (three.js) with voice-state
  profiles and logo absorption.

## Using the orb

The module imports three.js via the bare specifier `three`; the consuming page
must provide an import map (or bundler resolution):

```html
<script type="importmap">{ "imports": { "three": "/assets/vendor/three.module.min.js" } }</script>
```

```js
import { OceanOrb } from '/design/orb/ocean-orb.js';

const orb = new OceanOrb(document.getElementById('canvas'));
orb.start();
orb.setState('idle');        // idle | listening | thinking | speaking
orb.setIntensity(16);        // override the intensity target directly
orb.setParallax(x, y);       // pointer parallax, unit range
orb.absorbAsset('/assets/integrations/claude.svg'); // fly a mark into the orb
orb.dispose();
```

Each state is an intensity + color + motion-speed profile the render loop
eases toward. `prefers-reduced-motion` renders a single static frame.

## Serving

The local app (`scripts/ocean-local-app.mjs`) serves this directory at
`/design/` with path containment. A vendoring surface copies the whole
`design/` folder; relative paths inside it keep working.

## Rules

- No emoji anywhere. An emoji is the canary that a build diverged.
- No build step: plain CSS, plain ESM, vendored libraries only.
- One focal element per view; glass panels, never opaque boxes; cyan is
  reserved for the active and important.
