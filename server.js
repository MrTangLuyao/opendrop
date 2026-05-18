/* ============================================================
 * open.drop — server.js
 *
 * Self-hosted temporary file-drop service.
 *   - Anonymous or signed-in uploads behind a 6-digit pickup code
 *   - Optional per-parcel password (bcrypt-hashed)
 *   - Configurable per-upload cap (default 5 GB) + system-wide cap
 *     (default 30 GB). When full: REJECT — never auto-evict.
 *   - Parcels live up to 7 days
 *   - Accounts: any username/password; account valid 7 days, renewable
 *
 * Configurable via env vars (all optional):
 *   PORT                       default 3000
 *   OPENDROP_MAX_UPLOAD_GB     default 5     (per parcel)
 *   OPENDROP_MAX_STORAGE_GB    default 30    (whole system)
 *   OPENDROP_MAX_EXPIRY_HOURS  default 168   (= 7 days, hard ceiling)
 *   OPENDROP_DATA_DIR          default ./data
 * ============================================================ */

'use strict';

const fs       = require('fs');
const path     = require('path');
const crypto   = require('crypto');
const express  = require('express');
const multer   = require('multer');
const bcrypt   = require('bcryptjs');
const cookies  = require('cookie-parser');
const { Store } = require('./store');

/* ─── Config ──────────────────────────────────────────── */
const PORT                   = parseInt(process.env.PORT || '3000', 10);
const ENV_MAX_UPLOAD_GB      = parseFloat(process.env.OPENDROP_MAX_UPLOAD_GB    || '5');
const ENV_MAX_STORAGE_GB     = parseFloat(process.env.OPENDROP_MAX_STORAGE_GB   || '30');
const ENV_MAX_EXPIRY_HOURS   = parseInt  (process.env.OPENDROP_MAX_EXPIRY_HOURS || '168', 10);
const DATA_DIR               = path.resolve(process.env.OPENDROP_DATA_DIR || path.join(__dirname, 'data'));

// Multer's per-file streaming cap is set ONCE at startup from env, acting as
// the absolute hard ceiling. The admin-tunable per-upload cap can lower this
// further but never exceed it (would need a restart to raise).
const MULTER_HARD_BYTES = Math.floor(ENV_MAX_UPLOAD_GB * 1024 * 1024 * 1024);

// Dynamic limit getters — read from store.config first, fall back to env.
function maxUploadBytes()  {
  const c = store.getConfig();
  const gb = (c.maxUploadGB ?? ENV_MAX_UPLOAD_GB);
  return Math.min(MULTER_HARD_BYTES, Math.floor(gb * 1024 * 1024 * 1024));
}
function maxStorageBytes() {
  const c = store.getConfig();
  const gb = (c.maxStorageGB ?? ENV_MAX_STORAGE_GB);
  return Math.floor(gb * 1024 * 1024 * 1024);
}
function maxExpiryHours()  {
  const c = store.getConfig();
  return Math.max(1, parseInt(c.maxExpiryHours ?? ENV_MAX_EXPIRY_HOURS, 10));
}

const UPLOADS_DIR = path.join(DATA_DIR, 'uploads');

fs.mkdirSync(UPLOADS_DIR, { recursive: true });

/* ─── Store (JSON file index; blobs live on disk) ─────── */
const store = new Store(DATA_DIR);

// Bootstrap a default admin account if none exists. Username/password are
// both "admin" on first run — admin should change them via /admin → 改密.
// Admin accounts have is_admin=1 and never expire (sweeper skips them).
//
// Self-healing: an earlier bug let /api/auth/renew shorten an admin's
// expires_at to "now + 7 d". On every restart we restore any admin whose
// expires_at is sooner than ~99 years out, so any corrupted DB self-heals
// on the next boot.
(function bootstrapAdmin() {
  const FAR_FUTURE = Date.now() + 100 * 365 * 24 * 3600 * 1000;
  const existing = store.allAccounts().find(a => a.is_admin);
  if (existing) {
    if (existing.expires_at < FAR_FUTURE - 365 * 24 * 3600 * 1000) {
      store.renewAccount(existing.id, FAR_FUTURE);
      store.saveSync();        // flush immediately — don't rely on the 80 ms debounce
      console.log('[bootstrap] admin expires_at restored to far future');
    }
    return;
  }
  const hash = bcrypt.hashSync('admin', 10);
  const now  = Date.now();
  store.insertAccount({
    username:      'admin',
    password_hash: hash,
    expires_at:    FAR_FUTURE,
    created_at:    now,
    is_admin:      1,
  });
  console.log('[bootstrap] default admin created: username=admin, password=admin (CHANGE IMMEDIATELY)');
})();

