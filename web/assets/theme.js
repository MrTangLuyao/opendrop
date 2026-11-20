/* Apply the appearance before styles render. With no saved choice the
   page follows the system (light or dark, live). The navbar button cycles
   through three states:
     follow the system  → the opposite of the system → the same as the
     system, but fixed  → back to following the system
   so the first click always changes what you see. Only a fixed choice is
   stored; following the system stores nothing.
   data-theme is always the theme in effect (light / dark), which the
   stylesheet and the birthday fireworks read; data-theme-mode is the
   button's state (auto / light / dark). */
(function () {
  'use strict';
  const root = document.documentElement;
  const key = 'od-theme';
  const canvas = { light: '#FFFFFF', dark: '#0A111C' };
  const fixed = value => value === 'light' || value === 'dark';
  const query = window.matchMedia ? matchMedia('(prefers-color-scheme: dark)') : null;
  const system = () => (query && query.matches ? 'dark' : 'light');
  const other = theme => (theme === 'dark' ? 'light' : 'dark');

  let mode = 'auto';
  try {
    const saved = localStorage.getItem(key);
    if (fixed(saved)) mode = saved;
  } catch (_) { /* Appearance still works when storage is unavailable. */ }

  const theme = () => (mode === 'auto' ? system() : mode);

  function nextMode() {
    if (mode === 'auto') return other(system());
    if (mode !== system()) return system();
    return 'auto';
  }

  function updateControls() {
    const button = document.getElementById('theme-btn');
    if (!button) return;
    const lang = root.lang.toLowerCase();
    const next = nextMode();
    const now = theme();
    const label = lang.startsWith('zh')
      ? `外观：${mode === 'auto' ? `跟随系统（现在是${now === 'dark' ? '深色' : '浅色'}）` : (mode === 'dark' ? '深色' : '浅色')}；`
        + (next === 'auto' ? '点击改为跟随系统' : `点击切换为${next === 'dark' ? '深色' : '浅色'}`)
      : `Appearance: ${mode === 'auto' ? `follows the system (now ${now})` : mode}; `
        + (next === 'auto' ? 'click to follow the system' : `click to switch to ${next}`);
    button.title = label;
    button.setAttribute('aria-label', label);
  }

  function apply() {
    const t = theme();
    root.dataset.theme = t;
    root.dataset.themeMode = mode;
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.content = canvas[t];
    updateControls();
  }

  function setMode(next) {
    mode = fixed(next) ? next : 'auto';
    try {
      if (mode === 'auto') localStorage.removeItem(key);
      else localStorage.setItem(key, mode);
    } catch (_) {}
    apply();
  }

  apply();

  /* The system switched (sunset, or the user changed it): follow it
     while the page is following. */
  if (query) {
    const onSystem = () => { if (mode === 'auto') apply(); else updateControls(); };
    if (query.addEventListener) query.addEventListener('change', onSystem);
    else if (query.addListener) query.addListener(onSystem);   // Safari before 14
  }

  /* Another tab changed it: follow along. */
  window.addEventListener('storage', event => {
    if (event.key !== key && event.key !== null) return;
    mode = fixed(event.newValue) ? event.newValue : 'auto';
    apply();
  });

  document.addEventListener('DOMContentLoaded', () => {
    const button = document.getElementById('theme-btn');
    if (!button) return;
    button.addEventListener('click', () => setMode(nextMode()));
    /* The page switches language by changing <html lang>; re-word with it. */
    new MutationObserver(updateControls).observe(root, { attributes: true, attributeFilter: ['lang'] });
    updateControls();
  });
})();
