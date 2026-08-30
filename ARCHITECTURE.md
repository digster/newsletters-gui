# Architecture

## Overview

Newsletter Archive is a **Tauri v2** desktop app with a Rust backend and vanilla JS frontend. No build tools or bundlers — the frontend is served directly as static files.

```
┌───────────────────────────────────────────────────────────────────┐
│  Tauri Window (1280×800)                                          │
│ ┌──────────┬─┬────────────────┬─┬───────────────────────────────┐│
│ │ Sidebar  │║│  Email List    │║│  Email Viewer (iframe)        ││
│ │ (resizable│║│ (resizable)    │║│  (flex fill)                  ││
│ │ 160-400px)│║│ (240-600px)    │║│                               ││
│ └──────────┴─┴────────────────┴─┴───────────────────────────────┘│
│              ↑                 ↑                                  │
│         splitter bars (draggable, collapsible via chevron btns)   │
└───────────────────────────────────────────────────────────────────┘
```

## Data Flow

```
newsletters/ folder
    ↓ (scan_and_index: walkdir + serde_yaml)
SQLite + FTS5 database
    ↓ (Tauri invoke IPC)
Frontend components (vanilla JS)
    ↓ (iframe srcdoc)
Email HTML rendering
```

1. **Indexing**: Rust scans the `newsletters/` folder, parses `.md` YAML frontmatter, bulk-inserts into SQLite + FTS5
2. **Browsing**: Frontend calls Tauri commands (`get_labels`, `get_emails_by_label`) via `window.__TAURI__.core.invoke()`
3. **Viewing**: HTML emails are read as strings in Rust and rendered in a sandboxed `<iframe srcdoc>`
4. **Searching**: FTS5 `MATCH` queries with `snippet()` highlighting and `rank` ordering

## Project Structure

```
newsletters-gui/
├── src-tauri/                 # Rust backend
│   ├── Cargo.toml
│   ├── tauri.conf.json        # Window config, CSP, bundle settings
│   ├── capabilities/          # Tauri v2 permission model
│   └── src/
│       ├── main.rs            # Entry point
│       ├── lib.rs             # Plugin + command registration
│       ├── db.rs              # SQLite schema, migrations, helpers
│       ├── parser.rs          # YAML frontmatter parser
│       └── commands/
│           ├── scan.rs        # Directory walker + indexer
│           ├── emails.rs      # Label/email CRUD queries
│           ├── search.rs      # FTS5 search with snippet()
│           └── state.rs       # Bookmark/read toggle
├── src/                       # Frontend (no bundler)
│   ├── index.html             # Single page, three-panel layout
│   ├── style.css              # Design system (CSS custom properties)
│   ├── app.js                 # Main controller + init logic
│   ├── components/
│   │   ├── sidebar.js         # Label navigation
│   │   ├── email-list.js      # Virtual-scrolled list
│   │   ├── email-viewer.js    # Iframe renderer + controls
│   │   └── search-modal.js    # Cmd+K command palette
│   └── lib/
│       ├── virtual-scroll.js  # DOM-recycling scroll engine
│       ├── splitter.js        # Resizable/collapsible pane splitters
│       └── tauri-bridge.js    # Thin invoke() wrapper
└── package.json               # Only @tauri-apps/cli + serve
```

## Key Design Decisions

### Why rusqlite (not tauri-plugin-sql)?
Direct `rusqlite` with `bundled-full` gives us full FTS5 control: custom tokenizers, `snippet()`, `rank` ordering. The generic SQL plugin doesn't expose these.

### Why vanilla JS (no React/Vue)?
- No build step = instant dev reload
- The UI is simple enough (3 panels, list, iframe) that a framework adds complexity without benefit
- CSS custom properties handle theming; event delegation handles interactions

### Why virtual scroll?
With 14K+ emails, rendering all DOM nodes would consume ~500MB of memory. Virtual scroll renders only visible items (~30) plus a buffer, recycling DOM nodes as the user scrolls.

