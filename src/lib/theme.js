/**
 * Theme — Manual dark/light/system theme toggle.
 *
 * Cycles through System → Light → Dark. Persists preference in
 * localStorage and dispatches 'theme:change' events so other
 * components (e.g. email viewer iframe) can react.
 *
 * IIFE pattern matches splitter.js / virtual-scroll.js.
 */
const Theme = (() => {
  // ── Constants ──────────────────────────────────────────────────
  const STORAGE_KEY = 'theme-preference';
  const CYCLE = ['system', 'light', 'dark'];

  const ICONS = {
    system: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>',
    light:  '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="5"/><line x1="12" y1="1" x2="12" y2="3"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/><line x1="1" y1="12" x2="3" y2="12"/><line x1="21" y1="12" x2="23" y2="12"/><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/></svg>',
    dark:   '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>',
  };

  const LABELS = { system: 'System', light: 'Light', dark: 'Dark' };

  // ── State ──────────────────────────────────────────────────────
  let _preference = 'system'; // 'system' | 'light' | 'dark'
  let _mediaQuery = null;

  // ── Public API ─────────────────────────────────────────────────

  /** Initialize theme: load from storage, apply, bind listeners */
  function init() {
    _preference = localStorage.getItem(STORAGE_KEY) || 'system';
    // Validate stored value
    if (!CYCLE.includes(_preference)) _preference = 'system';

    _mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    _apply();
    _updateButton();
    _bindMediaQuery();
    _bindToggle();
  }

  /** Cycle to next theme preference: system → light → dark → system */
  function cycle() {
    const idx = CYCLE.indexOf(_preference);
    _preference = CYCLE[(idx + 1) % CYCLE.length];
    localStorage.setItem(STORAGE_KEY, _preference);
    _apply();
    _updateButton();
  }

  /** Returns the effective theme: 'light' | 'dark' */
  function getResolved() {
    return _resolve();
  }

  // ── Private ────────────────────────────────────────────────────

  /** Resolve preference to effective theme using OS setting for 'system' */
  function _resolve() {
    if (_preference === 'dark') return 'dark';
    if (_preference === 'light') return 'light';
    // 'system' — follow OS
    return _mediaQuery?.matches ? 'dark' : 'light';
  }

  /** Apply resolved theme to DOM and dispatch event */
  function _apply() {
    const resolved = _resolve();
    document.documentElement.setAttribute('data-theme', resolved);
    // Notify other components (e.g. email viewer iframe)
    document.dispatchEvent(new CustomEvent('theme:change', {
      detail: { preference: _preference, resolved },
    }));
  }

  /** Re-apply when OS theme changes (only matters in 'system' mode) */
  function _bindMediaQuery() {
    _mediaQuery?.addEventListener('change', () => {
      if (_preference === 'system') {
        _apply();
        _updateButton();
      }
    });
  }

  /** Bind click handler on theme toggle button */
  function _bindToggle() {
    const btn = document.getElementById('theme-toggle-btn');
    btn?.addEventListener('click', cycle);
  }

  /** Update toggle button icon, label, and title */
  function _updateButton() {
    const iconEl = document.getElementById('theme-icon');
    const labelEl = document.getElementById('theme-label');
    const btn = document.getElementById('theme-toggle-btn');
    if (iconEl) iconEl.innerHTML = ICONS[_preference];
    if (labelEl) labelEl.textContent = LABELS[_preference];
    if (btn) btn.dataset.tooltip = LABELS[_preference];
  }

  return { init, cycle, getResolved };
})();

// Run immediately — DOM already exists (script is at end of <body>)
Theme.init();
