// OceanOrb — Ocean's voice-mode presence element (canonical module).
//
// A noise-displaced liquid sphere (water shader) whose intensity, color and
// motion ease toward a voice-state profile: idle / listening / thinking /
// speaking. Framework-agnostic plain JS + three.js; embeddable in any webview
// (browser, Leptos WASM, Tauri). No emoji, no external assets, no build step.
//
// This is the single source of truth for the orb. Surface-specific mounts
// (e.g. ui/onboarding/ocean-three.js) import OceanOrb and wire it to their DOM;
// they do not re-implement it.
//
// Public API:
//   new OceanOrb(canvas, { reduced }?)   construct; reduced forces a static frame
//   .start()                             begin the render loop (idempotent)
//   .dispose()                           stop loop + free GPU resources
//   .setState(name)                      'idle' | 'listening' | 'thinking' | 'speaking'
//   .setIntensity(v)                     manual frequency override (loop eases toward v)
//   .setParallax(x, y)                   normalized screen-units camera nudge
//   .absorbAsset(url)                    fly a logo image into the orb + ripple it
//
// Import-map requirement: three is imported via the bare specifier 'three'.
// The consuming document MUST include an import map, e.g.
//   <script type="importmap">
//     { "imports": { "three": "/assets/vendor/three.module.min.js" } }
//   </script>
//
// State language (tune freely — these are just reasoned defaults):
//   idle       calm deep breath, baseline cyan rim
//   listening  rim lifts + brightens, surface a touch faster  (receptive)
//   thinking   high intensity churn, electric cyan, fast      (processing)
//   speaking   intensity pulses with a cadence                (talking back)
import * as THREE from 'three';

// Voice-state profiles. The render loop eases uniforms toward the active one.
const STATES = {
  idle:      { intensity: 6,  speed: 1.0, fresnel: 3.0, deep: [0.018, 0.055, 0.105], cyan: [0.30, 0.72, 1.00] },
  listening: { intensity: 9,  speed: 1.35, fresnel: 2.5, deep: [0.030, 0.080, 0.140], cyan: [0.42, 0.82, 1.00] },
  thinking:  { intensity: 16, speed: 1.8, fresnel: 2.2, deep: [0.020, 0.060, 0.120], cyan: [0.48, 0.86, 1.00] },
  speaking:  { intensity: 12, speed: 1.4, fresnel: 2.4, deep: [0.025, 0.070, 0.130], cyan: [0.40, 0.80, 1.00], pulse: 4.0 }
};

