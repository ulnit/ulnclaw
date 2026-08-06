#!/usr/bin/env node
/**
 * ulnclaw WhatsApp Baileys bridge (hermes parity, scripts/whatsapp-bridge).
 *
 * Runs the WhatsApp Web client (Baileys) and exposes a localhost HTTP API
 * consumed by the ulnclaw gateway (`src/whatsapp.rs`):
 *
 *   GET  /health      -> { status, botJid, scriptHash, sendReadReceipts, mode }
 *   GET  /messages    -> drains the inbound queue (array of envelopes)
 *   POST /read        -> { key } send read receipt
 *   POST /send        -> { chatId, message, replyTo? } send text
 *   POST /send-media  -> { to, path, mediaType, caption } send media file
 *
 * CLI: node bridge.js --port 3000 --session <dir> --mode self-chat
 *
 * The `scriptHash` reported by /health is the first 16 hex chars of the
 * SHA-256 of this file; the gateway compares it against the on-disk hash
 * to detect stale long-lived bridges after an update (hermes staleness
 * handshake).
 */
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const os = require('os');
const crypto = require('crypto');

// ---------------------------------------------------------------------------
// CLI / environment
// ---------------------------------------------------------------------------

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--port') out.port = argv[++i];
    else if (arg === '--session') out.session = argv[++i];
    else if (arg === '--mode') out.mode = argv[++i];
  }
  return out;
}

const args = parseArgs(process.argv.slice(2));
const PORT = parseInt(args.port || process.env.WHATSAPP_BRIDGE_PORT || '3000', 10);
const SESSION_DIR = path.resolve(
  args.session ||
    process.env.WHATSAPP_SESSION_PATH ||
    path.join(os.homedir(), '.ulnclaw', 'platforms', 'whatsapp', 'session')
);
const MODE = (args.mode || process.env.WHATSAPP_MODE || 'self-chat').toLowerCase();
const SEND_READ_RECEIPTS =
  String(process.env.WHATSAPP_SEND_READ_RECEIPTS || 'false').toLowerCase() === 'true';
const FORWARD_OWNER = ['1', 'true', 'yes', 'on'].includes(
  String(process.env.WHATSAPP_FORWARD_OWNER_MESSAGES || 'true').toLowerCase()
);
const DEBUG = ['1', 'true', 'yes', 'on'].includes(
  String(process.env.WHATSAPP_DEBUG || '').toLowerCase()
);
const MEDIA_DIR =
  process.env.ULNCLAW_MEDIA_CACHE_DIR ||
  process.env.HERMES_IMAGE_CACHE_DIR ||
  path.join(path.dirname(SESSION_DIR), 'media-cache');

const SCRIPT_HASH = crypto
  .createHash('sha256')
  .update(fs.readFileSync(__filename))
  .digest('hex')
  .slice(0, 16);

function log(...parts) {
  console.log(`[whatsapp-bridge] ${new Date().toISOString()}`, ...parts);
}
function debug(...parts) {
  if (DEBUG) log(...parts);
}

// ---------------------------------------------------------------------------
// Baileys setup
// ---------------------------------------------------------------------------

let baileys;
try {
  baileys = require('@whiskeysockets/baileys');
} catch (err) {
  console.error(
    `[whatsapp-bridge] @whiskeysockets/baileys is not installed in ${__dirname}.\n` +
      `Run "npm install" in the bridge directory first. (${err.message})`
  );
  process.exit(1);
}
const {
  default: makeWASocket,
  useMultiFileAuthState,
  DisconnectReason,
  downloadMediaMessage,
  getContentType,
  jidNormalizedUser,
} = baileys;

let qrcodeTerminal = null;
try {
  qrcodeTerminal = require('qrcode-terminal');
} catch (err) {
  // optional — QR still logged as raw code
}

/** Inbound envelopes waiting for GET /messages. */
const queue = [];
const QUEUE_CAP = 1000;

/** Message ids sent via /send or /send-media (echo suppression for fromMe). */
const sentIds = new Map(); // id -> expiry ms
const SENT_ID_TTL_MS = 5 * 60 * 1000;

