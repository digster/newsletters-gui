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

## Not every newsletter has an HTML part

400 emails (all under `Quincy`) ship only a `.txt` and a `.md`. The scanner skipped them
because the column was called `html_filename` and could only ever name an `.html` — the
schema encoded an assumption nobody had checked. They are now indexed as
`body_format = 'text'`.

The subtle half is deciding *which* `.txt` files count. Almost every `.txt` in the archive
is the plain-text alternative of the `.html` beside it — indexing those would have doubled
every email. The rule has to be identity-based, not filename-based: a `.txt` becomes a body
only when the message it names has no HTML body, resolved through the same `message_id`
chain as everything else. Filename-based rules ("prefer html, else txt" applied per file)
either double-count or drop the HTML-less member of a colliding pair.

Measured before choosing the rule: across 16,606 folders with HTML, **zero** had a `.txt`
whose stem didn't match an `.html`. So the promotion path is a guard against a known bug
class, not a live case — worth having, but don't mistake it for common.

**Rule:** keep `body_filename` NOT NULL. A nullable column would have been the smaller
diff, but SQLite treats NULLs as distinct in a unique index, so `idx_emails_identity`
would have silently stopped enforcing one-row-per-body.

## Escape plain text in Rust, not in the frontend

`get_email_html` output goes straight into an iframe `srcdoc`. A `.txt` body handed over
raw would be parsed as markup, so it is escaped and wrapped in `<pre>` before it leaves
Rust. The wrapper sets no colours: the viewer injects `color-scheme` into the document,
and the UA's default text colour follows it in both themes for free. Setting an explicit
colour there would have needed a second theme code path.

## FTS5 external-content tables can't be DELETEd

With `content='emails'`, `DELETE FROM emails_fts` misbehaves; drop and recreate the table
instead (`db::reset_index_tables`). Schema lives in `db.rs` constants so the migration,
the re-index path, and the tests cannot drift apart.
