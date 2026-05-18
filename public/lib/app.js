/* ============================================================
 * open.drop — app.js
 *
 * Client logic for the server-backed file-drop SPA.
 *   - Stages: landing / send / receive / my-files
 *   - Real uploads via XMLHttpRequest (so we get progress events)
 *   - Cookie-based account session, login modal in top-right
 *   - Storage gauge polled on load + after upload + after delete
 *   - Bilingual labels (zh ⇄ en) with the same scheme as homepage
 * ============================================================ */

(() => {
  'use strict';

  /* ─────────────────────────────────────────────────────────
   * i18n
   * ───────────────────────────────────────────────────────── */
  const T = {
    zh: {
      'brand-sub': '临时文件传输',
      'tagline': '拖拽即发 · 输码即取',
      'cta-send': '发送',
      'cta-receive': '接收',
      'hint-drag': '可直接拖拽文件到页面上以发送',
      'hint-paste': '可直接按 <kbd>Ctrl</kbd>+<kbd>V</kbd> 以发送剪贴板中的内容',
      'chip-files': '文件',
      'chip-text': '文本',
      'chip-folder': '文件夹',
      'send-title': '您选择了这些文件',
      'not-signed-in': '未登录',
      'signed-in-as': '已登录：{user}',
      'cfg-password': '密码',
      'cfg-username': '用户名',
      'cfg-downloads': '可下载次数',
      'cfg-expiry': '有效期（小时，最长 168）',
      'btn-upload': '上传',
      'uploading-title': '正在上传',
      'success-title': '文件已成功发送',
      'your-code': '您的取件码',
      'enter-code-hint': '接收文件时，请输入如上取件码',
      'copy-link': '复制下载链接',
      'show-qr': '显示二维码',
      'receive-title': '请输入取件码',
      'receive-sub': '六位数字',
      'btn-fetch': '接收',
      'receive-list-title': '文件已就绪',
      'btn-download-all': '下载全部',
      'drop-overlay': '松手以发送',
      'footer': '© <span id="year"></span> · open.drop · <span id="footer-storage">—</span>',
      'err-not-found': '取件码不存在',
      'err-bad-password': '密码错误',
      'err-needs-password': '此文件需要密码',
      'err-no-downloads': '下载次数已用完',
      'err-expired': '链接已过期',
      'err-storage-full': '系统存储已满，无法接收新文件',
      'err-file-too-large': '单次上传不能超过 {gb} GB',
      'err-parcel-too-large': '本次上传超过单次最大限制 {gb} GB',
      'err-network': '网络错误，请重试',
      'err-wrong-password': '密码错误',
      'err-username-required': '请输入用户名',
      'err-password-required': '请输入密码',
      'toast-copied': '链接已复制',
      'toast-no-files': '请先选择文件',
      'toast-deleted': '已删除',
      'toast-renewed': '账户已续期 7 天',
      'remaining': '剩余 {n} 次下载',
      'sent-clipboard': '已发送剪贴板内容',
      'text-snippet': 'clipboard',
      'send-text-title': '发送文本',
      'send-text-placeholder': '请输入纯文本',
      'receive-text-title': '接收到文本',
      'btn-copy-text': '复制',
      'toast-empty-text': '请输入内容',
      'nav-login': '登录',
      'menu-my-files': '我的文件',
      'menu-renew': '续期账户',
      'menu-logout': '登出',
      'login-title': '登录或注册',
      'login-sub': '用户名和密码可任意填写，账户有效期 7 天，可随时续期',
      'btn-login': '登录 / 注册',
      'my-files-title': '我的文件',
      'my-files-empty': '还没有上传过文件',
      'my-files-meta': '共 {n} 个包裹 · 占用 {used}',
      'file-row-meta': '{size} · 剩 {n} 次下载 · {expiry}',
      'file-row-expired': '已过期',
      'file-row-expires-in': '{h} 小时后过期',
      'btn-delete-parcel': '删除',
      'storage-tip': '系统总存储 {used} / {max}',
      'account-expires': '账户有效期至 {date}',
      'account-never-expires': '管理员账户 · 永不过期',
      'account-perm-never-expires': '长期账户 · 永不过期',
      'admin-console': '进入控制台',
    },
    en: {
      'brand-sub': 'Temporary file drop',
      'tagline': 'Drop to send · Code to receive',
      'cta-send': 'Send',
      'cta-receive': 'Receive',
      'hint-drag': 'Drag files anywhere on the page to send',
      'hint-paste': 'Press <kbd>Ctrl</kbd>+<kbd>V</kbd> to send clipboard contents',
      'chip-files': 'Files',
      'chip-text': 'Text',
      'chip-folder': 'Folder',
      'send-title': 'Selected files',
      'not-signed-in': 'Not signed in',
      'signed-in-as': 'Signed in as {user}',
      'cfg-password': 'Password',
      'cfg-username': 'Username',
      'cfg-downloads': 'Download limit',
      'cfg-expiry': 'Expiry (hours, max 168)',
      'btn-upload': 'Upload',
      'uploading-title': 'Uploading',
      'success-title': 'Sent successfully',
      'your-code': 'Your pickup code',
      'enter-code-hint': 'Enter the code above to retrieve files',
      'copy-link': 'Copy link',
      'show-qr': 'Show QR',
      'receive-title': 'Enter pickup code',
      'receive-sub': '6 digits',
      'btn-fetch': 'Retrieve',
      'receive-list-title': 'Files ready',
      'btn-download-all': 'Download all',
      'drop-overlay': 'Drop to send',
      'footer': '© <span id="year"></span> · open.drop · <span id="footer-storage">—</span>',
      'err-not-found': 'Code not found',
      'err-bad-password': 'Wrong password',
      'err-needs-password': 'This parcel needs a password',
      'err-no-downloads': 'Download limit reached',
      'err-expired': 'Link expired',
      'err-storage-full': 'System storage is full — try again later',
      'err-file-too-large': 'Per-upload limit is {gb} GB',
      'err-parcel-too-large': 'Upload exceeds the {gb} GB per-parcel limit',
      'err-network': 'Network error — please retry',
      'err-wrong-password': 'Wrong password',
      'err-username-required': 'Username required',
      'err-password-required': 'Password required',
      'toast-copied': 'Link copied',
      'toast-no-files': 'Pick some files first',
      'toast-deleted': 'Deleted',
      'toast-renewed': 'Account renewed for 7 days',
      'remaining': '{n} download(s) remaining',
      'sent-clipboard': 'Sent clipboard contents',
      'text-snippet': 'clipboard',
      'send-text-title': 'Send text',
      'send-text-placeholder': 'Type or paste text…',
      'receive-text-title': 'Received text',
      'btn-copy-text': 'Copy',
      'toast-empty-text': 'Type something first',
      'nav-login': 'Sign in',
      'menu-my-files': 'My files',
      'menu-renew': 'Renew account',
      'menu-logout': 'Sign out',
      'login-title': 'Sign in or register',
      'login-sub': 'Any username and password work — accounts last 7 days, renewable any time.',
      'btn-login': 'Sign in / Register',
      'my-files-title': 'My files',
      'my-files-empty': 'No uploads yet',
      'my-files-meta': '{n} parcel(s) · {used} stored',
      'file-row-meta': '{size} · {n} download(s) left · {expiry}',
      'file-row-expired': 'expired',
      'file-row-expires-in': 'expires in {h}h',
      'btn-delete-parcel': 'Delete',
      'storage-tip': 'System storage {used} / {max}',
      'account-expires': 'Account valid until {date}',
      'account-never-expires': 'Admin account · never expires',
      'account-perm-never-expires': 'Long-term account · never expires',
      'admin-console': 'Open admin panel',
    },
  };

  let lang = (navigator.language || 'en').toLowerCase().includes('zh') ? 'zh' : 'en';

  function tr(key, vars) {
    let s = (T[lang] && T[lang][key]) || T.en[key] || key;
    if (vars) for (const k in vars) s = s.replace('{' + k + '}', vars[k]);
    return s;
  }
  function applyLang() {
    document.querySelectorAll('[data-i18n]').forEach(el => {
      el.textContent = tr(el.getAttribute('data-i18n'));
    });
    document.querySelectorAll('[data-i18n-html]').forEach(el => {
      el.innerHTML = tr(el.getAttribute('data-i18n-html'));
    });
    const yr = document.getElementById('year');
    if (yr) yr.textContent = new Date().getFullYear();
    const langLabel = document.getElementById('lang-label');
    if (langLabel) langLabel.textContent = lang === 'zh' ? '文 ⇄ EN' : 'EN ⇄ 文';
    document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en';
    refreshAuthUi();
    refreshStorageGauge();
  }

  /* ─────────────────────────────────────────────────────────
   * Utilities
   * ───────────────────────────────────────────────────────── */
  function fmtBytes(n) {
    if (n == null) return '—';
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
    return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
  function fileExt(name) {
    const i = name.lastIndexOf('.');
    return i > 0 ? name.slice(i + 1, i + 5).toUpperCase() : 'FILE';
  }
  function escapeText(s) {
    return String(s).replace(/[&<>]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));
  }
  function escapeAttr(s) {
    return String(s).replace(/["&<>]/g, c => ({ '"': '&quot;', '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));
  }
  function fmtDate(ms) {
    const d = new Date(ms);
    const z = (n) => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${z(d.getMonth() + 1)}-${z(d.getDate())} ${z(d.getHours())}:${z(d.getMinutes())}`;
  }

  /* ─────────────────────────────────────────────────────────
   * Stage routing
   * ───────────────────────────────────────────────────────── */
  const stages = {
    landing:    document.getElementById('stage-landing'),
    send:       document.getElementById('stage-send'),
    receive:    document.getElementById('stage-receive'),
    'my-files': document.getElementById('stage-my-files'),
  };

  function showStage(name, substate) {
    Object.entries(stages).forEach(([k, el]) => {
      el.classList.toggle('is-active', k === name);
    });
    if (name === 'send' || name === 'receive') {
      const cards = stages[name].querySelectorAll('.card[data-substate]');
      cards.forEach(c => c.classList.toggle('is-active', c.dataset.substate === substate));
    } else if (name === 'my-files') {
      stages['my-files'].querySelector('.card').classList.add('is-active');
    }
  }

  /* ─────────────────────────────────────────────────────────
   * Auth state
   * ───────────────────────────────────────────────────────── */
  let me = null;
  let pendingAfterLogin = null;
  let sendKind = 'files';   // 'files' | 'text' — set by openSendTextModal before upload

  /* Open the AirPortal-style 发送文本 modal. The text is queued as a single
   * .txt blob and submitted via the existing /api/upload path with kind=text. */
  function openSendTextModal() {
    document.getElementById('send-text-area').value = '';
    document.getElementById('send-text-modal').hidden = false;
    setTimeout(() => document.getElementById('send-text-area').focus(), 60);
  }

  function showReceiveTextModal(text) {
    document.getElementById('receive-text-area').value = text;
    document.getElementById('receive-text-modal').hidden = false;
  }

  async function fetchMe() {
    try {
      const r = await fetch('/api/auth/me', { credentials: 'same-origin' });
      if (r.ok) me = await r.json();
      else      me = null;
    } catch (_) { me = null; }
    refreshAuthUi();
  }

  function refreshAuthUi() {
    const subEl = document.getElementById('send-card-sub');
    if (me) {
      if (subEl) subEl.textContent = tr('signed-in-as', { user: me.username });
    } else {
      if (subEl) subEl.textContent = tr('not-signed-in');
    }
    // 我的文件 is ALWAYS visible — gated by login at click time, not by visibility.
    // Other menu items still toggle by auth state.
    document.querySelectorAll('#nav-menu [data-when]').forEach(el => {
      const want = el.dataset.when;
      el.hidden = (want === 'signed-in' && !me) || (want === 'signed-out' && me);
    });
    // Hide 续期账户 for accounts that never expire (admin OR is_perm).
    const renewBtn = document.querySelector('#nav-menu [data-action="renew"]');
    if (renewBtn) renewBtn.hidden = !me || !!me.is_admin || !!me.is_perm;
    // Admin sees "进入控制台" instead of "我的文件" in both the quick link and menu.
    const myFilesText = (me && me.is_admin) ? tr('admin-console') : tr('menu-my-files');
    const quickLabel = document.querySelector('#my-files-link span');
    const menuLabel  = document.querySelector('#nav-menu [data-action="my-files"] span');
    if (quickLabel) quickLabel.textContent = myFilesText;
    if (menuLabel)  menuLabel.textContent  = myFilesText;
    refreshMenuMeta();
  }

  function neverExpiresLabel() {
    if (!me) return '';
    if (me.is_admin) return tr('account-never-expires');
    if (me.is_perm)  return tr('account-perm-never-expires');
    return '';
  }

  function refreshMenuMeta() {
    const meta = document.getElementById('menu-meta');
    if (!meta) return;
    const parts = [];
    if (storageInfo) parts.push(`${fmtBytes(storageInfo.used_bytes)} / ${fmtBytes(storageInfo.max_bytes)}`);
    if (me) {
      parts.push(
        (me.is_admin || me.is_perm)
          ? neverExpiresLabel()
          : tr('account-expires', { date: fmtDate(me.account_expires_at) }));
    }
    meta.textContent = parts.join(' · ') || '—';
  }

  function toggleNavMenu(force) {
    const menu = document.getElementById('nav-menu');
    if (!menu) return;
    const want = (force !== undefined) ? force : menu.hidden;
    menu.hidden = !want;
  }

  /* ─────────────────────────────────────────────────────────
   * Storage gauge
   * ───────────────────────────────────────────────────────── */
  let storageInfo = null;

  async function refreshStorageGauge() {
    try {
      const r = await fetch('/api/storage');
      if (!r.ok) return;
      storageInfo = await r.json();
    } catch (_) { return; }

    const footerStorage = document.getElementById('footer-storage');
    if (footerStorage) {
      footerStorage.textContent = `${fmtBytes(storageInfo.used_bytes)} / ${fmtBytes(storageInfo.max_bytes)}`;
    }
    if (storageInfo.max_expiry_hours) {
      const ex = document.getElementById('cfg-expiry');
      if (ex) ex.max = storageInfo.max_expiry_hours;
    }
    refreshMenuMeta();
  }

  /* ─────────────────────────────────────────────────────────
   * Send queue
   * ───────────────────────────────────────────────────────── */
  const send = {
    files: [],
    cancelled: false,
    xhr: null,
  };

  const sendFileList = document.getElementById('send-file-list');
  const fileInput    = document.getElementById('file-input');
  const folderInput  = document.getElementById('folder-input');

  function renderSendList() {
    sendFileList.innerHTML = '';
    send.files.forEach((f, idx) => {
      const row = document.createElement('div');
      row.className = 'file-row';
      row.innerHTML = `
        <span class="file-icon">${fileExt(f.name)}</span>
        <span class="file-name" title="${escapeAttr(f.name)}">${escapeText(f.name)}</span>
        <span class="file-size">${fmtBytes(f.size)}</span>
        <button class="file-remove ripple-surface" type="button" aria-label="Remove">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>`;
      row.querySelector('.file-remove').addEventListener('click', (e) => {
        e.stopPropagation();
        send.files.splice(idx, 1);
        renderSendList();
      });
      sendFileList.appendChild(row);
    });
  }

  function addFilesFromList(list) {
    let added = 0;
    for (const f of list) {
      if (!f) continue;
      send.files.push({ name: f.name || tr('text-snippet'), type: f.type || 'application/octet-stream', size: f.size, blob: f });
      added++;
    }
    if (added > 0) { showStage('send', 'pick'); renderSendList(); }
    return added;
  }

  /* ─────────────────────────────────────────────────────────
   * Upload — real XHR with progress
   * ───────────────────────────────────────────────────────── */
  const uploadPct      = document.getElementById('upload-pct');
  const uploadBytes    = document.getElementById('upload-bytes');
  const uploadBar      = document.getElementById('upload-bar');
  const uploadFilename = document.getElementById('upload-filename');
  const uploadError    = document.getElementById('upload-error');

  function startUpload() {
    uploadError.hidden = true;
    if (send.files.length === 0) { showUploadError(tr('toast-no-files')); return; }

    const password   = document.getElementById('cfg-password').value.trim();
    const downloads  = Math.max(1, parseInt(document.getElementById('cfg-downloads').value, 10) || 1);
    const expiryHrs  = Math.max(1, parseInt(document.getElementById('cfg-expiry').value, 10) || 24);

    const totalBytes = send.files.reduce((s, f) => s + f.size, 0);
    if (storageInfo) {
      if (totalBytes > storageInfo.max_upload_bytes) {
        showUploadError(tr('err-parcel-too-large', { gb: (storageInfo.max_upload_bytes / 1073741824).toFixed(0) }));
        return;
      }
      if (storageInfo.used_bytes + totalBytes > storageInfo.max_bytes) {
        showUploadError(tr('err-storage-full'));
        return;
      }
    }

    const headline = send.files.length === 1
      ? send.files[0].name
      : (lang === 'zh' ? `${send.files.length} 个文件` : `${send.files.length} files`);
    uploadFilename.textContent = headline;
    uploadPct.textContent = '0';
    uploadBytes.textContent = `0 B / ${fmtBytes(totalBytes)}`;
    uploadBar.style.width = '0%';

    showStage('send', 'uploading');
    send.cancelled = false;

    const fd = new FormData();
    fd.append('password', password);
    fd.append('downloads', String(downloads));
    fd.append('expiry_hours', String(expiryHrs));
    fd.append('kind', sendKind);
    for (const f of send.files) fd.append('files', f.blob, f.name);

    const xhr = new XMLHttpRequest();
    send.xhr = xhr;
    xhr.open('POST', '/api/upload', true);
    xhr.withCredentials = true;

    xhr.upload.addEventListener('progress', (e) => {
      if (!e.lengthComputable) return;
      const p = e.loaded / e.total;
      uploadPct.textContent = Math.floor(p * 100);
      uploadBar.style.width = (p * 100).toFixed(1) + '%';
      uploadBytes.textContent = `${fmtBytes(e.loaded)} / ${fmtBytes(e.total)}`;
    });

    xhr.addEventListener('load', () => {
      send.xhr = null;
      if (send.cancelled) return;
      let body = null;
      try { body = JSON.parse(xhr.responseText); } catch (_) {}

      if (xhr.status === 200 && body && body.code) {
        refreshStorageGauge();
        revealSuccess(body.code);
        return;
      }
      let msg = tr('err-network');
      if      (body && body.error === 'storage_full')      msg = tr('err-storage-full');
      else if (body && body.error === 'file_too_large')    msg = tr('err-file-too-large',   { gb: (body.max_bytes / 1073741824).toFixed(0) });
      else if (body && body.error === 'parcel_too_large')  msg = tr('err-parcel-too-large', { gb: (body.max_bytes / 1073741824).toFixed(0) });
      goBackToPickWithError(msg);
    });
    xhr.addEventListener('error', () => {
      send.xhr = null;
      if (!send.cancelled) goBackToPickWithError(tr('err-network'));
    });
    xhr.addEventListener('abort', () => { send.xhr = null; });

    xhr.send(fd);
  }

  function goBackToPickWithError(msg) {
    showStage('send', 'pick');
    showUploadError(msg);
  }
  function showUploadError(msg) {
    uploadError.textContent = msg;
    uploadError.hidden = false;
  }

  /* ─────────────────────────────────────────────────────────
   * Success view + share
   * ───────────────────────────────────────────────────────── */
  const pickupCodeEl  = document.getElementById('pickup-code');
  const qrPanel       = document.getElementById('qr-panel');
  const qrCanvas      = document.getElementById('qr-canvas');
  const successShare  = document.getElementById('success-share-url');
  let lastCode = '';

  function revealSuccess(code) {
    lastCode = code;
    pickupCodeEl.textContent = code.split('').join(' ');
    qrPanel.hidden = true;
    successShare.textContent = shareUrl(code);
    showStage('send', 'success');
  }

  function shareUrl(code) {
    const url = new URL(window.location.href);
    url.search = '';
    url.hash = '';
    url.pathname = '/d/' + code;
    return url.toString();
  }

  async function copyShareLink() {
    if (!lastCode) return;
    const url = shareUrl(lastCode);
    try {
      await navigator.clipboard.writeText(url);
      toast(tr('toast-copied'));
    } catch (_) {
      const t = document.createElement('textarea');
      t.value = url;
      document.body.appendChild(t);
      t.select();
      try { document.execCommand('copy'); toast(tr('toast-copied')); } catch (_) {}
      t.remove();
    }
  }

  let qrLibPromise = null;
  function ensureQrLib() {
    if (qrLibPromise) return qrLibPromise;
    qrLibPromise = new Promise((resolve, reject) => {
      const s = document.createElement('script');
      s.src = 'https://cdn.jsdelivr.net/npm/qrcode-generator@1.4.4/qrcode.min.js';
      s.onload  = () => resolve(window.qrcode);
      s.onerror = () => reject(new Error('qr lib failed'));
      document.head.appendChild(s);
    });
    return qrLibPromise;
  }

  async function renderQr() {
    if (!lastCode) return;
    if (!qrPanel.hidden) { qrPanel.hidden = true; return; }
    try {
      const qrcode = await ensureQrLib();
      const qr = qrcode(0, 'M');
      qr.addData(shareUrl(lastCode));
      qr.make();
      const ctx = qrCanvas.getContext('2d');
      const moduleCount = qr.getModuleCount();
      const size = 180, padding = 8;
      const cell = (size - padding * 2) / moduleCount;
      ctx.fillStyle = '#ffffff'; ctx.fillRect(0, 0, size, size);
      ctx.fillStyle = '#1f1f1e';
      for (let r = 0; r < moduleCount; r++) {
        for (let c = 0; c < moduleCount; c++) {
          if (qr.isDark(r, c)) ctx.fillRect(padding + c * cell, padding + r * cell, cell, cell);
        }
      }
      qrPanel.hidden = false;
    } catch (_) { toast('QR unavailable'); }
  }

  /* ─────────────────────────────────────────────────────────
   * Receive flow
   * ───────────────────────────────────────────────────────── */
  const codeInputs           = Array.from(document.querySelectorAll('.code-digit'));
  const codeError            = document.getElementById('code-error');
  const codePasswordRow      = document.getElementById('code-password-row');
  const receivePasswordInput = document.getElementById('receive-password');
  const receiveFileList      = document.getElementById('receive-file-list');
  const receiveMeta          = document.getElementById('receive-meta');

  let currentReceive = null;

  function readCode() { return codeInputs.map(i => i.value || '').join(''); }
  function setCode(code) {
    codeInputs.forEach((inp, idx) => {
      inp.value = code[idx] || '';
      inp.classList.toggle('is-filled', !!inp.value);
    });
  }
  function clearCodeInputs() {
    codeInputs.forEach(inp => { inp.value = ''; inp.classList.remove('is-filled'); });
    codeError.hidden = true;
    codePasswordRow.hidden = true;
    receivePasswordInput.value = '';
  }
  function showCodeError(msg) { codeError.textContent = msg; codeError.hidden = false; }

  codeInputs.forEach((inp, idx) => {
    inp.addEventListener('input', () => {
      const v = inp.value.replace(/\D/g, '').slice(0, 1);
      inp.value = v;
      inp.classList.toggle('is-filled', !!v);
      if (v && idx < codeInputs.length - 1) codeInputs[idx + 1].focus();
      if (readCode().length === 6) attemptFetch();
    });
    inp.addEventListener('keydown', (e) => {
      if (e.key === 'Backspace' && !inp.value && idx > 0) {
        codeInputs[idx - 1].focus();
        codeInputs[idx - 1].value = '';
        codeInputs[idx - 1].classList.remove('is-filled');
      }
      if (e.key === 'ArrowLeft' && idx > 0) codeInputs[idx - 1].focus();
      if (e.key === 'ArrowRight' && idx < codeInputs.length - 1) codeInputs[idx + 1].focus();
    });
    inp.addEventListener('paste', (e) => {
      const txt = (e.clipboardData || window.clipboardData).getData('text');
      const digits = (txt || '').replace(/\D/g, '').slice(0, 6);
      if (digits.length > 0) {
        e.preventDefault();
        setCode(digits.padEnd(6, ''));
        if (digits.length === 6) attemptFetch();
        else codeInputs[Math.min(digits.length, 5)].focus();
      }
    });
  });

  function mapParcelError(err) {
    switch (err) {
      case 'not_found':       return tr('err-not-found');
      case 'expired':         return tr('err-expired');
      case 'no_downloads':    return tr('err-no-downloads');
      case 'bad_password':    return tr('err-bad-password');
      case 'needs_password':  return tr('err-needs-password');
      default:                return tr('err-network');
    }
  }

  async function safeJson(r) {
    try { return await r.json(); } catch (_) { return null; }
  }

  async function attemptFetch() {
    const code = readCode();
    if (code.length !== 6) return;
    codeError.hidden = true;
    const password = receivePasswordInput.value.trim();

    let infoRes;
    try {
      infoRes = await fetch('/api/parcel/' + code + '/info', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ password }),
      });
    } catch (_) { showCodeError(tr('err-network')); return; }

    const body = await safeJson(infoRes);
    if (!infoRes.ok) {
      if (body && body.error === 'needs_password' && codePasswordRow.hidden) {
        codePasswordRow.hidden = false;
        receivePasswordInput.focus();
        return;
      }
      showCodeError(mapParcelError(body && body.error));
      return;
    }

    let claim;
    try {
      const r = await fetch('/api/parcel/' + code + '/claim', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ password }),
      });
      claim = await safeJson(r);
      if (!r.ok) { showCodeError(mapParcelError(claim && claim.error)); return; }
    } catch (_) { showCodeError(tr('err-network')); return; }

    currentReceive = { code, token: claim.token, files: body.files, kind: body.kind || 'files' };

    // If this is a text parcel, fetch its content and show in a dedicated modal
    // instead of routing the user through the file-list download flow.
    if (currentReceive.kind === 'text' && body.files.length === 1) {
      try {
        const url = `/api/parcel/${code}/file/${body.files[0].id}?token=${encodeURIComponent(claim.token)}`;
        const txt = await (await fetch(url)).text();
        showReceiveTextModal(txt);
        clearCodeInputs();
        showStage('landing');
        return;
      } catch (_) { /* fall through to file-list view */ }
    }

    receiveMeta.textContent = tr('remaining', { n: claim.downloads_left });
    receiveFileList.innerHTML = '';
    body.files.forEach((f) => {
      const row = document.createElement('div');
      row.className = 'file-row';
      row.innerHTML = `
        <span class="file-icon">${fileExt(f.name)}</span>
        <span class="file-name" title="${escapeAttr(f.name)}">${escapeText(f.name)}</span>
        <span class="file-size">${fmtBytes(f.size)}</span>
        <button class="file-remove ripple-surface" type="button" aria-label="Download">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
        </button>`;
      const trigger = () => triggerDownload(f);
      row.querySelector('.file-remove').addEventListener('click', (e) => { e.stopPropagation(); trigger(); });
      row.addEventListener('click', trigger);
      receiveFileList.appendChild(row);
    });

    showStage('receive', 'list');
  }

  function triggerDownload(f) {
    if (!currentReceive) return;
    const url = `/api/parcel/${currentReceive.code}/file/${f.id}?token=${encodeURIComponent(currentReceive.token)}`;
    const a = document.createElement('a');
    a.href = url;
    a.download = f.name;
    document.body.appendChild(a);
    a.click();
    a.remove();
  }

  /* ─────────────────────────────────────────────────────────
   * Drag-drop + paste
   * ───────────────────────────────────────────────────────── */
  const dropOverlay = document.getElementById('drop-overlay');
  let dragDepth = 0;

  window.addEventListener('dragenter', (e) => {
    if (!e.dataTransfer || !Array.from(e.dataTransfer.types).includes('Files')) return;
    e.preventDefault();
    dragDepth++;
    dropOverlay.classList.add('is-active');
  });
  window.addEventListener('dragover', (e) => {
    if (!e.dataTransfer || !Array.from(e.dataTransfer.types).includes('Files')) return;
    e.preventDefault();
  });
  window.addEventListener('dragleave', () => {
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) dropOverlay.classList.remove('is-active');
  });
  window.addEventListener('drop', (e) => {
    if (!e.dataTransfer) return;
    e.preventDefault();
    dragDepth = 0;
    dropOverlay.classList.remove('is-active');
    if (e.dataTransfer.files && e.dataTransfer.files.length) addFilesFromList(e.dataTransfer.files);
  });

  window.addEventListener('paste', (e) => {
    const t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA')) return;
    const items = e.clipboardData && e.clipboardData.items;
    if (!items) return;
    const files = [];
    for (const it of items) {
      if (it.kind === 'file') { const f = it.getAsFile(); if (f) files.push(f); }
    }
    if (files.length) { addFilesFromList(files); return; }
    const text = e.clipboardData.getData('text');
    if (text && text.trim()) {
      const name = tr('text-snippet') + '.txt';
      const blob = new Blob([text], { type: 'text/plain' });
      send.files.push({ name, type: 'text/plain', size: blob.size, blob });
      showStage('send', 'pick');
      renderSendList();
      toast(tr('sent-clipboard'));
    }
  });

  /* ─────────────────────────────────────────────────────────
   * Wiring
   * ───────────────────────────────────────────────────────── */
  document.getElementById('btn-send').addEventListener('click', () => {
    uploadError.hidden = true;
    // If the user already has files queued (from a drop/paste), jump to the
    // card. Otherwise just pop the OS picker and stay on the landing — the
    // card only appears AFTER files arrive (via addFilesFromList → showStage).
    if (send.files.length > 0) {
      showStage('send', 'pick');
      renderSendList();
    } else {
      fileInput.click();
    }
  });
  document.getElementById('btn-receive').addEventListener('click', () => {
    clearCodeInputs();
    showStage('receive', 'code');
    setTimeout(() => codeInputs[0].focus(), 60);
  });

  document.querySelectorAll('.hint-chip').forEach(chip => {
    chip.addEventListener('click', (e) => {
      e.stopPropagation();
      const pick = chip.dataset.pick;
      if (pick === 'files')      fileInput.click();
      else if (pick === 'folder') folderInput.click();
      else if (pick === 'text')   openSendTextModal();
    });
  });

  fileInput.addEventListener('change', () => {
    if (fileInput.files && fileInput.files.length) addFilesFromList(fileInput.files);
    fileInput.value = '';
  });
  folderInput.addEventListener('change', () => {
    if (folderInput.files && folderInput.files.length) addFilesFromList(folderInput.files);
    folderInput.value = '';
  });

  document.addEventListener('click', (e) => {
    const action = e.target.closest('[data-action]');
    if (!action) return;
    const a = action.getAttribute('data-action');
    switch (a) {
      case 'cancel':
      case 'reset':
        send.files = []; renderSendList();
        clearCodeInputs();
        showStage('landing');
        break;
      case 'cancel-upload':
        if (send.xhr) { send.cancelled = true; send.xhr.abort(); send.xhr = null; }
        send.files = []; renderSendList();
        showStage('landing');
        break;
      case 'login-close':
        document.getElementById('login-modal').hidden = true;
        break;
      case 'send-text-close':
        document.getElementById('send-text-modal').hidden = true;
        break;
      case 'receive-text-close':
        document.getElementById('receive-text-modal').hidden = true;
        showStage('landing');
        break;
      case 'login':
        toggleNavMenu(false);
        openLoginModal();
        break;
      case 'my-files':
        toggleNavMenu(false);
        if (me && me.is_admin) { window.location.href = '/admin'; break; }
        if (me) {
          loadMyFiles();
        } else {
          pendingAfterLogin = 'my-files';
          openLoginModal();
        }
        break;
      case 'renew':
        toggleNavMenu(false);
        renewAccount();
        break;
      case 'logout':
        toggleNavMenu(false);
        logout();
        break;
    }
  });

  // Standalone language toggle (sits left of the hamburger)
  document.getElementById('lang-toggle').addEventListener('click', () => {
    lang = lang === 'zh' ? 'en' : 'zh';
    applyLang();
  });

  // Text send modal submit
  document.getElementById('btn-send-text').addEventListener('click', () => {
    const text = document.getElementById('send-text-area').value;
    if (!text || !text.trim()) { toast(tr('toast-empty-text')); return; }
    const name = (tr('text-snippet') || 'clipboard') + '.txt';
    const blob = new Blob([text], { type: 'text/plain' });
    // Replace any pending file queue with this one text blob and mark kind=text
    send.files = [{ name, type: 'text/plain', size: blob.size, blob }];
    sendKind = 'text';
    document.getElementById('send-text-modal').hidden = true;
    // Reset to default config inputs that the text flow doesn't expose
    document.getElementById('cfg-password').value = '';
    document.getElementById('cfg-downloads').value = 2;
    document.getElementById('cfg-expiry').value = 24;
    startUpload();
  });

  // Copy button on the receive-text modal
  document.getElementById('btn-copy-text').addEventListener('click', async () => {
    const text = document.getElementById('receive-text-area').value;
    try { await navigator.clipboard.writeText(text); toast(tr('toast-copied')); }
    catch (_) {
      const ta = document.getElementById('receive-text-area');
      ta.select();
      try { document.execCommand('copy'); toast(tr('toast-copied')); } catch (_) {}
    }
  });

  // Reset sendKind back to files whenever we cancel/start fresh
  document.addEventListener('click', (e) => {
    const a = e.target.closest('[data-action]');
    if (a && (a.dataset.action === 'cancel' || a.dataset.action === 'reset' || a.dataset.action === 'cancel-upload')) {
      sendKind = 'files';
    }
  });

  function openLoginModal() {
    document.getElementById('login-modal').hidden = false;
    document.getElementById('login-error').hidden = true;
    setTimeout(() => document.getElementById('login-username').focus(), 60);
  }

  document.getElementById('btn-upload').addEventListener('click', startUpload);
  document.getElementById('btn-fetch').addEventListener('click', attemptFetch);
  document.getElementById('btn-copy-link').addEventListener('click', copyShareLink);
  document.getElementById('btn-show-qr').addEventListener('click', renderQr);
  document.getElementById('btn-download-all').addEventListener('click', () => {
    if (!currentReceive) return;
    currentReceive.files.forEach((f, i) => setTimeout(() => triggerDownload(f), i * 200));
  });

  // Hamburger menu open/close
  document.getElementById('menu-btn').addEventListener('click', (e) => {
    e.stopPropagation();
    toggleNavMenu();
  });
  document.addEventListener('click', (e) => {
    if (!e.target.closest('#menu-btn') && !e.target.closest('#nav-menu')) toggleNavMenu(false);
  });

  // "我的文件" — always visible. Behavior splits on auth state:
  //   admin    → /admin (the button reads "进入控制台")
  //   signed-in user → loadMyFiles()
  //   anonymous → open login modal, auto-jump after login
  document.getElementById('my-files-link').addEventListener('click', () => {
    if (me && me.is_admin) { window.location.href = '/admin'; return; }
    if (me) {
      loadMyFiles();
    } else {
      pendingAfterLogin = 'my-files';
      openLoginModal();
    }
  });

  document.getElementById('btn-login-submit').addEventListener('click', login);
  ['login-username', 'login-password'].forEach(id => {
    document.getElementById(id).addEventListener('keydown', (e) => { if (e.key === 'Enter') login(); });
  });

  async function login() {
    const u = document.getElementById('login-username').value.trim();
    const p = document.getElementById('login-password').value;
    const err = document.getElementById('login-error');
    if (!u) { err.textContent = tr('err-username-required'); err.hidden = false; return; }
    if (!p) { err.textContent = tr('err-password-required'); err.hidden = false; return; }
    err.hidden = true;
    try {
      const r = await fetch('/api/auth/login', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ username: u, password: p }),
      });
      const body = await safeJson(r);
      if (!r.ok) {
        err.textContent = (body && body.error === 'wrong password') ? tr('err-wrong-password')
                         : (body && body.error) || tr('err-network');
        err.hidden = false;
        return;
      }
      me = body;
      refreshAuthUi();
      document.getElementById('login-modal').hidden = true;
      document.getElementById('login-password').value = '';

      // If the modal was opened because the user tried to use a gated feature
      // while signed out, run that pending action now.
      const next = pendingAfterLogin;
      pendingAfterLogin = null;
      if (next === 'my-files') loadMyFiles();
    } catch (_) {
      err.textContent = tr('err-network');
      err.hidden = false;
    }
  }

  async function logout() {
    try { await fetch('/api/auth/logout', { method: 'POST', credentials: 'same-origin' }); } catch (_) {}
    me = null;
    refreshAuthUi();
    showStage('landing');
  }

  async function renewAccount() {
    try {
      const r = await fetch('/api/auth/renew', { method: 'POST', credentials: 'same-origin' });
      const body = await safeJson(r);
      if (r.ok && body) {
        me = Object.assign({}, me, { account_expires_at: body.account_expires_at });
        toast(tr('toast-renewed'));
        refreshAuthUi();
      }
    } catch (_) {}
  }

  async function loadMyFiles() {
    try {
      const r = await fetch('/api/me/parcels', { credentials: 'same-origin' });
      if (!r.ok) { showStage('landing'); return; }
      const body = await r.json();
      renderMyFiles(body.parcels || []);
      showStage('my-files');
    } catch (_) {}
  }

  function renderMyFiles(parcels) {
    const list = document.getElementById('my-files-list');
    const meta = document.getElementById('my-files-meta');
    const expiresLine = me
      ? ((me.is_admin || me.is_perm)
          ? neverExpiresLabel()
          : tr('account-expires', { date: fmtDate(me.account_expires_at) }))
      : '';
    if (parcels.length === 0) {
      meta.textContent = tr('my-files-empty') + (expiresLine ? ' · ' + expiresLine : '');
      list.innerHTML = '';
      return;
    }
    const used = parcels.reduce((s, p) => s + p.total_bytes, 0);
    meta.textContent = tr('my-files-meta', { n: parcels.length, used: fmtBytes(used) }) + ' · ' + expiresLine;

    list.innerHTML = '';
    const now = Date.now();
    parcels.forEach(p => {
      const row = document.createElement('div');
      row.className = 'parcel-row';
      const hLeft = (p.expires_at - now) / 3600000;
      const expiry = hLeft < 0
        ? tr('file-row-expired')
        : tr('file-row-expires-in', { h: Math.max(1, Math.ceil(hLeft)) });
      row.innerHTML = `
        <div class="parcel-row-main">
          <div class="parcel-code-cell">
            <span class="parcel-code-label">${p.code.split('').join(' ')}</span>
            ${p.has_password ? '<span class="parcel-lock" title="password">🔒</span>' : ''}
          </div>
          <div class="parcel-meta">${escapeText(tr('file-row-meta', { size: fmtBytes(p.total_bytes), n: p.downloads_left, expiry }))}</div>
        </div>
        <button class="parcel-delete ripple-surface" type="button">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-2 14a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/></svg>
          <span>${tr('btn-delete-parcel')}</span>
        </button>`;
      row.querySelector('.parcel-delete').addEventListener('click', async () => {
        try {
          const r = await fetch('/api/me/parcels/' + p.code, { method: 'DELETE', credentials: 'same-origin' });
          if (r.ok) {
            toast(tr('toast-deleted'));
            loadMyFiles();
            refreshStorageGauge();
          }
        } catch (_) {}
      });
      list.appendChild(row);
    });
  }

  /* ─────────────────────────────────────────────────────────
   * Ripples + Toast
   * ───────────────────────────────────────────────────────── */
  function createRipple(event) {
    const btn = event.currentTarget;
    const circle = document.createElement('span');
    const diameter = Math.max(btn.clientWidth, btn.clientHeight);
    const radius = diameter / 2;
    const rect = btn.getBoundingClientRect();
    circle.style.width = circle.style.height = `${diameter}px`;
    circle.style.left = `${event.clientX - rect.left - radius}px`;
    circle.style.top  = `${event.clientY - rect.top  - radius}px`;
    circle.classList.add('ripple');
    const existing = btn.querySelector('.ripple');
    if (existing) existing.remove();
    btn.appendChild(circle);
    // Remove the span once the animation finishes so a later `display:none → block`
    // toggle on a parent (e.g. closing then re-opening the hamburger menu) doesn't
    // re-trigger the CSS animation on a lingering ripple. animationend fires when
    // the parent is visible; the setTimeout is the safety net for the case where
    // the parent got hidden mid-animation.
    const cleanup = () => { try { circle.remove(); } catch (_) {} };
    circle.addEventListener('animationend', cleanup);
    setTimeout(cleanup, 800);
  }
  function bindRipples() {
    document.querySelectorAll('.ripple-surface').forEach(el => {
      if (el.dataset.rippleBound) return;
      el.dataset.rippleBound = '1';
      el.addEventListener('touchstart', (e) => {
        el._lastTouch = Date.now();
        if (e.touches[0]) createRipple({ currentTarget: el, clientX: e.touches[0].clientX, clientY: e.touches[0].clientY });
      }, { passive: true });
      el.addEventListener('mousedown', (e) => {
        if (Date.now() - (el._lastTouch || 0) < 500) return;
        createRipple(e);
      });
    });
  }

  let toastEl = null;
  let toastTimer = null;
  function toast(msg) {
    if (!toastEl) { toastEl = document.createElement('div'); toastEl.id = 'toast'; document.body.appendChild(toastEl); }
    toastEl.textContent = msg;
    toastEl.classList.add('is-visible');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toastEl.classList.remove('is-visible'), 1800);
  }

  /* ─────────────────────────────────────────────────────────
   * Boot
   * ───────────────────────────────────────────────────────── */
  applyLang();
  bindRipples();
  new MutationObserver(bindRipples).observe(document.body, { childList: true, subtree: true });

  fetchMe().then(refreshStorageGauge);

  // Deep link: /d/CODE redirects server-side to /?c=NNNNNN; either route auto-fetches.
  const params = new URLSearchParams(window.location.search);
  const cParam = params.get('c');
  if (cParam && /^\d{6}$/.test(cParam)) {
    setCode(cParam);
    showStage('receive', 'code');
    setTimeout(attemptFetch, 60);
  }
})();
