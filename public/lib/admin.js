/* ============================================================
 * open.drop — admin.js
 * Admin SPA: login + stats / parcels / accounts / config / password
 * Talks only to /api/admin/*; reuses the regular session cookie.
 * ============================================================ */

(() => {
  'use strict';

  /* ─── i18n ────────────────────────────────────────────── */
  const T = {
    zh: {
      'nav-back-to-app': '返回站点',
      'nav-logout':      '登出',
      'login-title':     '管理员登录',
      'login-sub':       '默认账号 / 密码均为 <code>admin</code>，登录后请尽快修改',
      'cfg-username':    '用户名',
      'cfg-password':    '密码',
      'btn-login':       '登录',
      'tab-stats':       '概览',
      'tab-parcels':     '所有包裹',
      'tab-accounts':    '所有账号',
      'tab-config':      '配置',
      'tab-password':    '修改密码',
      'stats-title':     '系统概览',
      'loading':         '— 加载中 —',
      'parcels-title':   '所有包裹',
      'th-code':         '取件码',
      'th-owner':        '所有者',
      'th-kind':         '类型',
      'th-size':         '大小',
      'th-downloads':    '剩余下载',
      'th-expires':      '过期时间',
      'th-actions':      '操作',
      'th-username':     '用户名',
      'th-parcels':      '包裹数',
      'th-storage':      '占用',
      'th-account-expires': '账户到期',
      'accounts-title':  '所有账号',
      'config-title':    '限额配置',
      'config-sub':      '可在线修改，无需重启。<br>单次上传上限受启动时环境变量约束（最高 <span id="hard-ceiling">—</span> GB），可下调不可上调。',
      'cfg-max-storage': '系统总存储 (GB)',
      'cfg-max-upload':  '单次上传 (GB)',
      'cfg-max-expiry':  '最长有效期（小时）',
      'btn-save':        '保存',
      'password-title':  '修改管理员账号',
      'password-sub':    '用户名留空则保持不变；密码必填，新密码即生效',
      'new-username':    '新用户名（可选）',
      'current-password':'当前密码',
      'new-password':    '新密码',
      'stat-used':       '已用存储',
      'stat-parcels':    '包裹数',
      'stat-accounts':   '用户账号数',
      'stat-max-upload': '单次上传上限',
      'stat-max-expiry': '最长有效期',
      'stat-hard-ceil':  '环境硬上限',
      'meta-parcels':    '共 {n} 个包裹 · 总占用 {used}',
      'meta-accounts':   '共 {n} 个用户账号',
      'cell-text-kind':  '文本',
      'cell-files-kind': '{n} 文件',
      'cell-empty':      '无',
      'cell-loading':    '加载中…',
      'btn-delete':      '删除',
      'btn-renew':       '续期 7 天',
      'confirm-del-parcel': '删除包裹 {code} ？这会立刻销毁里面的文件。',
      'confirm-del-account':'删除账户 #{id} ？这会同时销毁该账户的所有包裹。',
      'err-fill-login':  '请输入用户名和密码',
      'err-login-fail':  '登录失败',
      'err-not-admin':   '该账户不是管理员',
      'err-net':         '网络错误',
      'err-fill-pw':     '请填写当前密码和新密码',
      'err-wrong-pw':    '当前密码错误',
      'err-name-taken':  '该用户名已被占用',
      'err-save-fail':   '保存失败',
      'toast-deleted':   '已删除',
      'toast-renewed':   '已续期',
      'toast-saved':     '已保存',
      'toast-updated':   '已修改',
      'btn-edit':        '修改',
      'btn-save':        '保存',
      'btn-mark-permanent': '永久',
      'btn-mark-unlimited': '无限',
      'edit-parcel-title':     '编辑包裹',
      'edit-parcel-meta-line': '取件码 {code} · 所有者 {owner}',
      'edit-expiry':           '有效期 (小时)',
      'edit-downloads':        '剩余下载次数',
      'edit-password':         '密码',
      'edit-password-placeholder-with':    '已设置 · 留空保持不变',
      'edit-password-placeholder-without': '未设置 · 留空表示无密码',
      'btn-show':              '显示',
      'btn-hide':              '隐藏',
      'btn-clear-password':    '清除',
      'err-net':               '网络错误，请重试',
      'cell-permanent':        '永久',
      'cell-unlimited':        '无限',
      'err-save-fail':         '保存失败',
      'toast-perm-on':   '已标记为长期账户',
      'toast-perm-off':  '已取消长期标记',
      'h':               '小时',
      'btn-mark-perm':   '标记长期',
      'btn-unmark-perm': '取消长期',
      'perm-badge':      '长期',
      'cell-never-expires': '永不过期',
    },
    en: {
      'nav-back-to-app': 'Back to site',
      'nav-logout':      'Sign out',
      'login-title':     'Admin sign in',
      'login-sub':       'Default username / password are both <code>admin</code> — change them after first sign-in',
      'cfg-username':    'Username',
      'cfg-password':    'Password',
      'btn-login':       'Sign in',
      'tab-stats':       'Overview',
      'tab-parcels':     'All parcels',
      'tab-accounts':    'All accounts',
      'tab-config':      'Config',
      'tab-password':    'Change password',
      'stats-title':     'System overview',
      'loading':         '— loading —',
      'parcels-title':   'All parcels',
      'th-code':         'Code',
      'th-owner':        'Owner',
      'th-kind':         'Kind',
      'th-size':         'Size',
      'th-downloads':    'Downloads left',
      'th-expires':      'Expires',
      'th-actions':      'Actions',
      'th-username':     'Username',
      'th-parcels':      'Parcels',
      'th-storage':      'Storage',
      'th-account-expires': 'Account expires',
      'accounts-title':  'All accounts',
      'config-title':    'Limits',
      'config-sub':      'Editable at runtime, no restart.<br>Per-upload cap is bounded by the startup env var (max <span id="hard-ceiling">—</span> GB); can be lowered, not raised.',
      'cfg-max-storage': 'System storage (GB)',
      'cfg-max-upload':  'Per upload (GB)',
      'cfg-max-expiry':  'Max expiry (hours)',
      'btn-save':        'Save',
      'password-title':  'Admin credentials',
      'password-sub':    'Leave username blank to keep it; password is required, takes effect immediately',
      'new-username':    'New username (optional)',
      'current-password':'Current password',
      'new-password':    'New password',
      'stat-used':       'Used storage',
      'stat-parcels':    'Parcels',
      'stat-accounts':   'User accounts',
      'stat-max-upload': 'Per-upload cap',
      'stat-max-expiry': 'Max expiry',
      'stat-hard-ceil':  'Env hard ceiling',
      'meta-parcels':    '{n} parcel(s) · {used} total',
      'meta-accounts':   '{n} user account(s)',
      'cell-text-kind':  'text',
      'cell-files-kind': '{n} file(s)',
      'cell-empty':      'empty',
      'cell-loading':    'loading…',
      'btn-delete':      'Delete',
      'btn-renew':       'Renew 7d',
      'confirm-del-parcel': 'Delete parcel {code}? Its files will be destroyed immediately.',
      'confirm-del-account':'Delete account #{id}? All of its parcels will also be removed.',
      'err-fill-login':  'Please enter username and password',
      'err-login-fail':  'Sign-in failed',
      'err-not-admin':   'This account is not an admin',
      'err-net':         'Network error',
      'err-fill-pw':     'Please fill current and new password',
      'err-wrong-pw':    'Current password is wrong',
      'err-name-taken':  'Username already taken',
      'err-save-fail':   'Save failed',
      'toast-deleted':   'Deleted',
      'toast-renewed':   'Renewed',
      'toast-saved':     'Saved',
      'toast-updated':   'Updated',
      'btn-edit':        'Edit',
      'btn-save':        'Save',
      'btn-mark-permanent': 'Permanent',
      'btn-mark-unlimited': 'Unlimited',
      'edit-parcel-title':     'Edit parcel',
      'edit-parcel-meta-line': 'Code {code} · owner {owner}',
      'edit-expiry':           'Expiry (hours)',
      'edit-downloads':        'Downloads remaining',
      'edit-password':         'Password',
      'edit-password-placeholder-with':    'Set · leave blank to keep',
      'edit-password-placeholder-without': 'None · leave blank for open access',
      'btn-show':              'Show',
      'btn-hide':              'Hide',
      'btn-clear-password':    'Clear',
      'err-net':               'Network error — please retry',
      'cell-permanent':        'permanent',
      'cell-unlimited':        'unlimited',
      'err-save-fail':         'Save failed',
      'toast-perm-on':   'Marked as long-term account',
      'toast-perm-off':  'Long-term flag removed',
      'h':               'h',
      'btn-mark-perm':   'Mark long-term',
      'btn-unmark-perm': 'Unmark long-term',
      'perm-badge':      'long-term',
      'cell-never-expires': 'never expires',
    },
  };

  let lang = (navigator.language || 'en').toLowerCase().includes('zh') ? 'zh' : 'en';

  function tr(key, vars) {
    let s = (T[lang] && T[lang][key]) || T.en[key] || key;
    if (vars) for (const k in vars) s = s.replace('{' + k + '}', vars[k]);
    return s;
  }
  function applyLang() {
    // HTML pass first so any [data-i18n] children injected via innerHTML
    // (e.g. nested labels inside translated blocks) get filled by the text
    // pass below. Placeholder pass picks up <input data-i18n-placeholder="...">.
    document.querySelectorAll('[data-i18n-html]').forEach(el => { el.innerHTML = tr(el.getAttribute('data-i18n-html')); });
    document.querySelectorAll('[data-i18n]').forEach(el => { el.textContent = tr(el.getAttribute('data-i18n')); });
    document.querySelectorAll('[data-i18n-placeholder]').forEach(el => { el.setAttribute('placeholder', tr(el.getAttribute('data-i18n-placeholder'))); });
    document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en';
    const active = document.querySelector('.admin-tab.is-active');
    if (active) refreshPanel(active.dataset.tab);
  }

  /* ─── Utilities ───────────────────────────────────────── */
  function fmtBytes(n) {
    if (n == null) return '—';
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
    return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
  function fmtDate(ms) {
    const d = new Date(ms);
    const z = (n) => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${z(d.getMonth() + 1)}-${z(d.getDate())} ${z(d.getHours())}:${z(d.getMinutes())}`;
  }
  function escapeText(s) {
    return String(s).replace(/[&<>]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));
  }
  async function safeJson(r) { try { return await r.json(); } catch (_) { return null; } }

  /* ─── Toast ──────────────────────────────────────────── */
  let toastEl = null, toastTimer = null;
  function toast(msg) {
    if (!toastEl) { toastEl = document.createElement('div'); toastEl.id = 'toast'; document.body.appendChild(toastEl); }
    toastEl.textContent = msg;
    toastEl.classList.add('is-visible');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toastEl.classList.remove('is-visible'), 1800);
  }

  /* ─── Ripple binding ─────────────────────────────────── */
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
    // Remove the span as soon as the animation finishes so a later
    // display:none → block toggle on a parent doesn't replay the animation
    // on a lingering ripple element.
    const cleanup = () => { try { circle.remove(); } catch (_) {} };
    circle.addEventListener('animationend', cleanup);
    setTimeout(cleanup, 800);
  }
  function bindRipples() {
    document.querySelectorAll('.ripple-surface').forEach(el => {
      if (el.dataset.rippleBound) return;
      el.dataset.rippleBound = '1';
      el.addEventListener('mousedown', createRipple);
    });
  }
  new MutationObserver(bindRipples).observe(document.body, { childList: true, subtree: true });

  /* ─── Stage / tab routing ─────────────────────────────── */
  function showStage(name) {
    document.querySelectorAll('.admin-stage').forEach(s => s.classList.toggle('is-active', s.id === `admin-${name}`));
  }
  function showPanel(tab) {
    document.querySelectorAll('.admin-tab').forEach(t => t.classList.toggle('is-active', t.dataset.tab === tab));
    document.querySelectorAll('.admin-panel').forEach(p => p.classList.toggle('is-active', p.dataset.panel === tab));
    refreshPanel(tab);
  }

  /* ─── Auth ────────────────────────────────────────────── */
  async function checkAdmin() {
    try {
      const r = await fetch('/api/admin/me', { credentials: 'same-origin' });
      if (r.ok) {
        const me = await r.json();
        document.getElementById('admin-logout').hidden = false;
        showStage('dashboard');
        refreshPanel('stats');
        return me;
      }
    } catch (_) {}
    document.getElementById('admin-logout').hidden = true;
    showStage('login');
    return null;
  }

  document.getElementById('admin-login-submit').addEventListener('click', login);
  ['admin-user', 'admin-pass'].forEach(id => {
    document.getElementById(id).addEventListener('keydown', e => { if (e.key === 'Enter') login(); });
  });

  async function login() {
    const u = document.getElementById('admin-user').value.trim();
    const p = document.getElementById('admin-pass').value;
    const err = document.getElementById('admin-login-err');
    if (!u || !p) { err.textContent = tr('err-fill-login'); err.hidden = false; return; }
    err.hidden = true;
    try {
      const r = await fetch('/api/auth/login', {
        method: 'POST', credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ username: u, password: p }),
      });
      const body = await safeJson(r);
      if (!r.ok) { err.textContent = (body && body.error) || tr('err-login-fail'); err.hidden = false; return; }
      const me = await checkAdmin();
      if (!me) {
        err.textContent = tr('err-not-admin');
        err.hidden = false;
        await fetch('/api/auth/logout', { method: 'POST', credentials: 'same-origin' });
      } else {
        document.getElementById('admin-pass').value = '';
      }
    } catch (_) {
      err.textContent = tr('err-net');
      err.hidden = false;
    }
  }

  document.getElementById('admin-logout').addEventListener('click', async () => {
    try { await fetch('/api/auth/logout', { method: 'POST', credentials: 'same-origin' }); } catch (_) {}
    checkAdmin();
  });

  /* ─── Panel: stats ────────────────────────────────────── */
  async function loadStats() {
    const grid = document.getElementById('stat-grid');
    grid.innerHTML = tr('loading');
    try {
      const r = await fetch('/api/admin/stats', { credentials: 'same-origin' });
      if (!r.ok) return;
      const s = await r.json();
      const pct = s.storage_max > 0 ? Math.min(100, (s.storage_used / s.storage_max) * 100) : 0;
      grid.innerHTML = `
        <div class="stat-card stat-card-wide">
          <div class="stat-label">${tr('stat-used')}</div>
          <div class="stat-value">${fmtBytes(s.storage_used)} / ${fmtBytes(s.storage_max)}</div>
          <div class="progress-bar"><div class="progress-bar-fill" style="width:${pct}%"></div></div>
        </div>
        <div class="stat-card"><div class="stat-label">${tr('stat-parcels')}</div><div class="stat-value">${s.total_parcels}</div></div>
        <div class="stat-card"><div class="stat-label">${tr('stat-accounts')}</div><div class="stat-value">${s.total_accounts}</div></div>
        <div class="stat-card"><div class="stat-label">${tr('stat-max-upload')}</div><div class="stat-value">${fmtBytes(s.max_upload_bytes)}</div></div>
        <div class="stat-card"><div class="stat-label">${tr('stat-max-expiry')}</div><div class="stat-value">${s.max_expiry_hours} ${tr('h')}</div></div>
        <div class="stat-card"><div class="stat-label">${tr('stat-hard-ceil')}</div><div class="stat-value">${fmtBytes(s.hard_upload_ceiling_bytes)}</div></div>
      `;
    } catch (_) {}
  }

  /* ─── Panel: parcels ──────────────────────────────────── */
  async function loadParcels() {
    const tbody = document.querySelector('#parcels-table tbody');
    const meta  = document.getElementById('parcels-meta');
    tbody.innerHTML = `<tr><td colspan="7">${tr('cell-loading')}</td></tr>`;
    try {
      const r = await fetch('/api/admin/parcels', { credentials: 'same-origin' });
      if (!r.ok) return;
      const { parcels } = await r.json();
      meta.textContent = tr('meta-parcels', { n: parcels.length, used: fmtBytes(parcels.reduce((s, p) => s + p.total_bytes, 0)) });
      tbody.innerHTML = '';
      if (parcels.length === 0) {
        tbody.innerHTML = `<tr><td colspan="7" class="muted">${tr('cell-empty')}</td></tr>`;
        return;
      }
      const now = Date.now();
      const FIFTY_YRS_MS = 50 * 365 * 24 * 3600 * 1000;
      const HUGE_DL      = 99999;
      const parcelMap = new Map();
      for (const p of parcels) {
        parcelMap.set(p.code, p);
        const expSoon  = p.expires_at < now;
        const isPerm   = p.expires_at - now > FIFTY_YRS_MS;
        const isUnlim  = p.downloads_left >= HUGE_DL;
        const row = document.createElement('tr');
        const kindLabel = p.kind === 'text' ? tr('cell-text-kind') : tr('cell-files-kind', { n: p.file_count });
        const dlCell    = isUnlim ? `<span class="never-expires">${tr('cell-unlimited')}</span>` : p.downloads_left;
        const expCell   = isPerm
          ? `<span class="never-expires">${tr('cell-permanent')}</span>`
          : `<span class="${expSoon ? 'expired' : ''}">${fmtDate(p.expires_at)}</span>`;
        row.innerHTML = `
          <td class="mono">${p.code}${p.has_password ? ' 🔒' : ''}</td>
          <td>${escapeText(p.owner)}</td>
          <td>${kindLabel}</td>
          <td>${fmtBytes(p.total_bytes)}</td>
          <td>${dlCell}</td>
          <td>${expCell}</td>
          <td>
            <button class="row-action ripple-surface" data-action="edit"   data-code="${p.code}">${tr('btn-edit')}</button>
            <button class="row-action danger ripple-surface" data-action="delete" data-code="${p.code}">${tr('btn-delete')}</button>
          </td>`;
        tbody.appendChild(row);
      }
      tbody.querySelectorAll('.row-action').forEach(btn => {
        btn.addEventListener('click', async () => {
          const code = btn.dataset.code;
          const act  = btn.dataset.action;
          if (act === 'delete') {
            if (!confirm(tr('confirm-del-parcel', { code }))) return;
            const r = await fetch('/api/admin/parcels/' + code, { method: 'DELETE', credentials: 'same-origin' });
            if (r.ok) { toast(tr('toast-deleted')); loadParcels(); }
          } else if (act === 'edit') {
            const p = parcelMap.get(code);
            if (p) openAdminEditParcel(p);
          }
        });
      });
    } catch (_) {}
  }

  /* ─── Admin edit-parcel modal ──────────────────────────── */
  // Constants must mirror server.js ADMIN_PARCEL_MAX_* — these are the
  // values the 永久 / 无限 buttons preset and the server clamps to.
  const ADMIN_MAX_HOURS = 100 * 365 * 24;   // 100 years
  const ADMIN_MAX_DLS   = 999999;
  let adminEditingParcel = null;

  // Track whether admin pressed 清除 — distinguishes "intentionally clear the
  // password" (send empty string) from "leave the field blank to keep existing"
  // (omit the field entirely). Reset every time the modal opens.
  let adminPasswordCleared = false;

  function openAdminEditParcel(p) {
    adminEditingParcel = p;
    adminPasswordCleared = false;
    document.getElementById('admin-edit-parcel-meta').textContent =
      tr('edit-parcel-meta-line', { code: p.code, owner: p.owner });
    const hLeft = Math.max(1, Math.ceil((p.expires_at - Date.now()) / 3600000));
    document.getElementById('admin-edit-expiry-hours').value =
      Math.min(hLeft, ADMIN_MAX_HOURS);
    document.getElementById('admin-edit-downloads').value =
      Math.min(p.downloads_left, ADMIN_MAX_DLS);
    // Password field: always starts blank because the bcrypt hash on the
    // server is one-way; we can only signal whether one is currently set.
    const pw = document.getElementById('admin-edit-password');
    pw.value = '';
    pw.type = 'password';
    pw.placeholder = p.has_password
      ? tr('edit-password-placeholder-with')
      : tr('edit-password-placeholder-without');
    const toggle = document.getElementById('btn-admin-toggle-password');
    if (toggle) toggle.textContent = tr('btn-show');
    document.getElementById('admin-edit-parcel-err').hidden = true;
    document.getElementById('admin-edit-parcel-modal').hidden = false;
  }

  document.getElementById('btn-admin-set-permanent').addEventListener('click', () => {
    document.getElementById('admin-edit-expiry-hours').value = ADMIN_MAX_HOURS;
  });
  document.getElementById('btn-admin-set-unlimited').addEventListener('click', () => {
    document.getElementById('admin-edit-downloads').value = ADMIN_MAX_DLS;
  });
  // 显示 / 隐藏 toggle for the password input. Pure DOM, no network.
  document.getElementById('btn-admin-toggle-password').addEventListener('click', (e) => {
    const input  = document.getElementById('admin-edit-password');
    const btn    = e.currentTarget;
    const reveal = input.type === 'password';
    input.type        = reveal ? 'text' : 'password';
    btn.textContent   = reveal ? tr('btn-hide') : tr('btn-show');
  });
  // 清除 — flag the parcel for password removal and visually empty the field.
  // The save handler then ships `password: ''` so the server drops the hash.
  document.getElementById('btn-admin-clear-password').addEventListener('click', () => {
    const input = document.getElementById('admin-edit-password');
    input.value = '';
    input.type  = 'password';
    adminPasswordCleared = true;
    const toggle = document.getElementById('btn-admin-toggle-password');
    if (toggle) toggle.textContent = tr('btn-show');
  });

  document.getElementById('btn-admin-edit-parcel-save').addEventListener('click', async () => {
    if (!adminEditingParcel) return;
    const err = document.getElementById('admin-edit-parcel-err');
    err.hidden = true;
    const expiryHours = parseInt(document.getElementById('admin-edit-expiry-hours').value, 10);
    const downloads   = parseInt(document.getElementById('admin-edit-downloads').value, 10);
    const passwordRaw = document.getElementById('admin-edit-password').value;
    // Three-state password handling matching the server contract:
    //   - field has text                → ship the new plaintext (server rehashes)
    //   - field empty + 清除 was pressed → ship ''  (server clears the hash)
    //   - field empty, 清除 not pressed → omit the field (server preserves it)
    const payload = {
      // ship `permanent`/`unlimited` flags when the inputs are at the ceiling,
      // so the server doesn't end up re-clamping a near-Infinity timestamp.
      permanent:    expiryHours >= ADMIN_MAX_HOURS,
      unlimited:    downloads   >= ADMIN_MAX_DLS,
      expiry_hours: expiryHours,
      downloads:    downloads,
    };
    if (passwordRaw.length > 0)         payload.password = passwordRaw;
    else if (adminPasswordCleared)      payload.password = '';
    try {
      const r = await fetch('/api/admin/parcels/' + adminEditingParcel.code, {
        method: 'PATCH',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      const body = await safeJson(r);
      if (!r.ok) {
        err.textContent = (body && body.error) || tr('err-save-fail');
        err.hidden = false;
        return;
      }
      document.getElementById('admin-edit-parcel-modal').hidden = true;
      adminEditingParcel = null;
      toast(tr('toast-saved'));
      loadParcels();
    } catch (_) {
      err.textContent = tr('err-net');
      err.hidden = false;
    }
  });

  // Close the admin edit-parcel modal via the X button or the backdrop.
  document.addEventListener('click', (e) => {
    const a = e.target.closest('[data-action="admin-edit-parcel-close"]');
    if (!a) return;
    document.getElementById('admin-edit-parcel-modal').hidden = true;
    adminEditingParcel = null;
  });

  /* ─── Panel: accounts ─────────────────────────────────── */
  async function loadAccounts() {
    const tbody = document.querySelector('#accounts-table tbody');
    const meta  = document.getElementById('accounts-meta');
    tbody.innerHTML = `<tr><td colspan="6">${tr('cell-loading')}</td></tr>`;
    try {
      const r = await fetch('/api/admin/accounts', { credentials: 'same-origin' });
      if (!r.ok) return;
      const { accounts } = await r.json();
      meta.textContent = tr('meta-accounts', { n: accounts.length });
      tbody.innerHTML = '';
      if (accounts.length === 0) {
        tbody.innerHTML = `<tr><td colspan="6" class="muted">${tr('cell-empty')}</td></tr>`;
        return;
      }
      const now = Date.now();
      for (const a of accounts) {
        const expSoon = a.expires_at < now;
        const isPerm  = !!a.is_perm;
        const row = document.createElement('tr');
        // Long-term accounts: badge after the username, "永不过期" instead of the
        // date, no 续期 button (it just gets refused server-side anyway).
        const usernameCell = `${escapeText(a.username)}${isPerm ? ` <span class="perm-badge">${tr('perm-badge')}</span>` : ''}`;
        const expiryCell   = isPerm
          ? `<span class="never-expires">${tr('cell-never-expires')}</span>`
          : `<span class="${expSoon ? 'expired' : ''}">${fmtDate(a.expires_at)}</span>`;
        const renewBtn = isPerm
          ? ''
          : `<button class="row-action ripple-surface" data-action="renew" data-id="${a.id}">${tr('btn-renew')}</button>`;
        const permBtn = `<button class="row-action ripple-surface" data-action="toggle-perm" data-id="${a.id}">${isPerm ? tr('btn-unmark-perm') : tr('btn-mark-perm')}</button>`;
        row.innerHTML = `
          <td class="mono">#${a.id}</td>
          <td>${usernameCell}</td>
          <td>${a.parcel_count}</td>
          <td>${fmtBytes(a.storage_used)}</td>
          <td>${expiryCell}</td>
          <td>
            ${renewBtn}
            ${permBtn}
            <button class="row-action danger ripple-surface" data-action="delete" data-id="${a.id}">${tr('btn-delete')}</button>
          </td>`;
        tbody.appendChild(row);
      }
      tbody.querySelectorAll('.row-action').forEach(btn => {
        btn.addEventListener('click', async () => {
          const id = btn.dataset.id;
          if (btn.dataset.action === 'renew') {
            const r = await fetch(`/api/admin/accounts/${id}/renew`, { method: 'POST', credentials: 'same-origin' });
            if (r.ok) { toast(tr('toast-renewed')); loadAccounts(); }
          } else if (btn.dataset.action === 'toggle-perm') {
            const r = await fetch(`/api/admin/accounts/${id}/toggle-perm`, { method: 'POST', credentials: 'same-origin' });
            const body = await r.json().catch(() => ({}));
            if (r.ok) { toast(body.is_perm ? tr('toast-perm-on') : tr('toast-perm-off')); loadAccounts(); }
          } else if (btn.dataset.action === 'delete') {
            if (!confirm(tr('confirm-del-account', { id }))) return;
            const r = await fetch(`/api/admin/accounts/${id}`, { method: 'DELETE', credentials: 'same-origin' });
            if (r.ok) { toast(tr('toast-deleted')); loadAccounts(); }
          }
        });
      });
    } catch (_) {}
  }

  // Wire the lang toggle button + run initial localisation pass
  document.getElementById('admin-lang-toggle').addEventListener('click', () => {
    lang = lang === 'zh' ? 'en' : 'zh';
    applyLang();
  });

  /* ─── Panel: config ───────────────────────────────────── */
  async function loadConfig() {
    try {
      const r = await fetch('/api/admin/config', { credentials: 'same-origin' });
      if (!r.ok) return;
      const c = await r.json();
      document.getElementById('cfg-max-storage').value = c.max_storage_gb;
      document.getElementById('cfg-max-upload').value  = c.max_upload_gb;
      document.getElementById('cfg-max-expiry').value  = c.max_expiry_hours;
      document.getElementById('hard-ceiling').textContent = c.upload_hard_ceiling_gb;
    } catch (_) {}
  }
  document.getElementById('config-save').addEventListener('click', async () => {
    const body = {
      max_storage_gb:   parseFloat(document.getElementById('cfg-max-storage').value),
      max_upload_gb:    parseFloat(document.getElementById('cfg-max-upload').value),
      max_expiry_hours: parseInt  (document.getElementById('cfg-max-expiry').value, 10),
    };
    const err = document.getElementById('config-err');
    err.hidden = true;
    try {
      const r = await fetch('/api/admin/config', {
        method: 'POST', credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!r.ok) { err.textContent = tr('err-save-fail'); err.hidden = false; return; }
      toast(tr('toast-saved'));
      loadConfig();
    } catch (_) { err.textContent = tr('err-net'); err.hidden = false; }
  });

  /* ─── Panel: password ─────────────────────────────────── */
  document.getElementById('pw-save').addEventListener('click', async () => {
    const username = document.getElementById('pw-username').value.trim();
    const current  = document.getElementById('pw-current').value;
    const next     = document.getElementById('pw-next').value;
    const err      = document.getElementById('pw-err');
    err.hidden = true;
    if (!current || !next) { err.textContent = tr('err-fill-pw'); err.hidden = false; return; }
    try {
      const r = await fetch('/api/admin/password', {
        method: 'POST', credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ current, next, username: username || undefined }),
      });
      const body = await safeJson(r);
      if (!r.ok) {
        err.textContent =
            body && body.error === 'wrong_current_password' ? tr('err-wrong-pw') :
            body && body.error === 'username_taken'         ? tr('err-name-taken') :
            body && body.error || tr('err-save-fail');
        err.hidden = false;
        return;
      }
      toast(tr('toast-updated'));
      document.getElementById('pw-current').value = '';
      document.getElementById('pw-next').value = '';
      document.getElementById('pw-username').value = '';
    } catch (_) { err.textContent = tr('err-net'); err.hidden = false; }
  });

  document.querySelectorAll('.admin-tab').forEach(tab => {
    tab.addEventListener('click', () => showPanel(tab.dataset.tab));
  });

  function refreshPanel(tab) {
    if      (tab === 'stats')    loadStats();
    else if (tab === 'parcels')  loadParcels();
    else if (tab === 'accounts') loadAccounts();
    else if (tab === 'config')   loadConfig();
  }

  bindRipples();
  applyLang();
  checkAdmin();
})();
