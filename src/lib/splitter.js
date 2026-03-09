/**
 * PaneSplitter — Resizable & collapsible vertical pane splitters.
 *
 * Provides draggable splitter bars between the three-panel layout
 * (sidebar · email-list · email-viewer) with collapse/expand support,
 * keyboard shortcuts, and localStorage persistence.
 */
const PaneSplitter = (() => {
  // ── Configuration ────────────────────────────────────────────────
  const CONSTRAINTS = {
    sidebar:  { min: 160, max: 400, default: 240 },
    list:     { min: 240, max: 600, default: 360 },
    viewer:   { min: 300 },
  };

  const STORAGE_KEY = 'pane-layout';

  // ── State ────────────────────────────────────────────────────────
  let sidebarWidth  = CONSTRAINTS.sidebar.default;
  let listWidth     = CONSTRAINTS.list.default;
  let sidebarCollapsed = false;
  let listCollapsed    = false;

  // DOM references (set during init)
  let sidebar, listPanel, viewer;
  let splitterSidebar, splitterList;
  let collapseSidebarBtn, collapseListBtn;

  // ── Persistence ──────────────────────────────────────────────────
  function _loadState() {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (!raw) return;
      const state = JSON.parse(raw);
      if (state.sidebarWidth)     sidebarWidth     = state.sidebarWidth;
      if (state.listWidth)        listWidth        = state.listWidth;
      if (state.sidebarCollapsed) sidebarCollapsed = state.sidebarCollapsed;
      if (state.listCollapsed)    listCollapsed    = state.listCollapsed;
    } catch (_) { /* ignore corrupt data */ }
  }

  function _saveState() {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      sidebarWidth,
      listWidth,
      sidebarCollapsed,
      listCollapsed,
    }));
  }

  // ── Apply widths to DOM ──────────────────────────────────────────
  function _applyWidths(animate) {
    if (animate) {
      sidebar.classList.add('pane--animating');
      listPanel.classList.add('pane--animating');
    }

    if (sidebarCollapsed) {
      sidebar.classList.add('pane--collapsed');
      splitterSidebar.classList.add('splitter--collapsed');
    } else {
      sidebar.classList.remove('pane--collapsed');
      splitterSidebar.classList.remove('splitter--collapsed');
      sidebar.style.width = sidebarWidth + 'px';
    }

    if (listCollapsed) {
      listPanel.classList.add('pane--collapsed');
      splitterList.classList.add('splitter--collapsed');
    } else {
      listPanel.classList.remove('pane--collapsed');
      splitterList.classList.remove('splitter--collapsed');
      listPanel.style.width = listWidth + 'px';
    }

    // Update chevron directions
    _updateChevrons();

    if (animate) {
      // Remove animation class after transition completes
      const onEnd = () => {
        sidebar.classList.remove('pane--animating');
        listPanel.classList.remove('pane--animating');
        sidebar.removeEventListener('transitionend', onEnd);
        listPanel.removeEventListener('transitionend', onEnd);
        _dispatchResize();
      };
      sidebar.addEventListener('transitionend', onEnd);
      listPanel.addEventListener('transitionend', onEnd);
    }
  }

  /** Update chevron button rotation based on collapse state */
  function _updateChevrons() {
    // Sidebar chevron: points left when expanded (click to collapse),
    // points right when collapsed (click to expand)
    if (collapseSidebarBtn) {
      const svg = collapseSidebarBtn.querySelector('svg');
      if (svg) svg.style.transform = sidebarCollapsed ? 'rotate(180deg)' : '';
    }

    // List chevron: same logic
    if (collapseListBtn) {
      const svg = collapseListBtn.querySelector('svg');
      if (svg) svg.style.transform = listCollapsed ? 'rotate(180deg)' : '';
    }
  }

  /** Dispatch a pane:resize event so other components can react */
  function _dispatchResize() {
    document.dispatchEvent(new CustomEvent('pane:resize'));
  }

  // ── Drag handling ────────────────────────────────────────────────

  /**
   * Sets up pointer-event based drag on a splitter element.
   * @param {HTMLElement} splitter   The splitter bar element
   * @param {HTMLElement} pane       The pane to the left of the splitter
   * @param {object} constraints    { min, max, default }
   * @param {function} onResize     Called with the new width during drag
   */
  function _initDrag(splitter, pane, constraints, onResize) {
    let startX = 0;
    let startWidth = 0;
    let isDragging = false;

    splitter.addEventListener('pointerdown', (e) => {
      // Only handle primary button (left click)
      if (e.button !== 0) return;
      // Don't start drag if clicking the collapse button
      if (e.target.closest('.splitter__collapse-btn')) return;

      e.preventDefault();
      isDragging = true;
      startX = e.clientX;
      startWidth = pane.getBoundingClientRect().width;

      splitter.setPointerCapture(e.pointerId);
      document.body.classList.add('is-resizing');
    });

    splitter.addEventListener('pointermove', (e) => {
      if (!isDragging) return;

      const delta = e.clientX - startX;
      let newWidth = startWidth + delta;

      // Clamp to constraints
      newWidth = Math.max(constraints.min, Math.min(constraints.max, newWidth));

      // Enforce minimum viewer width
      const availableForViewer = window.innerWidth
        - (pane === sidebar ? newWidth : sidebar.getBoundingClientRect().width)
        - (pane === listPanel ? newWidth : listPanel.getBoundingClientRect().width)
        - 8; // account for splitter widths (4px each)
      if (availableForViewer < CONSTRAINTS.viewer.min) return;

      pane.style.width = newWidth + 'px';
      onResize(newWidth);
    });

    splitter.addEventListener('pointerup', (e) => {
      if (!isDragging) return;
      isDragging = false;
      splitter.releasePointerCapture(e.pointerId);
      document.body.classList.remove('is-resizing');
      _saveState();
      _dispatchResize();
    });

    // Double-click resets to default width
    splitter.addEventListener('dblclick', (e) => {
      if (e.target.closest('.splitter__collapse-btn')) return;
      onResize(constraints.default);
      pane.style.width = constraints.default + 'px';
      _saveState();
      _dispatchResize();
    });
  }

  // ── Collapse / Expand ────────────────────────────────────────────

  function toggleSidebar() {
    sidebarCollapsed = !sidebarCollapsed;
    _applyWidths(true);
    _saveState();
  }

  function toggleList() {
    listCollapsed = !listCollapsed;
    _applyWidths(true);
    _saveState();
  }

  // ── Keyboard shortcuts ───────────────────────────────────────────

  function _bindKeyboard() {
    document.addEventListener('keydown', (e) => {
      const mod = e.metaKey || e.ctrlKey;

      // Cmd+B / Ctrl+B → toggle sidebar
      if (mod && !e.shiftKey && e.key === 'b') {
        e.preventDefault();
        toggleSidebar();
        return;
      }

      // Cmd+Shift+B / Ctrl+Shift+B → toggle email list
      if (mod && e.shiftKey && (e.key === 'B' || e.key === 'b')) {
        e.preventDefault();
        toggleList();
        return;
      }
    });
  }

  // ── Window resize handler ────────────────────────────────────────

  function _bindWindowResize() {
    window.addEventListener('resize', () => {
      // Clamp pane widths if window shrinks
      const total = window.innerWidth;
      const splitterSpace = 8; // two 4px splitters
      const minViewer = CONSTRAINTS.viewer.min;

      if (!sidebarCollapsed && !listCollapsed) {
        const maxCombined = total - splitterSpace - minViewer;
        if (sidebarWidth + listWidth > maxCombined) {
          // Shrink list first, then sidebar
          listWidth = Math.max(CONSTRAINTS.list.min, maxCombined - sidebarWidth);
          if (sidebarWidth + listWidth > maxCombined) {
            sidebarWidth = Math.max(CONSTRAINTS.sidebar.min, maxCombined - listWidth);
          }
          _applyWidths(false);
          _saveState();
        }
      }

      _dispatchResize();
    });
  }

  // ── Initialization ───────────────────────────────────────────────

  function init() {
    // Grab DOM references
    sidebar       = document.getElementById('sidebar');
    listPanel     = document.getElementById('email-list-panel');
    viewer        = document.getElementById('email-viewer');
    splitterSidebar = document.getElementById('splitter-sidebar');
    splitterList    = document.getElementById('splitter-list');
    collapseSidebarBtn = document.getElementById('collapse-sidebar-btn');
    collapseListBtn    = document.getElementById('collapse-list-btn');

    if (!splitterSidebar || !splitterList) {
      console.warn('PaneSplitter: splitter elements not found');
      return;
    }

    // Load persisted state
    _loadState();

    // Apply initial widths (no animation on load)
    _applyWidths(false);

    // Set up drag on each splitter
    _initDrag(splitterSidebar, sidebar, CONSTRAINTS.sidebar, (w) => {
      sidebarWidth = w;
    });

    _initDrag(splitterList, listPanel, CONSTRAINTS.list, (w) => {
      listWidth = w;
    });

    // Collapse button click handlers
    collapseSidebarBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      toggleSidebar();
    });

    collapseListBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      toggleList();
    });

    // Keyboard shortcuts
    _bindKeyboard();

    // Window resize clamping
    _bindWindowResize();
  }

  return { init, toggleSidebar, toggleList };
})();