### FTS5 External Content Pattern
The FTS5 table uses `content='emails'` to share data with the emails table via rowid. On re-index, we DROP and recreate the FTS5 table (external content FTS5 can't be simply DELETEd without issues).

### Indexing Architecture
File I/O (reading 13K+ markdown files) happens BEFORE acquiring the database lock. This prevents blocking other Tauri commands during the slow I/O phase. The actual DB inserts (with lock held) complete in <1 second.

## SQLite Schema

```sql
CREATE TABLE emails (
    id TEXT PRIMARY KEY,           -- composite key: "<label>/<message_id>"
    label TEXT NOT NULL,
    dir_name TEXT NOT NULL,        -- on-disk email folder name (NOT unique across labels)
    message_id TEXT NOT NULL,      -- full Gmail message ID when determinable
    subject TEXT NOT NULL DEFAULT '',
    from_addr TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL DEFAULT '',  -- Truncated plaintext body (≤2000 chars) for FTS
    date TEXT,
    body_filename TEXT NOT NULL,   -- .html, or .txt for text-only newsletters
    body_format TEXT NOT NULL,     -- 'html' | 'text'

    md_filename TEXT,
    is_read INTEGER NOT NULL DEFAULT 0,
    is_bookmarked INTEGER NOT NULL DEFAULT 0
);

CREATE VIRTUAL TABLE emails_fts USING fts5(
    subject, body, from_addr, label,
    content='emails', content_rowid='rowid',
    tokenize='porter unicode61'
);

-- One row per physical body file on disk. This is the structural guard against
-- the directory-name collision bug described in "Email Identity" below.
CREATE UNIQUE INDEX idx_emails_identity ON emails(label, dir_name, body_filename);

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
```

## Email Identity

The single most important invariant in the backend: **`emails.id` is not the folder name.**

Email folders were originally named with an 8-char truncation of the Gmail message ID.
Those names collide (gmail-ingestor's `LEARNINGS.md` measured 6 colliding groups over
17,007 live messages — sequential IDs defeat birthday-bound intuition), and the *same*
name legitimately appears under multiple labels. Keying rows on the folder name meant
`INSERT OR REPLACE` silently overwrote one newsletter with another, and read/bookmark
state restored on re-index bled between unrelated emails.

Identity is therefore split across three columns, each with exactly one job:

| Column | Job |
|---|---|
| `id` | Primary key + the opaque handle passed to the frontend. Format: `"<label>/<message_id>"` |
| `dir_name` | On-disk folder, the only thing that can rebuild the path to the body file |
| `message_id` | Full Gmail message ID, resolved front matter `id:` → body filename stem → folder name |

Consequences worth knowing before touching `scan.rs` or `emails.rs`:

- **The label prefix is load-bearing.** It is what makes two identically-named folders
  under different labels distinct, and what makes cross-label state bleed structurally
  impossible rather than a guard someone has to remember.
- **Never rebuild a file path from `id`.** `get_email_html` joins `label` + `dir_name` +
  `html_filename` from their own columns.
- **The unit of indexing is one body file, not one folder.** A colliding pair of
  messages can share a folder; each body is indexed separately rather than picking a
  winner (which previously dropped one email entirely).
- **Directory traversal is fully sorted** — labels, folders, and files — so scan output
  is identical across machines and repeated runs. `read_dir` order is not stable.
- Only folders with **no body file at all** are skipped.

### Which files become bodies

Every `.html` is a body. A `.txt` is a body only when the *message* it names has no HTML
body — decided on resolved `message_id`, not on filenames:

- **Folder has no `.html`** → every `.txt` is a body. Some newsletters ship no HTML part
  (400 emails under `Quincy` in the current corpus). These were previously skipped
  outright and were invisible in the app.
- **Folder has `.html`** → a `.txt` is promoted only if its stem is a message ID that no
  HTML body resolved to. Nearly every `.txt` is just the plain-text alternative of the
  `.html` beside it (verified: zero exceptions across 16,606 folders), so promoting them
  unconditionally would double every email in the archive. The narrow rule exists for a
  colliding pair where one member arrived without an HTML part.

`get_email_html` wraps a `text` body in a minimal HTML document (escaped, inside a
`<pre>`) before it reaches the viewer, because the result goes straight into an iframe
`srcdoc`. The wrapper sets no colours — the viewer injects `color-scheme`, which makes the
UA's default text colour follow the theme on its own.

Legacy databases are migrated in place on open (`db.rs::migrate`): old rows get
`dir_name = id`, then `id = label || '/' || id`, so existing bookmarks survive the
upgrade. The rewrite is idempotent.

## Frontend Communication

All frontend→backend communication goes through `tauri-bridge.js`:

| JS Call | Rust Command | Purpose |
|---------|-------------|---------|
| `Bridge.getLabels()` | `get_labels` | Sidebar label list |
| `Bridge.getEmailsByLabel(label, ...)` | `get_emails_by_label` | Paginated email list |
| `Bridge.getEmailHtml(id)` | `get_email_html` | Read HTML file for iframe |
| `Bridge.searchEmails(query)` | `search_emails` | FTS5 full-text search |
| `Bridge.toggleBookmark(id)` | `toggle_bookmark` | Toggle bookmark state |
| `Bridge.markRead(id)` | `mark_read` | Auto-mark on view |
| `Bridge.scanAndIndex()` | `scan_and_index` | Re-index from disk |

## Frontend Event System

Components communicate via custom DOM events:

- `nav:change` — sidebar label selection → triggers email list reload
- `email:select` — email list click → triggers viewer display
- `search:navigate` — search result click → navigates to label + email
- `app:settings` — settings button → triggers folder picker
- `pane:resize` — splitter drag/collapse → triggers virtual scroll recalculation
