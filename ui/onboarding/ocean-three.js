// Onboarding mount for OceanOrb — thin facade over the canonical module.
//
// The orb itself (GLSL shaders, voice-state profiles, the OceanOrb class) lives
// in /design/orb/ocean-orb.js. This file owns only the onboarding-specific
// concerns: mounting one OceanOrb instance on #ocean, mousemove parallax wiring,
// and the window.oceanOrb exposure. ocean-app.js imports { init, setIntensity,
// absorbAsset } from here and is unchanged. The module auto-mounts on import.
import { OceanOrb } from '/design/orb/ocean-orb.js';

// ---- Onboarding mount (backward-compatible with ocean-app.js) ----
let orb;
export function init(){
  const canvas = document.getElementById('ocean');
  orb = new OceanOrb(canvas);
  if (!orb.reduced){
    addEventListener('mousemove', e => {
      orb.setParallax((e.clientX - innerWidth / 2) / 220, (e.clientY - innerHeight / 2) / 220);
    });
  }
  orb.start();
  if (typeof window !== 'undefined') window.oceanOrb = orb;
}
export function setState(name){ orb?.setState(name); }
export function setIntensity(v){ orb?.setIntensity(v); }
export function absorbAsset(url){ orb?.absorbAsset(url); }

export { OceanOrb };

if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
else init();
