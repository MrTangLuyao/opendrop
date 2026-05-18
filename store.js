/* ============================================================
 * open.drop — store.js
 *
 * Dependency-free, single-file JSON store for the parcel index +
 * accounts + sessions. SQLite was the first choice (cleaner queries,
 * WAL durability) but its native build wouldn't go on every host
 * out of the box, so we use an in-memory store backed by an atomic
 * file write.
 *
 * Disk layout (under DATA_DIR):
 *   db.json     — single JSON document holding all index rows
 *   db.json.tmp — atomic-rename target while persisting
 *
 * The actual file blobs still live under data/uploads/<code>/*.
 * This file only indexes them.
 *
 * Performance: for a few thousand parcels the whole document is
 * tens of KB → trivial. Saves are debounced (~80ms) so a burst of
 * mutations only writes once.
 * ============================================================ */

'use strict';

const fs   = require('fs');
const path = require('path');

class Store {
  constructor(dataDir) {
    this.dataDir = dataDir;
    this.dbPath  = path.join(dataDir, 'db.json');
    this.tmpPath = this.dbPath + '.tmp';
    fs.mkdirSync(dataDir, { recursive: true });

    this.data = this._load();
    this._saveTimer = null;
  }

  _load() {
    try {
      const raw = fs.readFileSync(this.dbPath, 'utf8');
      const j = JSON.parse(raw);
      return {
        nextAccountId: j.nextAccountId || 1,
        nextFileId:    j.nextFileId    || 1,
        accounts:      Array.isArray(j.accounts) ? j.accounts : [],
        sessions:      Array.isArray(j.sessions) ? j.sessions : [],
        parcels:       Array.isArray(j.parcels)  ? j.parcels  : [],
        config:        (j.config && typeof j.config === 'object') ? j.config : {},
      };
    } catch (_) {
      return { nextAccountId: 1, nextFileId: 1, accounts: [], sessions: [], parcels: [], config: {} };
    }
  }

  /* ─── Config (admin-tunable runtime limits) ─── */
  getConfig() { return Object.assign({}, this.data.config); }
  setConfig(patch) {
    this.data.config = Object.assign({}, this.data.config, patch);
    this.save();
    return this.getConfig();
  }

  /* ─── Admin-wide listings ─── */
  allParcels() {
    return this.data.parcels.slice().sort((a, b) => b.created_at - a.created_at);
  }
  allAccounts() {
    return this.data.accounts.slice().sort((a, b) => a.id - b.id);
  }

  _save() {
    const payload = JSON.stringify(this.data);
    fs.writeFileSync(this.tmpPath, payload);
    fs.renameSync(this.tmpPath, this.dbPath);
  }

  /** Debounced async save (~80 ms). Most callers want this — multiple
   *  mutations in the same tick collapse into a single fsync. */
  save() {
    if (this._saveTimer) return;
    this._saveTimer = setTimeout(() => {
      this._saveTimer = null;
      try { this._save(); } catch (e) { console.error('[store] save failed:', e); }
    }, 80);
  }

  /** Force a synchronous flush (used at shutdown). */
  saveSync() {
    if (this._saveTimer) { clearTimeout(this._saveTimer); this._saveTimer = null; }
    try { this._save(); } catch (e) { console.error('[store] saveSync failed:', e); }
  }

  /* ─── Parcels ─── */
  parcelByCode(code)        { return this.data.parcels.find(p => p.code === code) || null; }
  countParcelBytes()        { return this.data.parcels.reduce((s, p) => s + (p.total_bytes || 0), 0); }
  parcelsByAccount(accountId) {
    return this.data.parcels
      .filter(p => p.account_id === accountId)
      .sort((a, b) => b.created_at - a.created_at);
  }
  expiredParcels(now)       { return this.data.parcels.filter(p => p.expires_at < now); }
  exhaustedExpiredParcels(now) {
    return this.data.parcels.filter(p => p.downloads_left <= 0 && p.expires_at < now);
  }
  insertParcel(rec) {
    const files = (rec.files || []).map(f => Object.assign({ id: this.data.nextFileId++ }, f));
    const parcel = {
      code:            rec.code,
      account_id:      rec.account_id || null,
      password_hash:   rec.password_hash || '',
      downloads_left:  rec.downloads_left,
      total_bytes:     rec.total_bytes,
      expires_at:      rec.expires_at,
      created_at:      rec.created_at,
      kind:            rec.kind || 'files',
      files,
    };
    this.data.parcels.push(parcel);
    this.save();
    return parcel;
  }
  decrementDownloads(code) {
    const p = this.parcelByCode(code);
    if (!p || p.downloads_left <= 0) return false;
    p.downloads_left -= 1;
    this.save();
    return true;
  }
  fileById(code, fileId) {
    const p = this.parcelByCode(code);
    if (!p) return null;
    return p.files.find(f => f.id === fileId) || null;
  }
  deleteParcel(code) {
    const idx = this.data.parcels.findIndex(p => p.code === code);
    if (idx < 0) return null;
    const [removed] = this.data.parcels.splice(idx, 1);
    this.save();
    return removed;
  }

  /* ─── Accounts ─── */
  accountByUsername(u)      { return this.data.accounts.find(a => a.username === u) || null; }
  accountById(id)           { return this.data.accounts.find(a => a.id === id) || null; }
  expiredAccounts(now)      { return this.data.accounts.filter(a => !a.is_admin && a.expires_at < now); }
  insertAccount(rec) {
    const a = Object.assign({ id: this.data.nextAccountId++ }, rec);
    this.data.accounts.push(a);
    this.save();
    return a;
  }
  renewAccount(id, expiresAt) {
    const a = this.accountById(id);
    if (!a) return;
    a.expires_at = expiresAt;
    this.save();
  }
  deleteAccount(id) {
    const idx = this.data.accounts.findIndex(a => a.id === id);
    if (idx < 0) return;
    this.data.accounts.splice(idx, 1);
    // Cascade: drop sessions belonging to this account.
    this.data.sessions = this.data.sessions.filter(s => s.account_id !== id);
    // Null out account_id on this user's parcels so they survive (per spec).
    for (const p of this.data.parcels) {
      if (p.account_id === id) p.account_id = null;
    }
    this.save();
  }

  /* ─── Sessions ─── */
  sessionByToken(token) {
    const s = this.data.sessions.find(x => x.token === token);
    if (!s) return null;
    const a = this.accountById(s.account_id);
    if (!a) return null;
    return {
      token: s.token,
      account_id: s.account_id,
      expires_at: s.expires_at,
      username: a.username,
      account_expires_at: a.expires_at,
    };
  }
  insertSession(rec) {
    this.data.sessions.push(rec);
    this.save();
  }
  deleteSession(token) {
    const idx = this.data.sessions.findIndex(s => s.token === token);
    if (idx < 0) return;
    this.data.sessions.splice(idx, 1);
    this.save();
  }
  purgeExpiredSessions(now) {
    const before = this.data.sessions.length;
    this.data.sessions = this.data.sessions.filter(s => s.expires_at >= now);
    if (this.data.sessions.length !== before) this.save();
  }
}

module.exports = { Store };
