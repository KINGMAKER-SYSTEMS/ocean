// Ocean ceremony stage - anime.js UI overlays.
//
// Subscribes to the ceremony event bus and renders the agent-steered surface:
// phase narration, detected-item stream (left rail), the single dry-run
// preview card + human unlock beat (bottom dock), and a completion ripple.
// Reduced-motion: every transition collapses to an instant state change.
// Never auto-acks; the unlock beat performs the real POST /api/preview +
// POST /api/preview/ack only on a human click.

import { animate, createTimeline, stagger, utils } from '/assets/vendor/anime.js';

const PHASE_LABELS = {
  boot: 'Boot',
  scan: 'Scanning',
  review: 'Review',
  import: 'Import plan',
  connect: 'Connect',
  preview: 'Preview',
  ack: 'Review and unlock',
  sync: 'Syncing',
  complete: 'Complete',
};

// Detection categories rendered as their own rail group, in display order.
const GROUPS = [
  { key: 'source', label: 'Sources', empty: 'No sources yet' },
  { key: 'skill', label: 'Skills', empty: 'No skills yet' },
  { key: 'project', label: 'Projects', empty: 'No projects yet' },
  { key: 'mcp', label: 'MCP servers', empty: 'No MCP servers yet' },
  { key: 'doc', label: 'Docs', empty: 'No docs yet' },
];

function postJSON(url, body) {
  return fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body || {}),
  }).then(async (res) => {
    const text = await res.text();
    let json = {};
    if (text) { try { json = JSON.parse(text); } catch { json = { ok: false, error: text }; } }
    if (!res.ok) {
      const err = new Error((json && json.error) || `HTTP ${res.status}`);
      err.status = res.status;
      err.body = json;
      throw err;
    }
    return json;
  });
}

function el(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text != null) node.textContent = text;
  return node;
}

