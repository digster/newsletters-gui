/**
 * Search modal component — Cmd+K command palette for full-text search.
 * Uses FTS5 search with snippet highlighting and keyboard navigation.
 */
const SearchModal = (() => {
  let _isOpen = false;
  let _results = [];
  let _focusIndex = -1;
  let _debounceTimer = null;

  function init() {
    _bindEvents();
  }

  /** Open the search modal */
  function open() {
    _isOpen = true;
    const overlay = document.getElementById('search-overlay');
    overlay.classList.add('search-overlay--visible');
    const input = document.getElementById('search-input');
    input.value = '';
    input.focus();
    _results = [];
    _focusIndex = -1;
    _renderResults();
  }

  /** Close the search modal */
  function close() {
    _isOpen = false;
    document.getElementById('search-overlay').classList.remove('search-overlay--visible');
    document.getElementById('search-input').blur();
  }

  /** Toggle the modal */
  function toggle() {
    _isOpen ? close() : open();
  }

  function isOpen() {
    return _isOpen;
  }

  /** Perform the FTS5 search with debouncing */
  function _doSearch(query) {
    clearTimeout(_debounceTimer);

    if (!query.trim()) {
      _results = [];
      _focusIndex = -1;
      _renderResults();
      return;
    }

    // Show loading state
    document.getElementById('search-results').innerHTML =
      '<div class="search-modal__loading">Searching...</div>';

    _debounceTimer = setTimeout(async () => {
      try {
        _results = await Bridge.searchEmails(query, 30);
        _focusIndex = _results.length > 0 ? 0 : -1;
        _renderResults();
      } catch (err) {
        console.error('Search error:', err);
        document.getElementById('search-results').innerHTML =
          `<div class="search-modal__empty">Search error: ${_escapeHtml(err.toString())}</div>`;
      }
    }, 150); // 150ms debounce for responsive-feeling search
  }

  /** Render search results */
  function _renderResults() {
    const container = document.getElementById('search-results');

    if (_results.length === 0) {
      const input = document.getElementById('search-input');
      if (input && input.value.trim()) {
        container.innerHTML = '<div class="search-modal__empty">No results found</div>';
      } else {
        container.innerHTML = '<div class="search-modal__empty">Type to search across all emails</div>';
      }
      return;
    }

    container.innerHTML = '';
    _results.forEach((result, i) => {
      const item = document.createElement('div');
      item.className = 'search-result';
      if (i === _focusIndex) item.classList.add('search-result--focused');
      item.setAttribute('data-index', i);

      const dateStr = _formatDate(result.date);

      item.innerHTML = `
        <div class="search-result__subject">${_escapeHtml(result.subject || '(no subject)')}</div>
        <div class="search-result__meta">
          <span class="search-result__label">${_escapeHtml(result.label)}</span>
          <span class="search-result__date">${dateStr}</span>
        </div>
        ${result.snippet ? `<div class="search-result__snippet">${result.snippet}</div>` : ''}
      `;

      item.addEventListener('click', () => _selectResult(i));
      item.addEventListener('mouseenter', () => {
        _focusIndex = i;
        _updateFocus();
      });

      container.appendChild(item);
    });
  }

  /** Select a search result and navigate to it */
  function _selectResult(index) {
    const result = _results[index];
    if (!result) return;

    close();

    // Navigate to the email: select the label, then the email
    document.dispatchEvent(new CustomEvent('search:navigate', {
      detail: {
        emailId: result.id,
        label: result.label,
      }
    }));
  }

  /** Update visual focus indicator */
  function _updateFocus() {
    document.querySelectorAll('.search-result').forEach((el, i) => {
      el.classList.toggle('search-result--focused', i === _focusIndex);
    });

    // Scroll focused item into view
    const focused = document.querySelector('.search-result--focused');
    if (focused) {
      focused.scrollIntoView({ block: 'nearest' });
    }
  }

  /** Bind all event listeners */
  function _bindEvents() {
    const input = document.getElementById('search-input');
    const overlay = document.getElementById('search-overlay');

    // Search input
    input.addEventListener('input', (e) => {
      _doSearch(e.target.value);
    });

    // Keyboard navigation within the modal
    input.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        if (_results.length > 0) {
          _focusIndex = Math.min(_focusIndex + 1, _results.length - 1);
          _updateFocus();
        }
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        if (_results.length > 0) {
          _focusIndex = Math.max(_focusIndex - 1, 0);
          _updateFocus();
        }
      } else if (e.key === 'Enter') {
        e.preventDefault();
        if (_focusIndex >= 0) {
          _selectResult(_focusIndex);
        }
      } else if (e.key === 'Escape') {
        e.preventDefault();
        close();
      }
    });

    // Click outside to close
    overlay.addEventListener('click', (e) => {
      if (e.target === overlay) {
        close();
      }
    });

    // Global keyboard shortcut: Cmd+K or Ctrl+K
    document.addEventListener('keydown', (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        toggle();
      }
      // Also support "/" to open search (if not in an input)
      if (e.key === '/' && !_isOpen &&
          e.target.tagName !== 'INPUT' && e.target.tagName !== 'TEXTAREA') {
        e.preventDefault();
        open();
      }
    });
  }

  function _formatDate(dateStr) {
    if (!dateStr) return '';
    try {
      const d = new Date(dateStr.replace(' ', 'T'));
      return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
    } catch {
      return dateStr.substring(0, 10);
    }
  }

  function _escapeHtml(str) {
    if (!str) return '';
    const el = document.createElement('span');
    el.textContent = str;
    return el.innerHTML;
  }

  return { init, open, close, toggle, isOpen };
})();
