# Prompts

## 2026-03-08: Initial Implementation

Implement a Tauri v2 desktop app for browsing ~14,000 email newsletters. Three-panel layout (sidebar, email list, viewer) with SQLite/FTS5 backend, virtual scrolling, Cmd+K search, bookmarks, and read tracking. Port design system from newsletters-github static site. Full plan provided with architecture, schema, and implementation phases.

## 2026-03-08: App Icon

Create an app icon for the Newsletter Archive Tauri v2 desktop app. Use the image-generator skill to create a 1024×1024 icon with a stylized envelope/newsletter concept in the app's blue (#2563eb) color scheme. Convert to all required Tauri icon sizes using `tauri icon` CLI. Verify the build works with the new icons.

## 2026-03-08: Fix Broken Search + Add Universal Search Box

Fix Cmd+K search that returns no results due to FTS5/emails schema mismatch (missing `body` column). Add inline search bar to email-list panel replacing the search modal. Add `from_addr` to FTS5 for sender search. Add integration tests for the search pipeline.