process.on('SIGINT',  () => { store.saveSync(); process.exit(0); });
process.on('SIGTERM', () => { store.saveSync(); process.exit(0); });

/* ─── Pickup code generator ───────────────────────────── */
function newPickupCode() {
  for (let i = 0; i < 200; i++) {
    const code = String(crypto.randomInt(0, 1_000_000)).padStart(6, '0');
    if (!store.parcelByCode(code)) return code;
  }
  throw new Error('Could not allocate unique pickup code');
}

/* ─── Session helpers ─────────────────────────────────── */
const SESSION_COOKIE = 'od_session';
const SEVEN_DAYS_MS = 7 * 24 * 3600 * 1000;

function createSession(accountId) {
  const token = crypto.randomBytes(32).toString('hex');
  const expiresAt = Date.now() + SEVEN_DAYS_MS;
  store.insertSession({ token, account_id: accountId, expires_at: expiresAt });
  return { token, expiresAt };
}

function currentSession(req) {
  const token = req.cookies && req.cookies[SESSION_COOKIE];
  if (!token) return null;
  const row = store.sessionByToken(token);
  if (!row) return null;
  const now = Date.now();
  if (row.expires_at < now)         return null;
  if (row.account_expires_at < now) return null;
  return row;
}

/* ─── Cleanup (parcels + sessions + accounts) ─────────── */
function sweep() {
  const now = Date.now();
  let removed = 0;

  // Snapshot lists so we don't mutate while iterating.
  const expired = store.expiredParcels(now).map(p => p.code);
  for (const code of expired) { removeParcelFiles(code); removed++; }

  const exhausted = store.exhaustedExpiredParcels(now).map(p => p.code);
  for (const code of exhausted) { removeParcelFiles(code); removed++; }

  store.purgeExpiredSessions(now);

  // Expired accounts: delete the account row. Sessions get cascaded,
  // parcels' account_id is nulled (their files survive until natural
  // expiry; otherwise letting an account lapse would surprise the
  // recipient mid-transfer).
  for (const a of store.expiredAccounts(now)) {
    store.deleteAccount(a.id);
  }

  if (removed > 0) console.log(`[sweep] removed ${removed} expired parcel(s)`);
}

function removeParcelFiles(code) {
  const removed = store.deleteParcel(code);
  if (!removed) return;
  for (const f of removed.files) {
    try { fs.unlinkSync(f.path); } catch (_) {}
  }
  const dir = path.join(UPLOADS_DIR, code);
  try { fs.rmdirSync(dir); } catch (_) {}
}

sweep();
setInterval(sweep, 60 * 60 * 1000);  // hourly

