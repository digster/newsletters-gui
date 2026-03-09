# Prompts

## 2026-03-08: Initial Implementation

Implement a Tauri v2 desktop app for browsing ~14,000 email newsletters. Three-panel layout (sidebar, email list, viewer) with SQLite/FTS5 backend, virtual scrolling, Cmd+K search, bookmarks, and read tracking. Port design system from newsletters-github static site. Full plan provided with architecture, schema, and implementation phases.

## 2026-03-08: App Icon

Create an app icon for the Newsletter Archive Tauri v2 desktop app. Use the image-generator skill to create a 1024×1024 icon with a stylized envelope/newsletter concept in the app's blue (#2563eb) color scheme. Convert to all required Tauri icon sizes using `tauri icon` CLI. Verify the build works with the new icons.

## 2026-03-08: Fix Broken Search + Add Universal Search Box

Fix Cmd+K search that returns no results due to FTS5/emails schema mismatch (missing `body` column). Add inline search bar to email-list panel replacing the search modal. Add `from_addr` to FTS5 for sender search. Add integration tests for the search pipeline.

## 2026-03-08: Add Reindex Button to Sidebar Footer

Add a dedicated "Reindex" button next to the existing "Settings" button in the sidebar footer. This button re-indexes the already-configured newsletters folder without opening the folder picker dialog. Shows a confirmation dialog before proceeding, displays indexing progress in the list title, and reloads sidebar labels and email list after completion.

## 2026-03-08: Resizable & Collapsible Vertical Panes

Implement draggable splitter bars between the three-panel layout (sidebar, email-list, email-viewer) with collapse/expand support. Splitters use Pointer Events with setPointerCapture for smooth drag even outside the window. Chevron buttons toggle collapse with CSS transitions. Keyboard shortcuts: Cmd+B (sidebar), Cmd+Shift+B (email list). Double-click resets to default width. Layout persisted to localStorage.