/** Contact name cache (JID -> display name). */
const names = new Map();

let sock = null;
let status = 'connecting'; // connecting | qr | connected | disconnected | logged_out
let botJid = '';
let currentQr = '';
let reconnectAttempts = 0;
let shuttingDown = false;

function rememberSentId(id) {
  if (!id) return;
  const now = Date.now();
  for (const [key, expiry] of sentIds) {
    if (expiry < now) sentIds.delete(key);
  }
  sentIds.set(id, now + SENT_ID_TTL_MS);
}

function isOwnEcho(id) {
  return id ? sentIds.has(id) : false;
}

function extForMime(mime) {
  const table = {
    'image/jpeg': '.jpg',
    'image/png': '.png',
    'image/webp': '.webp',
    'image/gif': '.gif',
    'video/mp4': '.mp4',
    'audio/ogg': '.ogg',
    'audio/mpeg': '.mp3',
    'audio/mp4': '.m4a',
    'application/pdf': '.pdf',
  };
  return table[mime] || '.bin';
}

function mediaTypeForContent(contentType) {
  if (!contentType) return '';
  if (contentType.includes('imageMessage')) return 'image';
  if (contentType.includes('videoMessage')) return 'video';
  if (contentType.includes('audioMessage')) return 'audio';
  if (contentType.includes('stickerMessage')) return 'sticker';
  if (contentType.includes('documentMessage')) return 'document';
  return '';
}

function textOf(message) {
  if (!message) return '';
  return (
    message.conversation ||
    (message.extendedTextMessage && message.extendedTextMessage.text) ||
    (message.imageMessage && message.imageMessage.caption) ||
    (message.videoMessage && message.videoMessage.caption) ||
    (message.documentMessage && message.documentMessage.caption) ||
    ''
  );
}

async function resolveName(jid) {
  if (!jid) return '';
  if (names.has(jid)) return names.get(jid);
  let name = jid.split('@')[0];
  try {
    const [info] = await sock.onWhatsApp(jid);
    if (info && info[0] && info[0].notify) name = info[0].notify;
    else if (info && info.notify) name = info.notify;
  } catch (err) {
    debug('onWhatsApp lookup failed for', jid, err.message);
  }
  names.set(jid, name);
  return name;
}

async function cacheMedia(message, contentType) {
  try {
    const buffer = await downloadMediaMessage(message, 'buffer', {});
    if (!buffer || !buffer.length) return null;
    const hash = crypto.createHash('sha256').update(buffer).digest('hex');
    const mime =
      (message.message &&
        (message.message[contentType] && message.message[contentType].mimetype)) ||
      'application/octet-stream';
    fs.mkdirSync(MEDIA_DIR, { recursive: true });
    const file = path.join(MEDIA_DIR, hash + extForMime(mime));
    if (!fs.existsSync(file)) fs.writeFileSync(file, buffer);
    return { file, mime };
  } catch (err) {
    log('media download failed:', err.message);
    return null;
  }
}

