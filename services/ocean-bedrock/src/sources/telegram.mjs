import { SourceRunner, joinApiPath } from './runner.mjs';
import { resolveSecret } from './secrets.mjs';

const API_BASE = 'https://api.telegram.org';
const DEFAULT_LIMIT = Number(process.env.OCEAN_TELEGRAM_SYNC_LIMIT || 100);

function safeSegment(value, fallback = 'chat') {
  return String(value || fallback).trim().toLowerCase().replace(/[^a-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '') || fallback;
}

function messageFromUpdate(update) {
  return update.message || update.edited_message || update.channel_post || update.edited_channel_post || null;
}

function messageDateIso(message) {
  const seconds = Number(message?.date || message?.edit_date || 0);
  return seconds ? new Date(seconds * 1000).toISOString() : new Date().toISOString();
}

function chatId(message) {
  return message?.chat?.id === undefined || message?.chat?.id === null ? null : String(message.chat.id);
}

function authorName(message) {
  const from = message?.from || message?.sender_chat || {};
  return [from.first_name, from.last_name].filter(Boolean).join(' ') || from.username || from.title || 'unknown';
}

function mediaMetadata(message) {
  const media = [];
  for (const key of ['photo', 'document', 'audio', 'video', 'voice', 'video_note', 'animation', 'sticker']) {
    if (!message?.[key]) continue;
    const value = message[key];
    if (Array.isArray(value)) {
      const largest = value[value.length - 1] || {};
      media.push({ type: key, file_id: largest.file_id, file_unique_id: largest.file_unique_id, file_size: largest.file_size, width: largest.width, height: largest.height });
    } else {
      media.push({ type: key, file_id: value.file_id, file_unique_id: value.file_unique_id, file_name: value.file_name, mime_type: value.mime_type, file_size: value.file_size, duration: value.duration });
    }
  }
  return media;
}

function renderMessageMarkdown(update, message) {
  const text = message.text || message.caption || '';
  const date = messageDateIso(message);
  const author = authorName(message);
  const chat = message.chat || {};
  const media = mediaMetadata(message);
  return [
    `# Telegram message ${message.message_id}`,
    '',
    `- Update ID: ${update.update_id}`,
    `- Chat: ${chat.title || chat.username || chat.id}`,
    `- Chat ID: ${chat.id}`,
    `- Author: ${author}`,
    `- Date: ${date}`,
    media.length ? `- Media: ${media.map((item) => item.type).join(', ')}` : null,
    '',
    '## Text',
    '',
    text || '_No text body; metadata-only media/update record._',
    '',
  ].filter((line) => line !== null).join('\n');
}

export class TelegramSourceRunner extends SourceRunner {
  constructor() {
    super({ adapterId: 'telegram', name: 'telegram' });
  }

  async telegramFetch(token, method, params = {}) {
    const url = new URL(`${API_BASE}/bot${token}/${method}`);
    for (const [key, value] of Object.entries(params)) {
      if (value === undefined || value === null || value === '') continue;
      url.searchParams.set(key, typeof value === 'string' ? value : JSON.stringify(value));
    }
    const response = await fetch(url);
    const body = await response.json().catch(async () => ({ ok: false, description: await response.text() }));
    if (!response.ok || !body.ok) throw new Error(`Telegram Bot API ${method} failed: ${response.status} ${body.description || 'unknown error'}`);
    return body.result;
  }

  async plan(ctx) {
    const token = resolveSecret(ctx.instance.secret_ref, { required: true });
    const config = ctx.instance.config || {};
    const chat = config.chat_id === undefined || config.chat_id === null ? null : String(config.chat_id);
    if (!chat) throw new Error('Telegram source config requires chat_id.');
    ctx.telegram = { token, chatId: chat, limit: Math.max(1, Math.min(Number(ctx.maxRecords || config.limit || DEFAULT_LIMIT), 100)) };
    return {};
  }

  async *iterate(ctx) {
    const tg = ctx.telegram;
    if (!tg) throw new Error('Telegram runner plan() was not called.');
    const offset = ctx.cursorValue ? Number(ctx.cursorValue) : undefined;
    const updates = await this.telegramFetch(tg.token, 'getUpdates', {
      offset: Number.isFinite(offset) ? offset : undefined,
      limit: tg.limit,
      timeout: 0,
      allowed_updates: ['message', 'edited_message', 'channel_post', 'edited_channel_post'],
    });

    let maxUpdateId = null;
    for (const update of updates || []) {
      if (Number.isFinite(Number(update.update_id))) maxUpdateId = Math.max(maxUpdateId ?? Number(update.update_id), Number(update.update_id));
    }
    if (maxUpdateId !== null) ctx.setNextCursor(String(maxUpdateId + 1));

    const basePrefix = ctx.stream?.remote_prefix || ctx.instance.remote_prefix || `/context/telegram/${safeSegment(tg.chatId)}`;
    for (const update of updates || []) {
      const message = messageFromUpdate(update);
      if (!message || chatId(message) !== tg.chatId) continue;

      const date = messageDateIso(message);
      const day = date.slice(0, 10);
      const virtualPath = joinApiPath(basePrefix, day, `${message.message_id}.md`);
      const media = mediaMetadata(message);
      const chat = message.chat || {};
      yield {
        source_record_id: `message:${tg.chatId}:${message.message_id}`,
        virtual_path: virtualPath,
        content: renderMessageMarkdown(update, message),
        content_type: 'text/markdown; charset=utf-8',
        source_updated_at: date,
        cursor_value: maxUpdateId === null ? undefined : String(maxUpdateId + 1),
        metadata: {
          adapter_id: 'telegram', update_id: update.update_id, message_id: message.message_id,
          chat_id: tg.chatId, chat_title: chat.title || null, chat_username: chat.username || null,
          chat_type: chat.type || null, author: authorName(message), has_text: Boolean(message.text || message.caption), media,
        },
      };
    }
  }
}

export function createTelegramSourceRunner() {
  return new TelegramSourceRunner();
}