export function mount({ on, emit, reduced }) {
  injectStyles();
  const stage = document.getElementById('stage');
  const rail = document.getElementById('rail');
  const dock = document.getElementById('dock');
  const phaseBox = document.getElementById('phase');

  // --- Build static overlay scaffolding -----------------------------------
  const eyebrow = el('div', 'phase-eyebrow');
  const headline = el('div', 'phase-headline', 'Connecting to Ocean');
  const subline = el('div', 'phase-sub');
  phaseBox.append(eyebrow, headline, subline);

  const conn = el('div', 'connection', 'connecting');
  stage.appendChild(conn);

  const toastWrap = el('div', 'toast-wrap');
  stage.appendChild(toastWrap);

  // Rail groups (created lazily on first item, but header order is fixed).
  const groupEls = new Map();
  for (const g of GROUPS) groupEls.set(g.key, makeGroupSkeleton(rail, g));

  // Dock: preview card + the single human unlock beat.
  const previewCard = makePreviewCard();
  const beat = makeUnlockBeat();
  dock.append(previewCard.card, beat.root);
  dock.classList.add('is-hidden');

  let sensitiveCount = 0;
  let previewReady = false;

  // --- Motion helpers (collapse to instant under reduced-motion) ----------
  const reveal = (targets, { y = 12, delay = 0, staggerMs = 0, duration = 460, ease = 'easeOutExpo' } = {}) => {
    if (reduced) { utils.set(targets, { opacity: 1, translateY: 0 }); return null; }
    const d = staggerMs ? stagger(staggerMs, { start: delay }) : delay;
    return animate(targets, { opacity: [0, 1], translateY: [y, 0], duration, delay: d, easing: ease });
  };

  const fadePhase = () => {
    if (reduced) return;
    animate([eyebrow, headline], { opacity: [0.45, 1], duration: 420, delay: stagger(40), easing: 'easeOutQuad' });
  };

  const setPhase = (phase, msg, sub) => {
    eyebrow.textContent = PHASE_LABELS[phase] ? `${PHASE_LABELS[phase]}` : (phase || '');
    if (msg != null) headline.textContent = msg;
    subline.textContent = sub || '';
    fadePhase();
  };

  // --- Connection dot -----------------------------------------------------
  on('__status', (event) => {
    const state = event.status;
    const map = { connecting: 'connecting', connected: 'live', ready: 'live', snapshot: 'live', reconnecting: 'reconnecting', error: 'error', closed: 'idle' };
    conn.textContent = map[state] || state;
    conn.dataset.state = map[state] || state;
  });

  // --- Phase narration ----------------------------------------------------
  on('ceremony.run_started', (e) => setPhase('boot', e.message || 'Onboarding started'));
  on('ceremony.scene_set', (e) => {
    const phase = e.phase || (e.payload && e.payload.scene);
    setPhase(phase, e.message, '');
  });
  on('ceremony.loading_set', (e) => {
    const p = e.payload || {};
    let sub = p.label || '';
    if (typeof p.progress === 'number' && typeof p.total === 'number' && p.total > 0) {
      sub = `${sub} ${p.progress}/${p.total}`.trim();
    }
    if (sub) subline.textContent = sub;
  });
  on('ceremony.toast', (e) => {
    const text = (e.payload && e.payload.text) || e.message;
    if (text) showToast(text);
  });
  on('ceremony.run_completed', (e) => setPhase('complete', e.message || 'Onboarding complete'));

  // --- Detected item stream ----------------------------------------------
  on('agent.source_detected', (e) => addItem('source', e, chipSource));
  on('agent.skill_detected', (e) => addItem('skill', e, chipSkill));
  on('agent.project_detected', (e) => addItem('project', e, chipProject));
  on('agent.mcp_detected', (e) => addItem('mcp', e, chipMcp));
  on('agent.doc_detected', (e) => addItem('doc', e, chipDoc));
  on('agent.session_detected', (e) => addItem('project', e, chipSession, true));

  // Secrets excluded (redacted payload - only label/count-safe keys arrive).
  on('agent.secret_skipped', () => { sensitiveCount += 1; refreshSensitive(); });

  const addItem = (groupKey, event, chipBuilder, appendOnlyMeta) => {
    const group = groupEls.get(groupKey);
    if (!group) return;
    if (group.created === false) revealGroup(group);
    const chip = chipBuilder(event, appendOnlyMeta);
    group.list.appendChild(chip);
    group.count.textContent = String(group.items + 1);
    group.items += 1;
    reveal(chip, { y: 8, duration: 380, ease: 'easeOutQuad' });
  };

  function revealGroup(group) {
    group.created = true;
    group.section.classList.remove('is-pending');
    rail.appendChild(group.section);
    reveal(group.section, { y: 16, duration: 520 });
  }

  // --- Preview + single human unlock beat --------------------------------
  on('ocean.preview_started', (e) => {
    showDock();
    setStat('changed', '-'); setStat('skipped', '-');
    beat.help.textContent = e.message || 'Dry-run preview generating.';
  });

  on('ocean.preview_completed', (e) => {
    showDock();
    applyPreviewPayload(e.payload || {});
    previewReady = true;
    armUnlock();
  });

  on('ocean.ack_required', () => { armUnlock(); });
  on('ceremony.await_user', (e) => {
    if (e.payload && e.payload.gate === 'preview_ack') armUnlock();
  });
  on('ceremony.highlight', (e) => {
    if (e.payload && e.payload.target === 'ack_button') armUnlock();
  });

  on('ocean.ack_observed', () => markUnlocked());
  on('ocean.sync_started', (e) => {
    beat.help.textContent = e.message || 'Syncing into Ocean.';
    beat.label.textContent = 'Sync unlocked';
  });
  on('ocean.sync_completed', () => {
    ripple();
    beat.help.textContent = 'Sync complete.';
  });

  // The real human gate. Never invoked automatically.
  beat.button.addEventListener('click', onUnlockClick);

  async function onUnlockClick() {
    if (beat.button.disabled) return;
    if (beat.root.classList.contains('is-working') || beat.root.classList.contains('is-unlocked')) return;
    beat.root.classList.add('is-working');
    beat.button.disabled = true;
    beat.label.textContent = 'Running dry-run preview';
    try {
      const previewRes = await postJSON('/api/preview', {});
      applyRealTotals(previewRes);
      beat.label.textContent = 'Recording your review';
      const ackRes = await postJSON('/api/preview/ack', {});
      markUnlocked();
      if (!(ackRes && ackRes.state && ackRes.state.syncUnlocked)) {
        beat.help.textContent = 'Review recorded. Sync will continue shortly.';
      }
    } catch (err) {
      beat.root.classList.remove('is-working');
      beat.button.disabled = false;
      beat.label.textContent = 'Review and unlock sync';
      beat.help.textContent = humanizeError(err);
    }
  }

  function armUnlock() {
    if (beat.root.classList.contains('is-unlocked')) return;
    if (previewReady) {
      beat.button.disabled = false;
      beat.root.classList.remove('is-hidden');
    }
  }

  function markUnlocked() {
    beat.root.classList.remove('is-working');
    beat.root.classList.add('is-unlocked');
    beat.button.disabled = true;
    beat.label.textContent = 'Sync unlocked';
    beat.help.textContent = 'You reviewed the preview. Sync is unlocked.';
  }

  function showDock() {
    if (!dock.classList.contains('is-hidden')) return;
    dock.classList.remove('is-hidden');
    reveal(dock, { y: 22, duration: 540 });
  }

  function applyPreviewPayload(payload) {
    const totals = payload.totals || {};
    const changed = numberOr(totals.changed, payload.changed, payload.queued);
    const skipped = numberOr(totals.skipped, payload.skipped);
    const sensitive = numberOr(totals.sensitive, payload.sensitive, sensitiveCount);
    setStat('changed', changed);
    setStat('skipped', skipped);
    setStat('sensitive', sensitive);
    beat.help.textContent = 'Review the preview, then unlock sync.';
  }

  // Pull truthful numbers from the real dry-run preview result.
  function applyRealTotals(previewRes) {
    const totals = previewRes && previewRes.preview && previewRes.preview.result && previewRes.preview.result.totals;
    if (!totals) return;
    setStat('changed', numberOr(totals.changed));
    setStat('skipped', numberOr(totals.skipped));
    setStat('sensitive', numberOr(totals.sensitive, sensitiveCount));
  }

  function refreshSensitive() {
    if (previewReady) setStat('sensitive', numberOr(null, sensitiveCount));
  }

  function setStat(name, value) {
    const node = previewCard.stats[name];
    if (!node) return;
    node.textContent = String(value);
    if (!reduced) animate(node, { opacity: [0.3, 1], duration: 360, easing: 'easeOutQuad' });
  }

  // --- Toast + completion ripple -----------------------------------------
  function showToast(text) {
    const node = el('div', 'toast', text);
    toastWrap.appendChild(node);
    reveal(node, { y: 10, duration: 420 });
    const remove = () => {
      if (reduced) { node.remove(); return; }
      animate(node, { opacity: [1, 0], translateY: [0, -8], duration: 420, easing: 'easeInQuad', onComplete: () => node.remove() });
    };
    setTimeout(remove, 4600);
  }

  function ripple() {
    if (reduced) {
      const node = el('div', 'ripple is-done');
      stage.appendChild(node);
      setTimeout(() => node.remove(), 1400);
      return;
    }
    const node = el('div', 'ripple');
    stage.appendChild(node);
    const tl = createTimeline({ defaults: { easing: 'easeOutExpo' } });
    tl.add(node, { scale: [0, 1], opacity: [0.55, 0], duration: 1700 });
    tl.add(node, { opacity: 0, duration: 200 }, '-=200');
    setTimeout(() => node.remove(), 2000);
  }

  // Initial phase.
  setPhase('boot', 'Connecting to Ocean', '');
}

