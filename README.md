# Newsletter Archive — Desktop App

A portable Tauri desktop app for browsing ~14,000 email newsletters with full-text search, bookmarks, and read tracking.

## Features

- **Three-panel layout**: Sidebar (labels) → Email list (virtual-scrolled) → Email viewer (sandboxed iframe)
- **Full-text search**: FTS5-powered search with snippet highlighting, accessible via `Cmd+K`
- **User state**: Bookmarks and read tracking persisted in local SQLite
- **Keyboard navigation**: `j`/`k` to navigate emails, `/` or `Cmd+K` for search, `Escape` to close
- **Dark mode**: Automatic via `prefers-color-scheme`
- **No build step**: Vanilla JS frontend, no bundler required
- **Fast indexing**: ~14K emails indexed in <2 seconds (release mode)

## Prerequisites

- [Rust](https://rustup.rs/) (1.70+)
- [Node.js](https://nodejs.org/) (18+)
- macOS (for `.dmg` builds; Linux/Windows should also work with Tauri)

## Development

```bash
# Install dependencies
npm install

# Run in dev mode (hot-reload for frontend, file watcher for Rust)
npm run dev

# Or equivalently
npx tauri dev
```

On first launch, click "Choose Newsletters Folder" and select the `newsletters/` directory.

## Production Build

```bash
# Build optimized binary + DMG
npx tauri build --bundles dmg

# Output: src-tauri/target/release/bundle/dmg/Newsletter Archive_0.1.0_aarch64.dmg
```

## Data Source

The app reads from a `newsletters/` folder with this structure:

```
newsletters/
  {label}/                     # 65 label folders
    {8-hex-id}/                # Individual email folders
      {16-hex-id}.html         # Full HTML email (displayed in iframe)
      {slug}_{8-hex-id}.md     # Markdown with YAML frontmatter (indexed)
      {16-hex-id}.txt          # Plain text (optional)
```

The `.md` frontmatter contains: `subject`, `from`, `to`, `date`, `labels`, `label_ids`.

## Tech Stack

- **Backend**: Tauri v2, Rust, rusqlite (with FTS5), serde_yaml
- **Frontend**: Vanilla JS (no framework/bundler), CSS custom properties
- **Database**: SQLite with WAL mode, stored in `~/Library/Application Support/com.newsletters.gui/`
