/**
 * Main app controller — initialization, state management, and routing.
 * Coordinates between Sidebar, EmailList, EmailViewer, and SearchModal.
 */
const App = (() => {
  let _initialized = false;

  /** Boot the application */
  async function init() {
    if (_initialized) return;
    _initialized = true;

    // Initialize components
    EmailList.init();
    EmailViewer.init();
    SearchModal.init();

    // Check if newsletters path is configured
    try {
      const path = await Bridge.getNewslettersPath();
      if (path) {
        // Path exists — check if we have indexed data
        await _showMainUI();
      } else {
        _showWelcome();
      }
    } catch (err) {
      console.error('Init error:', err);
      _showWelcome();
    }

    // Bind global navigation events
    _bindNavigation();
  }

  /** Show the welcome/setup screen */
  function _showWelcome() {
    document.getElementById('welcome-screen').classList.remove('hidden');
    document.getElementById('app-layout').classList.add('hidden');

    // Bind the folder picker button
    const pickBtn = document.getElementById('welcome-pick-btn');
    pickBtn.addEventListener('click', async () => {
      try {
        const selectedPath = await Bridge.openFolderDialog();
        if (!selectedPath) return; // User cancelled

        pickBtn.disabled = true;
        pickBtn.textContent = 'Indexing...';

        // Save the path
        await Bridge.setNewslettersPath(selectedPath);

        // Show progress bar
        document.getElementById('welcome-progress').classList.remove('hidden');

        // Listen for progress events
        const unlisten = await Bridge.onIndexProgress((progress) => {
          _updateProgress(progress);
        });

        // Start indexing
        const count = await Bridge.scanAndIndex();

        // Clean up listener
        if (unlisten) unlisten();

        // Transition to main UI
        await _showMainUI();
      } catch (err) {
        console.error('Setup error:', err);
        pickBtn.disabled = false;
        pickBtn.textContent = 'Choose Newsletters Folder';
        const progressText = document.getElementById('progress-text');
        if (progressText) progressText.textContent = `Error: ${err}`;
      }
    });
  }

  /** Update the progress bar during indexing */
  function _updateProgress(progress) {
    const fill = document.getElementById('progress-fill');
    const text = document.getElementById('progress-text');

    if (progress.phase === 'scanning') {
      if (fill) fill.style.width = '0%';
      if (text) text.textContent = 'Scanning newsletters folder...';
    } else if (progress.phase === 'indexing') {
      const pct = progress.total > 0
        ? Math.round((progress.current / progress.total) * 100)
        : 0;
      if (fill) fill.style.width = `${pct}%`;
      if (text) text.textContent = `Indexing: ${progress.current.toLocaleString()} / ${progress.total.toLocaleString()} emails`;
    } else if (progress.phase === 'done') {
      if (fill) fill.style.width = '100%';
      if (text) text.textContent = `Done! ${progress.current.toLocaleString()} emails indexed.`;
    }
  }

  /** Transition to the main three-panel UI */
  async function _showMainUI() {
    document.getElementById('welcome-screen').classList.add('hidden');
    document.getElementById('app-layout').classList.remove('hidden');

    // Initialize sidebar (loads labels)
    await Sidebar.init();
    await Sidebar.updateBookmarksCount();

    // If no labels loaded, auto-trigger indexing (path exists but DB empty)
    const labels = await Bridge.getLabels();
    if (labels.length === 0) {
      const title = document.getElementById('list-title');
      if (title) title.textContent = 'Indexing...';

      const unlisten = await Bridge.onIndexProgress((progress) => {
        if (title && progress.phase === 'indexing') {
          const pct = progress.total > 0 ? Math.round((progress.current / progress.total) * 100) : 0;
          title.textContent = `Indexing: ${pct}% (${progress.current.toLocaleString()} emails)`;
        }
      });

      try {
        await Bridge.scanAndIndex();
      } catch (err) {
        console.error('Auto-index error:', err);
      }
      if (unlisten) unlisten();

      // Reload sidebar labels after indexing
      await Sidebar.loadLabels();
      await Sidebar.updateBookmarksCount();
    }

    // Load "All Emails" view by default
    Sidebar.setActive('all');
    await EmailList.loadView('all');
  }

  /** Bind global navigation event handlers */
  function _bindNavigation() {
    // Sidebar navigation changes
    document.addEventListener('nav:change', async (e) => {
      const { view, label } = e.detail;
      EmailViewer.showEmpty();

      if (view === 'all') {
        await EmailList.loadView('all');
      } else if (view === 'bookmarks') {
        await EmailList.loadView('bookmarks');
      } else if (view === 'label') {
        await EmailList.loadView('label', label);
      }
    });

    // Search result navigation
    document.addEventListener('search:navigate', async (e) => {
      const { emailId, label } = e.detail;

      // Select the label in sidebar
      Sidebar.setActive(label);

      // Load that label's emails
      await EmailList.loadView('label', label);

      // Find and select the specific email (small delay for list to render)
      setTimeout(async () => {
        try {
          const email = await Bridge.getEmail(emailId);
          if (email) {
            EmailViewer.showEmail(email, { hasPrev: false, hasNext: false });
          }
        } catch (err) {
          console.error('Failed to navigate to search result:', err);
        }
      }, 100);
    });

    // Settings (re-index / change folder)
    document.addEventListener('app:settings', async () => {
      try {
        const selectedPath = await Bridge.openFolderDialog();
        if (!selectedPath) return;

        await Bridge.setNewslettersPath(selectedPath);

        // Re-index with progress (shown in a simple way for now)
        const unlisten = await Bridge.onIndexProgress((progress) => {
          const title = document.getElementById('list-title');
          if (title && progress.phase === 'indexing') {
            title.textContent = `Indexing: ${progress.current}/${progress.total}`;
          }
        });

        await Bridge.scanAndIndex();
        if (unlisten) unlisten();

        // Reload the UI
        await Sidebar.loadLabels();
        await Sidebar.updateBookmarksCount();
        await EmailList.loadView('all');
        Sidebar.setActive('all');

        document.getElementById('list-title').textContent = 'All Emails';
      } catch (err) {
        console.error('Settings error:', err);
      }
    });
  }

  return { init };
})();

// Boot the app when DOM is ready
document.addEventListener('DOMContentLoaded', () => {
  App.init();
});
