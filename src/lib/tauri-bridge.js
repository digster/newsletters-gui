/**
 * Thin wrapper around Tauri's invoke() IPC.
 * Provides typed-ish helpers and centralizes error handling.
 */
const Bridge = (() => {
  function invoke(cmd, args = {}) {
    return window.__TAURI__.core.invoke(cmd, args);
  }

  /** Listen to a Tauri event from the backend */
  function listen(event, callback) {
    return window.__TAURI__.event.listen(event, callback);
  }

  return {
    // Settings
    getNewslettersPath: () => invoke('get_newsletters_path'),
    setNewslettersPath: (path) => invoke('set_newsletters_path', { path }),

    // Indexing
    scanAndIndex: () => invoke('scan_and_index'),
    onIndexProgress: (cb) => listen('index-progress', (e) => cb(e.payload)),

    // Labels / Emails
    getLabels: () => invoke('get_labels'),
    getEmailsByLabel: (label, offset = 0, limit = 100, sort = null, filter = null) =>
      invoke('get_emails_by_label', { label, offset, limit, sort, filter }),
    getEmailCount: (label, filter = null) =>
      invoke('get_email_count', { label, filter }),
    getEmail: (emailId) => invoke('get_email', { emailId }),
    getEmailHtml: (emailId) => invoke('get_email_html', { emailId }),

    // Search
    searchEmails: (query, limit = 50) => invoke('search_emails', { query, limit }),

    // State
    toggleBookmark: (emailId) => invoke('toggle_bookmark', { emailId }),
    toggleRead: (emailId) => invoke('toggle_read', { emailId }),
    markRead: (emailId) => invoke('mark_read', { emailId }),
    getBookmarkedEmails: (offset = 0, limit = 100) =>
      invoke('get_bookmarked_emails', { offset, limit }),

    // Dialog (folder picker)
    openFolderDialog: async () => {
      const result = await window.__TAURI__.dialog.open({
        directory: true,
        multiple: false,
        title: 'Select newsletters folder',
      });
      return result; // string path or null
    },

    // Confirmation dialog — returns true if user clicks Yes
    confirmDialog: (message, title = 'Confirm') => {
      return window.__TAURI__.dialog.ask(message, { title });
    },
  };
})();