async function handleIncoming(message) {
  const key = message.key || {};
  const chatId = key.remoteJid || '';
  if (!chatId || chatId === 'status@broadcast') return;
  if (chatId.endsWith('@broadcast') || chatId.endsWith('@newsletter')) return;
  const contentType = getContentType(message.message);
  if (!contentType) return; // reactions / protocol / empty
  const isGroup = chatId.endsWith('@g.us');
  const fromMe = !!key.fromMe;

  // fromMe echoes of our own /send calls are dropped; in self-chat mode a
  // fromMe message that is NOT an echo was typed by the owner on a linked
  // device — forward it flagged fromOwner (hermes bridge behavior, gated by
  // WHATSAPP_FORWARD_OWNER_MESSAGES).
  let fromOwner = false;
  if (fromMe) {
    if (MODE === 'self-chat' && FORWARD_OWNER && !isOwnEcho(key.id)) {
      fromOwner = true;
    } else {
      return;
    }
  }

  const senderId = isGroup ? key.participant || chatId : chatId;
  const text = textOf(message.message);
  const mediaType = mediaTypeForContent(contentType);
  let hasMedia = false;
  let mime = '';
  const mediaUrls = [];
  if (mediaType) {
    const cached = await cacheMedia(message, contentType);
    if (cached) {
      hasMedia = true;
      mime = cached.mime;
      mediaUrls.push(cached.file);
    }
  }
  if (!text && !hasMedia) return;

  const envelope = {
    chatId,
    senderId,
    senderName: await resolveName(senderId),
    text,
    messageId: key.id || '',
    timestamp: Number(message.messageTimestamp || 0),
    isGroup,
    fromMe,
    fromOwner,
    hasMedia,
    mediaType,
    mime,
    mediaUrls,
    readReceiptKey: { remoteJid: chatId, fromMe, id: key.id, participant: key.participant },
  };
  queue.push(envelope);
  if (queue.length > QUEUE_CAP) queue.splice(0, queue.length - QUEUE_CAP);
  debug('queued', envelope.messageId, 'from', senderId, 'in', chatId);
}

// ---------------------------------------------------------------------------
// Connection lifecycle
// ---------------------------------------------------------------------------

async function connect() {
  if (shuttingDown) return;
  status = 'connecting';
  fs.mkdirSync(SESSION_DIR, { recursive: true });
  const { state, saveCreds } = useMultiFileAuthState(SESSION_DIR);
  sock = makeWASocket({
    auth: state,
    printQRInTerminal: false,
    browser: ['ulnclaw', 'Chrome', '1.0'],
    markOnlineOnConnect: false,
    generateHighQualityLinkPreview: false,
  });

  sock.ev.on('creds.update', saveCreds);

  sock.ev.on('connection.update', (update) => {
    const { connection, lastDisconnect, qr } = update;
    if (qr) {
      status = 'qr';
      currentQr = qr;
      log('QR code received — pair WhatsApp now (expires quickly).');
      if (qrcodeTerminal) {
        qrcodeTerminal.generate(qr, { small: true });
      } else {
        log('QR (raw):', qr);
      }
    }
    if (connection === 'open') {
      status = 'connected';
      reconnectAttempts = 0;
      botJid = sock.user && sock.user.id ? jidNormalizedUser(sock.user.id) : '';
      log('connected as', botJid || '(unknown JID)');
    } else if (connection === 'close') {
      const reason =
        lastDisconnect && lastDisconnect.error && lastDisconnect.error.output
          ? lastDisconnect.error.output.statusCode
          : 0;
      if (reason === DisconnectReason.loggedOut) {
        status = 'logged_out';
        log('logged out — clearing session, pair again via QR.');
        try {
          fs.rmSync(SESSION_DIR, { recursive: true, force: true });
        } catch (err) {
          log('session cleanup failed:', err.message);
        }
        setTimeout(connect, 1000);
      } else {
        status = 'disconnected';
        reconnectAttempts += 1;
        const delay = Math.min(30000, 1000 * 2 ** Math.min(reconnectAttempts, 5));
        log(`connection closed (reason ${reason}); reconnecting in ${delay}ms`);
        setTimeout(connect, delay);
      }
    }
  });

  sock.ev.on('messages.upsert', async ({ messages }) => {
    for (const message of messages || []) {
      try {
        await handleIncoming(message);
      } catch (err) {
        log('inbound handling error:', err.message);
      }
    }
  });
}

// ---------------------------------------------------------------------------
// HTTP API
// ---------------------------------------------------------------------------

function sendJson(res, code, body) {
  const data = JSON.stringify(body);
  res.writeHead(code, {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(data),
  });
  res.end(data);
}