/* ─── Multer disk storage with per-request code folder ─ */
const storage = multer.diskStorage({
  destination: (req, file, cb) => {
    if (!req._parcelCode) {
      req._parcelCode = newPickupCode();
    }
    const dir = path.join(UPLOADS_DIR, req._parcelCode);
    fs.mkdirSync(dir, { recursive: true });
    cb(null, dir);
  },
  filename: (req, file, cb) => {
    // Normalize latin-1 → utf-8 (multer default mangles non-ASCII names)
    try {
      file.originalname = Buffer.from(file.originalname, 'latin1').toString('utf8');
    } catch (_) {}
    const safe = file.originalname.replace(/[\\/:*?"<>|]/g, '_').slice(0, 200);
    const id = crypto.randomBytes(6).toString('hex');
    cb(null, `${id}__${safe}`);
  },
});

const upload = multer({
  storage,
  limits: {
    fileSize: MULTER_HARD_BYTES,   // env-level hard ceiling, immutable at runtime
    files: 50,
    fields: 20,
  },
});

/* ─── App ─────────────────────────────────────────────── */
const app = express();
app.disable('x-powered-by');
app.use(cookies());

const json64 = express.json({ limit: '64kb' });

// Deep link: /d/CODE → SPA receive flow with the code prefilled.
app.get(/^\/d\/(\d{6})$/, (req, res) => {
  res.redirect(302, `/?c=${req.params[0]}`);
});

/* ─── /api/storage ────────────────────────────────────── */
app.get('/api/storage', (req, res) => {
  res.json({
    used_bytes:        store.countParcelBytes(),
    max_bytes:         maxStorageBytes(),
    max_upload_bytes:  maxUploadBytes(),
    max_expiry_hours:  maxExpiryHours(),
  });
});

/* ─── Auth ────────────────────────────────────────────── */
app.post('/api/auth/login', json64, async (req, res) => {
  const { username, password } = req.body || {};
  if (typeof username !== 'string' || typeof password !== 'string') {
    return res.status(400).json({ error: 'username and password required' });
  }
  const u = username.trim();
  if (u.length < 1 || u.length > 64)  return res.status(400).json({ error: 'username 1–64 chars' });
  if (password.length < 1 || password.length > 128) return res.status(400).json({ error: 'password 1–128 chars' });

  let acct = store.accountByUsername(u);
  if (!acct) {
    // Auto-register on first login — the spec says any username/password is fine.
    const hash = await bcrypt.hash(password, 10);
    const now  = Date.now();
    const exp  = now + SEVEN_DAYS_MS;
    acct = store.insertAccount({ username: u, password_hash: hash, expires_at: exp, created_at: now });
  } else {
    const ok = await bcrypt.compare(password, acct.password_hash);
    if (!ok) return res.status(401).json({ error: 'wrong password' });
    // If the account had silently expired between sweeps, restore it for 7 days.
    if (acct.expires_at < Date.now()) {
      const exp = Date.now() + SEVEN_DAYS_MS;
      store.renewAccount(acct.id, exp);
      acct.expires_at = exp;
    }
  }

  const { token, expiresAt } = createSession(acct.id);
  res.cookie(SESSION_COOKIE, token, {
    httpOnly: true,
    sameSite: 'lax',
    secure:   req.secure,
    maxAge:   SEVEN_DAYS_MS,
    path:     '/',
  });
  res.json({ username: acct.username, account_expires_at: acct.expires_at });
});

app.post('/api/auth/logout', (req, res) => {
  const tk = req.cookies && req.cookies[SESSION_COOKIE];
  if (tk) store.deleteSession(tk);
  res.clearCookie(SESSION_COOKIE, { path: '/' });
  res.json({ ok: true });
});

app.get('/api/auth/me', (req, res) => {
  const s = currentSession(req);
  if (!s) return res.status(401).json({ error: 'not signed in' });
  const a = store.accountById(s.account_id);
  res.json({
    username:           s.username,
    account_expires_at: s.account_expires_at,
    is_admin:           !!(a && a.is_admin),
    is_perm:            !!(a && a.is_perm),
  });
});

app.post('/api/auth/renew', (req, res) => {
  const s = currentSession(req);
  if (!s) return res.status(401).json({ error: 'not signed in' });
  // Accounts that never expire (admin OR admin-flagged 长期账户) reject renewal.
  const a = store.accountById(s.account_id);
  if (a && a.is_admin) return res.status(400).json({ error: 'admin_never_expires' });
  if (a && a.is_perm)  return res.status(400).json({ error: 'perm_never_expires' });

  const exp = Date.now() + SEVEN_DAYS_MS;
  store.renewAccount(s.account_id, exp);
  res.json({ account_expires_at: exp });
});

/* ─── Upload ──────────────────────────────────────────── */
app.post('/api/upload', (req, res) => {
  const claimed       = parseInt(req.headers['content-length'] || '0', 10);
  const used          = store.countParcelBytes();
  const dynMaxUpload  = maxUploadBytes();
  const dynMaxStorage = maxStorageBytes();

  if (claimed > 0 && claimed > dynMaxUpload + 256 * 1024) {
    // Reject oversize uploads BEFORE bytes hit the wire. The 256 KB padding
    // covers multipart-framing overhead so legit edge-case uploads aren't
    // misclassified.
    req.resume(); // Drain stream to avoid hangs
    return res.status(413).json({ error: 'parcel_too_large', max_bytes: dynMaxUpload });
  }
  if (claimed > 0 && used + claimed > dynMaxStorage) {
    req.resume(); // Drain stream to avoid hangs
    return res.status(507).json({
      error: 'storage_full',
      used_bytes: used,
      max_bytes:  dynMaxStorage,
      attempted:  claimed,
    });
  }

  // Pre-generate code and directory BEFORE starting the multipart stream
  // This reduces blocking logic during the actual data transfer.
  try {
    if (!req._parcelCode) {
      req._parcelCode = newPickupCode();
      const dir = path.join(UPLOADS_DIR, req._parcelCode);
      fs.mkdirSync(dir, { recursive: true });
    }
  } catch (e) {
    return res.status(500).json({ error: 'setup_failed', detail: e.message });
  }

  upload.array('files')(req, res, async (err) => {
    try {
      if (err) {
        cleanupRequestUploads(req);
        if (err.code === 'LIMIT_FILE_SIZE') {
          return res.status(413).json({ error: 'file_too_large', max_bytes: dynMaxUpload });
        }
        return res.status(400).json({ error: 'upload_failed', detail: err.message });
      }

      const files = req.files || [];
      if (files.length === 0) return res.status(400).json({ error: 'no_files' });

      const totalBytes = files.reduce((s, f) => s + f.size, 0);
      if (totalBytes > dynMaxUpload) {
        cleanupRequestUploads(req);
        return res.status(413).json({ error: 'parcel_too_large', max_bytes: dynMaxUpload });
      }

      const usedNow = store.countParcelBytes();
      if (usedNow + totalBytes > dynMaxStorage) {
        cleanupRequestUploads(req);
        return res.status(507).json({
          error: 'storage_full',
          used_bytes: usedNow,
          max_bytes:  dynMaxStorage,
          attempted:  totalBytes,
        });
      }

      const password  = (req.body.password || '').trim();
      const downloads = clamp(parseInt(req.body.downloads || '2', 10), 1, 999);
      const expiryHrs = clamp(parseInt(req.body.expiry_hours || '24', 10), 1, maxExpiryHours());
      const kind      = req.body.kind === 'text' ? 'text' : 'files';
      const code      = req._parcelCode;
      const now       = Date.now();
      const expiresAt = now + expiryHrs * 3600 * 1000;
      const passHash  = password ? await bcrypt.hash(password, 10) : '';
      const session   = currentSession(req);
      const accountId = session ? session.account_id : null;

      store.insertParcel({
        code,
        account_id:     accountId,
        password_hash:  passHash,
        downloads_left: downloads,
        total_bytes:    totalBytes,
        expires_at:     expiresAt,
        created_at:     now,
        kind,
        files: files.map(f => ({
          name: f.originalname,
          mime: f.mimetype || 'application/octet-stream',
          size: f.size,
          path: f.path,
        })),
      });

      res.json({
        code,
        expires_at: expiresAt,
        downloads_left: downloads,
        total_bytes: totalBytes,
        file_count: files.length,
        kind,
      });
    } catch (unexpected) {
      console.error('[upload] unexpected error:', unexpected);
      cleanupRequestUploads(req);
      if (!res.headersSent) {
        res.status(500).json({ error: 'internal_error' });
      }
    }
  });
});

function cleanupRequestUploads(req) {
  if (req.files) for (const f of req.files) { try { fs.unlinkSync(f.path); } catch (_) {} }
  if (req._parcelCode) {
    try { fs.rmdirSync(path.join(UPLOADS_DIR, req._parcelCode)); } catch (_) {}
  }
}

function clamp(n, lo, hi) { return Math.max(lo, Math.min(hi, n || lo)); }

/* ─── Parcel fetch (info + claim + file) ──────────────── */
app.post('/api/parcel/:code/info', json64, async (req, res) => {
  const code = String(req.params.code || '');
  if (!/^\d{6}$/.test(code)) return res.status(400).json({ error: 'bad_code' });

  const p = store.parcelByCode(code);
  if (!p)                                return res.status(404).json({ error: 'not_found' });
  if (p.expires_at < Date.now())         return res.status(410).json({ error: 'expired' });
  if (p.downloads_left <= 0)             return res.status(410).json({ error: 'no_downloads' });

  if (p.password_hash) {
    const provided = (req.body && req.body.password) || '';
    if (!provided) return res.status(401).json({ error: 'needs_password' });
    const ok = await bcrypt.compare(provided, p.password_hash);
    if (!ok)       return res.status(401).json({ error: 'bad_password' });
  }

  res.json({
    code,
    kind: p.kind || 'files',
    files: p.files.map(f => ({ id: f.id, name: f.name, mime: f.mime, size: f.size })),
    expires_at: p.expires_at,
    downloads_left: p.downloads_left,
    has_password: !!p.password_hash,
  });
});

const claimTokens = new Map();
setInterval(() => {
  const now = Date.now();
  for (const [k, v] of claimTokens) if (v.expires_at < now) claimTokens.delete(k);
}, 5 * 60 * 1000);

app.post('/api/parcel/:code/claim', json64, async (req, res) => {
  const code = String(req.params.code || '');
  if (!/^\d{6}$/.test(code)) return res.status(400).json({ error: 'bad_code' });

  const p = store.parcelByCode(code);
  if (!p)                                return res.status(404).json({ error: 'not_found' });
  if (p.expires_at < Date.now())         return res.status(410).json({ error: 'expired' });
  if (p.downloads_left <= 0)             return res.status(410).json({ error: 'no_downloads' });

  if (p.password_hash) {
    const provided = (req.body && req.body.password) || '';
    if (!provided) return res.status(401).json({ error: 'needs_password' });
    const ok = await bcrypt.compare(provided, p.password_hash);
    if (!ok)       return res.status(401).json({ error: 'bad_password' });
  }

  if (!store.decrementDownloads(code)) return res.status(410).json({ error: 'no_downloads' });

  const token = crypto.randomBytes(16).toString('hex');
  const exp   = Date.now() + 30 * 60 * 1000;
  claimTokens.set(token, { code, expires_at: exp });

  // p was fetched before the decrement, but the store mutates in place,
  // so p.downloads_left is already the post-decrement value.
  res.json({ token, expires_at: exp, downloads_left: p.downloads_left });
});

app.get('/api/parcel/:code/file/:fileId', (req, res) => {
  const code   = String(req.params.code || '');
  const fileId = parseInt(req.params.fileId, 10);
  const token  = String(req.query.token || '');

  const claim = claimTokens.get(token);
  if (!claim || claim.code !== code || claim.expires_at < Date.now()) {
    return res.status(403).json({ error: 'bad_token' });
  }

  const f = store.fileById(code, fileId);
  if (!f) return res.status(404).json({ error: 'not_found' });
  if (!fs.existsSync(f.path)) return res.status(410).json({ error: 'gone' });

  res.setHeader('Content-Type', f.mime || 'application/octet-stream');
  res.setHeader('Content-Length', f.size);
  res.setHeader('Content-Disposition',
    `attachment; filename*=UTF-8''${encodeURIComponent(f.name)}`);
  fs.createReadStream(f.path).pipe(res);
});

/* ─── My files ────────────────────────────────────────── */
app.get('/api/me/parcels', (req, res) => {
  const s = currentSession(req);
  if (!s) return res.status(401).json({ error: 'not signed in' });

  const parcels = store.parcelsByAccount(s.account_id).map(p => ({
    code:           p.code,
    total_bytes:    p.total_bytes,
    created_at:     p.created_at,
    expires_at:     p.expires_at,
    downloads_left: p.downloads_left,
    has_password:   !!p.password_hash,
  }));
  res.json({ parcels });
});

app.delete('/api/me/parcels/:code', (req, res) => {
  const s = currentSession(req);
  if (!s) return res.status(401).json({ error: 'not signed in' });

  const code = String(req.params.code || '');
  const p = store.parcelByCode(code);
  if (!p)                            return res.status(404).json({ error: 'not_found' });
  if (p.account_id !== s.account_id) return res.status(403).json({ error: 'not_owner' });

  removeParcelFiles(code);
  res.json({ ok: true });
});

/* ─── Admin ──────────────────────────────────────────────
 * Admin auth reuses the regular session cookie BUT every admin endpoint
 * checks `session.is_admin === 1`. That way an admin can also use the
 * normal SPA features (uploads, my-files) without juggling two cookies.
 * ─────────────────────────────────────────────────────── */

function currentAdmin(req) {
  const s = currentSession(req);
  if (!s) return null;
  const a = store.accountById(s.account_id);
  if (!a || !a.is_admin) return null;
  return { ...s, is_admin: 1 };
}

function requireAdmin(req, res, next) {
  const a = currentAdmin(req);
  if (!a) return res.status(401).json({ error: 'admin_required' });
  req.admin = a;
  next();
}

// Serve the admin SPA shell at /admin. The page itself handles login.
app.get('/admin', (req, res) => {
  res.sendFile(path.join(__dirname, 'public', 'admin.html'));
});

app.get('/api/admin/me', requireAdmin, (req, res) => {
  res.json({ username: req.admin.username });
});

app.get('/api/admin/stats', requireAdmin, (req, res) => {
  const parcels  = store.allParcels();
  const accounts = store.allAccounts();
  res.json({
    total_parcels:    parcels.length,
    total_accounts:   accounts.length - accounts.filter(a => a.is_admin).length,
    storage_used:     store.countParcelBytes(),
    storage_max:      maxStorageBytes(),
    max_upload_bytes: maxUploadBytes(),
    max_expiry_hours: maxExpiryHours(),
    hard_upload_ceiling_bytes: MULTER_HARD_BYTES,
  });
});

app.get('/api/admin/parcels', requireAdmin, (req, res) => {
  const accounts = new Map(store.allAccounts().map(a => [a.id, a.username]));
  const parcels = store.allParcels().map(p => ({
    code:           p.code,
    owner:          p.account_id ? (accounts.get(p.account_id) || '(deleted user)') : '(anonymous)',
    total_bytes:    p.total_bytes,
    file_count:     (p.files || []).length,
    kind:           p.kind || 'files',
    downloads_left: p.downloads_left,
    has_password:   !!p.password_hash,
    expires_at:     p.expires_at,
    created_at:     p.created_at,
  }));
  res.json({ parcels });
});

app.delete('/api/admin/parcels/:code', requireAdmin, (req, res) => {
  const code = String(req.params.code || '');
  const p = store.parcelByCode(code);
  if (!p) return res.status(404).json({ error: 'not_found' });
  removeParcelFiles(code);
  res.json({ ok: true });
});

app.get('/api/admin/accounts', requireAdmin, (req, res) => {
  const accounts = store.allAccounts()
    .filter(a => !a.is_admin)
    .map(a => {
      const ps = store.parcelsByAccount(a.id);
      return {
        id:           a.id,
        username:     a.username,
        created_at:   a.created_at,
        expires_at:   a.expires_at,
        is_perm:      !!a.is_perm,
        parcel_count: ps.length,
        storage_used: ps.reduce((s, p) => s + p.total_bytes, 0),
      };
    });
  res.json({ accounts });
});

app.post('/api/admin/accounts/:id/toggle-perm', requireAdmin, (req, res) => {
  // Flip the long-term flag for a given user account. Long-term accounts
  // skip the sweep and refuse renewal — same protections admin gets.
  const id = parseInt(req.params.id, 10);
  const a = store.accountById(id);
  if (!a)         return res.status(404).json({ error: 'not_found' });
  if (a.is_admin) return res.status(400).json({ error: 'admin_already_perm' });
  a.is_perm = !a.is_perm;
  store.saveSync();
  res.json({ is_perm: !!a.is_perm });
});

app.delete('/api/admin/accounts/:id', requireAdmin, (req, res) => {
  const id = parseInt(req.params.id, 10);
  const a = store.accountById(id);
  if (!a)              return res.status(404).json({ error: 'not_found' });
  if (a.is_admin)      return res.status(403).json({ error: 'cannot_delete_admin' });

  // Cascade: delete the account's parcels (files + index)
  for (const p of store.parcelsByAccount(id)) removeParcelFiles(p.code);
  store.deleteAccount(id);
  res.json({ ok: true });
});

app.post('/api/admin/accounts/:id/renew', requireAdmin, (req, res) => {
  const id = parseInt(req.params.id, 10);
  const a = store.accountById(id);
  if (!a)         return res.status(404).json({ error: 'not_found' });
  if (a.is_admin) return res.status(400).json({ error: 'admin_never_expires' });
  const exp = Date.now() + SEVEN_DAYS_MS;
  store.renewAccount(id, exp);
  res.json({ expires_at: exp });
});

app.get('/api/admin/config', requireAdmin, (req, res) => {
  const c = store.getConfig();
  res.json({
    max_storage_gb:    c.maxStorageGB ?? ENV_MAX_STORAGE_GB,
    max_upload_gb:     c.maxUploadGB  ?? ENV_MAX_UPLOAD_GB,
    max_expiry_hours:  c.maxExpiryHours ?? ENV_MAX_EXPIRY_HOURS,
    upload_hard_ceiling_gb: ENV_MAX_UPLOAD_GB,
  });
});

app.post('/api/admin/config', requireAdmin, json64, (req, res) => {
  const patch = {};
  if ('max_storage_gb'   in req.body) patch.maxStorageGB    = Math.max(0.1, parseFloat(req.body.max_storage_gb)   || 0);
  if ('max_upload_gb'    in req.body) patch.maxUploadGB     = Math.max(0.1, Math.min(ENV_MAX_UPLOAD_GB, parseFloat(req.body.max_upload_gb) || 0));
  if ('max_expiry_hours' in req.body) patch.maxExpiryHours  = Math.max(1, Math.min(168 * 4, parseInt(req.body.max_expiry_hours, 10) || 0));
  store.setConfig(patch);
  res.json({
    max_storage_gb:    store.getConfig().maxStorageGB    ?? ENV_MAX_STORAGE_GB,
    max_upload_gb:     store.getConfig().maxUploadGB     ?? ENV_MAX_UPLOAD_GB,
    max_expiry_hours:  store.getConfig().maxExpiryHours  ?? ENV_MAX_EXPIRY_HOURS,
    upload_hard_ceiling_gb: ENV_MAX_UPLOAD_GB,
  });
});

app.post('/api/admin/password', requireAdmin, json64, async (req, res) => {
  const { current, next, username } = req.body || {};
  if (typeof current !== 'string' || typeof next !== 'string' || next.length < 1) {
    return res.status(400).json({ error: 'bad_request' });
  }
  const a = store.accountById(req.admin.account_id);
  const ok = await bcrypt.compare(current, a.password_hash);
  if (!ok) return res.status(401).json({ error: 'wrong_current_password' });

  // Username change (optional): only if new and not taken
  if (typeof username === 'string' && username.trim() && username.trim() !== a.username) {
    const u = username.trim();
    if (u.length > 64) return res.status(400).json({ error: 'username_too_long' });
    if (store.accountByUsername(u)) return res.status(409).json({ error: 'username_taken' });
    a.username = u;
  }
  a.password_hash = await bcrypt.hash(next, 10);
  store.save();
  res.json({ ok: true, username: a.username });
});

// Static files. Caching disabled so deployments roll out immediately.
app.use(express.static(path.join(__dirname, 'public'), {
  etag: false,
  setHeaders: (res) => res.setHeader('Cache-Control', 'no-store'),
}));

/* ─── Start ───────────────────────────────────────────── */
app.listen(PORT, () => {
  console.log(`open.drop listening on http://localhost:${PORT}`);
  console.log(`  data dir:        ${DATA_DIR}`);
  console.log(`  upload cap:      ${(maxUploadBytes() / 1073741824).toFixed(2)} GB / parcel  (hard ceiling ${ENV_MAX_UPLOAD_GB} GB)`);
  console.log(`  storage cap:     ${(maxStorageBytes() / 1073741824).toFixed(2)} GB / system`);
  console.log(`  expiry ceiling:  ${maxExpiryHours()} h (${(maxExpiryHours() / 24).toFixed(1)} d)`);
  console.log(`  admin panel:     http://localhost:${PORT}/admin   (default admin/admin)`);
});