// --- Chip builders ---------------------------------------------------------
function chipSource(event) {
  const p = event.payload || {};
  const li = el('li', 'chip');
  const name = el('span', 'chip-name', p.label || event.message || 'source');
  const meta = el('span', 'chip-meta', p.root || '');
  li.append(name, meta);
  if (p.counts && typeof p.counts === 'object') {
    const parts = [];
    if (p.counts.agents) parts.push(`${p.counts.agents} agents`);
    if (p.counts.skills) parts.push(`${p.counts.skills} skills`);
    if (p.counts.projects) parts.push(`${p.counts.projects} projects`);
    if (p.counts.mcpServers) parts.push(`${p.counts.mcpServers} MCP`);
    if (parts.length) li.append(el('span', 'chip-counts', parts.join(' / ')));
  }
  return li;
}

function chipSkill(event) {
  const p = event.payload || {};
  const li = el('li', 'chip');
  li.append(el('span', 'chip-name', p.name || event.message || 'skill'), el('span', 'chip-meta', p.sourceId || ''));
  return li;
}

function chipProject(event) {
  const p = event.payload || {};
  const li = el('li', 'chip');
  const meta = p.root || (p.configDocs && p.configDocs.join(', ')) || '';
  li.append(el('span', 'chip-name', p.name || event.message || 'project'), el('span', 'chip-meta', meta));
  if (typeof p.sessions === 'number') li.append(el('span', 'chip-counts', `${p.sessions} sessions`));
  return li;
}