function readBody(req, capBytes) {
  return new Promise((resolve, reject) => {
    let size = 0;
    const chunks = [];
    req.on('data', (chunk) => {
      size += chunk.length;
      if (size > capBytes) {
        reject(new Error('body too large'));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
    req.on('error', reject);
  });
}

function mediaPayload(filePath, mediaType, caption) {
  const ext = path.extname(filePath).toLowerCase();
  const url = { url: filePath };
  switch (mediaType) {
    case 'image':
      return { image: url, caption: caption || undefined };
    case 'video':
      return { video: url, caption: caption || undefined };
    case 'sticker':
      return { sticker: url };
    case 'audio':
      return {
        audio: url,
        mimetype: ext === '.mp3' ? 'audio/mpeg' : 'audio/ogg',
        ptt: ext === '.ogg' || ext === '.opus',
      };
    default:
      return {
        document: url,
        mimetype: 'application/octet-stream',
        fileName: path.basename(filePath),
        caption: caption || undefined,
      };
  }
}

const server = http.createServer(async (req, res) => {
  try {
    const url = (req.url || '/').split('?')[0];
    if (req.method === 'GET' && url === '/health') {
      sendJson(res, 200, {
        status,
        botJid,
        scriptHash: SCRIPT_HASH,
        sendReadReceipts: SEND_READ_RECEIPTS,
        mode: MODE,
        qr: status === 'qr' ? currentQr : undefined,
        queued: queue.length,
      });
      return;
    }
    if (req.method === 'GET' && url === '/messages') {
      const drained = queue.splice(0, queue.length);
      sendJson(res, 200, drained);
      return;
    }
    if (req.method === 'POST' && url === '/read') {
      const body = JSON.parse((await readBody(req, 1024 * 1024)) || '{}');
      if (sock && status === 'connected' && body.key && body.key.id) {
        await sock.readMessages([body.key]);
      }
      sendJson(res, 200, { ok: true });
      return;
    }
    if (req.method === 'POST' && url === '/send') {
      const body = JSON.parse((await readBody(req, 1024 * 1024)) || '{}');
      if (!sock || status !== 'connected') {
        sendJson(res, 503, { ok: false, error: 'not connected' });
        return;
      }
      if (!body.chatId || typeof body.message !== 'string') {
        sendJson(res, 400, { ok: false, error: 'chatId and message required' });
        return;
      }
      const sent = await sock.sendMessage(body.chatId, { text: body.message });
      rememberSentId(sent && sent.key && sent.key.id);
      sendJson(res, 200, { ok: true, messageId: sent && sent.key ? sent.key.id : '' });
      return;
    }
    if (req.method === 'POST' && url === '/send-media') {
      const body = JSON.parse((await readBody(req, 4 * 1024 * 1024)) || '{}');
      if (!sock || status !== 'connected') {
        sendJson(res, 503, { ok: false, error: 'not connected' });
        return;
      }
      if (!body.to || !body.path) {
        sendJson(res, 400, { ok: false, error: 'to and path required' });
        return;
      }
      if (!fs.existsSync(body.path)) {
        sendJson(res, 404, { ok: false, error: `file not found: ${body.path}` });
        return;
      }
      const payload = mediaPayload(body.path, body.mediaType || 'document', body.caption || '');
      const sent = await sock.sendMessage(body.to, payload);
      rememberSentId(sent && sent.key && sent.key.id);
      sendJson(res, 200, { ok: true, messageId: sent && sent.key ? sent.key.id : '' });
      return;
    }
    sendJson(res, 404, { ok: false, error: 'not found' });
  } catch (err) {
    log('HTTP error:', err.message);
    try {
      sendJson(res, 500, { ok: false, error: err.message });
    } catch (_) {
      /* response already closed */
    }
  }
});

process.on('SIGTERM', () => {
  shuttingDown = true;
  log('SIGTERM received, shutting down');
  try {
    server.close();
  } catch (_) {}
  process.exit(0);
});
process.on('SIGINT', () => {
  shuttingDown = true;
  process.exit(0);
});

fs.mkdirSync(MEDIA_DIR, { recursive: true });
server.listen(PORT, '127.0.0.1', () => {
  log(`listening on 127.0.0.1:${PORT} (session ${SESSION_DIR}, mode ${MODE}, hash ${SCRIPT_HASH})`);
  connect();
});
