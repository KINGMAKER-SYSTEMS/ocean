import path from 'node:path';
import { promises as fs } from 'node:fs';
import { recordObjectWrite } from './metadata.mjs';

export async function notifyTriage(state, { day, slot, findings, dbCounts }, principal = {}) {
  const actionable = findings.filter((f) => f.severity === 'high' || f.severity === 'medium');
  if (!actionable.length) return;

  await writeDigestFile(state, { day, slot, findings: actionable, dbCounts }, principal);

  if (process.env.OCEAN_TRIAGE_TELEGRAM_BOT_TOKEN && process.env.OCEAN_TRIAGE_TELEGRAM_CHAT_ID) {
    await notifyTelegram({ day, slot, findings: actionable }).catch((err) => {
      console.error('[notify] telegram failed (non-fatal):', err.message);
    });
  }

  if (process.env.OCEAN_TRIAGE_WEBHOOK_URL) {
    await notifyWebhook({ day, slot, findings: actionable, dbCounts }).catch((err) => {
      console.error('[notify] webhook failed (non-fatal):', err.message);
    });
  }
}

async function writeDigestFile(state, { day, slot, findings, dbCounts }, principal) {
  const digestPath = `/context/ocean-bedrock/triage/_alerts/${day}-${slot}.md`;
  const content = renderDigest({ day, slot, findings, dbCounts });
  try {
    const diskPath = path.join(state.filesRoot, digestPath.replace(/^\//, ''));
    await fs.mkdir(path.dirname(diskPath), { recursive: true });
    await fs.writeFile(diskPath, content, 'utf8');
    const stat = await fs.stat(diskPath);
    await recordObjectWrite(
      state,
      digestPath,
      { type: 'file', size: stat.size, etag: `"${stat.size.toString(16)}-${Math.floor(stat.mtimeMs).toString(16)}"` },
      principal,
    );
  } catch (err) {
    console.error('[notify] digest write failed (non-fatal):', err.message);
  }
}

function renderDigest({ day, slot, findings, dbCounts }) {
  const lines = [
    `# Ocean Triage Alert — ${day} slot ${slot}`,
    '',
    '## Findings',
    '',
    ...findings.map((f) => `- **${f.severity.toUpperCase()}** [${f.area}] ${f.message}`),
  ];
  if (dbCounts) {
    lines.push('', '## Counts', '', '| metric | value |', '|---|---|');
    for (const [k, v] of Object.entries(dbCounts)) {
      lines.push(`| ${k} | ${v} |`);
    }
  }
  return `${lines.join('\n')}\n`;
}

async function notifyTelegram({ day, slot, findings }) {
  const token = process.env.OCEAN_TRIAGE_TELEGRAM_BOT_TOKEN;
  const chatId = process.env.OCEAN_TRIAGE_TELEGRAM_CHAT_ID;
  const text = `Ocean Triage ${day}/${slot}\n${findings.map((f) => `• ${f.severity.toUpperCase()} [${f.area}] ${f.message}`).join('\n')}`;
  const resp = await fetch(`https://api.telegram.org/bot${token}/sendMessage`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ chat_id: chatId, text }),
  });
  if (!resp.ok) throw new Error(`Telegram API ${resp.status}`);
}

async function notifyWebhook({ day, slot, findings, dbCounts }) {
  const url = process.env.OCEAN_TRIAGE_WEBHOOK_URL;
  const resp = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ day, slot, findings, dbCounts }),
  });
  if (!resp.ok) throw new Error(`Webhook ${resp.status}`);
}
