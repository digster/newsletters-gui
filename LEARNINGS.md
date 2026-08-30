# Learnings

Patterns and pitfalls discovered while working on this codebase. Read before touching
identifier handling, the scanner, or the SQLite schema.

## A filesystem name is never a primary key

`emails.id` was the email folder name. Folder names came from `message_id[:8]`, and
truncated Gmail IDs collide — gmail-ingestor's `LEARNINGS.md` measured 6 colliding groups
over 17,007 live messages, ~180× the birthday-bound prediction, because Gmail IDs encode a
delivery timestamp and newsletters arrive in bursts. The same 8-char name also appears
legitimately under multiple labels.

Three separate failures followed from that one assumption, and only the first is obvious:

1. `INSERT OR REPLACE` overwrote colliding rows — one newsletter's body published under
   another's headline.
2. The re-index state restore was keyed on the same value, so **read/bookmark state bled
   between unrelated emails** in different labels.
3. The FTS row was built via `SELECT rowid FROM emails WHERE id = ?` *after* the insert,
   which under collisions could resolve to a different row than the one just written,
   mismatching search snippets against headlines. Use `conn.last_insert_rowid()`.

**Rule:** keep identity (`id`), location (`dir_name`), and provenance (`message_id`) in
separate columns. The moment one value has to be both "globally unique row identity" and
"whatever the folder is called", the corruption comes back. `id` is `"<label>/<message_id>"`,
and the label prefix is load-bearing: it makes cross-label bleed *structurally* impossible
instead of something a future guard has to catch.

Note that fixing the data upstream is not a fix here. Full 16-char IDs make folder names
unique again, but the app must stay correct against an archive that hasn't been rebuilt.

## `read_dir` order is not an ordering

`scan.rs` picked the first `*.html` it encountered, so which email won varied by machine
and by rescan. Worse, it *discarded* the rest — and a colliding pair of messages can end
up in one folder, which is exactly how one email went missing entirely.

**Rule:** sort at every level (labels, folders, files), and index one row per `.html`
file rather than one per folder. Determinism here is not cosmetic — row insertion order
sets rowids, which feed FTS5 `rank` tie-breaks.

## Quoted vs unquoted IDs in YAML front matter

`Frontmatter.id` must go through the same lenient scalar deserializer as `date`, not a
plain `String`. ~30 live Gmail IDs are all digits (e.g. `1637675546614607`); unquoted,
`serde_yaml` yields an integer and a strict `String` field fails — silently dropping the
ID for exactly those messages and falling back to a weaker key. Covered by
`numeric_unquoted_front_matter_id_is_still_read`.

## The database is a rebuildable cache — except for user state

Everything in `emails` can be regenerated from disk except `is_read` / `is_bookmarked`.
That is why `migrate()` promotes legacy ids in place (`dir_name = id`, then
`id = label || '/' || id`) instead of dropping the table, and why `ExistingState` also
keeps a `(label, dir_name, html_filename)` fallback map: it restores state when the *way*
a uid is derived changes (front matter gaining an `id:` field) while the files stay put.
Both lookup paths are label-scoped, so neither can bleed.

## 400 emails have no `.html` at all

Plain-text-only newsletters (`.txt` + `.md`, no HTML body) exist in the corpus and have
always been skipped by the scanner — `html_filename` is `NOT NULL` and the viewer renders
HTML into an iframe. This is pre-existing, not a scan regression. Supporting them means a
nullable `html_filename` plus a text-rendering path in the viewer.

## FTS5 external-content tables can't be DELETEd

With `content='emails'`, `DELETE FROM emails_fts` misbehaves; drop and recreate the table
instead (`db::reset_index_tables`). Schema lives in `db.rs` constants so the migration,
the re-index path, and the tests cannot drift apart.