const vertexShader = /* glsl */ `
  uniform float u_time;
  uniform float u_frequency;
  uniform float u_speed;
  uniform float u_ripple;
  varying vec3 v_normal;
  varying vec3 v_viewPos;
  varying float v_height;
  // Periodic Perlin noise (Stefan Gustavson).
  vec3 mod289(vec3 x){ return x - floor(x * (1.0 / 289.0)) * 289.0; }
  vec4 mod289(vec4 x){ return x - floor(x * (1.0 / 289.0)) * 289.0; }
  vec4 permute(vec4 x){ return mod289(((x*34.0)+10.0)*x); }
  vec4 taylorInvSqrt(vec4 r){ return 1.79284291400159 - 0.85373472095314 * r; }
  vec3 fade(vec3 t){ return t*t*t*(t*(t*6.0-15.0)+10.0); }
  float pnoise(vec3 P, vec3 rep){
    vec3 Pi0 = mod(floor(P), rep); vec3 Pi1 = mod(Pi0 + vec3(1.0), rep);
    Pi0 = mod289(Pi0); Pi1 = mod289(Pi1);
    vec3 Pf0 = fract(P); vec3 Pf1 = Pf0 - vec3(1.0);
    vec4 ix = vec4(Pi0.x, Pi1.x, Pi0.x, Pi1.x);
    vec4 iy = vec4(Pi0.yy, Pi1.yy);
    vec4 iz0 = Pi0.zzzz; vec4 iz1 = Pi1.zzzz;
    vec4 ixy = permute(permute(ix) + iy);
    vec4 ixy0 = permute(ixy + iz0); vec4 ixy1 = permute(ixy + iz1);
    vec4 gx0 = ixy0 * (1.0 / 7.0);
    vec4 gy0 = fract(floor(gx0) * (1.0 / 7.0)) - 0.5; gx0 = fract(gx0);
    vec4 gz0 = vec4(0.5) - abs(gx0) - abs(gy0);
    vec4 sz0 = step(gz0, vec4(0.0));
    gx0 -= sz0 * (step(0.0, gx0) - 0.5); gy0 -= sz0 * (step(0.0, gy0) - 0.5);
    vec4 gx1 = ixy1 * (1.0 / 7.0);
    vec4 gy1 = fract(floor(gx1) * (1.0 / 7.0)) - 0.5; gx1 = fract(gx1);
    vec4 gz1 = vec4(0.5) - abs(gx1) - abs(gy1);
    vec4 sz1 = step(gz1, vec4(0.0));
    gx1 -= sz1 * (step(0.0, gx1) - 0.5); gy1 -= sz1 * (step(0.0, gy1) - 0.5);
    vec3 g000 = vec3(gx0.x,gy0.x,gz0.x); vec3 g100 = vec3(gx0.y,gy0.y,gz0.y);
    vec3 g010 = vec3(gx0.z,gy0.z,gz0.z); vec3 g110 = vec3(gx0.w,gy0.w,gz0.w);
    vec3 g001 = vec3(gx1.x,gy1.x,gz1.x); vec3 g101 = vec3(gx1.y,gy1.y,gz1.y);
    vec3 g011 = vec3(gx1.z,gy1.z,gz1.z); vec3 g111 = vec3(gx1.w,gy1.w,gz1.w);
    vec4 norm0 = taylorInvSqrt(vec4(dot(g000,g000), dot(g010,g010), dot(g100,g100), dot(g110,g110)));
    g000 *= norm0.x; g010 *= norm0.y; g100 *= norm0.z; g110 *= norm0.w;
    vec4 norm1 = taylorInvSqrt(vec4(dot(g001,g001), dot(g011,g011), dot(g101,g101), dot(g111,g111)));
    g001 *= norm1.x; g011 *= norm1.y; g101 *= norm1.z; g111 *= norm1.w;
    float n000 = dot(g000, Pf0);
    float n100 = dot(g100, vec3(Pf1.x, Pf0.yz));
    float n010 = dot(g010, vec3(Pf0.x, Pf1.y, Pf0.z));
    float n110 = dot(g110, vec3(Pf1.xy, Pf0.z));
    float n001 = dot(g001, vec3(Pf0.xy, Pf1.z));
    float n101 = dot(g101, vec3(Pf1.x, Pf0.y, Pf1.z));
    float n011 = dot(g011, vec3(Pf0.x, Pf1.yz));
    float n111 = dot(g111, Pf1);
    vec3 f = fade(Pf0);
    vec4 nz = mix(vec4(n000,n100,n010,n110), vec4(n001,n101,n011,n111), f.z);
    vec2 nyz = mix(nz.xy, nz.zw, f.y);
    return 2.2 * mix(nyz.x, nyz.y, f.x);
  }
  float surf(vec3 p){
    float n = pnoise(p * 0.5 + u_time * 0.12 * u_speed, vec3(6.0));
    return 0.22 * n + (u_frequency / 30.0) * (n / 5.0) + u_ripple * 0.12 * pnoise(p * 3.0 + u_time * 2.0, vec3(6.0));
  }
  void main(){
    float d = surf(position);
    vec3 displaced = position + normal * d;
    float eps = 0.20;
    vec3 ax = abs(normal.y) < 0.99 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
    vec3 t1 = normalize(cross(normal, ax));
    vec3 t2 = normalize(cross(normal, t1));
    vec3 a = (position + t1 * eps) + normal * surf(position + t1 * eps);
    vec3 b = (position + t2 * eps) + normal * surf(position + t2 * eps);
    vec3 nrm = normalize(cross(a - displaced, b - displaced));
    if (dot(nrm, normal) < 0.0) nrm = -nrm;
    vec4 mv = modelViewMatrix * vec4(displaced, 1.0);
    v_viewPos = mv.xyz;
    v_normal = normalize(normalMatrix * nrm);
    v_height = d;
    gl_Position = projectionMatrix * mv;
  }
`;