function chipMcp(event) {
  const p = event.payload || {};
  const li = el('li', 'chip');
  const meta = [p.transport, p.scope].filter(Boolean).join(' / ');
  li.append(el('span', 'chip-name', p.name || event.message || 'mcp'), el('span', 'chip-meta', meta));
  return li;
}

function chipDoc(event) {
  const p = event.payload || {};
  const li = el('li', 'chip');
  li.append(el('span', 'chip-name', p.name || event.message || 'doc'), el('span', 'chip-meta', p.relPath || ''));
  return li;
}

function chipSession(event) {
  const p = event.payload || {};
  const li = el('li', 'chip');
  li.append(el('span', 'chip-name', p.project || 'session'), el('span', 'chip-meta', `${p.count != null ? p.count : 0} sessions`));
  return li;
}

// --- Skeleton builders -----------------------------------------------------
function makeGroupSkeleton(rail, group) {
  const section = el('section', 'card group is-pending');
  section.dataset.group = group.key;
  const head = el('header', 'group-head');
  const label = el('span', 'group-label', group.label);
  const count = el('span', 'group-count', '0');
  head.append(label, count);
  const list = el('ul', 'group-items');
  section.append(head, list);
  return { section, head, label, count, list, items: 0, created: false };
}

function makePreviewCard() {
  const card = el('div', 'card preview');
  const head = el('div', 'preview-head', 'Dry-run preview');
  const stats = el('div', 'preview-stats');
  const statsByName = {};
  for (const s of [
    { key: 'changed', label: 'Changed' },
    { key: 'skipped', label: 'Skipped' },
    { key: 'sensitive', label: 'Sensitive excluded' },
  ]) {
    const wrap = el('div', 'stat');
    const num = el('span', 'stat-num', '-');
    num.dataset.stat = s.key;
    const lab = el('span', 'stat-label', s.label);
    wrap.append(num, lab);
    stats.appendChild(wrap);
    statsByName[s.key] = num;
  }
  card.append(head, stats);
  return { card, stats: statsByName };
}

function makeUnlockBeat() {
  const root = el('div', 'beat');
  const button = el('button', 'btn btn-unlock');
  button.type = 'button';
  button.disabled = true;
  const label = el('span', 'btn-label', 'Review and unlock sync');
  button.append(label);
  const help = el('p', 'beat-help', 'Nothing syncs until you click. Auth files and secrets are excluded.');
  root.append(button, help);
  return { root, button, label, help };
}

function numberOr(...candidates) {
  for (const c of candidates) {
    if (typeof c === 'number' && Number.isFinite(c)) return c;
  }
  return 0;
}

