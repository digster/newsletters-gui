/**
 * Sidebar component — label navigation, bookmarks, settings.
 * Communicates with App via custom events.
 */
const Sidebar = (() => {
  let _labels = [];
  let _activeLabel = null;
  let _activeView = 'all'; // 'all', 'bookmarks', or label name

  const ICON = {
    bookmark: '<svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor" stroke="currentColor" stroke-width="1.5"><path d="M3.5 2.5h9v12L8 11l-4.5 3.5v-12z"/></svg>',
  };

  /** Initialize the sidebar and load labels */
  async function init() {
    _bindEvents();
    await loadLabels();
  }

  /** Fetch labels from backend and render the list */
  async function loadLabels() {
    try {
      _labels = await Bridge.getLabels();
      _renderLabels(_labels);
      _updateTotalCount();
    } catch (err) {
      console.error('Failed to load labels:', err);
    }
  }

  /** Render label items in the sidebar nav */
  function _renderLabels(labels) {
    const container = document.getElementById('label-list');
    // Keep the section title
    container.innerHTML = '<div class="sidebar__section-title">Labels</div>';

    labels.forEach(label => {
      const item = document.createElement('div');
      item.className = 'sidebar__item';
      if (_activeView === label.name) {
        item.classList.add('sidebar__item--active');
      }
      item.setAttribute('data-action', 'select-label');
      item.setAttribute('data-label', label.name);
      item.innerHTML = `
        <span class="sidebar__item-name" title="${_escapeAttr(label.name)}">${_escapeHtml(label.name)}</span>
        <span class="sidebar__item-count">${label.count}</span>
      `;
      container.appendChild(item);
    });
  }

  /** Update total email count in "All Emails" nav item */
  function _updateTotalCount() {
    const total = _labels.reduce((sum, l) => sum + l.count, 0);
    const el = document.getElementById('total-count');
    if (el) el.textContent = total.toLocaleString();
  }

  /** Update bookmarks count */
  async function updateBookmarksCount() {
    try {
      const bookmarks = await Bridge.getBookmarkedEmails(0, 1);
      // We only need the count, do a quick query
      const all = await Bridge.getBookmarkedEmails(0, 99999);
      const el = document.getElementById('bookmarks-count');
      if (el) el.textContent = all.length > 0 ? all.length : '';
    } catch (err) {
      console.error('Failed to get bookmarks count:', err);
    }
  }

  /** Set the active view and update styling */
  function setActive(view) {
    _activeView = view;
    // Update active state on all nav items
    document.querySelectorAll('.sidebar__item').forEach(item => {
      item.classList.remove('sidebar__item--active');
    });

    if (view === 'all') {
      document.getElementById('nav-all')?.classList.add('sidebar__item--active');
    } else if (view === 'bookmarks') {
      document.getElementById('nav-bookmarks')?.classList.add('sidebar__item--active');
    } else {
      const labelItem = document.querySelector(`.sidebar__item[data-label="${CSS.escape(view)}"]`);
      if (labelItem) labelItem.classList.add('sidebar__item--active');
    }
  }

  /** Bind sidebar events */
  function _bindEvents() {
    const sidebar = document.getElementById('sidebar');
    const searchInput = document.getElementById('sidebar-search');

    // Click delegation for nav items
    sidebar.addEventListener('click', (e) => {
      const item = e.target.closest('[data-action]');
      if (!item) return;

      const action = item.getAttribute('data-action');

      if (action === 'show-all') {
        setActive('all');
        document.dispatchEvent(new CustomEvent('nav:change', { detail: { view: 'all' } }));
      } else if (action === 'show-bookmarks') {
        setActive('bookmarks');
        document.dispatchEvent(new CustomEvent('nav:change', { detail: { view: 'bookmarks' } }));
      } else if (action === 'select-label') {
        const label = item.getAttribute('data-label');
        setActive(label);
        document.dispatchEvent(new CustomEvent('nav:change', { detail: { view: 'label', label } }));
      }
    });

    // Label filter search
    if (searchInput) {
      searchInput.addEventListener('input', (e) => {
        const q = e.target.value.toLowerCase().trim();
        if (!q) {
          _renderLabels(_labels);
        } else {
          const filtered = _labels.filter(l => l.name.toLowerCase().includes(q));
          _renderLabels(filtered);
        }
      });
    }

    // Settings button
    document.getElementById('settings-btn')?.addEventListener('click', () => {
      document.dispatchEvent(new CustomEvent('app:settings'));
    });
  }

  function _escapeHtml(str) {
    if (!str) return '';
    const el = document.createElement('span');
    el.textContent = str;
    return el.innerHTML;
  }

  function _escapeAttr(str) {
    return str.replace(/"/g, '&quot;').replace(/'/g, '&#39;');
  }

  return { init, loadLabels, setActive, updateBookmarksCount };
})();