const fragmentShader = /* glsl */ `
  uniform vec3 u_deep;
  uniform vec3 u_cyan;
  uniform float u_fresnel;
  uniform float u_ripple;
  varying vec3 v_normal;
  varying vec3 v_viewPos;
  varying float v_height;
  void main(){
    vec3 N = normalize(v_normal);
    vec3 V = normalize(-v_viewPos);
    vec3 L = normalize(vec3(-0.5, 0.75, 0.6));
    vec3 H = normalize(L + V);
    float fres = pow(1.0 - max(dot(N, V), 0.0), u_fresnel);
    float spec = pow(max(dot(N, H), 0.0), 90.0);
    vec3 col = mix(u_deep, u_cyan, fres);
    col += u_cyan * max(v_height, 0.0) * 0.6;
    col += vec3(0.9, 0.97, 1.0) * spec;
    col = mix(col, u_cyan, u_ripple * 0.30);
    gl_FragColor = vec4(col, 1.0);
  }
`;

export class OceanOrb {
  constructor(canvas, opts = {}){
    this.canvas = canvas;
    this.reduced = opts.reduced ?? window.matchMedia('(prefers-reduced-motion: reduce)').matches;

    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true, preserveDrawingBuffer: true });
    this.renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    this.renderer.setSize(canvas.clientWidth || innerWidth, canvas.clientHeight || innerHeight);

    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(42, this._aspect(), 0.1, 100);
    this.camera.position.set(0, 0, 15);
    this.camera.lookAt(0, 0, 0);

    this.uniforms = {
      u_time:       { value: 0 },
      u_frequency:  { value: 0 },
      u_speed:      { value: 1.0 },
      u_deep:       { value: new THREE.Vector3(...STATES.idle.deep) },
      u_cyan:       { value: new THREE.Vector3(...STATES.idle.cyan) },
      u_fresnel:    { value: STATES.idle.fresnel },
      u_ripple:     { value: 0 },
    };

    const mat = new THREE.ShaderMaterial({ uniforms: this.uniforms, vertexShader, fragmentShader });
    this.mesh = new THREE.Mesh(new THREE.IcosahedronGeometry(4, 24), mat);
    this.scene.add(this.mesh);

    this.clock = new THREE.Clock();
    this._target = { ...STATES.idle, name: 'idle' };
    this._intensityTarget = STATES.idle.intensity;
    this._px = 0; this._py = 0;        // parallax target
    this._absorbing = [];
    this._boost = 0;
    this._running = false;
  }

  _aspect(){ const w = this.canvas.clientWidth || innerWidth, h = this.canvas.clientHeight || innerHeight; return w / h; }

  start(){
    if (this._running) return;
    this._running = true;
    if (this.reduced){ this.renderer.render(this.scene, this.camera); return; }
    this._resize();
    addEventListener('resize', this._resize);
    requestAnimationFrame(this._loop);
    this.renderer.render(this.scene, this.camera);
  }

  _resize = () => {
    this.camera.aspect = this._aspect();
    this.camera.updateProjectionMatrix();
    this.renderer.setSize(this.canvas.clientWidth || innerWidth, this.canvas.clientHeight || innerHeight);
  };

  // Drive the orb with a named voice state (see STATES).
  setState(name){
    const s = STATES[name]; if (!s) return;
    this._target = { ...s, name };
    this._intensityTarget = s.intensity;
  }

  // Manual intensity override (e.g. ceremony source count). Snaps the target;
  // the loop eases the actual uniform.
  setIntensity(v){ this._intensityTarget = v; }

  // Parallax target in normalized screen units (onboarding wires mousemove).
  setParallax(x, y){ this._px = x; this._py = y; }

  // Phase 2: fly a logo/textured plane into the orb and dissolve it.
  // API present now so callers can wire it; visual lands in the absorption pass.
  absorbAsset(url){ if (!url) return; this._loadLogo(url).then(tex => this._spawn(tex)).catch(() => {}); }

  _loop = () => {
    if (!this._running) return;
    const t = this.clock.getElapsedTime();
    const u = this.uniforms;
    u.u_time.value = t;
    const tgt = this._target;
    // Speaking = cadence pulse around the state's intensity baseline.
    const pulse = tgt.pulse ? Math.sin(t * 6.0) * tgt.pulse : 0;
    const intensityTarget = this._intensityTarget + pulse + this._boost;
    u.u_frequency.value += (intensityTarget - u.u_frequency.value) * 0.06;
    u.u_speed.value    += (tgt.speed    - u.u_speed.value)    * 0.05;
    u.u_fresnel.value  += (tgt.fresnel  - u.u_fresnel.value)  * 0.05;
    u.u_deep.value.lerp(new THREE.Vector3(...tgt.deep), 0.05);
    u.u_cyan.value.lerp(new THREE.Vector3(...tgt.cyan), 0.05);
    u.u_ripple.value *= 0.94;
    this._boost *= 0.94;
    this._updateAbsorbing(t);
    this.mesh.rotation.y += 0.0014 * tgt.speed;
    this.mesh.rotation.x = Math.sin(t * 0.15) * 0.08;
    this.camera.position.x += (this._px - this.camera.position.x) * 0.04;
    this.camera.position.y += (-this._py - this.camera.position.y) * 0.04;
    this.camera.lookAt(this.scene.position);
    this.renderer.render(this.scene, this.camera);
    requestAnimationFrame(this._loop);
  }

  _loadLogo(url){
    return new Promise((resolve, reject) => {
      const img = new Image();
      img.onload = () => {
        const S = 256;
        const c = document.createElement('canvas'); c.width = S; c.height = S;
        const ctx = c.getContext('2d');
        const ar = (img.width || 1) / (img.height || 1);
        let w = S, h = S;
        if (ar > 1) h = S / ar; else w = S * ar;
        ctx.drawImage(img, (S - w) / 2, (S - h) / 2, w, h);
        const tex = new THREE.CanvasTexture(c);
        tex.colorSpace = THREE.SRGBColorSpace;
        resolve(tex);
      };
      img.onerror = reject;
      img.src = url;
    });
  }
  _spawn(tex){
    const mat = new THREE.SpriteMaterial({ map: tex, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending, opacity: 0.95 });
    const sp = new THREE.Sprite(mat);
    const dir = new THREE.Vector3(Math.random() - 0.5, Math.random() - 0.5, Math.random() - 0.5).normalize();
    sp.position.copy(dir.multiplyScalar(9));
    sp.scale.set(1.5, 1.5, 1);
    this.scene.add(sp);
    this._absorbing.push({ sprite: sp, start: sp.position.clone(), t0: this.clock.getElapsedTime(), dur: 1.3 });
  }
  _updateAbsorbing(t){
    const list = this._absorbing;
    if (!list.length) return;
    for (let i = list.length - 1; i >= 0; i--){
      const item = list[i];
      const k = Math.max(0, Math.min(1, (t - item.t0) / item.dur));
      if (k >= 1){
        this.uniforms.u_ripple.value = 1.0;
        this._boost += 4;
        this.scene.remove(item.sprite);
        item.sprite.material.map?.dispose();
        item.sprite.material.dispose();
        list.splice(i, 1);
        continue;
      }
      const e = k * (2 - k);
      const target = item.start.clone().normalize().multiplyScalar(4.2);
      item.sprite.position.lerpVectors(item.start, target, e);
      item.sprite.material.opacity = 0.95 * (1 - Math.max(0, (k - 0.65) / 0.35));
    }
  }
  dispose(){
    this._running = false;
    removeEventListener('resize', this._resize);
    this.mesh.geometry.dispose();
    this.mesh.material.dispose();
    this.renderer.dispose();
  }
}
