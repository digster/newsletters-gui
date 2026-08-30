/**
 * Email list component — middle panel with virtual-scrolled email rows.
 * Handles pagination, sort, filter, and keyboard navigation.
 */
const EmailList = (() => {
  let _emails = [];
  let _currentLabel = null;
  let _currentView = 'all'; // 'all', 'bookmarks', or label name
  let _sort = 'date_desc';
  let _filter = null;
  let _activeEmailId = null;
  let _activeIndex = -1;
  let _virtualScroll = null;
  let _totalCount = 0;
  const PAGE_SIZE = 200;

  // Search mode state
  let _isSearchMode = false;
  let _searchResults = [];
  let _savedView = null;
  let _searchDebounceTimer = null;

  const ICON = {
    bookmarkSmall: '<svg width="10" height="10" viewBox="0 0 16 16" fill="currentColor" stroke="currentColor" stroke-width="1.5"><path d="M3.5 2.5h9v12L8 11l-4.5 3.5v-12z"/></svg>',
    dot: '<svg width="6" height="6" viewBox="0 0 6 6"><circle cx="3" cy="3" r="3" fill="var(--accent)"/></svg>',
  };

  function init() {
    _bindToolbar();
    _bindKeyboard();
  }

  /** Load emails for a specific view */
  async function loadView(view, label = null) {
    _currentView = view;
    _currentLabel = label;
    _activeEmailId = null;
    _activeIndex = -1;
    _emails = [];

    const titleEl = document.getElementById('list-title');
    const countEl = document.getElementById('list-count');

    if (view === 'bookmarks') {
      if (titleEl) titleEl.textContent = 'Bookmarks';
      try {
        _emails = await Bridge.getBookmarkedEmails(0, 10000);
        _totalCount = _emails.length;
        if (countEl) countEl.textContent = `${_totalCount}`;
        _renderList();
      } catch (err) {
        console.error('Failed to load bookmarks:', err);
      }
      return;
    }

    // Label or "All" view — use paginated query
    if (view === 'all') {
      if (titleEl) titleEl.textContent = 'All Emails';
      // Load all labels and get emails from each
      try {
        const labels = await Bridge.getLabels();
        // For "all" view, we query without a specific label
        // We need a dedicated approach — load from all labels
        _emails = [];
        for (const lbl of labels) {
          const batch = await Bridge.getEmailsByLabel(lbl.name, 0, 10000, _sort, _filter);
          _emails.push(...batch);
        }
        // Sort the combined results
        _sortEmails();
        _totalCount = _emails.length;
        if (countEl) countEl.textContent = `${_totalCount.toLocaleString()}`;
        _renderList();
      } catch (err) {
        console.error('Failed to load all emails:', err);
      }
      return;
    }

    // Specific label view
    if (titleEl) titleEl.textContent = label;
    try {
      _emails = await Bridge.getEmailsByLabel(label, 0, 10000, _sort, _filter);
      _totalCount = _emails.length;
      if (countEl) countEl.textContent = `${_totalCount}`;
      _renderList();
    } catch (err) {
      console.error('Failed to load emails:', err);
    }
  }

  /** Re-sort the current emails array in place */
  function _sortEmails() {
    if (_sort === 'date_desc') {
      _emails.sort((a, b) => (b.date || '').localeCompare(a.date || ''));
    } else if (_sort === 'date_asc') {
      _emails.sort((a, b) => (a.date || '').localeCompare(b.date || ''));
    } else if (_sort === 'subject') {
      _emails.sort((a, b) => (a.subject || '').localeCompare(b.subject || ''));
    }
  }

  /** Render the email list using virtual scroll */
  function _renderList() {
    const container = document.getElementById('email-list-scroll');

    // Destroy existing virtual scroll
    if (_virtualScroll) {
      _virtualScroll.destroy();
      _virtualScroll = null;
    }

    if (_emails.length === 0) {
      container.innerHTML = '<div class="empty-state">No emails found</div>';
      return;
    }

    container.innerHTML = '';

    // Search mode uses taller rows to display snippet + label badge
    const rowHeight = _isSearchMode ? 72 : 52;

    _virtualScroll = new VirtualScroll({
      container,
      itemHeight: rowHeight,
      totalItems: _emails.length,
      bufferSize: 15,
      renderItem: (index) => _createRow(index),
    });
  }

  /** Create a DOM element for an email row at the given index */
  function _createRow(index) {
    const email = _emails[index];
    if (!email) return null;

    const row = document.createElement('div');
    row.className = 'email-row';
    if (email.is_read) row.classList.add('email-row--read');
    if (email.id === _activeEmailId) row.classList.add('email-row--active');
    row.setAttribute('data-email-id', email.id);
    row.setAttribute('data-index', index);

    // Format date
    const dateStr = _formatDate(email.date);

    // Status icons
    let icons = '';
    if (email.is_bookmarked) {
      icons += `<span class="email-row__icon email-row__icon--active">${ICON.bookmarkSmall}</span>`;
    }
    if (!email.is_read) {
      icons += `<span class="email-row__icon email-row__icon--active">${ICON.dot}</span>`;
    }

    // In search mode, show snippet + label badge; otherwise normal row
    if (_isSearchMode && email._snippet) {
      row.innerHTML = `
        <div class="email-row__subject">${_escapeHtml(email.subject || '(no subject)')}</div>
        <div class="email-row__meta">
          <span class="email-row__label-badge">${_escapeHtml(email.label)}</span>
          <span class="email-row__from">${_escapeHtml(_extractSender(email.from_addr))}</span>
          <span class="email-row__date">${dateStr}</span>
          <span class="email-row__icons">${icons}</span>
        </div>
        <div class="email-row__snippet">${email._snippet}</div>
      `;
    } else {
      row.innerHTML = `
        <div class="email-row__subject">${_escapeHtml(email.subject || '(no subject)')}</div>
        <div class="email-row__meta">
          <span class="email-row__date">${dateStr}</span>
          <span class="email-row__from">${_escapeHtml(_extractSender(email.from_addr))}</span>
          <span class="email-row__icons">${icons}</span>
        </div>
      `;
    }

    row.addEventListener('click', () => _selectEmail(index));
    return row;
  }

  /** Select an email by index and notify viewer */
  function _selectEmail(index) {
    const email = _emails[index];
    if (!email) return;

    _activeIndex = index;
    _activeEmailId = email.id;

    // Update active styling
    document.querySelectorAll('.email-row--active').forEach(el => {
      el.classList.remove('email-row--active');
    });
    // Email ids are composite keys ("<label>/<message_id>"), so they carry arbitrary
    // label text — escape the quotes/backslashes an attribute selector would choke on.
    const selectorId = String(email.id).replace(/\\/g, '\\\\').replace(/"/g, '\\"');
    const row = document.querySelector(`.email-row[data-email-id="${selectorId}"]`);
    if (row) row.classList.add('email-row--active');

    // Dispatch event to open in viewer
    document.dispatchEvent(new CustomEvent('email:select', {
      detail: {
        email,
        index,
        total: _emails.length,
        hasPrev: index > 0,
        hasNext: index < _emails.length - 1,
      }
    }));
  }

  /** Navigate to adjacent email */
  function selectByOffset(offset) {
    const newIndex = _activeIndex + offset;
    if (newIndex >= 0 && newIndex < _emails.length) {
      _selectEmail(newIndex);
      // Scroll the virtual list to keep the item visible
      if (_virtualScroll) {
        _virtualScroll.scrollToIndex(newIndex);
      }
    }
  }

  /** Get the current active email ID */
  function getActiveId() {
    return _activeEmailId;
  }

  /** Update a specific email's state in the local cache */
  function updateEmailState(emailId, updates) {
    const email = _emails.find(e => e.id === emailId);
    if (email) {
      Object.assign(email, updates);
      // Re-render just that row if visible
      if (_virtualScroll) {
        _virtualScroll.refresh();
      }
    }
  }

  /** Bind toolbar sort/filter buttons */
  function _bindToolbar() {
    // Sort buttons
    document.querySelectorAll('[data-sort]').forEach(btn => {
      btn.addEventListener('click', () => {
        const sort = btn.getAttribute('data-sort');
        _sort = sort;
        // Update active state
        document.querySelectorAll('[data-sort]').forEach(b => b.classList.remove('toolbar-btn--active'));
        btn.classList.add('toolbar-btn--active');
        // Re-sort and render
        _sortEmails();
        _renderList();
      });
    });

    // Filter buttons (toggle behavior)
    document.querySelectorAll('[data-filter]').forEach(btn => {
      btn.addEventListener('click', () => {
        const filter = btn.getAttribute('data-filter');
        if (_filter === filter) {
          _filter = null;
          btn.classList.remove('toolbar-btn--active');
        } else {
          _filter = filter;
          document.querySelectorAll('[data-filter]').forEach(b => b.classList.remove('toolbar-btn--active'));
          btn.classList.add('toolbar-btn--active');
        }
        // Reload current view with new filter
        loadView(_currentView, _currentLabel);
      });
    });

    // Set initial sort state
    document.getElementById('sort-newest')?.classList.add('toolbar-btn--active');
  }

  /** Bind keyboard navigation for the email list */
  function _bindKeyboard() {
    document.addEventListener('keydown', (e) => {
      // Don't handle if focus is in an input or the search modal is open
      if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return;
      if (document.getElementById('search-overlay')?.classList.contains('search-overlay--visible')) return;

      if (e.key === 'j' || e.key === 'ArrowDown') {
        e.preventDefault();
        selectByOffset(1);
      } else if (e.key === 'k' || e.key === 'ArrowUp') {
        e.preventDefault();
        selectByOffset(-1);
      } else if (e.key === 'Enter') {
        // Enter selects the current email (re-trigger viewer)
        if (_activeIndex >= 0) {
          _selectEmail(_activeIndex);
        }
      }
    });
  }

  /** Format a date string to readable format */
  function _formatDate(dateStr) {
    if (!dateStr) return '—';
    try {
      const d = new Date(dateStr.replace(' ', 'T'));
      return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
    } catch {
      return dateStr.substring(0, 10);
    }
  }

  /** Extract sender name from "Name <email>" format */
  function _extractSender(fromAddr) {
    if (!fromAddr) return '';
    const match = fromAddr.match(/^([^<]+)</);
    return match ? match[1].trim().replace(/^"|"$/g, '') : fromAddr;
  }

  function _escapeHtml(str) {
    if (!str) return '';
    const el = document.createElement('span');
    el.textContent = str;
    return el.innerHTML;
  }

  /** Enter search mode — save current view, show search results */
  function enterSearchMode(query) {
    clearTimeout(_searchDebounceTimer);

    if (!query || !query.trim()) {
      // Empty query: exit search mode and restore previous view
      if (_isSearchMode) exitSearchMode();
      return;
    }

    _searchDebounceTimer = setTimeout(async () => {
      try {
        // Save current view on first search entry
        if (!_isSearchMode) {
          _savedView = { view: _currentView, label: _currentLabel };
        }

        _isSearchMode = true;
        _searchResults = await Bridge.searchEmails(query, 50);

        // Hide toolbar (sort/filter don't apply to FTS-ranked results)
        const toolbar = document.getElementById('email-list-toolbar');
        if (toolbar) toolbar.classList.add('hidden');

        // Update header
        const titleEl = document.getElementById('list-title');
        const countEl = document.getElementById('list-count');
        if (titleEl) titleEl.textContent = 'Search Results';
        if (countEl) countEl.textContent = `${_searchResults.length}`;

        // Map search results to email-like objects for the list renderer
        _emails = _searchResults.map(r => ({
          id: r.id,
          label: r.label,
          subject: r.subject,
          from_addr: r.from_addr,
          date: r.date,
          is_read: r.is_read,
          is_bookmarked: r.is_bookmarked,
          _snippet: r.snippet, // raw HTML snippet from FTS5 (already has <mark> tags)
        }));

        _activeEmailId = null;
        _activeIndex = -1;
        _renderList();
      } catch (err) {
        console.error('Search error:', err);
      }
    }, 150); // 150ms debounce for responsive search
  }

  /** Exit search mode — restore the previously active view */
  function exitSearchMode() {
    if (!_isSearchMode) return;

    clearTimeout(_searchDebounceTimer);
    _isSearchMode = false;
    _searchResults = [];
    _activeEmailId = null;
    _activeIndex = -1;

    // Show toolbar again
    const toolbar = document.getElementById('email-list-toolbar');
    if (toolbar) toolbar.classList.remove('hidden');

    // Restore the saved view
    if (_savedView) {
      loadView(_savedView.view, _savedView.label);
      _savedView = null;
    }
  }

  /** Check if currently in search mode */
  function isSearchMode() {
    return _isSearchMode;
  }

  return { init, loadView, selectByOffset, getActiveId, updateEmailState, enterSearchMode, exitSearchMode, isSearchMode };
})();
