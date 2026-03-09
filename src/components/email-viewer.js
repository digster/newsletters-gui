/**
 * Email viewer component — right panel with iframe rendering.
 * Displays email HTML in a sandboxed iframe with read/bookmark controls.
 */
const EmailViewer = (() => {
  let _currentEmail = null;
  let _hasPrev = false;
  let _hasNext = false;

  const ICON = {
    bookmarkOutline: '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 2.5h9v12L8 11l-4.5 3.5v-12z"/></svg>',
    bookmarkFilled: '<svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 2.5h9v12L8 11l-4.5 3.5v-12z"/></svg>',
    eyeOpen: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>',
    eyeClosed: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>',
  };

  function init() {
    _bindEvents();
  }

  /** Display an email in the viewer */
  async function showEmail(email, { hasPrev, hasNext } = {}) {
    _currentEmail = email;
    _hasPrev = hasPrev || false;
    _hasNext = hasNext || false;

    // Show viewer content, hide empty state
    document.getElementById('viewer-empty')?.classList.add('hidden');
    const content = document.getElementById('viewer-content');
    if (content) {
      content.classList.remove('hidden');
      content.style.display = 'flex';
    }

    // Update header metadata
    document.getElementById('viewer-subject').textContent = email.subject || '(no subject)';
    document.getElementById('viewer-date').textContent = _formatDate(email.date);
    document.getElementById('viewer-from').textContent = email.from_addr || '';

    // Update nav buttons
    const prevBtn = document.getElementById('viewer-prev');
    const nextBtn = document.getElementById('viewer-next');
    if (prevBtn) prevBtn.disabled = !_hasPrev;
    if (nextBtn) nextBtn.disabled = !_hasNext;

    // Update bookmark and read buttons
    _updateBookmarkBtn(email.is_bookmarked);
    _updateReadBtn(email.is_read);

    // Auto-mark as read
    if (!email.is_read) {
      try {
        await Bridge.markRead(email.id);
        email.is_read = true;
        _updateReadBtn(true);
        EmailList.updateEmailState(email.id, { is_read: true });
      } catch (err) {
        console.error('Failed to mark read:', err);
      }
    }

    // Load HTML content for iframe with theme-aware styles
    try {
      const html = await Bridge.getEmailHtml(email.id);
      const iframe = document.getElementById('viewer-frame');
      if (iframe) {
        iframe.srcdoc = _themedSrcdoc(html);
      }
    } catch (err) {
      console.error('Failed to load email HTML:', err);
      const iframe = document.getElementById('viewer-frame');
      if (iframe) {
        iframe.srcdoc = `<html><body style="font-family:system-ui;padding:40px;color:#888;text-align:center;">
          <p>Failed to load email content</p>
          <p style="font-size:12px;">${_escapeHtml(err.toString())}</p>
        </body></html>`;
      }
    }
  }

  /** Show the empty state */
  function showEmpty() {
    _currentEmail = null;
    document.getElementById('viewer-empty')?.classList.remove('hidden');
    const content = document.getElementById('viewer-content');
    if (content) {
      content.classList.add('hidden');
      content.style.display = 'none';
    }
  }

  /** Update the bookmark button appearance */
  function _updateBookmarkBtn(isBookmarked) {
    const btn = document.getElementById('viewer-bookmark-btn');
    if (!btn) return;
    btn.innerHTML = isBookmarked ? ICON.bookmarkFilled : ICON.bookmarkOutline;
    btn.classList.toggle('viewer__action-btn--active', isBookmarked);
    btn.title = isBookmarked ? 'Remove bookmark' : 'Bookmark';
  }

  /** Update the read button appearance */
  function _updateReadBtn(isRead) {
    const btn = document.getElementById('viewer-read-btn');
    if (!btn) return;
    btn.innerHTML = isRead ? ICON.eyeOpen : ICON.eyeClosed;
    btn.classList.toggle('viewer__action-btn--active', isRead);
    btn.title = isRead ? 'Mark as unread' : 'Mark as read';
  }

  /** Bind viewer event listeners */
  function _bindEvents() {
    // Prev/Next navigation
    document.getElementById('viewer-prev')?.addEventListener('click', () => {
      EmailList.selectByOffset(-1);
    });
    document.getElementById('viewer-next')?.addEventListener('click', () => {
      EmailList.selectByOffset(1);
    });

    // Toggle bookmark
    document.getElementById('viewer-bookmark-btn')?.addEventListener('click', async () => {
      if (!_currentEmail) return;
      try {
        const result = await Bridge.toggleBookmark(_currentEmail.id);
        _currentEmail.is_bookmarked = result.value;
        _updateBookmarkBtn(result.value);
        EmailList.updateEmailState(_currentEmail.id, { is_bookmarked: result.value });
        Sidebar.updateBookmarksCount();
      } catch (err) {
        console.error('Failed to toggle bookmark:', err);
      }
    });

    // Toggle read
    document.getElementById('viewer-read-btn')?.addEventListener('click', async () => {
      if (!_currentEmail) return;
      try {
        const result = await Bridge.toggleRead(_currentEmail.id);
        _currentEmail.is_read = result.value;
        _updateReadBtn(result.value);
        EmailList.updateEmailState(_currentEmail.id, { is_read: result.value });
      } catch (err) {
        console.error('Failed to toggle read:', err);
      }
    });

    // Listen for email selection events
    document.addEventListener('email:select', (e) => {
      const { email, hasPrev, hasNext } = e.detail;
      showEmail(email, { hasPrev, hasNext });
    });

    // Re-render iframe when theme changes so background matches
    document.addEventListener('theme:change', () => {
      if (_currentEmail) {
        showEmail(_currentEmail, { hasPrev: _hasPrev, hasNext: _hasNext });
      }
    });
  }

  function _formatDate(dateStr) {
    if (!dateStr) return '—';
    try {
      const d = new Date(dateStr.replace(' ', 'T'));
      return d.toLocaleDateString('en-US', {
        weekday: 'short', month: 'short', day: 'numeric', year: 'numeric',
        hour: 'numeric', minute: '2-digit',
      });
    } catch {
      return dateStr;
    }
  }

  /**
   * Wrap email HTML with a theme-aware <style> tag so the iframe
   * background and color-scheme match the active app theme.
   */
  function _themedSrcdoc(html) {
    const isDark = document.documentElement.getAttribute('data-theme') === 'dark';
    const themeStyle = `<style data-theme-inject>
      html { color-scheme: ${isDark ? 'dark' : 'light'}; }
      body { background: ${isDark ? '#1a1a1a' : '#fff'}; }
    </style>`;
    // Inject before </head> if present, otherwise prepend
    if (html.includes('</head>')) {
      return html.replace('</head>', themeStyle + '</head>');
    }
    return themeStyle + html;
  }

  function _escapeHtml(str) {
    if (!str) return '';
    const el = document.createElement('span');
    el.textContent = str;
    return el.innerHTML;
  }

  return { init, showEmail, showEmpty };
})();