function humanizeError(err) {
  const msg = (err && err.body && err.body.code === 'PREVIEW_ACK_REQUIRED' && 'Run the preview first, then unlock.')
    || (err && err.body && err.body.code === 'FRESH_PREVIEW_REQUIRED' && 'Folders changed. Reopen the preview.')
    || (err && err.message) || 'Unlock failed. Try again.';
  return msg;
}

// --- Styles (injected, co-located with the component) ---------------------
function injectStyles() {
  if (document.getElementById('ocean-ui-styles')) return;
  const style = el('style');
  style.id = 'ocean-ui-styles';
  style.textContent = `
#stage { color: var(--ocean-text); }
.phase {
  position: absolute; top: 28px; left: 50%; transform: translateX(-50%);
  text-align: center; pointer-events: none; max-width: min(620px, 80vw);
}
.phase-eyebrow {
  font-size: 11px; letter-spacing: 0.34em; text-transform: uppercase;
  color: var(--ocean-accent); opacity: 0.82; margin-bottom: 6px;
}
.phase-headline {
  font-size: 18px; font-weight: 600; letter-spacing: 0.01em; color: var(--ocean-text);
  text-shadow: 0 1px 24px rgba(0,0,0,.5);
}
.phase-sub {
  margin-top: 6px; font-size: 12px; letter-spacing: 0.12em; text-transform: uppercase;
  color: var(--ocean-text-muted); opacity: 0.8; min-height: 1em;
}
.connection {
  position: absolute; top: 30px; right: 30px;
  font-size: 10px; letter-spacing: 0.28em; text-transform: uppercase;
  color: var(--ocean-text-muted); opacity: 0.8; pointer-events: none;
}
.connection::before {
  content: ""; display: inline-block; width: 7px; height: 7px; border-radius: 999px;
  margin-right: 8px; vertical-align: 1px; background: var(--ocean-text-muted);
}
.connection[data-state="live"]::before { background: var(--ocean-accent); box-shadow: 0 0 10px var(--ocean-accent); }
.connection[data-state="reconnecting"]::before { background: #d8b35a; }
.connection[data-state="error"]::before { background: #e07a6a; }
.rail {
  position: absolute; left: 28px; top: 92px; bottom: 150px; width: 312px;
  display: flex; flex-direction: column; gap: 14px;
  overflow-y: auto; padding-right: 6px; pointer-events: auto;
  scrollbar-width: thin; scrollbar-color: rgba(98,157,207,.35) transparent;
}
.rail::-webkit-scrollbar { width: 6px; }
.rail::-webkit-scrollbar-thumb { background: rgba(98,157,207,.35); border-radius: 999px; }
.card {
  background: var(--ocean-glass-fill);
  -webkit-backdrop-filter: blur(var(--ocean-glass-blur)); backdrop-filter: blur(var(--ocean-glass-blur));
  border: 1px solid var(--ocean-glass-border);
  border-radius: var(--ocean-radius-card); padding: 16px 18px;
  box-shadow: 0 18px 50px -28px rgba(0,0,0,.85), inset 0 1px 0 rgba(160,210,255,.06);
}
.group.is-pending { display: none; }
.group-head {
  display: flex; align-items: baseline; justify-content: space-between;
  margin-bottom: 10px;
}
.group-label { font-size: 11px; letter-spacing: var(--ocean-track-label); text-transform: uppercase; color: var(--ocean-text-muted); }
.group-count {
  font-size: 12px; font-weight: 600; color: #cfe6ff;
  background: rgba(127,209,255,.12); border: 1px solid rgba(127,209,255,.22);
  border-radius: 999px; padding: 1px 10px;
}
.group-items { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
.chip {
  display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 10px;
  padding: 7px 12px; border-radius: 999px;
  background: rgba(20,42,66,.55); border: 1px solid rgba(98,157,207,.16);
  font-size: 13px; line-height: 1.25;
}
.chip-name { color: var(--ocean-text); font-weight: 500; }
.chip-meta { color: var(--ocean-text-muted); font-size: 11px; letter-spacing: 0.04em; word-break: break-all; }
.chip-counts { width: 100%; color: var(--ocean-text-muted); font-size: 10px; letter-spacing: 0.12em; text-transform: uppercase; }
.dock {
  position: absolute; left: 50%; bottom: 34px; transform: translateX(-50%);
  width: min(580px, 92vw); display: flex; flex-direction: column; align-items: center; gap: 14px;
  pointer-events: auto;
}
.dock.is-hidden { display: none; }
.preview { width: 100%; }
.preview-head {
  font-size: 11px; letter-spacing: var(--ocean-track-label); text-transform: uppercase; color: var(--ocean-text-muted);
  margin-bottom: 12px;
}
.preview-stats { display: flex; gap: 10px; }
.preview-stats .stat {
  flex: 1; text-align: center; padding: 12px 8px;
  background: rgba(20,42,66,.5); border: 1px solid rgba(98,157,207,.16);
  border-radius: 18px;
}
.stat-num { display: block; font-size: 26px; font-weight: 700; color: var(--ocean-text); }
.stat-label {
  display: block; margin-top: 4px; font-size: 10px; letter-spacing: 0.18em;
  text-transform: uppercase; color: var(--ocean-text-muted);
}
.beat { width: 100%; display: flex; flex-direction: column; align-items: center; gap: 8px; }
.btn-unlock {
  font-family: inherit; cursor: pointer; pointer-events: auto;
  border-radius: var(--ocean-radius-pill); border: 1px solid rgba(127,209,255,.55);
  background: linear-gradient(180deg, rgba(127,209,255,.96), rgba(87,170,240,.96));
  color: #04121f; font-weight: 700; font-size: 14px; letter-spacing: 0.06em;
  padding: 14px 30px; transition: transform .12s ease, box-shadow .2s ease, opacity .2s ease;
  box-shadow: 0 14px 40px -16px rgba(127,209,255,.7);
}
.btn-unlock:hover:not(:disabled) { transform: translateY(-1px); }
.btn-unlock:focus-visible { outline: 2px solid var(--ocean-focus); outline-offset: 3px; }
.btn-unlock:disabled { cursor: default; opacity: .5; }
.beat.is-working .btn-unlock { opacity: .85; }
.beat.is-unlocked .btn-unlock {
  background: rgba(20,42,66,.6); color: var(--ocean-accent); border-color: rgba(127,209,255,.35);
  box-shadow: none;
}
.beat-help {
  margin: 0; max-width: 460px; text-align: center;
  font-size: 12px; line-height: 1.5; color: var(--ocean-text-muted); opacity: .9;
}
.toast-wrap {
  position: absolute; left: 50%; bottom: 220px; transform: translateX(-50%);
  display: flex; flex-direction: column; align-items: center; gap: 8px;
  pointer-events: none; width: min(520px, 90vw);
}
.toast {
  background: var(--ocean-glass-fill); -webkit-backdrop-filter: blur(var(--ocean-glass-blur)); backdrop-filter: blur(var(--ocean-glass-blur));
  border: 1px solid var(--ocean-glass-border); border-radius: var(--ocean-radius-pill);
  padding: 8px 18px; font-size: 12px; color: var(--ocean-text); text-align: center;
  letter-spacing: 0.04em;
}
.ripple {
  position: absolute; left: 50%; top: 50%; width: 120px; height: 120px;
  margin: -60px 0 0 -60px; border-radius: 999px; pointer-events: none;
  border: 2px solid rgba(127,209,255,.6);
  box-shadow: 0 0 60px rgba(127,209,255,.35), inset 0 0 40px rgba(127,209,255,.2);
  transform-origin: center;
}
.ripple.is-done { animation: none; opacity: .5; }
@media (prefers-reduced-motion: reduce) {
  .btn-unlock { transition: none; }
}
@media (max-width: 760px) {
  .rail { width: calc(100vw - 56px); top: auto; bottom: 260px; max-height: 38vh; }
  .phase { top: 18px; }
}
`;
  document.head.appendChild(style);
}
