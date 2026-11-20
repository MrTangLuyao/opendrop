/* open.drop admin panel: sign in, then overview / all parcels / all
   accounts / config / admin credentials, on the v2 admin API. */
(function () {
  'use strict';
  const tr = OD.tr, h = OD.h, api = OD.api, toast = OD.toast;
  const fmtBytes = OD.fmtBytes, fmtDate = OD.fmtDate;
  const $ = OD.$;
  const GIB = OD.GIB, TIB = GIB * 1024, MIB = 1024 * 1024;

  /* ─── Strings ─── */
  Object.assign(OD.T.zh, {
    'nav-back-to-app': '返回站点',
    'nav-logout': '登出',
    'login-title': '管理员登录',
    'login-sub-a': '用户名默认为 admin，初始密码在首次启动时打印到服务日志，忘记时可运行 ',
    'login-sub-b': ' 重置',
    'cfg-username': '用户名',
    'cfg-password': '密码',
    'btn-login': '登录',
    'tab-stats': '概览',
    'tab-parcels': '所有包裹',
    'tab-accounts': '所有账号',
    'tab-config': '配置',
    'tab-password': '修改密码',
    'stats-title': '系统概览',
    'parcels-title': '所有包裹',
    'th-code': '取件码',
    'th-owner': '所有者',
    'th-kind': '类型',
    'th-size': '大小',
    'th-downloads': '剩余下载',
    'th-expires': '过期时间',
    'th-actions': '操作',
    'th-username': '用户名',
    'th-parcels': '包裹数',
    'th-storage': '占用',
    'th-account-expires': '账户到期',
    'accounts-title': '所有账号',
    'config-title': '限额配置',
    'config-sub': '可在线修改，无需重启。',
    'cfg-site-name': '站点名称',
    'cfg-site-name-support': '第一个点前后分开着色',
    'cfg-max-storage': '系统总存储',
    'mode-percent': '按硬盘比例',
    'mode-bytes': '固定大小',
    'percent-value': '{p}%，约 {size}',
    'lg-od': 'opendrop 已用 {v}',
    'lg-other': '其他占用 {v}',
    'lg-free': '可用 {v}',
    'lg-quota': '上限 {v}',
    'warn-below-used': '上限低于当前已用的 {used}，在有包裹过期之前，新的上传都会被拒绝',
    'warn-over-disk': '上限不能超过硬盘容量 {total}',
    'cfg-max-upload': '单次上传 (GB)',
    'cfg-max-expiry': '最长有效期（小时）',
    'cfg-anon': '允许匿名上传',
    'cfg-registration': '允许注册',
    'btn-save': '保存',
    'password-title': '修改管理员账号',
    'password-sub': '填写当前密码；用户名或新密码留空则保持不变，新密码至少 8 位，修改后其他已登录的管理员会话会退出',
    'new-username': '新用户名（可选）',
    'current-password': '当前密码',
    'new-password': '新密码',
    'stat-used': '已用存储',
    'stat-parcels': '包裹数',
    'stat-accounts': '用户账号数',
    'stat-max-upload': '单次上传上限',
    'stat-max-expiry': '最长有效期',
    'stat-disk': '硬盘可用',
    'meta-parcels': '共 {n} 个包裹，总占用 {used}',
    'meta-accounts': '共 {n} 个用户账号',
    'cell-text-kind': '文本',
    'cell-files-kind': '{n} 文件',
    'cell-empty': '无',
    'btn-load-more': '加载更多',
    'cell-anon': '匿名',
    'cell-uploading': '上传中',
    'btn-delete': '删除',
    'btn-renew': '续期 7 天',
    'btn-cancel': '取消',
    'confirm-title': '确定要删除？',
    'confirm-del-parcel': '删除包裹 {code}？这会立刻销毁里面的文件。',
    'confirm-del-account': '删除账户 #{id}？这会同时销毁该账户的所有包裹。',
    'err-fill-login': '请输入用户名和密码',
    'err-login-fail': '登录失败',
    'err-bad-credentials': '用户名或密码错误',
    'err-not-admin': '该账户不是管理员',
    'err-fill-current': '请填写当前密码',
    'err-fill-change': '请填写新用户名或新密码',
    'err-pw-short': '新密码需为 8 到 128 个字符',
    'err-parcel-pw-blank': '密码不能只有空格',
    'err-parcel-pw-len': '密码最多 128 个字符',
    'err-bad-username': '用户名需为 1 到 32 个字符，且不含空格',
    'err-wrong-pw': '当前密码错误',
    'err-name-taken': '该用户名已被占用',
    'err-save-fail': '保存失败',
    'err-bad-field': '{field} 的值无效',
    'err-bad-expiry': '有效期需在 1 到 876000 小时之间',
    'err-bad-downloads': '下载次数需在 1 到 999999 之间',
    'err-uploading': '这个包裹还在上传中',
    'err-gone': '包裹不存在或已被删除',
    'toast-deleted': '已删除',
    'toast-renewed': '已续期',
    'toast-saved': '已保存',
    'toast-updated': '已修改',
    'btn-edit': '修改',
    'btn-mark-permanent': '设为永久',
    'btn-mark-unlimited': '设为无限',
    'edit-parcel-title': '编辑包裹',
    'edit-parcel-meta-line': '取件码 {code}，所有者 {owner}',
    'edit-expiry': '有效期 (小时)',
    'edit-downloads': '剩余下载次数',
    'edit-password': '修改密码',
    'edit-password-placeholder-with': '已设置，留空保持不变',
    'edit-password-placeholder-without': '未设置，留空表示无密码',
    'edit-password-placeholder-cleared': '保存后清除密码',
    'btn-clear-password': '清除密码',
    'cell-permanent': '永久',
    'cell-unlimited': '无限',
    'toast-perm-on': '已标记为长期账户',
    'toast-perm-off': '已取消长期标记',
    'h': '小时',
    'btn-mark-perm': '标记长期',
    'btn-unmark-perm': '取消长期',
    'perm-badge': '长期',
    'cell-never-expires': '永不过期',
    'has-password': '有密码',
    'close': '关闭'
  });
  Object.assign(OD.T.en, {
    'nav-back-to-app': 'Back to site',
    'nav-logout': 'Sign out',
    'login-title': 'Admin sign in',
    'login-sub-a': 'The username is admin unless changed. The first password is printed to the server log on first start; to reset it run ',
    'login-sub-b': '',
    'cfg-username': 'Username',
    'cfg-password': 'Password',
    'btn-login': 'Sign in',
    'tab-stats': 'Overview',
    'tab-parcels': 'All parcels',
    'tab-accounts': 'All accounts',
    'tab-config': 'Config',
    'tab-password': 'Change password',
    'stats-title': 'System overview',
    'parcels-title': 'All parcels',
    'th-code': 'Code',
    'th-owner': 'Owner',
    'th-kind': 'Kind',
    'th-size': 'Size',
    'th-downloads': 'Downloads left',
    'th-expires': 'Expires',
    'th-actions': 'Actions',
    'th-username': 'Username',
    'th-parcels': 'Parcels',
    'th-storage': 'Storage',
    'th-account-expires': 'Account expires',
    'accounts-title': 'All accounts',
    'config-title': 'Limits',
    'config-sub': 'Editable at runtime, no restart.',
    'cfg-site-name': 'Site name',
    'cfg-site-name-support': 'The first dot splits the colours',
    'cfg-max-storage': 'System storage',
    'mode-percent': 'Share of disk',
    'mode-bytes': 'Fixed size',
    'percent-value': '{p}%, about {size}',
    'lg-od': 'opendrop {v}',
    'lg-other': 'other {v}',
    'lg-free': 'free {v}',
    'lg-quota': 'limit {v}',
    'warn-below-used': 'The limit is below the {used} already stored; new uploads are refused until parcels expire',
    'warn-over-disk': 'The limit cannot exceed the disk size, {total}',
    'cfg-max-upload': 'Per upload (GB)',
    'cfg-max-expiry': 'Max expiry (hours)',
    'cfg-anon': 'Allow anonymous uploads',
    'cfg-registration': 'Allow sign ups',
    'btn-save': 'Save',
    'password-title': 'Admin credentials',
    'password-sub': 'Enter the current password. Leave the username or the new password blank to keep it; new passwords need 8 or more characters. Other admin sessions are signed out.',
    'new-username': 'New username (optional)',
    'current-password': 'Current password',
    'new-password': 'New password',
    'stat-used': 'Used storage',
    'stat-parcels': 'Parcels',
    'stat-accounts': 'User accounts',
    'stat-max-upload': 'Per-upload cap',
    'stat-max-expiry': 'Max expiry',
    'stat-disk': 'Disk free',
    'meta-parcels': '{n} parcel(s), {used} total',
    'meta-accounts': '{n} user account(s)',
    'cell-text-kind': 'text',
    'cell-files-kind': '{n} file(s)',
    'cell-empty': 'empty',
    'btn-load-more': 'Load more',
    'cell-anon': 'anonymous',
    'cell-uploading': 'uploading',
    'btn-delete': 'Delete',
    'btn-renew': 'Renew 7d',
    'btn-cancel': 'Cancel',
    'confirm-title': 'Delete this?',
    'confirm-del-parcel': 'Delete parcel {code}? Its files will be destroyed immediately.',
    'confirm-del-account': 'Delete account #{id}? All of its parcels will also be removed.',
    'err-fill-login': 'Please enter username and password',
    'err-login-fail': 'Sign in failed',
    'err-bad-credentials': 'Wrong username or password',
    'err-not-admin': 'This account is not an admin',
    'err-fill-current': 'Enter the current password',
    'err-fill-change': 'Enter a new username or a new password',
    'err-pw-short': 'New passwords are 8 to 128 characters',
    'err-parcel-pw-blank': 'A password cannot be only spaces',
    'err-parcel-pw-len': 'Passwords are at most 128 characters',
    'err-bad-username': 'Usernames are 1 to 32 characters without spaces',
    'err-wrong-pw': 'Current password is wrong',
    'err-name-taken': 'Username already taken',
    'err-save-fail': 'Save failed',
    'err-bad-field': 'Invalid value: {field}',
    'err-bad-expiry': 'Expiry must be 1 to 876000 hours',
    'err-bad-downloads': 'Downloads must be 1 to 999999',
    'err-uploading': 'This parcel is still uploading',
    'err-gone': 'The parcel no longer exists',
    'toast-deleted': 'Deleted',
    'toast-renewed': 'Renewed',
    'toast-saved': 'Saved',
    'toast-updated': 'Updated',
    'btn-edit': 'Edit',
    'btn-mark-permanent': 'Set permanent',
    'btn-mark-unlimited': 'Set unlimited',
    'edit-parcel-title': 'Edit parcel',
    'edit-parcel-meta-line': 'Code {code}, owner {owner}',
    'edit-expiry': 'Expiry (hours)',
    'edit-downloads': 'Downloads remaining',
    'edit-password': 'Change password',
    'edit-password-placeholder-with': 'Set, leave blank to keep',
    'edit-password-placeholder-without': 'None, leave blank for open access',
    'edit-password-placeholder-cleared': 'Cleared when saved',
    'btn-clear-password': 'Clear password',
    'cell-permanent': 'permanent',
    'cell-unlimited': 'unlimited',
    'toast-perm-on': 'Marked as long-term account',
    'toast-perm-off': 'Long-term flag removed',
    'h': 'h',
    'btn-mark-perm': 'Mark long-term',
    'btn-unmark-perm': 'Unmark long-term',
    'perm-badge': 'long-term',
    'cell-never-expires': 'never expires',
    'has-password': 'Password protected',
    'close': 'Close'
  });

  const ADMIN_MAX_HOURS = 876000;
  const ADMIN_MAX_DLS = 999999;
  /* A failure this page has no words of its own for (too many tries,
     busy, offline, any other refusal: OD.errText), or the given message. */
  const failText = (e, key) => (!e.status || e.status === 429 || e.status >= 500 ? OD.errText(e) : tr(key));
  /* A list or a number that did not load: a lost session signs in again,
     anything else says why. */
  const loadFailed = e => { if (!lostSession(e)) toast(OD.errText(e)); };
  const showErr = (el, msg) => { el.textContent = msg; el.hidden = false; };
  const round2 = v => Math.round(v * 100) / 100;
  /* A stored size as shown in a field: rounded down, so it never reads
     as more than it is (small values keep enough digits not to read as
     0). An untouched field saves the stored bytes, not this. */
  const shownSize = (bytes, unit) => {
    const v = bytes / unit;
    const d = v >= 0.01 || v <= 0 ? 100 : 10000;
    return Math.floor(v * d + 1e-6) / d;
  };
  const num = raw => {
    const s = String(raw).trim();
    return s === '' ? NaN : Number(s);
  };

  /* ─── Stage and tab routing ─── */
  function showStage(name) {
    document.querySelectorAll('.admin-stage').forEach(s => s.classList.toggle('is-active', s.id === 'admin-' + name));
  }
  const activeTab = () => {
    const t = document.querySelector('.admin-tab.is-active');
    return t ? t.dataset.tab : 'stats';
  };
  function showPanel(tab) {
    document.querySelectorAll('.admin-tab').forEach(t => {
      const on = t.dataset.tab === tab;
      t.classList.toggle('is-active', on);
      t.setAttribute('aria-selected', String(on));
    });
    document.querySelectorAll('.admin-panel').forEach(p => p.classList.toggle('is-active', p.dataset.panel === tab));
    refreshPanel(tab);
  }
  /* A panel opened loads afresh: what it showed before goes at once and
     its loader stands in (OD.loading, clear) until the answer is in. */
  function refreshPanel(tab) {
    if (tab === 'stats') loadStats();
    else if (tab === 'parcels') loadParcels();
    else if (tab === 'accounts') loadAccounts();
    else if (tab === 'config') loadConfig();
  }
  document.querySelectorAll('.admin-tab').forEach(tab => {
    tab.addEventListener('click', () => showPanel(tab.dataset.tab));
  });

  /* A lost session (expired, or signed out elsewhere) goes back to sign
     in. An action that met one says so with LOST, and nothing reloads. */
  const LOST = {};
  function lostSession(err) {
    if (err && (err.code === 'unauthorized' || err.code === 'forbidden')) { checkAdmin(); return true; }
    return false;
  }

  /* ─── Auth ─── */
  /* Who is signed in (null when no one, or when this browser has not
     signed in since it last signed out). Never throws. */
  async function whoAmI() {
    if (!OD.authHint.get()) return null;
    try {
      return await api('/api/auth/me');
    } catch (err) {
      if (err.code === 'unauthorized') OD.authHint.set(false);
      return null;
    }
  }
  function applyAdmin(me) {
    if (me && me.is_admin) {
      $('admin-logout').hidden = false;
      showStage('dashboard');
      refreshPanel(activeTab());
      return me;
    }
    $('admin-logout').hidden = true;
    showStage('login');
    return null;
  }
  /* On the first visit neither stage shows until the answer is in; a
     long wait shows the loader in their place. */
  async function checkAdmin(boot) {
    const work = whoAmI();
    return applyAdmin(boot === true ? await OD.loading($('admin-boot'), work, { clear: true }) : await work);
  }

  $('admin-login-submit').addEventListener('click', login);
  ['admin-user', 'admin-pass'].forEach(id => {
    $(id).addEventListener('keydown', e => { if (e.key === 'Enter') login(); });
  });

  let loggingIn = false;
  async function login() {
    if (loggingIn) return;
    const u = $('admin-user').value.trim();
    const p = $('admin-pass').value;
    const err = $('admin-login-err');
    if (!u || !p) { showErr(err, tr('err-fill-login')); return; }
    err.hidden = true;
    loggingIn = true;
    /* 登录 waits with the sign in and with the check of who that is,
       so the card stays until the dashboard can take its place. */
    let who;
    try {
      /* admin: true never registers, and refuses a non-admin account. */
      who = await OD.busy($('admin-login-submit'), api('/api/auth/login', { body: { username: u, password: p, admin: true } })
        .then(() => {
          /* An admin never expires: the main page's bar shows sign out. */
          OD.authHint.set(true, true);
          return whoAmI();
        }));
    } catch (e) {
      showErr(err, e.code === 'forbidden' ? tr('err-not-admin')
        : e.code === 'bad_credentials' || e.code === 'bad_field' ? tr('err-bad-credentials')
          : failText(e, 'err-login-fail'));
      return;
    } finally {
      loggingIn = false;
    }
    $('admin-pass').value = '';
    applyAdmin(who);
  }

  /* Only the server can end the session (its cookie is HttpOnly): a
     failed call leaves the panel signed in and says why. */
  let loggingOut = false;
  $('admin-logout').addEventListener('click', async () => {
    if (loggingOut) return;
    loggingOut = true;
    try {
      await OD.busy($('admin-logout'), api('/api/auth/logout', { method: 'POST' }));
    } catch (e) {
      toast(OD.errText(e));
      return;
    } finally {
      loggingOut = false;
    }
    OD.authHint.set(false);
    checkAdmin();
  });

  /* ─── Confirm modal ───
     删除 waits with the delete (onConfirm returns its promise), then the
     dialog closes and `after` (the list reloading) runs. */
  let confirmAction = null;
  let confirmAfter = null;
  let confirmSeq = 0;
  function confirmDialog(message, onConfirm, after) {
    $('confirm-title').textContent = tr('confirm-title');
    $('confirm-message').textContent = message;
    confirmAction = onConfirm;
    confirmAfter = after || null;
    confirmSeq++;
    OD.openModal($('confirm-modal'), $('btn-confirm-ok'));
  }
  $('btn-confirm-ok').addEventListener('click', async () => {
    const fn = confirmAction, after = confirmAfter, seq = confirmSeq;
    confirmAction = null;
    confirmAfter = null;
    const work = fn ? fn() : null;
    let result = null;
    if (work && typeof work.then === 'function') result = await OD.busy($('btn-confirm-ok'), work).catch(() => null);
    /* Another dialog opened meanwhile stays open. */
    if (seq === confirmSeq) OD.closeModal($('confirm-modal'));
    if (after && result !== LOST) after();
  });

  document.addEventListener('click', e => {
    const a = e.target.closest('[data-action]');
    if (!a) return;
    if (a.dataset.action === 'confirm-cancel') {
      OD.closeModal($('confirm-modal'));
      confirmAction = null;
    } else if (a.dataset.action === 'admin-edit-parcel-close') {
      OD.closeModal($('admin-edit-parcel-modal'));
      editing = null;
    }
  });

  /* ─── Overview ─── */
  let overview = null;
  let statsSeq = 0;

  async function loadStats() {
    const seq = ++statsSeq;
    let data;
    try { data = await OD.loading($('stat-grid'), api('/api/admin/overview'), { clear: true }); } catch (e) { if (seq === statsSeq) { loadFailed(e); renderStats(); } return; }
    if (seq !== statsSeq) return;
    overview = data;
    renderStats();
  }
  function statCard(label, value, wide, pct) {
    return h('div', { class: 'stat-card' + (wide ? ' stat-card-wide' : '') },
      h('div', { class: 'stat-label', text: label }),
      h('div', { class: 'stat-value tnum', text: value }),
      pct === undefined ? null : progressBar(pct));
  }
  function progressBar(pct) {
    const fill = h('div', { class: 'progress-bar-fill' });
    fill.style.width = pct.toFixed(1) + '%';
    return h('div', { class: 'progress-bar' }, fill);
  }
  function renderStats() {
    const s = overview;
    if (!s || OD.isLoading($('stat-grid'))) return;
    const st = s.settings || {};
    const pct = s.quota > 0 ? Math.min(100, (s.used / s.quota) * 100) : 0;
    OD.clear($('stat-grid')).append(
      statCard(tr('stat-used'), fmtBytes(s.used) + ' / ' + fmtBytes(s.quota), true, pct),
      statCard(tr('stat-parcels'), String(s.parcels)),
      statCard(tr('stat-accounts'), String(s.accounts)),
      statCard(tr('stat-max-upload'), fmtBytes(st.max_upload_bytes)),
      statCard(tr('stat-max-expiry'), st.max_expiry_hours + ' ' + tr('h')),
      statCard(tr('stat-disk'), fmtBytes(s.disk.avail) + ' / ' + fmtBytes(s.disk.total)));
  }

  /* ─── All parcels ─── */
  let parcelsCache = null;
  /* The server sends one page at a time; `next` is the cursor of the
     following page, or null on the last one. */
  let parcelsNext = null;
  let parcelsSeq = 0;

  /* 加载更多 waits with the page it asks for (onMore gets the button). */
  function moreRow(tbody, cols, onMore) {
    const btn = h('button', { class: 'row-action btn-line ripple-surface', type: 'button', text: tr('btn-load-more') });
    btn.addEventListener('click', () => { onMore(btn); });
    tbody.append(h('tr', null, h('td', { colspan: cols, class: 'load-more' }, btn)));
  }
  /* A list loads in one of three ways: `more` true, the next page, with
     `btn` (加载更多) waiting; 'reload', the first page again in place
     (after an action in it: the rows stay through a quick answer); or
     afresh, when its panel opens (the old rows and count go at once). */
  function listLoad(tbody, meta, more, btn, work) {
    if (more === true) return OD.busy(btn, work);
    if (more !== 'reload') meta.textContent = '\u00a0';
    return OD.loading(tbody, work, { clear: more !== 'reload' });
  }
  async function loadParcels(more, btn) {
    const seq = ++parcelsSeq;
    const cursor = more === true ? parcelsNext : null;
    let data;
    try {
      data = await listLoad($('parcels-table').tBodies[0], $('parcels-meta'), more, btn,
        api('/api/admin/parcels' + (cursor !== null ? '?before=' + encodeURIComponent(cursor) : '')));
    } catch (e) { if (seq !== parcelsSeq) return; loadFailed(e); renderParcels(); return; }
    if (seq !== parcelsSeq) return;
    const rows = Array.isArray(data.parcels) ? data.parcels : [];
    parcelsCache = cursor !== null && parcelsCache ? parcelsCache.concat(rows) : rows;
    parcelsNext = data.next === undefined ? null : data.next;
    renderParcels();
  }
  function renderParcels() {
    const tbody = $('parcels-table').tBodies[0];
    const parcels = parcelsCache;
    if (!parcels || OD.isLoading(tbody)) return;
    $('parcels-meta').textContent = tr('meta-parcels', {
      n: parcels.length, used: fmtBytes(parcels.reduce((s, p) => s + p.total_bytes, 0))
    });
    OD.clear(tbody);
    if (parcels.length === 0) {
      tbody.append(h('tr', null, h('td', { colspan: 7, class: 'muted', text: tr('cell-empty') })));
      return;
    }
    const now = Date.now();
    parcels.forEach(p => {
      const uploading = p.state === 'uploading';
      const dlCell = p.downloads_left === null || p.downloads_left === undefined
        ? h('span', { class: 'never-expires', text: tr('cell-unlimited') })
        : String(p.downloads_left);
      const expCell = uploading
        ? h('span', { class: 'muted', text: tr('cell-uploading') })
        : p.expires_at === null || p.expires_at === undefined
          ? h('span', { class: 'never-expires', text: tr('cell-permanent') })
          : h('span', { class: p.expires_at < now ? 'expired' : null, text: fmtDate(p.expires_at) });
      tbody.append(h('tr', null,
        h('td', { class: 'mono' },
          h('span', { class: 'code-cell' },
            h('span', { class: 'sel', text: p.code }),
            p.has_password ? h('span', { class: 'cell-lock', title: tr('has-password') }, OD.icon('lock')) : null)),
        h('td', { class: p.owner ? 'sel' : 'muted', text: p.owner || tr('cell-anon') }),
        h('td', { text: p.kind === 'text' ? tr('cell-text-kind') : tr('cell-files-kind', { n: p.file_count }) }),
        h('td', { class: 'tnum', text: fmtBytes(p.total_bytes) }),
        h('td', { class: 'tnum' }, dlCell),
        h('td', { class: 'tnum' }, expCell),
        h('td', { class: 'actions' },
          h('button', {
            class: 'row-action btn-line ripple-surface', type: 'button', text: tr('btn-edit'), disabled: uploading,
            onclick: () => openAdminEditParcel(p)
          }),
          h('button', {
            class: 'row-action btn-line danger ripple-surface', type: 'button', text: tr('btn-delete'),
            onclick: () => confirmDialog(tr('confirm-del-parcel', { code: p.code }), () => deleteParcel(p), () => loadParcels('reload'))
          }))));
    });
    if (parcelsNext !== null) moreRow(tbody, 7, btn => loadParcels(true, btn));
  }
  /* The delete, while 删除 waits; the list reloads once the dialog has
     closed (confirmDialog's `after`), unless the session was lost. */
  async function deleteParcel(p) {
    try {
      await api('/api/admin/parcels/' + p.id, { method: 'DELETE' });
      toast(tr('toast-deleted'));
    } catch (e) {
      if (lostSession(e)) return LOST;
      toast(e.code === 'not_found' ? tr('toast-deleted') : failText(e, 'err-save-fail'));
    }
    return null;
  }

  /* ─── Admin edit-parcel modal ───
     Expiry and downloads go as a number, or null for 永久 / 无限. Only what
     was changed is sent, so saving never moves an untouched expiry. The
     password has three states: typed text replaces it, 清除密码 sends ''
     to remove it, a blank field leaves it as it is. Typed text goes
     exactly as typed (the main page sends and checks passwords the same
     way, so a recipient can always enter it); text of nothing but spaces
     is refused. */
  let editing = null;
  let edit = null;
  const exInput = $('admin-edit-expiry-hours');
  const dlInput = $('admin-edit-downloads');
  const pwInput = $('admin-edit-password');
  const permBtn = $('btn-admin-set-permanent');
  const unlimBtn = $('btn-admin-set-unlimited');
  const clearBtn = $('btn-admin-clear-password');

  function renderEditFlags() {
    if (!edit) return;
    /* A blank field keeps a one-space placeholder, which lets the label
       rest inside it (:placeholder-shown). */
    exInput.placeholder = edit.permanent ? tr('cell-permanent') : ' ';
    dlInput.placeholder = edit.unlimited ? tr('cell-unlimited') : ' ';
    pwInput.placeholder = edit.cleared ? tr('edit-password-placeholder-cleared')
      : editing && editing.has_password ? tr('edit-password-placeholder-with') : tr('edit-password-placeholder-without');
    permBtn.setAttribute('aria-pressed', String(edit.permanent));
    unlimBtn.setAttribute('aria-pressed', String(edit.unlimited));
    clearBtn.setAttribute('aria-pressed', String(edit.cleared));
  }
  function openAdminEditParcel(p) {
    editing = p;
    const permanent = p.expires_at === null || p.expires_at === undefined;
    const unlimited = p.downloads_left === null || p.downloads_left === undefined;
    exInput.value = permanent ? '' : Math.min(Math.max(1, Math.ceil((p.expires_at - Date.now()) / 3600000)), ADMIN_MAX_HOURS);
    dlInput.value = unlimited ? '' : Math.min(p.downloads_left, ADMIN_MAX_DLS);
    pwInput.value = '';
    edit = {
      permanent, unlimited, cleared: false,
      startPermanent: permanent, startUnlimited: unlimited,
      startExp: String(exInput.value), startDl: String(dlInput.value)
    };
    $('admin-edit-parcel-meta').textContent = tr('edit-parcel-meta-line', { code: p.code, owner: p.owner || tr('cell-anon') });
    renderEditFlags();
    $('admin-edit-parcel-err').hidden = true;
    OD.openModal($('admin-edit-parcel-modal'));
  }

  permBtn.addEventListener('click', () => { if (!edit) return; edit.permanent = true; exInput.value = ''; renderEditFlags(); });
  unlimBtn.addEventListener('click', () => { if (!edit) return; edit.unlimited = true; dlInput.value = ''; renderEditFlags(); });
  clearBtn.addEventListener('click', () => { if (!edit) return; edit.cleared = true; pwInput.value = ''; renderEditFlags(); });
  exInput.addEventListener('input', () => { if (edit && exInput.value !== '') { edit.permanent = false; renderEditFlags(); } });
  dlInput.addEventListener('input', () => { if (edit && dlInput.value !== '') { edit.unlimited = false; renderEditFlags(); } });

  $('btn-admin-edit-parcel-save').addEventListener('click', async () => {
    const p = editing;
    if (!p || !edit) return;
    const err = $('admin-edit-parcel-err');
    err.hidden = true;
    const body = {};
    if (edit.permanent) {
      if (!edit.startPermanent) body.expiry_hours = null;
    } else if (edit.startPermanent || String(exInput.value) !== edit.startExp) {
      const n = num(exInput.value);
      if (!Number.isInteger(n) || n < 1 || n > ADMIN_MAX_HOURS) { showErr(err, tr('err-bad-expiry')); return; }
      body.expiry_hours = n;
    }
    if (edit.unlimited) {
      if (!edit.startUnlimited) body.downloads = null;
    } else if (edit.startUnlimited || String(dlInput.value) !== edit.startDl) {
      const n = num(dlInput.value);
      if (!Number.isInteger(n) || n < 1 || n > ADMIN_MAX_DLS) { showErr(err, tr('err-bad-downloads')); return; }
      body.downloads = n;
    }
    const pw = pwInput.value;
    if (pw !== '' && !pw.trim()) { showErr(err, tr('err-parcel-pw-blank')); return; }
    if (pw !== '') body.password = pw;
    else if (edit.cleared) body.password = '';

    if (Object.keys(body).length) {
      try {
        await OD.busy($('btn-admin-edit-parcel-save'), api('/api/admin/parcels/' + p.id, { method: 'PATCH', body }));
      } catch (e) {
        if (lostSession(e)) return;
        const field = e.data && e.data.field;
        showErr(err, e.code === 'bad_field' && field === 'expiry_hours' ? tr('err-bad-expiry')
          : e.code === 'bad_field' && field === 'downloads' ? tr('err-bad-downloads')
            : e.code === 'bad_field' && field === 'password' ? tr(pw.trim() ? 'err-parcel-pw-len' : 'err-parcel-pw-blank')
              : e.code === 'uploading' ? tr('err-uploading')
                : e.code === 'not_found' ? tr('err-gone')
                  : failText(e, 'err-save-fail'));
        return;
      }
    }
    /* While 保存 waited the dialog may have been opened for another
       parcel: that one stays open. */
    if (editing === p) {
      OD.closeModal($('admin-edit-parcel-modal'));
      editing = null;
    }
    toast(tr('toast-saved'));
    loadParcels('reload');
  });

  /* ─── All accounts ─── */
  let accountsCache = null;
  let accountsNext = null;
  let accountsSeq = 0;

  /* As loadParcels: `more` true (with `btn`), 'reload', or afresh. */
  async function loadAccounts(more, btn) {
    const seq = ++accountsSeq;
    const cursor = more === true ? accountsNext : null;
    let data;
    try {
      data = await listLoad($('accounts-table').tBodies[0], $('accounts-meta'), more, btn,
        api('/api/admin/accounts' + (cursor !== null ? '?after=' + encodeURIComponent(cursor) : '')));
    } catch (e) { if (seq !== accountsSeq) return; loadFailed(e); renderAccounts(); return; }
    if (seq !== accountsSeq) return;
    const rows = Array.isArray(data.accounts) ? data.accounts : [];
    accountsCache = cursor !== null && accountsCache ? accountsCache.concat(rows) : rows;
    accountsNext = data.next === undefined ? null : data.next;
    renderAccounts();
  }
  function renderAccounts() {
    const tbody = $('accounts-table').tBodies[0];
    const accounts = accountsCache;
    if (!accounts || OD.isLoading(tbody)) return;
    $('accounts-meta').textContent = tr('meta-accounts', { n: accounts.length });
    OD.clear(tbody);
    if (accounts.length === 0) {
      tbody.append(h('tr', null, h('td', { colspan: 6, class: 'muted', text: tr('cell-empty') })));
      return;
    }
    const now = Date.now();
    accounts.forEach(a => {
      const isPerm = !!a.is_perm;
      /* Long-term accounts: a badge, 永不过期, and no renew button. */
      const expiryCell = isPerm || a.expires_at === null || a.expires_at === undefined
        ? h('span', { class: 'never-expires', text: tr('cell-never-expires') })
        : h('span', { class: a.expires_at < now ? 'expired' : null, text: fmtDate(a.expires_at) });
      tbody.append(h('tr', null,
        h('td', { class: 'mono', text: '#' + a.id }),
        h('td', null, h('span', { class: 'sel', text: a.username }), isPerm ? h('span', { class: 'perm-badge', text: tr('perm-badge') }) : null),
        h('td', { class: 'tnum', text: String(a.parcels) }),
        h('td', { class: 'tnum', text: fmtBytes(a.bytes) }),
        h('td', { class: 'tnum' }, expiryCell),
        h('td', { class: 'actions' },
          isPerm ? null : h('button', {
            class: 'row-action btn-line ripple-surface', type: 'button', text: tr('btn-renew'),
            onclick: e => accountAction(a, { renew: true }, tr('toast-renewed'), e.currentTarget)
          }),
          h('button', {
            class: 'row-action btn-line ripple-surface', type: 'button', text: isPerm ? tr('btn-unmark-perm') : tr('btn-mark-perm'),
            onclick: e => accountAction(a, { is_perm: !isPerm }, null, e.currentTarget)
          }),
          h('button', {
            class: 'row-action btn-line danger ripple-surface', type: 'button', text: tr('btn-delete'),
            onclick: () => confirmDialog(tr('confirm-del-account', { id: a.id }), () => deleteAccount(a), () => loadAccounts('reload'))
          }))));
    });
    if (accountsNext !== null) moreRow(tbody, 6, btn => loadAccounts(true, btn));
  }
  /* Renew and the long-term mark: the row's button waits with the call. */
  async function accountAction(a, body, okMsg, btn) {
    try {
      const row = await OD.busy(btn, api('/api/admin/accounts/' + a.id, { method: 'PATCH', body }));
      toast(okMsg || (row && row.is_perm ? tr('toast-perm-on') : tr('toast-perm-off')));
    } catch (e) {
      if (lostSession(e)) return;
      toast(failText(e, 'err-save-fail'));
    }
    loadAccounts('reload');
  }
  /* As deleteParcel. */
  async function deleteAccount(a) {
    try {
      await api('/api/admin/accounts/' + a.id, { method: 'DELETE' });
      toast(tr('toast-deleted'));
    } catch (e) {
      if (lostSession(e)) return LOST;
      toast(e.code === 'not_found' ? tr('toast-deleted') : failText(e, 'err-save-fail'));
    }
    return null;
  }

  /* ─── Config ───
     System storage is either a share of the disk (1 to 95 %, a slider)
     or a fixed size (GB or TB, at most the disk). The bar shows the disk:
     opendrop's parcels, everything else on it, the free part, and a
     mark where the limit falls. The fixed size and the per upload cap
     are shown rounded down; until a field is typed in, saving sends the
     stored bytes as they are, so an untouched form always saves. */
  const cfg = {
    mode: 'percent', percent: 70, unit: 'GB', loaded: false,
    storageBytes: 0, sizeEdited: false, uploadBytes: 0, uploadEdited: false
  };
  const pctInput = $('cfg-storage-percent');
  const sizeInput = $('cfg-storage-size');
  const upInput = $('cfg-max-upload');
  const unitSize = u => (u === 'TB' ? TIB : GIB);
  /* A typed size in bytes; a hair over the disk (a shown value rounded
     up by hand) is the disk. */
  const typedBytes = (v, unit, total) => {
    const b = Math.round(v * unit);
    return b > total && b - total < unit / 100 ? total : b;
  };

  /* Opened, the panel loads afresh: the form hides and the loader stands
     in until the settings are in (a failure brings back the form as it
     was, if it ever loaded). */
  let configSeq = 0;
  async function loadConfig() {
    const seq = ++configSeq;
    const form = $('config-body');
    form.hidden = true;
    let data;
    try {
      data = await OD.loading($('config-loader'), api('/api/admin/overview'), { clear: true });
    } catch (e) {
      if (seq !== configSeq) return;
      loadFailed(e);
      if (cfg.loaded) { form.hidden = false; OD.fadeIn(form); }
      return;
    }
    if (seq !== configSeq) return;
    fillConfig(data);
    form.hidden = false;
    OD.fadeIn(form);
  }
  function fillConfig(data) {
    overview = data;
    const s = overview.settings || {};
    const total = overview.disk.total;
    $('cfg-site-name').value = s.site_name || OD.siteName();
    OD.setSiteName(s.site_name);
    cfg.mode = s.storage_mode === 'bytes' ? 'bytes' : 'percent';
    cfg.percent = s.storage_percent || 70;
    cfg.unit = s.storage_bytes >= TIB ? 'TB' : 'GB';
    pctInput.value = cfg.percent;
    cfg.storageBytes = s.storage_bytes;
    cfg.sizeEdited = false;
    sizeInput.value = shownSize(s.storage_bytes, unitSize(cfg.unit));
    cfg.uploadBytes = s.max_upload_bytes;
    cfg.uploadEdited = false;
    upInput.value = shownSize(s.max_upload_bytes, GIB);
    upInput.max = shownSize(total, GIB);
    $('cfg-max-expiry').value = s.max_expiry_hours;
    setSwitch($('cfg-anon'), !!s.anon_upload);
    setSwitch($('cfg-registration'), !!s.registration);
    $('config-err').hidden = true;
    cfg.loaded = true;
    renderStorageCtl();
  }

  /* The limit the controls describe now, in bytes (null when the typed
     size is not a number). */
  function quotaNow() {
    const total = overview.disk.total;
    if (cfg.mode === 'percent') return Math.floor(total * cfg.percent / 100);
    if (!cfg.sizeEdited) return Math.min(cfg.storageBytes, total);
    const v = num(sizeInput.value);
    return isFinite(v) && v > 0 ? typedBytes(v, unitSize(cfg.unit), total) : null;
  }

  function renderStorageCtl() {
    if (!overview || !cfg.loaded) return;
    const total = overview.disk.total;
    const used = overview.used;
    const avail = overview.disk.avail;
    document.querySelectorAll('#storage-mode .seg-btn').forEach(b => b.setAttribute('aria-checked', String(b.dataset.mode === cfg.mode)));
    document.querySelectorAll('#storage-unit .seg-btn').forEach(b => b.setAttribute('aria-checked', String(b.dataset.unit === cfg.unit)));
    $('storage-percent-line').hidden = cfg.mode !== 'percent';
    $('storage-bytes-line').hidden = cfg.mode !== 'bytes';
    sizeInput.max = shownSize(total, unitSize(cfg.unit));
    sizeInput.step = cfg.unit === 'TB' ? '0.01' : '1';

    const quota = quotaNow();
    pctInput.style.setProperty('--fill', ((cfg.percent - 1) / 94 * 100).toFixed(2) + '%');
    $('storage-percent-value').textContent = tr('percent-value', { p: cfg.percent, size: fmtBytes(Math.floor(total * cfg.percent / 100)) });

    const share = v => (total > 0 ? Math.max(0, Math.min(100, v / total * 100)) : 0);
    const other = Math.max(0, total - avail - used);
    $('disk-od').style.width = share(used).toFixed(2) + '%';
    $('disk-other').style.width = share(other).toFixed(2) + '%';
    const mark = $('disk-mark');
    mark.hidden = quota === null;
    if (quota !== null) mark.style.left = share(quota).toFixed(2) + '%';
    $('lg-od').textContent = tr('lg-od', { v: fmtBytes(used) });
    $('lg-other').textContent = tr('lg-other', { v: fmtBytes(other) });
    $('lg-free').textContent = tr('lg-free', { v: fmtBytes(avail) });
    $('lg-quota').textContent = tr('lg-quota', { v: quota === null ? '?' : fmtBytes(quota) });

    const warn = $('storage-warn');
    if (quota !== null && quota > total) {
      warn.textContent = tr('warn-over-disk', { total: fmtBytes(total) });
      warn.hidden = false;
    } else if (quota !== null && quota < used) {
      warn.textContent = tr('warn-below-used', { used: fmtBytes(used) });
      warn.hidden = false;
    } else {
      warn.hidden = true;
    }
  }

  function setSwitch(btn, on) { btn.setAttribute('aria-checked', String(on)); }
  ['cfg-anon', 'cfg-registration'].forEach(id => {
    $(id).addEventListener('click', () => setSwitch($(id), $(id).getAttribute('aria-checked') !== 'true'));
  });

  document.querySelectorAll('#storage-mode .seg-btn').forEach(b => b.addEventListener('click', () => {
    cfg.mode = b.dataset.mode;
    renderStorageCtl();
  }));
  document.querySelectorAll('#storage-unit .seg-btn').forEach(b => b.addEventListener('click', () => {
    const next = b.dataset.unit;
    if (next === cfg.unit) return;
    const v = num(sizeInput.value);
    if (!cfg.sizeEdited) sizeInput.value = shownSize(cfg.storageBytes, unitSize(next));
    else if (isFinite(v)) sizeInput.value = round2(v * unitSize(cfg.unit) / unitSize(next));
    cfg.unit = next;
    renderStorageCtl();
  }));
  pctInput.addEventListener('input', () => {
    cfg.percent = Math.min(95, Math.max(1, parseInt(pctInput.value, 10) || 70));
    renderStorageCtl();
  });
  sizeInput.addEventListener('input', () => { cfg.sizeEdited = true; renderStorageCtl(); });
  upInput.addEventListener('input', () => { cfg.uploadEdited = true; });

  const fieldLabel = f => ({
    site_name: tr('cfg-site-name'),
    storage_percent: tr('mode-percent'),
    storage_bytes: tr('cfg-max-storage'),
    storage_mode: tr('cfg-max-storage'),
    max_upload_bytes: tr('cfg-max-upload'),
    max_expiry_hours: tr('cfg-max-expiry'),
    anon_upload: tr('cfg-anon'),
    registration: tr('cfg-registration')
  }[f] || f);

  $('config-save').addEventListener('click', async () => {
    if (!overview || !cfg.loaded) return;
    const err = $('config-err');
    err.hidden = true;
    const total = overview.disk.total;
    /* The site name: 1 to 32 characters once trimmed (as the server checks). */
    const siteName = $('cfg-site-name').value.trim();
    const nameLen = Array.from(siteName).length;
    if (nameLen < 1 || nameLen > 32) { showErr(err, tr('err-bad-field', { field: tr('cfg-site-name') })); return; }
    const body = { site_name: siteName, storage_mode: cfg.mode, storage_percent: cfg.percent };
    if (cfg.mode === 'bytes') {
      const q = quotaNow();
      if (q === null || q < 64 * MIB || q > total) { showErr(err, tr('err-bad-field', { field: tr('cfg-max-storage') })); return; }
      body.storage_bytes = q;
    }
    const up = num(upInput.value);
    const upBytes = cfg.uploadEdited ? typedBytes(up, GIB, total) : Math.min(cfg.uploadBytes, total);
    if (!isFinite(upBytes) || upBytes < MIB || upBytes > total) { showErr(err, tr('err-bad-field', { field: tr('cfg-max-upload') })); return; }
    body.max_upload_bytes = upBytes;
    const ex = num($('cfg-max-expiry').value);
    if (!Number.isInteger(ex) || ex < 1 || ex > 8760) { showErr(err, tr('err-bad-field', { field: tr('cfg-max-expiry') })); return; }
    body.max_expiry_hours = ex;
    body.anon_upload = $('cfg-anon').getAttribute('aria-checked') === 'true';
    body.registration = $('cfg-registration').getAttribute('aria-checked') === 'true';
    /* 保存 waits with the save and with the settings read back, so the
       form shows what was stored as soon as it is free again. */
    let saved, fresh;
    try {
      [saved, fresh] = await OD.busy($('config-save'), api('/api/admin/settings', { method: 'PUT', body })
        .then(r => api('/api/admin/overview').then(o => [r, o], () => [r, null])));
    } catch (e) {
      if (lostSession(e)) return;
      showErr(err, e.code === 'bad_field' ? tr('err-bad-field', { field: fieldLabel(e.data && e.data.field) })
        : failText(e, 'err-save-fail'));
      return;
    }
    OD.setSiteName(saved && typeof saved.site_name === 'string' ? saved.site_name : siteName);
    toast(tr('toast-saved'));
    if (fresh) { configSeq++; fillConfig(fresh); } else loadConfig();
  });

  /* ─── Admin credentials ─── */
  $('pw-save').addEventListener('click', async () => {
    const username = $('pw-username').value.trim();
    const current = $('pw-current').value;
    const next = $('pw-next').value;
    const err = $('pw-err');
    err.hidden = true;
    if (!current) { showErr(err, tr('err-fill-current')); return; }
    if (!username && !next) { showErr(err, tr('err-fill-change')); return; }
    if (next && (next.length < 8 || next.length > 128)) { showErr(err, tr('err-pw-short')); return; }
    try {
      await OD.busy($('pw-save'), api('/api/admin/credentials', { body: { current, username: username || undefined, password: next || undefined } }));
    } catch (e) {
      if (lostSession(e)) return;
      const field = e.data && e.data.field;
      showErr(err, e.code === 'bad_credentials' ? tr('err-wrong-pw')
        : e.code === 'username_taken' ? tr('err-name-taken')
          : e.code === 'bad_field' && field === 'username' ? tr('err-bad-username')
            : e.code === 'bad_field' && field === 'password' ? tr('err-pw-short')
              : e.code === 'no_changes' ? tr('err-fill-change')
                : failText(e, 'err-save-fail'));
      return;
    }
    toast(tr('toast-updated'));
    $('pw-current').value = '';
    $('pw-next').value = '';
    $('pw-username').value = '';
  });

  /* ─── Language (the picker is common.js's) ─── */
  OD.onLang(() => {
    renderStats();
    renderParcels();
    renderAccounts();
    renderStorageCtl();
    if (editing) renderEditFlags();
  });

  /* ─── Boot ─── */
  OD.start();
  checkAdmin(true);
  api('/api/info').then(info => { if (info) OD.setSiteName(info.site_name); }, () => { /* the last name seen stays */ });
})();
