/* open.drop main page: the old app's flows on the v2 API.
   Stages: landing / send (pick, uploading, success) / receive (code,
   list) / my files, plus the text, login, edit and confirm modals. */
(function () {
  'use strict';
  const tr = OD.tr, h = OD.h, api = OD.api, toast = OD.toast;
  const fmtBytes = OD.fmtBytes, fmtDate = OD.fmtDate;
  const $ = OD.$;

  /* ─── Strings ─── */
  Object.assign(OD.T.zh, {
    'tagline': '把文件拖入这里即可发送',
    'cta-send': '发送',
    'cta-receive': '接收',
    'hint-drag': '可直接拖拽文件到页面上以发送',
    'hint-paste-a': '可直接按 ',
    'hint-paste-b': ' 以发送剪贴板中的内容',
    'chip-files': '文件',
    'chip-text': '文本',
    'chip-folder': '文件夹',
    'send-title': '您选择了这些文件',
    'not-signed-in': '未登录',
    'signed-in-as': '已登录：{user}',
    'cfg-password': '密码',
    'cfg-password-ph': '可留空',
    'cfg-username': '用户名',
    'cfg-downloads': '可下载次数',
    'cfg-expiry-max': '最长 {max} 小时',
    'cfg-expiry-plain': '有效期（小时）',
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
    'receive-text-ready': '文本已就绪',
    'btn-show-text': '查看文本',
    'drop-overlay': '松手以发送',
    'err-not-found': '取件码不存在',
    'err-bad-password': '密码错误',
    'err-needs-password': '此文件需要密码',
    'err-no-downloads': '下载次数已用完',
    'err-expired': '链接已过期',
    'err-gone': '文件已失效',
    'err-token-expired': '下载链接已过期，请重新接收',
    'err-storage-full': '系统存储已满，无法接收新文件',
    'err-disk-full': '服务器磁盘空间不足，暂时无法接收新文件',
    'err-parcel-too-large': '本次上传超过单次最大限制 {max}',
    'err-too-many-files': '单次最多上传 1000 个文件',
    'err-text-too-long': '文本不能超过 1 MB',
    'err-size-mismatch': '有文件在上传时发生了变化，请重新选择',
    'err-login-required': '请先登录再上传',
    'err-network': '网络错误，请重试',
    'err-wrong-password': '用户名或密码错误',
    'err-username-required': '请输入用户名',
    'err-password-required': '请输入密码',
    'err-bad-username': '用户名需为 1 到 32 个字符，且不含空格',
    'err-bad-password-len': '密码最多 128 个字符',
    'err-password-blank': '密码不能只有空格',
    'err-bad-downloads': '可下载次数需在 1 到 999 之间',
    'err-bad-expiry': '有效期需在 1 到 {max} 小时之间',
    'err-registration-closed': '暂不开放注册',
    'err-not-owner': '没有权限修改这个包裹',
    'err-qr': '二维码生成失败',
    'toast-copied': '链接已复制',
    'toast-code-copied': '取件码已复制',
    'copy-code': '复制取件码',
    'toast-text-copied': '已复制',
    'toast-no-files': '请先选择文件',
    'toast-deleted': '已删除',
    'toast-renewed': '账户已续期 7 天',
    'remaining': '剩余 {n} 次下载',
    'remaining-unlimited': '不限下载次数',
    'sent-clipboard': '已添加剪贴板内容',
    'text-snippet': 'clipboard',
    'send-text-title': '发送文本',
    'send-text-placeholder': '请输入纯文本',
    'receive-text-title': '接收到文本',
    'btn-copy-text': '复制',
    'toast-empty-text': '请输入内容',
    'nav-login': '登录',
    'menu-my-files': '我的文件',
    'menu-renew': '续期账户',
    'menu-logout': '退出登录',
    'login-title': '登录或注册',
    'login-title-closed': '登录',
    'login-sub': '用户名和密码可任意填写，账户有效期 7 天，可随时续期',
    'login-sub-closed': '暂不开放注册，请使用已有账户登录',
    'btn-login': '登录 / 注册',
    'btn-login-closed': '登录',
    'my-files-title': '我的文件',
    'my-files-empty': '还没有上传过文件',
    'my-files-meta': '共 {n} 个包裹，占用 {used}',
    'file-row-meta': '{size}，剩 {n} 次下载，{expiry}',
    'file-row-meta-unlimited': '{size}，不限下载次数，{expiry}',
    'file-row-expired': '已过期',
    'file-row-expires-in': '{h} 小时后过期',
    'file-row-never': '永不过期',
    'btn-delete-parcel': '删除',
    'account-expires': '账户有效期至 {date}',
    'account-never-expires': '管理员账户，永不过期',
    'account-perm-never-expires': '长期账户，永不过期',
    'admin-console': '管理员面板',
    'edit-parcel-title': '编辑包裹',
    'edit-parcel-meta-line': '取件码 {code}',
    'edit-expiry': '有效期 (小时)',
    'edit-downloads': '剩余下载次数',
    'btn-save': '保存',
    'btn-cancel': '取消',
    'btn-confirm': '确定',
    'confirm-delete-title': '确定要删除？',
    'confirm-delete-message': '删除包裹 {code} 会立刻销毁里面的文件，操作无法撤销。',
    'toast-saved': '已保存',
    'err-save-fail': '保存失败',
    'n-files': '{n} 个文件',
    'close': '关闭',
    'cancel': '取消',
    'remove': '移除',
    'download': '下载',
    'edit': '编辑',
    'has-password': '有密码'
  });
  Object.assign(OD.T.en, {
    'tagline': 'Drop files here to send',
    'cta-send': 'Send',
    'cta-receive': 'Receive',
    'hint-drag': 'Drag files anywhere on the page to send',
    'hint-paste-a': 'Press ',
    'hint-paste-b': ' to send clipboard contents',
    'chip-files': 'Files',
    'chip-text': 'Text',
    'chip-folder': 'Folder',
    'send-title': 'Selected files',
    'not-signed-in': 'Not signed in',
    'signed-in-as': 'Signed in as {user}',
    'cfg-password': 'Password',
    'cfg-password-ph': 'Optional',
    'cfg-username': 'Username',
    'cfg-downloads': 'Download limit',
    'cfg-expiry-max': 'Up to {max} hours',
    'cfg-expiry-plain': 'Expiry (hours)',
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
    'receive-text-ready': 'Text ready',
    'btn-show-text': 'Show text',
    'drop-overlay': 'Drop to send',
    'err-not-found': 'Code not found',
    'err-bad-password': 'Wrong password',
    'err-needs-password': 'This parcel needs a password',
    'err-no-downloads': 'Download limit reached',
    'err-expired': 'Link expired',
    'err-gone': 'These files are no longer available',
    'err-token-expired': 'The download link expired, retrieve again',
    'err-storage-full': 'System storage is full, try again later',
    'err-disk-full': 'The server disk is full, try again later',
    'err-parcel-too-large': 'Upload exceeds the {max} per-upload limit',
    'err-too-many-files': 'At most 1000 files per upload',
    'err-text-too-long': 'Text is limited to 1 MB',
    'err-size-mismatch': 'A file changed during the upload, pick it again',
    'err-login-required': 'Sign in to upload',
    'err-network': 'Network error, please retry',
    'err-wrong-password': 'Wrong username or password',
    'err-username-required': 'Username required',
    'err-password-required': 'Password required',
    'err-bad-username': 'Usernames are 1 to 32 characters without spaces',
    'err-bad-password-len': 'Passwords are at most 128 characters',
    'err-password-blank': 'A password cannot be only spaces',
    'err-bad-downloads': 'Downloads must be 1 to 999',
    'err-bad-expiry': 'Expiry must be 1 to {max} hours',
    'err-registration-closed': 'Sign ups are closed',
    'err-not-owner': 'You cannot change this parcel',
    'err-qr': 'QR code unavailable',
    'toast-copied': 'Link copied',
    'toast-code-copied': 'Code copied',
    'copy-code': 'Copy the code',
    'toast-text-copied': 'Copied',
    'toast-no-files': 'Pick some files first',
    'toast-deleted': 'Deleted',
    'toast-renewed': 'Account renewed for 7 days',
    'remaining': '{n} download(s) remaining',
    'remaining-unlimited': 'Unlimited downloads',
    'sent-clipboard': 'Clipboard contents added',
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
    'login-title-closed': 'Sign in',
    'login-sub': 'Any username and password work. Accounts last 7 days and can be renewed any time.',
    'login-sub-closed': 'Sign ups are closed. Sign in with an existing account.',
    'btn-login': 'Sign in / Register',
    'btn-login-closed': 'Sign in',
    'my-files-title': 'My files',
    'my-files-empty': 'No uploads yet',
    'my-files-meta': '{n} parcel(s), {used} stored',
    'file-row-meta': '{size}, {n} download(s) left, {expiry}',
    'file-row-meta-unlimited': '{size}, unlimited downloads, {expiry}',
    'file-row-expired': 'expired',
    'file-row-expires-in': 'expires in {h}h',
    'file-row-never': 'never expires',
    'btn-delete-parcel': 'Delete',
    'account-expires': 'Account valid until {date}',
    'account-never-expires': 'Admin account, never expires',
    'account-perm-never-expires': 'Long-term account, never expires',
    'admin-console': 'Admin panel',
    'edit-parcel-title': 'Edit parcel',
    'edit-parcel-meta-line': 'Pickup code {code}',
    'edit-expiry': 'Expiry (hours from now)',
    'edit-downloads': 'Downloads remaining',
    'btn-save': 'Save',
    'btn-cancel': 'Cancel',
    'btn-confirm': 'Confirm',
    'confirm-delete-title': 'Delete this parcel?',
    'confirm-delete-message': 'Deleting parcel {code} destroys its files immediately and cannot be undone.',
    'toast-saved': 'Saved',
    'err-save-fail': 'Save failed',
    'n-files': '{n} files',
    'close': 'Close',
    'cancel': 'Cancel',
    'remove': 'Remove',
    'download': 'Download',
    'edit': 'Edit',
    'has-password': 'Password protected'
  });

  const TEXT_MAX = 1024 * 1024;
  const MAX_FILES = 1000;

  /* ─── Small helpers ─── */
  function fileExt(name) {
    const base = String(name).split('/').pop();
    const i = base.lastIndexOf('.');
    return i > 0 ? base.slice(i + 1, i + 5).toUpperCase() : 'FILE';
  }
  function intIn(raw, lo, hi) {
    const n = Number(String(raw).trim());
    return Number.isInteger(n) && n >= lo && n <= hi ? n : null;
  }
  function clampInt(raw, lo, hi, fallback) {
    const n = parseInt(raw, 10);
    if (!isFinite(n)) return fallback;
    return Math.min(hi, Math.max(lo, n));
  }
  const retryAfter = err => OD.retryAfter(err);
  /* A call that failed for a reason this page has no words of its own
     for: too many tries, busy, offline, anything else (OD.errText);
     otherwise the given message. */
  const failText = (err, key) => (!err.status || err.status === 429 || err.status >= 500 ? OD.errText(err) : tr(key));
  /* Lines stacked inside one meta paragraph, instead of a separator. */
  function fillLines(el, lines) {
    OD.clear(el);
    lines.filter(Boolean).forEach(s => el.append(h('span', { class: 'line', text: s })));
  }

  /* ─── Stage routing ─── */
  const stages = {
    landing: $('stage-landing'),
    send: $('stage-send'),
    receive: $('stage-receive'),
    'my-files': $('stage-my-files')
  };
  let stageNow = 'landing';
  function showStage(name, substate) {
    stageNow = name;
    Object.keys(stages).forEach(k => stages[k].classList.toggle('is-active', k === name));
    if (name === 'send' || name === 'receive') {
      stages[name].querySelectorAll('.card[data-substate]').forEach(c => {
        c.classList.toggle('is-active', c.dataset.substate === substate);
      });
    } else if (name === 'my-files') {
      stages['my-files'].querySelector('.card').classList.add('is-active');
    }
    window.scrollTo(0, 0);
  }

  /* ─── Auth state ─── */
  let me = null;
  let pendingAfterLogin = null;
  let sendKind = 'files';   // 'files' | 'text' (the text modal sets it)
  let info = null;          // GET /api/info

  /* authEpoch moves on every sign in and sign out, so an answer to an
     older /api/auth/me cannot put back an account signed out since. */
  let authEpoch = 0;
  async function fetchMe() {
    const epoch = authEpoch;
    if (OD.authHint.get()) {
      /* The bar's account control (sign in, sign out or the menu, as this
         browser last was) waits with the answer. */
      const ctl = ['login-btn', 'logout-btn', 'menu-btn'].map(id => $(id)).find(b => b.getClientRects().length > 0) || null;
      const who = await OD.busy(ctl, api('/api/auth/me').catch(err => {
        if (epoch === authEpoch && err.code === 'unauthorized') OD.authHint.set(false);
        return null;
      }));
      if (epoch !== authEpoch) return;
      me = who;
    }
    refreshAuthUi();
  }

  /* The account menu (the homepage's menu, common.js wires it). */
  const navMenu = $('nav-menu');
  const setNavMenu = OD.popover($('menu-btn'), navMenu);

  /* Signed out, the bar offers sign in. Signed in to an account that
     expires, the account menu (renew, sign out; on phones GitHub moves
     into it, od.css: .is-signed-in). An admin or long-term account has
     nothing to renew, and a menu never holds a single action, so sign
     out sits in the bar instead, as sign in does (.is-direct); only on
     phones, where GitHub joins it, does the menu come back, holding
     the two. */
  function setAuthChrome(signedIn, direct) {
    $('login-btn').hidden = signedIn;
    $('logout-btn').hidden = !(signedIn && direct);
    $('menu-wrap').hidden = !signedIn;
    $('navbar').classList.toggle('is-signed-in', signedIn);
    $('navbar').classList.toggle('is-direct', signedIn && direct);
    if (!signedIn) setNavMenu(false);
  }
  const neverExpires = () => !!me && (!!me.is_admin || !!me.is_perm);

  function refreshAuthUi() {
    $('send-card-sub').textContent = me ? tr('signed-in-as', { user: me.username }) : tr('not-signed-in');
    setAuthChrome(!!me, neverExpires());
    if (me) OD.authHint.set(true, neverExpires());
    /* Renew only for accounts that expire (not an admin, not long-term). */
    navMenu.querySelector('[data-action="renew"]').hidden = !me || neverExpires();
    /* An admin also gets the admin panel, left of my files. */
    $('admin-link').hidden = !(me && me.is_admin);
  }

  function accountLine() {
    if (!me) return '';
    if (me.is_admin) return tr('account-never-expires');
    if (me.is_perm || me.expires_at === null || me.expires_at === undefined) return tr('account-perm-never-expires');
    return tr('account-expires', { date: fmtDate(me.expires_at) });
  }

  /* ─── Site name, storage numbers (footer, send limits) ───
     True when `info` is now fresh from the server. */
  async function refreshStorageGauge() {
    try { info = await api('/api/info'); } catch (_) { return false; }
    renderStorage();
    return true;
  }
  function renderStorage() {
    if (info) {
      OD.setSiteName(info.site_name);
      $('footer-storage').textContent = fmtBytes(info.used) + ' / ' + fmtBytes(info.quota);
      $('cfg-expiry').max = info.max_expiry_hours;
    }
    /* The label names the field; the limit sits under it as supporting text. */
    $('cfg-expiry-label').textContent = tr('cfg-expiry-plain');
    $('cfg-expiry-support').textContent = info ? tr('cfg-expiry-max', { max: info.max_expiry_hours }) : '';
    $('cfg-expiry-support').hidden = !info;
  }

  /* ─── Send queue ─── */
  const send = { files: [], job: null };
  const sendFileList = $('send-file-list');
  const fileInput = $('file-input');
  const folderInput = $('folder-input');

  function renderSendList() {
    OD.clear(sendFileList);
    send.files.forEach((f, idx) => {
      sendFileList.append(h('div', { class: 'file-row' },
        h('span', { class: 'file-icon', text: fileExt(f.name), 'aria-hidden': 'true' }),
        h('span', { class: 'file-name sel', title: f.name, text: f.name }),
        h('span', { class: 'file-size tnum', text: fmtBytes(f.size) }),
        h('button', {
          class: 'file-remove ghost icon-btn ripple-surface', type: 'button', 'aria-label': tr('remove') + ' ' + f.name,
          onclick: e => {
            e.stopPropagation();
            send.files.splice(idx, 1);
            renderSendList();
          }
        }, OD.icon('x'))));
    });
  }

  /* items: [{blob, name}] */
  function addFiles(items) {
    if (send.job) return 0;
    let added = 0;
    items.forEach(it => {
      if (!it || !it.blob) return;
      send.files.push({ name: it.name || it.blob.name || tr('text-snippet') + '.txt', size: it.blob.size, blob: it.blob });
      added++;
    });
    if (added > 0) {
      /* Files join a queued text: the whole batch goes as files. */
      sendKind = 'files';
      hideUploadError();
      showStage('send', 'pick');
      renderSendList();
    }
    return added;
  }
  const fromList = (list, relative) => Array.from(list || []).map(f => ({
    blob: f,
    name: (relative && f.webkitRelativePath) || f.name
  }));

  /* ─── Upload ─── */
  const uploadPct = $('upload-pct');
  const uploadBytes = $('upload-bytes');
  const uploadBar = $('upload-bar');
  const uploadBarWrap = $('upload-bar-wrap');
  const uploadFilename = $('upload-filename');
  const uploadError = $('upload-error');

  function showUploadError(msg) {
    uploadError.textContent = msg;
    uploadError.hidden = false;
  }
  function hideUploadError() { uploadError.hidden = true; }
  function goBackToPickWithError(msg) {
    showStage('send', 'pick');
    renderSendList();
    showUploadError(msg);
  }

  let bytesLine = '';
  let processing = null;
  function setProgress(loaded, total) {
    const p = total > 0 ? Math.min(1, loaded / total) : 1;
    const pct = Math.floor(p * 100);
    uploadPct.textContent = String(pct);
    uploadBar.style.width = (p * 100).toFixed(1) + '%';
    uploadBarWrap.setAttribute('aria-valuenow', String(pct));
    bytesLine = fmtBytes(loaded) + ' / ' + fmtBytes(total);
    if (!processing) uploadBytes.textContent = bytesLine;
  }
  /* Before the first byte (the parcel being made) and after the last
     (the parcel being made ready) nothing moves on the card: once such a
     wait is due, the bytes line says 处理中 instead. */
  function whileProcessing(work) {
    const token = {};
    return OD.pace(work, () => {
      processing = token;
      OD.clear(uploadBytes).append(OD.spinner(true), h('span', { 'data-i18n': 'processing', text: tr('processing') }));
      uploadBytes.classList.add('is-processing');
    }, () => {
      if (processing !== token) return;
      processing = null;
      uploadBytes.classList.remove('is-processing');
      uploadBytes.textContent = bytesLine;
    });
  }
  function headline(files) {
    return files.length === 1 ? files[0].name : tr('n-files', { n: files.length });
  }

  function uploadErrText(err) {
    const field = err.data && err.data.field;
    switch (err.code) {
      case 'too_large':
        return tr('err-parcel-too-large', { max: fmtBytes((err.data && err.data.max) || (info && info.max_upload) || 0) });
      case 'quota_full': return tr('err-storage-full');
      case 'disk_full': return tr('err-disk-full');
      case 'login_required':
      case 'unauthorized':
        pendingAfterLogin = 'upload';
        setTimeout(openLoginModal, 0);
        return tr('err-login-required');
      case 'size_mismatch': return tr('err-size-mismatch');
      case 'bad_field':
        if (field === 'files') return tr('err-too-many-files');
        if (field === 'text') return tr('err-text-too-long');
        if (field === 'downloads') return tr('err-bad-downloads');
        if (field === 'expiry_hours') return tr('err-bad-expiry', { max: info ? info.max_expiry_hours : 168 });
        if (field === 'password') return tr('err-bad-password-len');
        break;
      case 'timeout':
      case 'too_slow':
      case 'network':
      case 'aborted':
        return tr('err-network');
    }
    return OD.errText(err);
  }

  /* The send settings: read (and tidied) from the pick card's fields, or
     the defaults, which the text modal sends with (as before) without
     writing them into the fields. A password goes exactly as typed, the
     rule everywhere a parcel password is entered; only one made of
     nothing but spaces is refused, since it would read as no password. */
  const maxHours = () => (info ? info.max_expiry_hours : 168);
  const defaultOpts = () => ({ password: '', downloads: 20, expiry_hours: Math.min(48, maxHours()) });
  function readOpts() {
    const maxH = maxHours();
    const password = $('cfg-password').value;
    if (password !== '' && !password.trim()) { showUploadError(tr('err-password-blank')); return null; }
    const opts = {
      password,
      downloads: clampInt($('cfg-downloads').value, 1, 999, 20),
      expiry_hours: clampInt($('cfg-expiry').value, 1, maxH, Math.min(48, maxH))
    };
    $('cfg-downloads').value = opts.downloads;
    $('cfg-expiry').value = opts.expiry_hours;
    return opts;
  }
  /* A finished, cancelled or closed send leaves the defaults for the next
     one, so a password never rides along unseen. */
  function resetSendSettings() {
    const d = defaultOpts();
    $('cfg-password').value = d.password;
    $('cfg-downloads').value = d.downloads;
    $('cfg-expiry').value = d.expiry_hours;
  }

  /* The limits are asked for fresh right before the checks; when that
     fails the upload goes ahead and the server's answer is shown. The
     button that sent it (上传, or 发送 in the text modal) waits with that
     call; then `onGo` (closing the text modal) runs. */
  let starting = false;
  async function startUpload(useDefaults, btn, onGo) {
    if (send.job || starting) return;
    starting = true;
    try { await prepareUpload(useDefaults === true, btn, onGo); } finally { starting = false; }
  }
  async function prepareUpload(useDefaults, btn, onGo) {
    hideUploadError();
    if (send.files.length === 0) { showUploadError(tr('toast-no-files')); return; }
    const isText = sendKind === 'text' && send.files.length === 1;
    if (!isText && send.files.length > MAX_FILES) { showUploadError(tr('err-too-many-files')); return; }
    const fresh = await OD.busy(btn, refreshStorageGauge());
    if (onGo) onGo();
    if (send.job || send.files.length === 0) return;
    if (fresh && !me && info.anon_upload === false) {
      showUploadError(tr('err-login-required'));
      pendingAfterLogin = 'upload';
      openLoginModal();
      return;
    }
    const opts = useDefaults ? defaultOpts() : readOpts();
    if (!opts) return;

    if (isText) {
      let text = '';
      try { text = await send.files[0].blob.text(); } catch (_) { text = ''; }
      if (send.job) return;
      uploadText(text, opts);
      return;
    }
    const total = send.files.reduce((s, f) => s + f.size, 0);
    if (fresh) {
      if (total > info.max_upload) { showUploadError(tr('err-parcel-too-large', { max: fmtBytes(info.max_upload) })); return; }
      if (info.used + total > info.quota) { showUploadError(tr('err-storage-full')); return; }
    }
    uploadFiles(opts, total);
  }

  function beginUploadCard(files, total) {
    uploadFilename.textContent = headline(files);
    setProgress(0, total);
    showStage('send', 'uploading');
  }

  /* Each file job guards the tab with its own listener, so an old job
     winding down after a cancel never removes the guard of a newer one. */
  function guardLeave(job) {
    job.onLeave = e => { e.preventDefault(); e.returnValue = ''; };
    window.addEventListener('beforeunload', job.onLeave);
  }
  function unguardLeave(job) {
    if (job.onLeave) window.removeEventListener('beforeunload', job.onLeave);
    job.onLeave = null;
  }

  /* Text: one call makes the parcel and makes it ready, so from the
     moment it is sent the parcel may exist (no abort could be sure to
     stop it): cancel never aborts it, see cancelUpload. */
  async function uploadText(text, opts) {
    const bytes = new TextEncoder().encode(text).length;
    if (!text.trim()) { showStage('send', 'pick'); renderSendList(); showUploadError(tr('toast-empty-text')); return; }
    if (bytes > TEXT_MAX) { goBackToPickWithError(tr('err-text-too-long')); return; }
    const job = { stop: false, text: true, committing: true, xhrs: new Set() };
    send.job = job;
    beginUploadCard(send.files, bytes);
    try {
      const r = await whileProcessing(api('/api/parcels', { body: Object.assign({ kind: 'text', text }, opts) }));
      if (job.stop) { lateCode(r.code); return; }
      setProgress(bytes, bytes);
      finishSend(r.code);
    } catch (err) {
      if (job.stop) return;
      goBackToPickWithError(uploadErrText(err));
    } finally {
      if (send.job === job) send.job = null;
    }
  }

  /* Files: create the parcel, PUT each file as its raw body (XHR, for
     progress), three at a time, each retried up to 3 times after 1, 3
     and 9 s (longer when the server asks), then finish. Cancel aborts
     everything and deletes the parcel. */
  function dropParcel(job) {
    api('/api/parcels/' + job.id, { method: 'DELETE', headers: { 'X-Upload-Token': job.token } }).catch(() => {});
  }
  async function uploadFiles(opts, total) {
    const files = send.files.slice();
    const job = {
      id: null, token: null, files, total, stop: false, committing: false,
      loaded: files.map(() => 0), xhrs: new Set(), timers: new Set(), frame: 0
    };
    send.job = job;
    guardLeave(job);
    beginUploadCard(files, total);
    try {
      const r = await whileProcessing(api('/api/parcels', {
        body: Object.assign({ kind: 'files', files: files.map(f => ({ name: f.name, size: f.size })) }, opts)
      }));
      job.id = r.id;
      job.token = r.token;
      if (job.stop) { dropParcel(job); return; }
      let next = 0;
      const worker = async () => {
        while (!job.stop && next < files.length) await putFile(job, next++);
      };
      await Promise.all(Array.from({ length: Math.min(3, files.length) }, worker));
      if (job.stop) return;
      job.committing = true;
      const fin = await whileProcessing(api('/api/parcels/' + job.id + '/finish', { method: 'POST', headers: { 'X-Upload-Token': job.token } }));
      if (job.stop) { lateCode(fin.code); return; }
      setProgress(total, total);
      finishSend(fin.code);
    } catch (err) {
      if (job.stop) return;
      job.stop = true;
      job.xhrs.forEach(x => x.abort());
      if (job.id) dropParcel(job);
      goBackToPickWithError(uploadErrText(err));
    } finally {
      job.timers.forEach(clearTimeout);
      unguardLeave(job);
      if (send.job === job) send.job = null;
    }
  }
  function wait(job, ms) {
    return new Promise(resolve => {
      const id = setTimeout(() => { job.timers.delete(id); resolve(); }, ms);
      job.timers.add(id);
    });
  }
  async function putFile(job, i) {
    const delays = [1000, 3000, 9000];
    for (let attempt = 0; ; attempt++) {
      try {
        await putOnce(job, i);
        return;
      } catch (err) {
        if (job.stop || err.fatal || attempt >= delays.length) throw err;
        job.loaded[i] = 0;
        drawJob(job);
        const ms = err.status === 429 ? Math.min(30000, Math.max(delays[attempt], retryAfter(err) * 1000)) : delays[attempt];
        await wait(job, ms);
        if (job.stop) throw new OD.ApiError('aborted', 0);
      }
    }
  }
  function drawJob(job) {
    if (job.frame) return;
    job.frame = requestAnimationFrame(() => {
      job.frame = 0;
      if (send.job !== job || job.stop) return;
      setProgress(job.loaded.reduce((a, b) => a + b, 0), job.total);
    });
  }
  function putOnce(job, i) {
    return new Promise((resolve, reject) => {
      const it = job.files[i];
      const x = new XMLHttpRequest();
      job.xhrs.add(x);
      x.open('PUT', '/api/parcels/' + job.id + '/files/' + i);
      x.setRequestHeader('X-Upload-Token', job.token);
      x.setRequestHeader('Content-Type', 'application/octet-stream');
      x.upload.onprogress = e => {
        job.loaded[i] = Math.min(e.loaded, it.size);
        drawJob(job);
      };
      x.onload = () => {
        job.xhrs.delete(x);
        if (x.status >= 200 && x.status < 300) { job.loaded[i] = it.size; drawJob(job); resolve(); return; }
        let code = null, data = null;
        try {
          data = JSON.parse(x.responseText);
          if (data && OD.codeOk(data.error)) code = data.error;
        } catch (_) { data = null; }
        if (x.status === 409 && code === 'already_done') { job.loaded[i] = it.size; resolve(); return; }
        const err = new OD.ApiError(code || OD.statusCode(x.status), x.status, data);
        err.fatal = [400, 401, 403, 404, 409, 413, 507].indexOf(x.status) >= 0;
        reject(err);
      };
      x.onerror = () => { job.xhrs.delete(x); reject(new OD.ApiError('network', 0)); };
      x.onabort = () => {
        job.xhrs.delete(x);
        const err = new OD.ApiError('aborted', 0);
        err.fatal = true;
        reject(err);
      };
      x.send(it.blob);
    });
  }

  /* Cancel aborts what is running and deletes the parcel. Once the call
     that makes it ready is out (job.committing: a text's only call, the
     finish of files) the parcel may exist already: signed in, that call
     is left to answer and its code is deleted (lateCode); anonymous,
     nothing could delete it, so the card stays and shows the code when
     it comes. False when the card stays. */
  function cancelUpload() {
    const job = send.job;
    if (!job) return true;
    if (job.committing && !me) return false;
    job.stop = true;
    if (!job.committing) job.xhrs.forEach(x => x.abort());
    if (job.id) dropParcel(job);
    unguardLeave(job);
    send.job = null;
    return true;
  }
  /* The code of a parcel that a cancel came too late to stop: deleted
     when it is this account's, otherwise shown, never thrown away. */
  function lateCode(code) {
    if (!code) return;
    const show = () => { if (!send.job) revealSuccess(code); };
    if (!me) { show(); return; }
    api('/api/me/parcels/' + code, { method: 'DELETE' }).then(refreshStorageGauge, err => {
      if (err.code !== 'not_found') show();
    });
  }

  /* A finished send: the queue empties, the settings go back to the
     defaults, the success card shows. */
  function finishSend(code) {
    send.files = [];
    sendKind = 'files';
    renderSendList();
    resetSendSettings();
    refreshStorageGauge();
    revealSuccess(code);
  }

  /* ─── Success view and sharing ─── */
  const pickupCodeEl = $('pickup-code');
  const qrPanel = $('qr-panel');
  const qrImg = $('qr-img');
  const qrBtn = $('btn-show-qr');
  const successShare = $('success-share-url');
  let lastCode = '';

  const shareUrl = code => location.origin + '/d/' + code;

  function revealSuccess(code) {
    lastCode = code;
    pickupCodeEl.textContent = code.split('').join(' ');
    pickupCodeEl.setAttribute('aria-label', tr('copy-code') + ' ' + code);
    qrPanel.hidden = true;
    qrBtn.setAttribute('aria-expanded', 'false');
    qrImg.removeAttribute('src');
    successShare.textContent = shareUrl(code);
    successShare.setAttribute('aria-label', tr('copy-link'));
    showStage('send', 'success');
  }

  async function copyShareLink() {
    if (!lastCode) return;
    if (await OD.copy(shareUrl(lastCode))) toast(tr('toast-copied'));
  }

  /* The code and the link copy themselves when clicked (or Enter/Space). */
  async function copyCode() {
    if (!lastCode) return;
    if (await OD.copy(lastCode)) toast(tr('toast-code-copied'));
  }
  const pressable = (el, fn) => {
    el.addEventListener('click', fn);
    el.addEventListener('keydown', e => {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); fn(); }
    });
  };
  pressable(pickupCodeEl, copyCode);
  pressable(successShare, copyShareLink);

  /* The code opens once its image is in, the button waiting with it. */
  function toggleQr() {
    if (!lastCode) return;
    const show = qrPanel.hidden;
    const open = () => {
      qrPanel.hidden = !show;
      qrBtn.setAttribute('aria-expanded', String(show));
    };
    if (!show || (qrImg.getAttribute('src') && qrImg.complete)) { open(); return; }
    const src = '/api/qr?u=' + encodeURIComponent(shareUrl(lastCode));
    const loaded = new Promise(resolve => {
      const done = () => {
        qrImg.removeEventListener('load', done);
        qrImg.removeEventListener('error', done);
        clearTimeout(timer);
        resolve();
      };
      const timer = setTimeout(done, 20000);
      qrImg.addEventListener('load', done);
      qrImg.addEventListener('error', done);
    });
    if (!qrImg.getAttribute('src')) qrImg.src = src;
    OD.busy(qrBtn, loaded).then(() => {
      /* Opened only for the code it was asked for, and only with its image
         (a failed one has had its src taken away, and said so). */
      if (qrImg.getAttribute('src') === src && qrImg.complete && qrPanel.hidden) open();
    });
  }
  qrImg.addEventListener('error', () => {
    if (!qrImg.getAttribute('src')) return;
    qrImg.removeAttribute('src');
    qrPanel.hidden = true;
    qrBtn.setAttribute('aria-expanded', 'false');
    toast(tr('err-qr'));
  });

  /* ─── Receive ─── */
  const codeInputs = Array.from(document.querySelectorAll('.code-digit'));
  const codeError = $('code-error');
  const codePasswordRow = $('code-password-row');
  const receivePasswordInput = $('receive-password');
  const receiveFileList = $('receive-file-list');
  const receiveMeta = $('receive-meta');

  let currentReceive = null;
  let fetchSeq = 0;

  const readCode = () => codeInputs.map(i => i.value || '').join('');
  function setCode(code) {
    codeInputs.forEach((inp, idx) => {
      inp.value = code[idx] || '';
      inp.classList.toggle('is-filled', !!inp.value);
    });
  }
  function clearCodeInputs() {
    fetchSeq++;
    codeInputs.forEach(inp => { inp.value = ''; inp.classList.remove('is-filled'); });
    codeError.hidden = true;
    codePasswordRow.hidden = true;
    receivePasswordInput.value = '';
  }
  function showCodeError(msg) {
    codeError.textContent = msg;
    codeError.hidden = false;
  }

  codeInputs.forEach((inp, idx) => {
    inp.addEventListener('input', () => {
      const v = inp.value.replace(/\D/g, '').slice(0, 1);
      inp.value = v;
      inp.classList.toggle('is-filled', !!v);
      if (v && idx < codeInputs.length - 1) codeInputs[idx + 1].focus();
      if (readCode().length === 6) attemptFetch();
    });
    inp.addEventListener('keydown', e => {
      if (e.key === 'Backspace' && !inp.value && idx > 0) {
        codeInputs[idx - 1].focus();
        codeInputs[idx - 1].value = '';
        codeInputs[idx - 1].classList.remove('is-filled');
      }
      if (e.key === 'ArrowLeft' && idx > 0) codeInputs[idx - 1].focus();
      if (e.key === 'ArrowRight' && idx < codeInputs.length - 1) codeInputs[idx + 1].focus();
      if (e.key === 'Enter') attemptFetch();
    });
    inp.addEventListener('paste', e => {
      const txt = (e.clipboardData || window.clipboardData).getData('text');
      const digits = (txt || '').replace(/\D/g, '').slice(0, 6);
      if (digits.length > 0) {
        e.preventDefault();
        setCode(digits);
        if (digits.length === 6) attemptFetch();
        else codeInputs[Math.min(digits.length, 5)].focus();
      }
    });
  });
  receivePasswordInput.addEventListener('keydown', e => { if (e.key === 'Enter') attemptFetch(); });

  function mapParcelError(err) {
    switch (err && err.code) {
      case 'not_found':
      case 'bad_code': return tr('err-not-found');
      case 'expired': return tr('err-expired');
      case 'exhausted': return tr('err-no-downloads');
      case 'bad_password': return tr('err-bad-password');
      case 'password_required': return tr('err-needs-password');
      case 'gone': return tr('err-gone');
      case 'bad_token': return tr('err-token-expired');
      default: return OD.errText(err);
    }
  }

  async function attemptFetch() {
    const code = readCode();
    if (code.length !== 6) return;
    codeError.hidden = true;
    /* Sent exactly as typed, the way the sender's was stored. */
    const password = receivePasswordInput.value;
    const seq = ++fetchSeq;

    /* Info only: it does not use up a download, for files or text. The
       claim (which does) waits for the first download click, or for
       查看文本 on a text; ensureClaim() below. So a typed code, an
       opened /d/ link or a link scanner never spends one. */
    let body;
    try {
      body = await OD.busy($('btn-fetch'), api('/api/p/' + code, { body: password ? { password } : {} }));
    } catch (err) {
      if (seq !== fetchSeq) return;
      if (err.code === 'password_required' && codePasswordRow.hidden) {
        codePasswordRow.hidden = false;
        receivePasswordInput.focus();
        return;
      }
      showCodeError(mapParcelError(err));
      return;
    }
    if (seq !== fetchSeq) return;

    currentReceive = {
      code,
      password,
      files: Array.isArray(body.files) ? body.files : [],
      kind: body.kind || 'files',
      downloads_left: body.downloads_left,
      token: null,
      tokenUntil: 0,
      text: null
    };

    renderReceiveMeta();
    renderReceiveList();
    showStage('receive', 'list');
    /* The code card is hidden now: the focus moves to the card's action. */
    $('btn-download-all').focus();
  }

  /* The list card names what is waiting: files (each a row, and 下载全部)
     or a text (no rows, and 查看文本, which opens it). */
  function renderReceiveMeta() {
    const rx = currentReceive;
    if (!rx) return;
    const isText = rx.kind === 'text';
    $('receive-list-title').textContent = tr(isText ? 'receive-text-ready' : 'receive-list-title');
    $('btn-download-all-label').textContent = tr(isText ? 'btn-show-text' : 'btn-download-all');
    receiveMeta.textContent = rx.downloads_left === null || rx.downloads_left === undefined
      ? tr('remaining-unlimited')
      : tr('remaining', { n: rx.downloads_left });
  }
  function renderReceiveList() {
    const rx = currentReceive;
    OD.clear(receiveFileList);
    if (!rx || rx.kind === 'text') return;
    rx.files.forEach(f => {
      /* The row's download button waits with the download it starts. */
      const btn = h('button', {
        class: 'file-remove ghost icon-btn ripple-surface', type: 'button', 'aria-label': tr('download') + ' ' + f.name,
        onclick: e => { e.stopPropagation(); go(); }
      }, OD.icon('download'));
      const go = () => { if (!btn.disabled) triggerDownload(String(f.i), btn); };
      receiveFileList.append(h('div', { class: 'file-row is-click', onclick: go },
        h('span', { class: 'file-icon', text: fileExt(f.name), 'aria-hidden': 'true' }),
        h('span', { class: 'file-name sel', title: f.name, text: f.name }),
        h('span', { class: 'file-size tnum', text: fmtBytes(f.size) }),
        btn));
    });
  }

  /* One claim per receive: the first download click makes it (costing
     one download); later clicks, and download all, reuse its token. The
     token lives an hour on the server; here it is trusted for 55 minutes
     from when the answer arrived, on this clock alone (the server's
     expires_at is on its clock, which may not match this one). A token
     that still turns out expired is dropped by triggerDownload. */
  const TOKEN_TRUST_MS = 55 * 60000;
  function ensureClaim(quiet) {
    const rx = currentReceive;
    if (!rx) return Promise.resolve(null);
    if (rx.token && Date.now() < rx.tokenUntil) return Promise.resolve(rx.token);
    if (rx.claiming) return rx.claiming;
    rx.claimError = null;
    rx.claiming = api('/api/p/' + rx.code + '/claim', { body: rx.password ? { password: rx.password } : {} })
      .then(c => {
        rx.token = c.token;
        rx.tokenUntil = Date.now() + TOKEN_TRUST_MS;
        if (typeof c.text === 'string') rx.text = c.text;
        rx.downloads_left = c.downloads_left;
        if (currentReceive === rx) renderReceiveMeta();
        return c.token;
      }, err => {
        rx.claimError = err;
        if (!quiet) toast(mapParcelError(err));
        return null;
      })
      .finally(() => { rx.claiming = null; });
    return rx.claiming;
  }

  /* `btn`, the button clicked, waits with the claim and the check. */
  async function triggerDownload(which, btn) {
    const rx = currentReceive;
    if (!rx) return;
    const url = await OD.busy(btn, (async () => {
      const token = await ensureClaim(false);
      if (!token) return null;
      const link = '/api/dl/' + encodeURIComponent(token) + '/' + which;
      /* Ask first, so an expired link shows a message instead of saving
         an error as a file. */
      let res = null;
      try { res = await fetch(link, { method: 'HEAD', credentials: 'same-origin', cache: 'no-store' }); } catch (_) { res = null; }
      if (res && !res.ok) {
        if (res.status === 403 || res.status === 410) { rx.token = null; rx.tokenUntil = 0; }
        toast(res.status === 410 ? tr('err-gone') : res.status === 403 ? tr('err-token-expired')
          : OD.errText({ code: OD.statusCode(res.status), status: res.status, data: { retry_after: res.headers.get('Retry-After') } }));
        return null;
      }
      return link;
    })());
    if (!url) return;
    const a = h('a', { href: url, download: '', class: 'offscreen' });
    document.body.appendChild(a);
    a.click();
    a.remove();
  }

  /* Text: the shown text is the download, so it is claimed only here,
     when 查看文本 asks for it; then the receive-text modal opens over the
     landing, as before. A failed claim says why in a toast. */
  async function openReceivedText(btn) {
    const rx = currentReceive;
    if (!rx) return;
    if (typeof rx.text !== 'string') {
      const token = await OD.busy(btn, ensureClaim(false));
      if (currentReceive !== rx || !token || typeof rx.text !== 'string') return;
    }
    showReceiveTextModal(rx.text);
    clearCodeInputs();
    currentReceive = null;
    showStage('landing');
  }
  function showReceiveTextModal(text) {
    $('receive-text-area').value = text;
    /* Closing it lands on the landing, so the focus goes back to 接收. */
    OD.openModal($('receive-text-modal'), $('btn-copy-text'), $('btn-receive'));
  }

  /* ─── Text modal ─── */
  function openSendTextModal() {
    $('send-text-area').value = '';
    OD.openModal($('send-text-modal'), $('send-text-area'));
    setTimeout(() => $('send-text-area').focus(), 60);
  }

  /* ─── Drag and drop, paste ─── */
  const dropOverlay = $('drop-overlay');
  let dragDepth = 0;
  const hasFiles = e => !!(e.dataTransfer && Array.from(e.dataTransfer.types || []).indexOf('Files') >= 0);
  const blocked = () => !!send.job || OD.modalOpen();

  window.addEventListener('dragenter', e => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    if (blocked()) return;
    dragDepth++;
    dropOverlay.classList.add('is-active');
  });
  window.addEventListener('dragover', e => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = blocked() ? 'none' : 'copy';
  });
  window.addEventListener('dragleave', () => {
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) dropOverlay.classList.remove('is-active');
  });

  /* Folders are walked to every file inside, keeping relative paths
     (dir/sub/file.txt). readEntries returns in chunks: read until empty. */
  function walkEntry(entry, path) {
    return new Promise(resolve => {
      if (!entry) { resolve([]); return; }
      if (entry.isFile) {
        entry.file(f => resolve([{ blob: f, name: path + f.name }]), () => resolve([]));
      } else if (entry.isDirectory) {
        const reader = entry.createReader();
        const all = [];
        const readBatch = () => {
          reader.readEntries(async entries => {
            if (!entries.length) { resolve(all); return; }
            for (const sub of entries) all.push(...await walkEntry(sub, path + entry.name + '/'));
            readBatch();
          }, () => resolve(all));
        };
        readBatch();
      } else {
        resolve([]);
      }
    });
  }

  window.addEventListener('drop', async e => {
    if (!e.dataTransfer) return;
    e.preventDefault();
    dragDepth = 0;
    dropOverlay.classList.remove('is-active');
    if (blocked()) return;
    const items = e.dataTransfer.items;
    if (items && items.length && typeof items[0].webkitGetAsEntry === 'function') {
      const entries = [];
      for (const it of items) {
        if (it.kind !== 'file') continue;
        const en = it.webkitGetAsEntry();
        if (en) entries.push(en);
      }
      if (entries.length) {
        const found = [];
        for (const en of entries) found.push(...await walkEntry(en, ''));
        if (found.length) addFiles(found);
        return;
      }
    }
    if (e.dataTransfer.files && e.dataTransfer.files.length) addFiles(fromList(e.dataTransfer.files, false));
  });

  window.addEventListener('paste', e => {
    const t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA')) return;
    if (blocked()) return;
    const items = e.clipboardData && e.clipboardData.items;
    if (!items) return;
    const files = [];
    for (const it of items) {
      if (it.kind === 'file') {
        const f = it.getAsFile();
        if (f) files.push({ blob: f, name: f.name });
      }
    }
    if (files.length) { addFiles(files); return; }
    const text = e.clipboardData.getData('text');
    if (text && text.trim()) {
      const blob = new Blob([text], { type: 'text/plain' });
      if (addFiles([{ blob, name: tr('text-snippet') + '.txt' }])) toast(tr('sent-clipboard'));
    }
  });

  /* ─── Wiring ─── */
  /* Back to the start, or, while an upload runs, back to its card, so
     it never goes on hidden and out of reach of its cancel. */
  function home() {
    if (send.job) showStage('send', 'uploading');
    else showStage('landing');
  }
  $('btn-send').addEventListener('click', () => {
    if (send.job) { showStage('send', 'uploading'); return; }
    hideUploadError();
    /* Files already queued (from a drop or a paste): back to the card.
       Otherwise the OS picker opens and the landing stays until files
       arrive. */
    if (send.files.length > 0) {
      showStage('send', 'pick');
      renderSendList();
    } else {
      fileInput.click();
    }
  });
  $('btn-receive').addEventListener('click', () => {
    clearCodeInputs();
    showStage('receive', 'code');
    setTimeout(() => codeInputs[0].focus(), 60);
  });

  document.querySelectorAll('.hint-chip').forEach(chip => {
    chip.addEventListener('click', e => {
      e.stopPropagation();
      const pick = chip.dataset.pick;
      if (pick === 'files') fileInput.click();
      else if (pick === 'folder') folderInput.click();
      else if (pick === 'text') openSendTextModal();
    });
  });

  fileInput.addEventListener('change', () => {
    if (fileInput.files && fileInput.files.length) addFiles(fromList(fileInput.files, false));
    fileInput.value = '';
  });
  folderInput.addEventListener('change', () => {
    if (folderInput.files && folderInput.files.length) addFiles(fromList(folderInput.files, true));
    folderInput.value = '';
  });

  document.addEventListener('click', e => {
    const action = e.target.closest('[data-action]');
    if (!action) return;
    switch (action.dataset.action) {
      case 'cancel':
      case 'reset':
        /* A running upload keeps its queue and settings (a failure puts
           them back on the pick card for a retry). */
        if (!send.job) {
          send.files = [];
          sendKind = 'files';
          renderSendList();
          resetSendSettings();
        }
        clearCodeInputs();
        currentReceive = null;
        home();
        break;
      case 'cancel-upload':
        if (!cancelUpload()) break;
        send.files = [];
        sendKind = 'files';
        renderSendList();
        resetSendSettings();
        showStage('landing');
        break;
      case 'login-close':
        OD.closeModal($('login-modal'));
        pendingAfterLogin = null;
        break;
      case 'send-text-close':
        OD.closeModal($('send-text-modal'));
        break;
      case 'receive-text-close':
        OD.closeModal($('receive-text-modal'));
        home();
        break;
      case 'edit-parcel-close':
        OD.closeModal($('edit-parcel-modal'));
        editingParcel = null;
        break;
      case 'confirm-cancel':
        OD.closeModal($('confirm-modal'));
        confirmAction = null;
        break;
      case 'login':
        openLoginModal();
        break;
      /* From the account menu, which closes, the menu button waits. */
      case 'renew':
        setNavMenu(false);
        renewAccount($('menu-btn'));
        break;
      case 'logout':
        setNavMenu(false);
        logout(navMenu.contains(action) ? $('menu-btn') : action);
        break;
    }
  });

  /* Text send: the queue becomes this one text, sent with the default
     settings (no password, 2 downloads, 24 hours) as before; the pick
     card's fields are left as they are. 发送 waits with the first call,
     then the modal closes and the upload card takes over. */
  $('btn-send-text').addEventListener('click', () => {
    if (send.job || starting) return;
    const text = $('send-text-area').value;
    if (!text || !text.trim()) { toast(tr('toast-empty-text')); return; }
    if (new TextEncoder().encode(text).length > TEXT_MAX) { toast(tr('err-text-too-long')); return; }
    const blob = new Blob([text], { type: 'text/plain' });
    send.files = [{ name: tr('text-snippet') + '.txt', size: blob.size, blob }];
    sendKind = 'text';
    startUpload(true, $('btn-send-text'), () => OD.closeModal($('send-text-modal')));
  });

  $('btn-copy-text').addEventListener('click', async () => {
    if (await OD.copy($('receive-text-area').value)) toast(tr('toast-text-copied'));
  });

  $('btn-upload').addEventListener('click', () => startUpload(false, $('btn-upload')));
  $('btn-fetch').addEventListener('click', attemptFetch);
  $('btn-copy-link').addEventListener('click', copyShareLink);
  $('btn-show-qr').addEventListener('click', toggleQr);
  /* Files: more than one comes as one zip (separate clicks get blocked).
     A text opens in the receive-text modal. */
  $('btn-download-all').addEventListener('click', () => {
    const rx = currentReceive;
    const btn = $('btn-download-all');
    if (!rx) return;
    if (rx.kind === 'text') { openReceivedText(btn); return; }
    if (!rx.files.length) return;
    triggerDownload(rx.files.length > 1 ? 'zip' : String(rx.files[0].i), btn);
  });

  /* My files: the list for anyone signed in (an admin's own parcels
     too); anyone else signs in first. */
  function openMyFiles() {
    if (me) {
      loadMyFiles();
    } else {
      pendingAfterLogin = 'my-files';
      openLoginModal();
    }
  }
  $('my-files-link').addEventListener('click', openMyFiles);

  /* ─── Login ─── */
  function setLoginTexts() {
    const closed = !!info && info.registration === false;
    $('login-title').textContent = tr(closed ? 'login-title-closed' : 'login-title');
    $('login-sub').textContent = tr(closed ? 'login-sub-closed' : 'login-sub');
    $('btn-login-label').textContent = tr(closed ? 'btn-login-closed' : 'btn-login');
  }
  function openLoginModal() {
    setLoginTexts();
    $('login-error').hidden = true;
    OD.openModal($('login-modal'), $('login-username'));
    setTimeout(() => $('login-username').focus(), 60);
  }

  $('btn-login-submit').addEventListener('click', login);
  ['login-username', 'login-password'].forEach(id => {
    $(id).addEventListener('keydown', e => { if (e.key === 'Enter') login(); });
  });

  let loggingIn = false;
  async function login() {
    if (loggingIn) return;
    const u = $('login-username').value.trim();
    const p = $('login-password').value;
    const err = $('login-error');
    const fail = msg => { err.textContent = msg; err.hidden = false; };
    if (!u) { fail(tr('err-username-required')); return; }
    if (!p) { fail(tr('err-password-required')); return; }
    err.hidden = true;
    loggingIn = true;
    try {
      me = await OD.busy($('btn-login-submit'), api('/api/auth/login', { body: { username: u, password: p } }));
      authEpoch++;
    } catch (e) {
      const field = e.data && e.data.field;
      fail(e.code === 'bad_credentials' ? tr('err-wrong-password')
        : e.code === 'registration_closed' ? tr('err-registration-closed')
          : e.code === 'bad_field' && field === 'username' ? tr('err-bad-username')
            : e.code === 'bad_field' && field === 'password' ? tr('err-bad-password-len')
              : OD.errText(e));
      return;
    } finally {
      loggingIn = false;
    }
    $('login-password').value = '';
    OD.closeModal($('login-modal'));
    refreshAuthUi();
    const pending = pendingAfterLogin;
    pendingAfterLogin = null;
    if (pending === 'my-files') openMyFiles();
    else if (pending === 'upload') hideUploadError();
  }

  /* Only the server can end the session (its cookie is HttpOnly), so
     this page counts as signed out only once the server says so; a
     failed call leaves it signed in, says why, and can be tried again. */
  let loggingOut = false;
  async function logout(btn) {
    if (loggingOut) return;
    loggingOut = true;
    try {
      await OD.busy(btn, api('/api/auth/logout', { method: 'POST' }));
    } catch (err) {
      toast(OD.errText(err));
      return;
    } finally {
      loggingOut = false;
    }
    authEpoch++;
    OD.authHint.set(false);
    me = null;
    refreshAuthUi();
    home();
  }

  async function renewAccount(btn) {
    try {
      const r = await OD.busy(btn, api('/api/auth/renew', { method: 'POST' }));
      me = Object.assign({}, me, { expires_at: r.expires_at });
      toast(tr('toast-renewed'));
      refreshAuthUi();
    } catch (err) {
      toast(OD.errText(err));
    }
  }

  /* ─── My files ─── */
  let myParcels = null;
  let myFilesSeq = 0;

  /* Opening: the stage comes at once, the list with its loader (and the
     count line held blank), never the list of a visit before. Reloading
     (after an edit or a delete, `reload`): the list stays through a
     quick answer, the loader takes over a slow one. */
  async function loadMyFiles(reload) {
    const seq = ++myFilesSeq;
    const list = $('my-files-list');
    if (!reload) {
      fillLines($('my-files-meta'), ['\u00a0', accountLine()]);
      showStage('my-files');
    }
    let body;
    try {
      body = await OD.loading(list, api('/api/me/parcels'), { clear: !reload });
    } catch (err) {
      if (seq !== myFilesSeq) return;
      if (err.code === 'unauthorized') { me = null; OD.authHint.set(false); refreshAuthUi(); home(); }
      else {
        toast(OD.errText(err));
        if (reload && myParcels) renderMyFiles();
        else if (stageNow === 'my-files') home();
      }
      return;
    }
    if (seq !== myFilesSeq) return;
    myParcels = Array.isArray(body.parcels) ? body.parcels : [];
    renderMyFiles();
  }

  function expiryText(p) {
    if (p.expires_at === null || p.expires_at === undefined) return tr('file-row-never');
    const hLeft = (p.expires_at - Date.now()) / 3600000;
    return hLeft < 0 ? tr('file-row-expired') : tr('file-row-expires-in', { h: Math.max(1, Math.ceil(hLeft)) });
  }

  function renderMyFiles() {
    const list = $('my-files-list');
    const meta = $('my-files-meta');
    const parcels = myParcels || [];
    OD.clear(list);
    if (parcels.length === 0) {
      fillLines(meta, [tr('my-files-empty'), accountLine()]);
      return;
    }
    const used = parcels.reduce((s, p) => s + p.total_bytes, 0);
    fillLines(meta, [tr('my-files-meta', { n: parcels.length, used: fmtBytes(used) }), accountLine()]);

    parcels.forEach(p => {
      const unlimited = p.downloads_left === null || p.downloads_left === undefined;
      const metaText = tr(unlimited ? 'file-row-meta-unlimited' : 'file-row-meta',
        { size: fmtBytes(p.total_bytes), n: p.downloads_left, expiry: expiryText(p) });
      list.append(h('div', { class: 'parcel-row' },
        h('div', { class: 'parcel-row-main' },
          h('div', { class: 'parcel-code-cell' },
            h('span', { class: 'parcel-code-label sel', text: p.code.split('').join(' ') }),
            p.has_password ? h('span', { class: 'parcel-lock', title: tr('has-password') }, OD.icon('lock')) : null),
          h('div', { class: 'parcel-meta', text: metaText })),
        h('button', {
          class: 'parcel-edit btn-line icon-only ripple-surface', type: 'button', 'aria-label': tr('edit') + ' ' + p.code,
          onclick: () => openEditModal(p)
        }, OD.icon('pencil')),
        h('button', {
          class: 'parcel-delete btn-line danger ripple-surface', type: 'button',
          onclick: () => confirmDialog({
            title: tr('confirm-delete-title'),
            message: tr('confirm-delete-message', { code: p.code }),
            onConfirm: () => deleteParcel(p),
            after: () => { loadMyFiles(true); refreshStorageGauge(); }
          })
        }, OD.icon('trash'), h('span', { text: tr('btn-delete-parcel') }))));
    });
  }

  /* The delete itself, while 确定 waits; the list reloads once the
     dialog has closed (`after` above). */
  async function deleteParcel(p) {
    /* A 30 s deadline so a wedged request cannot hang the list. */
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), 30000);
    try {
      await api('/api/me/parcels/' + p.code, { method: 'DELETE', signal: ctrl.signal });
      toast(tr('toast-deleted'));
    } catch (err) {
      /* Already gone (expired and swept) still counts as deleted. */
      if (err.code === 'not_found') toast(tr('toast-deleted'));
      else toast(failText(err, 'err-save-fail'));
    } finally {
      clearTimeout(timer);
    }
  }

  /* ─── Edit-parcel modal ─── */
  let editingParcel = null;
  let editStart = null;

  function openEditModal(p) {
    editingParcel = p;
    $('edit-parcel-meta').textContent = tr('edit-parcel-meta-line', { code: p.code });
    const maxH = info ? info.max_expiry_hours : 168;
    const exInput = $('edit-expiry-hours');
    exInput.max = maxH;
    exInput.value = p.expires_at === null || p.expires_at === undefined
      ? ''
      : Math.min(Math.max(1, Math.ceil((p.expires_at - Date.now()) / 3600000)), maxH);
    $('edit-downloads').value = p.downloads_left === null || p.downloads_left === undefined ? '' : p.downloads_left;
    editStart = { expiry: String(exInput.value), downloads: String($('edit-downloads').value) };
    $('edit-parcel-err').hidden = true;
    OD.openModal($('edit-parcel-modal'));
  }

  $('btn-edit-parcel-save').addEventListener('click', async () => {
    const p = editingParcel;
    if (!p) return;
    const err = $('edit-parcel-err');
    const fail = msg => { err.textContent = msg; err.hidden = false; };
    err.hidden = true;
    const maxH = info ? info.max_expiry_hours : 168;
    const exRaw = $('edit-expiry-hours').value;
    const dlRaw = $('edit-downloads').value;
    /* Only what was changed is sent: a new expiry counts from now. */
    const body = {};
    if (String(exRaw) !== editStart.expiry) {
      const n = intIn(exRaw, 1, maxH);
      if (n === null) { fail(tr('err-bad-expiry', { max: maxH })); return; }
      body.expiry_hours = n;
    }
    if (String(dlRaw) !== editStart.downloads) {
      const n = intIn(dlRaw, 1, 999);
      if (n === null) { fail(tr('err-bad-downloads')); return; }
      body.downloads = n;
    }
    if (Object.keys(body).length) {
      try {
        await OD.busy($('btn-edit-parcel-save'), api('/api/me/parcels/' + p.code, { method: 'PATCH', body }));
      } catch (e) {
        const field = e.data && e.data.field;
        fail(e.code === 'bad_field' && field === 'expiry_hours' ? tr('err-bad-expiry', { max: maxH })
          : e.code === 'bad_field' && field === 'downloads' ? tr('err-bad-downloads')
            : e.code === 'not_owner' ? tr('err-not-owner')
              : e.code === 'not_found' ? tr('err-not-found')
                : failText(e, 'err-save-fail'));
        return;
      }
    }
    /* While 保存 waited the dialog may have been opened for another
       parcel: that one stays open. */
    if (editingParcel === p) {
      OD.closeModal($('edit-parcel-modal'));
      editingParcel = null;
    }
    toast(tr('toast-saved'));
    loadMyFiles(true);
  });
  ['edit-expiry-hours', 'edit-downloads'].forEach(id => {
    $(id).addEventListener('keydown', e => { if (e.key === 'Enter') $('btn-edit-parcel-save').click(); });
  });

  /* ─── Confirm modal ─── */
  /* 确定 waits with the action (when it returns a promise), then the
     dialog closes and `after` runs. */
  let confirmAction = null;
  let confirmAfter = null;
  let confirmSeq = 0;
  function confirmDialog(opts) {
    $('confirm-title').textContent = opts.title || tr('btn-confirm');
    $('confirm-message').textContent = opts.message || '';
    $('confirm-ok-label').textContent = opts.okLabel || tr('btn-confirm');
    confirmAction = opts.onConfirm || null;
    confirmAfter = opts.after || null;
    confirmSeq++;
    OD.openModal($('confirm-modal'), $('btn-confirm-ok'));
  }
  $('btn-confirm-ok').addEventListener('click', async () => {
    const fn = confirmAction, after = confirmAfter, seq = confirmSeq;
    confirmAction = null;
    confirmAfter = null;
    const work = fn ? fn() : null;
    if (work && typeof work.then === 'function') await OD.busy($('btn-confirm-ok'), work).catch(() => {});
    /* Another dialog opened meanwhile stays open. */
    if (seq === confirmSeq) OD.closeModal($('confirm-modal'));
    if (after) after();
  });

  /* ─── Language ─── */
  function onLang() {
    refreshAuthUi();
    renderStorage();
    setLoginTexts();
    renderSendList();
    renderReceiveMeta();
    renderReceiveList();
    if (send.job) uploadFilename.textContent = headline(send.job.text ? send.files : send.job.files);
    if (myParcels && stageNow === 'my-files' && !OD.isLoading($('my-files-list'))) renderMyFiles();
  }

  /* ─── Boot ─── */
  OD.onLang(onLang);
  OD.start();
  $('year').textContent = String(new Date().getFullYear());
  /* Until /api/auth/me answers, the bar shows what this browser last was. */
  let lastSignedIn = false;
  try { lastSignedIn = /^[12]$/.test(localStorage.getItem('od-auth') || ''); } catch (_) { /* signed out */ }
  setAuthChrome(lastSignedIn, OD.authHint.direct());
  refreshStorageGauge();
  fetchMe();

  /* Deep link: /d/123456 (and the old /?c=123456) fills the code and
     looks it up (info only: opening a link never spends a download);
     the address goes back to / so a reload does not repeat it. */
  const m = location.pathname.match(/^\/d\/(\d{6})\/?$/);
  const c = new URLSearchParams(location.search).get('c');
  const deep = m ? m[1] : (c && /^\d{6}$/.test(c) ? c : null);
  if (m || location.search) history.replaceState(null, '', '/');
  if (deep) {
    setCode(deep);
    showStage('receive', 'code');
    setTimeout(attemptFetch, 60);
  }
})();
