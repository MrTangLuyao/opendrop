/* Shared by index.html and admin.html: the site name, strings and
   language (with the homepage's language picker), the API helper, a
   small DOM builder, icons, ripple, loading states, toast, modals, the
   bar's menus, formatting, the bar's scroll state and the page
   scrollbar. Everything sits on
   window.OD; a page script adds its strings to OD.T and then calls
   OD.start(). Data only ever reaches the page through textContent and
   attributes, never as markup. */
(function () {
  'use strict';
  const root = document.documentElement;
  const OD = window.OD = {};

  /* ─── The site name ───
     Set by the admin (1 to 32 characters). The text before the first dot
     is the head, the rest the tail, set as open . drop are; with no dot
     the whole name is the head. The last name seen is kept, so a visit
     draws it before the server answers (od.css holds the name back until
     then). Only ever textContent. */
  const NAME_KEY = 'od-name';
  const nameOk = n => typeof n === 'string' && n.trim() !== '' && Array.from(n.trim()).length <= 32;
  let siteName = 'open.drop';
  try {
    const saved = localStorage.getItem(NAME_KEY);
    if (nameOk(saved)) siteName = saved.trim();
  } catch (_) { /* no storage: the default until the server answers */ }

  function drawName() {
    const dot = siteName.indexOf('.');
    const head = dot < 0 ? siteName : siteName.slice(0, dot);
    const tail = dot < 0 ? '' : siteName.slice(dot + 1);
    /* Each wordmark holds three spans: head, dot, tail. */
    document.querySelectorAll('[data-site-name]').forEach(el => {
      const [a, b, c] = el.children;
      if (!c) return;
      a.textContent = head; a.hidden = !head;
      b.textContent = dot < 0 ? '' : '.'; b.hidden = dot < 0;
      c.textContent = tail; c.hidden = !tail;
    });
    document.querySelectorAll('[data-site-name-text]').forEach(el => { el.textContent = siteName; });
    const brand = document.getElementById('brand');
    if (brand) brand.setAttribute('aria-label', siteName);
    document.title = siteName + (document.body.classList.contains('admin-page') ? ' admin' : '');
    root.classList.add('has-name');
  }
  drawName();
  OD.siteName = () => siteName;
  OD.setSiteName = name => {
    if (!nameOk(name)) return;
    siteName = name.trim();
    try { localStorage.setItem(NAME_KEY, siteName); } catch (_) { /* not kept */ }
    drawName();
  };

  /* ─── Strings and language ─── */
  const T = OD.T = {
    zh: {
      'github': 'GitHub 仓库', 'menu': '菜单',
      'err-rate-limited': '操作太频繁，请 {s} 秒后再试',
      'err-busy': '服务器繁忙，请 {s} 秒后再试',
      'err-offline': '网络错误，请重试',
      'err-request': '操作失败（{code}）',
      'loading': '加载中', 'processing': '处理中'
    },
    en: {
      'github': 'GitHub repository', 'menu': 'Menu',
      'err-rate-limited': 'Too many tries, wait {s} s',
      'err-busy': 'The server is busy, try again in {s} s',
      'err-offline': 'Network error, please retry',
      'err-request': 'Request failed ({code})',
      'loading': 'Loading', 'processing': 'Processing'
    }
  };
  /* The language picker, as on the homepage: each language named in
     itself, the tooltip and the spoken name of the button. */
  const UI = {
    zh: { name: '简体中文', language: '语言', say: '语言：' },
    en: { name: 'English', language: 'Language', say: 'Language: ' }
  };

  function initialLang() {
    try {
      const saved = localStorage.getItem('od-lang');
      if (saved === 'zh' || saved === 'en') return saved;
    } catch (_) { /* no storage: fall back to the browser */ }
    return (navigator.language || 'en').toLowerCase().indexOf('zh') === 0 ? 'zh' : 'en';
  }
  let lang = initialLang();
  OD.lang = () => lang;

  /* tr('key', {n: 3}): every {name} is replaced through a function, so a
     '$' in a value stays a '$'. Missing keys fall back to English, then
     to the key itself. */
  function tr(key, vars) {
    let s = T[lang][key];
    if (s === undefined) s = T.en[key];
    if (s === undefined) s = key;
    if (vars) {
      s = s.replace(/\{(\w+)\}/g, (m, k) => (Object.prototype.hasOwnProperty.call(vars, k) ? String(vars[k]) : m));
    }
    return s;
  }
  OD.tr = tr;

  const langHooks = [];
  OD.onLang = fn => { langHooks.push(fn); };

  function applyLang() {
    root.lang = lang === 'zh' ? 'zh-CN' : 'en';
    document.querySelectorAll('[data-i18n]').forEach(el => { el.textContent = tr(el.dataset.i18n); });
    document.querySelectorAll('[data-i18n-ph]').forEach(el => { el.placeholder = tr(el.dataset.i18nPh); });
    document.querySelectorAll('[data-i18n-label]').forEach(el => { el.setAttribute('aria-label', tr(el.dataset.i18nLabel)); });
    /* The picker shows the language in use, named in itself, and ticks it
       in the list; the tooltip and the spoken name say what it is. */
    const ui = UI[lang];
    const label = document.getElementById('lang-label');
    const langBtn = document.getElementById('lang-btn');
    const langMenu = document.getElementById('lang-menu');
    if (label) label.textContent = ui.name;
    if (langBtn) {
      langBtn.title = ui.language;
      langBtn.setAttribute('aria-label', ui.say + ui.name);
    }
    if (langMenu) {
      langMenu.querySelectorAll('[data-lang]').forEach(item =>
        item.setAttribute('aria-checked', String(item.dataset.lang === lang)));
    }
    langHooks.forEach(fn => fn());
  }
  OD.applyLang = applyLang;
  OD.setLang = next => {
    if ((next !== 'zh' && next !== 'en') || next === lang) return;
    lang = next;
    try { localStorage.setItem('od-lang', lang); } catch (_) { /* not kept */ }
    applyLang();
  };

  /* ─── API ───
     JSON in and out, same origin. A failed call throws ApiError with the
     server's snake_case code (never its text), 'network' when the
     request did not get through. Writes always carry Content-Type:
     application/json, which the server requires. */
  class ApiError extends Error {
    constructor(code, status, data) {
      super(code);
      this.code = code;
      this.status = status || 0;
      this.data = data || {};
    }
  }
  OD.ApiError = ApiError;

  const codeOk = c => typeof c === 'string' && /^[a-z0-9_]{1,48}$/.test(c);
  OD.codeOk = codeOk;
  OD.statusCode = status => {
    if (status === 401) return 'unauthorized';
    if (status === 403) return 'forbidden';
    if (status === 404) return 'not_found';
    if (status === 413) return 'too_large';
    if (status === 429) return 'rate_limited';
    return 'http_' + status;
  };

  OD.api = async function (path, opts) {
    opts = opts || {};
    const method = opts.method || (opts.body !== undefined ? 'POST' : 'GET');
    const headers = Object.assign({ Accept: 'application/json' }, opts.headers);
    const init = { method, headers, credentials: 'same-origin', cache: 'no-store' };
    if (method !== 'GET' && method !== 'HEAD') {
      headers['Content-Type'] = 'application/json';
      if (opts.body !== undefined) init.body = JSON.stringify(opts.body);
      else if (method !== 'DELETE') init.body = '{}';
    }
    if (opts.signal) init.signal = opts.signal;
    let res;
    try {
      res = await fetch(path, init);
    } catch (e) {
      throw new ApiError(e && e.name === 'AbortError' ? 'aborted' : 'network', 0);
    }
    let data = null;
    if (res.status !== 204 && (res.headers.get('Content-Type') || '').indexOf('json') >= 0) {
      try { data = await res.json(); } catch (_) { data = null; }
    }
    if (!res.ok) {
      throw new ApiError(data && codeOk(data.error) ? data.error : OD.statusCode(res.status), res.status, data);
    }
    return data;
  };

  /* ─── DOM ─── */
  OD.$ = id => document.getElementById(id);

  /* h('button', {class: 'x', text: 'Hi', onclick: fn}, child, ...) */
  function h(tag, attrs) {
    const el = document.createElement(tag);
    if (attrs) {
      Object.keys(attrs).forEach(k => {
        const v = attrs[k];
        if (v === null || v === undefined || v === false) return;
        if (k === 'class') el.className = v;
        else if (k === 'text') el.textContent = v;
        else if (k.indexOf('on') === 0 && typeof v === 'function') el.addEventListener(k.slice(2), v);
        else if (k === 'value' || k === 'disabled' || k === 'hidden') el[k] = v;
        else el.setAttribute(k, v === true ? '' : String(v));
      });
    }
    for (let i = 2; i < arguments.length; i++) add(el, arguments[i]);
    return el;
  }
  function add(el, c) {
    if (c === null || c === undefined || c === false) return;
    if (Array.isArray(c)) { c.forEach(x => add(el, x)); return; }
    el.append(c instanceof Node ? c : String(c));
  }
  OD.h = h;
  OD.clear = el => { while (el.firstChild) el.removeChild(el.firstChild); return el; };

  /* ─── Icons: Lucide (ISC) at stroke 1.5, static strings only ─── */
  const ICONS = {
    x: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
    download: '<path d="M12 15V3"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/>',
    lock: '<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
    pencil: '<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>',
    trash: '<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/><path d="M10 11v6"/><path d="M14 11v6"/>'
  };
  const NS = 'http://www.w3.org/2000/svg';
  OD.icon = (name, cls) => {
    const s = document.createElementNS(NS, 'svg');
    s.setAttribute('class', cls || 'icon');
    s.setAttribute('viewBox', '0 0 24 24');
    s.setAttribute('fill', 'none');
    s.setAttribute('stroke', 'currentColor');
    s.setAttribute('stroke-width', '1.5');
    s.setAttribute('stroke-linecap', 'round');
    s.setAttribute('stroke-linejoin', 'round');
    s.setAttribute('aria-hidden', 'true');
    s.setAttribute('focusable', 'false');
    s.innerHTML = ICONS[name] || '';
    return s;
  };

  /* ─── Ripple on every .ripple-surface: one delegated listener, so rows
     built later ripple too. Touch and mouse both arrive as pointerdown. ─── */
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)');
  /* In view: has a box (nothing up the tree is display:none) and is not
     hidden by visibility (a closed menu, the send hint). */
  const onScreen = el => el.isConnected && el.getClientRects().length > 0 &&
    getComputedStyle(el).visibility !== 'hidden';
  document.addEventListener('pointerdown', e => {
    if (e.button !== 0 || reduced.matches) return;
    const el = e.target.closest && e.target.closest('.ripple-surface');
    if (!el || el.disabled) return;
    const circle = document.createElement('span');
    const diameter = Math.max(el.clientWidth, el.clientHeight);
    const radius = diameter / 2;
    const rect = el.getBoundingClientRect();
    circle.style.width = circle.style.height = diameter + 'px';
    circle.style.left = (e.clientX - rect.left - radius) + 'px';
    circle.style.top = (e.clientY - rect.top - radius) + 'px';
    circle.className = 'ripple';
    el.querySelectorAll(':scope > .ripple').forEach(r => r.remove());
    el.appendChild(circle);
    let live = true;
    const done = () => { live = false; circle.remove(); };
    /* The look belongs to the stylesheet (.ripple and its overrides: the
       bar and the send hint have their own colour and strength). The
       script only plays it: from that look, growing to four times the
       size while fading to nothing over 600 ms, exactly what the CSS
       keyframes did. It is played by script because a CSS animation
       starts over from the beginning when a hidden surface shows again. */
    if (typeof circle.animate === 'function') {
      const o = parseFloat(getComputedStyle(circle).opacity);
      const anim = circle.animate(
        [{ transform: 'scale(0)', opacity: Number.isFinite(o) ? o : 0.12 }, { transform: 'scale(4)', opacity: 0 }],
        { duration: 600, easing: 'linear', fill: 'forwards' });
      anim.finished.then(done, done);
    } else {
      circle.classList.add('ripple-css');
      circle.addEventListener('animationend', done, { once: true });
      circle.addEventListener('animationcancel', done, { once: true });
    }
    setTimeout(done, 700);
    /* A ripple lives only while its surface stays in view. It plays on
       through a menu's fade out, as it always did, but once the surface
       is really gone (a dialog closed, a card or a tab changed, a menu
       hidden) the ripple goes with it, so nothing is left to play when
       the surface shows again. */
    const watch = () => {
      if (!live) return;
      if (!onScreen(el)) { done(); return; }
      requestAnimationFrame(watch);
    };
    requestAnimationFrame(watch);
  }, { passive: true });
  /* Nothing stale in a view about to show (the dialogs call this). */
  OD.dropRipples = root => root.querySelectorAll('.ripple').forEach(r => r.remove());

  /* ─── Text fields ───
     A field is a label around its input. A press on the label's own area
     (the icon, the edges beside the input) lands on something that cannot
     hold the focus: the input lost it on the press and only got it back
     on the release, so the field flashed. The press is handed straight
     to the input instead, as if the input filled the whole field. */
  document.addEventListener('mousedown', e => {
    if (e.button !== 0 || !(e.target instanceof Element)) return;
    const field = e.target.closest('.field');
    if (!field || e.target.closest('input, textarea, select, button, a')) return;
    const input = field.querySelector('input:not([type="hidden"]), textarea');
    if (!input || input.disabled) return;
    e.preventDefault();
    if (document.activeElement !== input) input.focus({ preventScroll: true });
  });

  /* ─── Loading ───
     Anything that waits on the server says so, the M3 way: an
     indeterminate circular indicator, in an area with 加载中 under it,
     or small inside the button that started the wait. A quick answer
     shows nothing: the indicator comes only once a wait passes 150 ms,
     and once up it stays at least 300 ms, so it never flickers. */
  const SHOW_AFTER = 150, MIN_SHOWN = 300;

  /* pace(work, show, hide): settles as the promise `work` does, but not
     while an indicator still owes its time: show() runs once the wait
     passes SHOW_AFTER, and then hide() no sooner than MIN_SHOWN later. */
  function pace(work, show, hide) {
    let shownAt = 0;
    const timer = setTimeout(() => { shownAt = Date.now(); show(); }, SHOW_AFTER);
    const settle = () => {
      clearTimeout(timer);
      if (!shownAt) return Promise.resolve();
      const left = MIN_SHOWN - (Date.now() - shownAt);
      return new Promise(r => setTimeout(r, Math.max(0, left))).then(hide);
    };
    return Promise.resolve(work).then(v => settle().then(() => v), e => settle().then(() => { throw e; }));
  }
  OD.pace = pace;

  /* The indicator: an arc that grows and shrinks while the whole turns
     (od.css). 28px with a 3px stroke, or 16px with 2px in a button. */
  OD.spinner = small => {
    const stroke = small ? 5.5 : 4.7;
    const s = document.createElementNS(NS, 'svg');
    s.setAttribute('class', small ? 'spinner spinner-sm' : 'spinner');
    s.setAttribute('viewBox', '0 0 44 44');
    s.setAttribute('aria-hidden', 'true');
    s.setAttribute('focusable', 'false');
    const c = document.createElementNS(NS, 'circle');
    c.setAttribute('cx', '22');
    c.setAttribute('cy', '22');
    c.setAttribute('r', String(22 - stroke / 2));
    c.setAttribute('fill', 'none');
    c.setAttribute('stroke', 'currentColor');
    c.setAttribute('stroke-width', String(stroke));
    c.setAttribute('stroke-linecap', 'round');
    s.appendChild(c);
    return s;
  };

  /* Content that took the place of a loader fades in. */
  OD.fadeIn = el => {
    if (!el || reduced.matches) return;
    el.classList.remove('is-fading-in');
    void el.offsetWidth;
    el.classList.add('is-fading-in');
  };
  const fadeDone = e => {
    const el = e.target;
    if (el.classList && el.classList.contains('is-fading-in') && e.animationName === 'fade-in') el.classList.remove('is-fading-in');
  };
  document.addEventListener('animationend', fadeDone);
  document.addEventListener('animationcancel', fadeDone);

  /* OD.loading(area, work, {clear}): `area` shows the loader (the
     indicator over 加载中, centred; a table body takes it as one full
     width row) while `work` runs. With clear, the old content goes at
     once, as a panel just opened must not show what it held before, and
     the loader holds its place from the start, unseen until it is due.
     Without, the old content stays through a quick answer and gives way
     to the loader only once it is due. Settles as `work` does, with the
     loader gone, so the caller draws the answer in its place. */
  OD.loading = (area, work, opts) => {
    const clear = !!(opts && opts.clear);
    const token = {};
    area.odLoad = token;
    let box = null;
    const put = () => {
      OD.clear(area);
      box = h('div', { class: 'loader', role: 'status' }, OD.spinner(false),
        h('span', { class: 'loader-text', 'data-i18n': 'loading', text: tr('loading') }));
      if (area.tagName === 'TBODY') {
        const head = area.parentNode && area.parentNode.tHead;
        const cols = head && head.rows[0] ? head.rows[0].cells.length : 1;
        area.append(h('tr', { class: 'loader-row' }, h('td', { colspan: cols }, box)));
      } else {
        area.append(box);
      }
      area.classList.add('is-loading');
    };
    if (clear) put();
    return pace(work, () => {
      if (area.odLoad !== token) return;
      if (!box) put();
      box.classList.add('is-shown');
    }, () => {}).finally(() => {
      if (area.odLoad !== token) return;
      area.odLoad = null;
      area.classList.remove('is-loading');
      const left = area.querySelectorAll(':scope > .loader, :scope > .loader-row');
      left.forEach(n => n.remove());
      if (left.length) OD.fadeIn(area);
    });
  };
  /* True while an area shows (or keeps the place of) its loader, so
     a redraw (a language switch) leaves it be. */
  OD.isLoading = area => !!area && area.classList.contains('is-loading');

  /* OD.busy(button, work): the button that started `work` waits with it.
     It is disabled at once but keeps its look, so a quick answer leaves
     no trace; once the wait is due a small indicator shows inside it, in
     place of its icon or before its label, and the button keeps its
     width. Settles as `work` does, with the button back as it was. */
  OD.busy = (btn, work) => {
    if (!btn) return Promise.resolve(work);
    const st = btn.odBusy || (btn.odBusy = { n: 0 });
    if (st.n++ === 0) {
      st.disabled = btn.disabled;
      st.focused = document.activeElement === btn;
      btn.classList.add('is-busy');
      btn.setAttribute('aria-busy', 'true');
      btn.disabled = true;
    }
    return pace(work, () => spinIn(btn), () => {}).finally(() => {
      if (--st.n === 0) unbusy(btn, st);
    });
  };
  function spinIn(btn) {
    if (btn.odSpin || !btn.isConnected) return;
    const before = btn.getBoundingClientRect().width;
    const icon = Array.from(btn.children).find(c => c.classList.contains('icon'));
    /* In place of an icon the indicator is a box of the same size; ahead
       of a label it comes in a mark that keeps the button's baseline on
       the text (od.css), so the button does not move in its line. */
    const spin = icon ? OD.spinner(true) : h('span', { class: 'busy-mark' }, OD.spinner(true));
    if (icon) icon.classList.add('busy-hide');
    btn.insertBefore(spin, icon || btn.firstChild);
    btn.odSpin = spin;
    /* With no icon to stand in for, the label makes room: the width
       stays, and when the two do not fit the padding gives way. */
    if (btn.getBoundingClientRect().width > before + 0.5) {
      btn.style.width = before + 'px';
      if (btn.scrollWidth > btn.clientWidth) btn.classList.add('is-tight');
    }
  }
  function unbusy(btn, st) {
    if (btn.odSpin) { btn.odSpin.remove(); btn.odSpin = null; }
    Array.from(btn.children).forEach(c => c.classList.remove('busy-hide'));
    btn.style.width = '';
    btn.classList.remove('is-busy', 'is-tight');
    btn.removeAttribute('aria-busy');
    btn.disabled = st.disabled;
    /* Disabling took the focus away: it comes back if nothing has it. */
    const at = document.activeElement;
    if (st.focused && btn.isConnected && (!at || at === document.body) && usable(btn) && !btn.closest('[inert]')) btn.focus();
  }

  /* ─── Toast: one pill at the bottom, 1.8 s; a new one replaces it ─── */
  let toastEl = null, toastTimer = 0;
  OD.toast = msg => {
    if (!toastEl) {
      toastEl = h('div', { id: 'toast', role: 'status', 'aria-live': 'polite' });
      document.body.appendChild(toastEl);
    }
    toastEl.textContent = msg;
    void toastEl.offsetWidth;
    toastEl.classList.add('is-visible');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toastEl.classList.remove('is-visible'), 1800);
  };

  /* ─── Modals: div.modal toggled by [hidden]. Esc presses the close
     button of the one on top, so each page keeps one close path. While
     one is open everything else on the page is inert and Tab goes round
     its card; opening moves the focus in (to `focusEl`, or the first
     field, or the first button), closing puts it back where it was (or
     on `returnTo`, when the opener will be gone by then). ─── */
  const FOCUSABLE = 'input:not([type="hidden"]), textarea, select, button, a[href], [tabindex]:not([tabindex="-1"])';
  const openModals = () => Array.from(document.querySelectorAll('.modal:not([hidden])'));
  const usable = el => !el.disabled && el.getClientRects().length > 0;
  const modalCard = m => m.querySelector('.modal-card') || m;
  const focusables = card => Array.from(card.querySelectorAll(FOCUSABLE)).filter(usable);
  function firstStop(card) {
    const all = focusables(card);
    return all.find(el => /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName))
      || all.find(el => !el.classList.contains('card-close'))
      || all[0] || null;
  }
  /* The page behind (and a modal under another) takes no clicks, no
     focus and is out of the accessibility tree; the toast stays heard.
     A closed modal still fading out (od.css) is out of reach too. */
  function syncInert() {
    const open = openModals();
    const top = open[open.length - 1] || null;
    Array.from(document.body.children).forEach(el => {
      if (el.id === 'toast' || el.classList.contains('scrollbar')) return;
      el.inert = (!!top && el !== top) || (el.hidden && el.classList.contains('modal'));
    });
  }
  OD.openModal = (el, focusEl, returnTo) => {
    if (el.hidden) { el.odOpener = returnTo || document.activeElement; OD.dropRipples(el); }
    el.hidden = false;
    document.body.classList.add('has-modal');
    syncInert();
    const target = focusEl || firstStop(modalCard(el));
    if (target) target.focus();
  };
  OD.closeModal = el => {
    if (el.hidden) return;
    el.hidden = true;
    if (!OD.modalOpen()) document.body.classList.remove('has-modal');
    syncInert();
    const back = el.odOpener;
    el.odOpener = null;
    if (back && back.isConnected && usable(back) && !back.closest('[inert]')) back.focus();
  };
  OD.modalOpen = () => openModals().length > 0;
  document.addEventListener('keydown', e => {
    if (e.key !== 'Escape' && e.key !== 'Tab') return;
    const open = openModals();
    const top = open[open.length - 1];
    if (!top) return;
    if (e.key === 'Escape') {
      const close = top.querySelector('.card-close');
      if (close) { e.preventDefault(); close.click(); }
      return;
    }
    const card = modalCard(top);
    const all = focusables(card);
    if (!all.length) { e.preventDefault(); return; }
    const first = all[0], last = all[all.length - 1];
    const at = document.activeElement;
    const inside = card.contains(at);
    if (e.shiftKey && (at === first || !inside)) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && (at === last || !inside)) { e.preventDefault(); first.focus(); }
  });

  /* ─── The bar's menus (language, account), as on the homepage: a button
     opens its menu and closes the others; a click outside or Esc closes
     them, and Esc puts the focus back on the button. ─── */
  const popovers = [];
  function setPopover(p, open) {
    p.menu.classList.toggle('is-open', open);
    p.btn.setAttribute('aria-expanded', String(open));
  }
  OD.popover = (btn, menu) => {
    const p = { btn, menu };
    popovers.push(p);
    /* The button stops its click from reaching the outside-click closer,
       so opening one menu closes the others by hand. */
    btn.addEventListener('click', e => {
      e.stopPropagation();
      popovers.forEach(o => { if (o !== p) setPopover(o, false); });
      setPopover(p, !menu.classList.contains('is-open'));
    });
    return open => setPopover(p, open);
  };
  document.addEventListener('click', e => {
    popovers.forEach(p => {
      if (!p.btn.contains(e.target) && !p.menu.contains(e.target)) setPopover(p, false);
    });
  });
  document.addEventListener('keydown', e => {
    if (e.key !== 'Escape') return;
    popovers.forEach(p => {
      if (p.menu.classList.contains('is-open')) { setPopover(p, false); p.btn.focus(); }
    });
  });

  /* ─── Signed-in hint: the session cookie is HttpOnly, so a flag in
     localStorage remembers that this browser signed in, and anonymous
     visits skip the /api/auth/me call (and its 401). Without storage the
     page always asks. '2' also remembers an account that never expires
     (admin or long-term), whose bar shows sign out instead of the menu,
     so the bar does not change once the server answers. ─── */
  OD.authHint = {
    get() {
      try { return localStorage.getItem('od-auth') !== '0'; } catch (_) { return true; }
    },
    direct() {
      try { return localStorage.getItem('od-auth') === '2'; } catch (_) { return false; }
    },
    set(on, direct) {
      try { localStorage.setItem('od-auth', on ? (direct ? '2' : '1') : '0'); } catch (_) { /* not kept */ }
    }
  };

  /* ─── Errors every call can meet: too many tries (429, whatever the
     code), a busy server (503), no connection, and any other refusal,
     named by its code. ─── */
  OD.retryAfter = (err, fallback) => {
    const s = err && err.data && Number(err.data.retry_after);
    return s > 0 ? Math.ceil(s) : (fallback || 10);
  };
  OD.errText = err => {
    const code = (err && err.code) || 'network';
    const status = (err && err.status) || 0;
    if (status === 429 || code === 'rate_limited') return tr('err-rate-limited', { s: OD.retryAfter(err) });
    if (status === 503 || code === 'busy') return tr('err-busy', { s: OD.retryAfter(err, 5) });
    if (code === 'network' || code === 'aborted' || status === 0) return tr('err-offline');
    return tr('err-request', { code: status >= 500 ? 'HTTP ' + status : code });
  };

  /* ─── Copy, with the old textarea way as a fallback ─── */
  OD.copy = async text => {
    try {
      if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(text);
        return true;
      }
    } catch (_) { /* falls through */ }
    const ta = h('textarea', { class: 'offscreen', readonly: '' });
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    let ok = false;
    try { ok = document.execCommand('copy'); } catch (_) { ok = false; }
    ta.remove();
    return ok;
  };

  /* ─── Numbers and time ─── */
  OD.fmtBytes = n => {
    n = Number(n);
    if (!isFinite(n) || n < 0) n = 0;
    if (n < 1024) return Math.round(n) + ' B';
    if (n < 1024 * 1024) return (n / 1024).toFixed(1) + ' KB';
    if (n < 1024 * 1024 * 1024) return (n / (1024 * 1024)).toFixed(2) + ' MB';
    if (n < 1024 * 1024 * 1024 * 1024) return (n / (1024 * 1024 * 1024)).toFixed(2) + ' GB';
    return (n / (1024 * 1024 * 1024 * 1024)).toFixed(2) + ' TB';
  };
  OD.fmtDate = ms => {
    const d = new Date(ms);
    const z = v => String(v).padStart(2, '0');
    return `${d.getFullYear()}-${z(d.getMonth() + 1)}-${z(d.getDate())} ${z(d.getHours())}:${z(d.getMinutes())}`;
  };
  OD.GIB = 1024 * 1024 * 1024;

  /* ─── Start: word the page, wire the bar ─── */
  OD.start = () => {
    /* The bar lifts off the page once it has something to sit over. */
    const header = document.getElementById('navbar');
    if (header) {
      const update = () => header.classList.toggle('is-scrolled', window.scrollY > 0);
      window.addEventListener('scroll', update, { passive: true });
      update();
    }
    const langBtn = document.getElementById('lang-btn');
    const langMenu = document.getElementById('lang-menu');
    if (langBtn && langMenu) {
      const setLangMenu = OD.popover(langBtn, langMenu);
      langMenu.addEventListener('click', e => {
        const item = e.target.closest('[data-lang]');
        if (!item) return;
        setLangMenu(false);
        OD.setLang(item.dataset.lang);
        langBtn.focus();
      });
    }
    applyLang();
    /* Another tab switched the language: follow along. */
    window.addEventListener('storage', e => {
      if (e.key === 'od-lang' && (e.newValue === 'zh' || e.newValue === 'en') && e.newValue !== lang) {
        lang = e.newValue;
        applyLang();
      }
    });
  };

  /* ─── The page's own scrollbar (homepage lib/design/scrollbar.js): a
     slim pill over the right edge on mouse and trackpad screens; it shows
     while the page scrolls or the pointer comes near, fades when idle,
     can be dragged, and a click on the track pages. ─── */
  (function () {
    const fine = window.matchMedia('(hover: hover) and (pointer: fine) and (forced-colors: none)');
    const IDLE = 900, PAD = 4, MIN = 32;
    let track, thumb, hideTimer = 0, frame = 0;
    let hovering = false, dragging = false;
    let thumbTop = 0, thumbH = 0, travel = 0, range = 0;

    function build() {
      track = document.createElement('div');
      track.className = 'scrollbar';
      track.setAttribute('aria-hidden', 'true');
      thumb = document.createElement('div');
      thumb.className = 'scrollbar-thumb';
      track.appendChild(thumb);
      document.body.appendChild(track);
      track.addEventListener('pointerenter', () => { hovering = true; show(); });
      track.addEventListener('pointerleave', () => { hovering = false; later(); });
      track.addEventListener('pointerdown', onDown);
    }
    function measure() {
      const view = window.innerHeight;
      const full = Math.max(root.scrollHeight, document.body.scrollHeight);
      range = Math.max(0, full - view);
      const length = track.clientHeight - PAD * 2;
      track.classList.toggle('is-idle', range < 1 || length <= MIN);
      if (range < 1 || length <= MIN) return;
      thumbH = Math.max(MIN, Math.round(length * view / full));
      travel = length - thumbH;
      const y = Math.min(Math.max(window.scrollY, 0), range);
      thumbTop = PAD + travel * y / range;
      thumb.style.height = thumbH + 'px';
      thumb.style.transform = 'translateY(' + thumbTop + 'px)';
    }
    function update() {
      if (frame) return;
      frame = requestAnimationFrame(() => { frame = 0; measure(); });
    }
    function show() { clearTimeout(hideTimer); track.classList.add('is-shown'); }
    function later() {
      clearTimeout(hideTimer);
      hideTimer = setTimeout(() => {
        if (!hovering && !dragging) track.classList.remove('is-shown');
      }, IDLE);
    }
    function onDown(e) {
      if (e.button !== 0) return;
      e.preventDefault();
      measure();
      if (e.target === thumb) {
        dragging = true;
        track.classList.add('is-dragging');
        root.style.scrollBehavior = 'auto';
        const startY = e.clientY, startScroll = window.scrollY;
        const ratio = travel > 0 ? range / travel : 0;
        const move = ev => { window.scrollTo(0, startScroll + (ev.clientY - startY) * ratio); };
        const up = () => {
          dragging = false;
          track.classList.remove('is-dragging');
          root.style.scrollBehavior = '';
          thumb.removeEventListener('pointermove', move);
          thumb.removeEventListener('pointerup', up);
          thumb.removeEventListener('pointercancel', up);
          later();
        };
        thumb.setPointerCapture(e.pointerId);
        thumb.addEventListener('pointermove', move);
        thumb.addEventListener('pointerup', up);
        thumb.addEventListener('pointercancel', up);
      } else {
        const y = e.clientY - track.getBoundingClientRect().top;
        const page = window.innerHeight * 0.875;
        window.scrollBy(0, y < thumbTop ? -page : page);
      }
    }
    function enable() { if (!track) build(); root.classList.add('has-scrollbar'); measure(); }
    function disable() { root.classList.remove('has-scrollbar'); }
    function start() {
      if (fine.matches) enable();
      const onChange = () => { if (fine.matches) enable(); else disable(); };
      if (fine.addEventListener) fine.addEventListener('change', onChange);
      else if (fine.addListener) fine.addListener(onChange);
      window.addEventListener('scroll', () => {
        if (track && fine.matches) { update(); show(); later(); }
      }, { passive: true });
      window.addEventListener('resize', () => { if (track) update(); });
      if (window.ResizeObserver) new ResizeObserver(() => { if (track) update(); }).observe(document.body);
      document.addEventListener('pointermove', e => {
        if (!track || e.pointerType === 'touch' || !fine.matches) return;
        if (e.clientX >= root.clientWidth - 24) { show(); later(); }
      }, { passive: true });
    }
    if (document.body) start();
    else document.addEventListener('DOMContentLoaded', start);
  })();
})();
