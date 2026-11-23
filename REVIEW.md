# Sage: review findings (2026-11-19)

These findings come from reading the code. Three reviewers each took one area: `ui`; `core`/`editor`/`indexer`/`db`/`watcher`/`app`; and `query`/`markdown`. Items marked ✔ were reproduced with a throwaway probe crate or checked by hand against the code. Effort: S < ½ day, M = 1–3 days, L = more.

Baseline: `cargo clippy --workspace --all-targets` shows only the 2 known warnings (`editor/rename.rs:114`, `ui/lib.rs:164`). The query and markdown tests pass.

## Fix first

**All ten fixed on 2026-11-19.** See PLAN.md §8 ("Fixes after the review") and §4 (schema version 7).

| # | Finding | Where | Effort |
|---|---|---|---|
| 1 | ✔ **Closing the window can lose edits.** Nothing handles `CloseRequested`, and iced exits at once. Text typed in the last 1.5 s before autosave is lost, as are tabs whose save failed or is in conflict. Fix: set `exit_on_close_request: false`, save everything (as `vault.rs` does before a switch), then exit. | `ui/lib.rs:115`, autosave `:66` | M |
| 2 | ✔ **Closing a tab to the left of the active one switches notes.** `close_tab` clamps `active` but never decrements it. Example: with tabs A, B, C and B active, closing A shows C. `Trashed` and a failed first read have the same pattern. | `ui/lib.rs:2551`, `:1279`, `:2100` | S |
| 3 | ✔ **A failed folder walk empties the index.** `filter_map(\|e\| e.ok())` drops read errors, so every note under a folder that could not be read for a moment counts as deleted. This can happen with a USB drive, network share or 9p hiccup. | `indexer/lib.rs:225`, `:244` | S |
| 4 | ✔ **If the watcher can't start, there is no index.** `watch(...)?` aborts `spawn`. Causes include inotify `max_user_watches` and some virtual drives. Fix: fall back to `Backend::Poll`. | `indexer/lib.rs:95` | S |
| 5 | **The indexer's `known` cache is updated before commit.** If a statement fails, the transaction rolls back, but the cache still thinks the rows were written, so those notes stay stale until they change again. | `indexer/lib.rs:346–476` | S |
| 6 | **A panic on Windows is silent.** The build uses `windows_subsystem = "windows"`, and there is no panic hook and no log anywhere. Fix: add a panic hook that writes `<data>/crash.log` and shows an `rfd` message box, plus a small log file. | `app/main.rs` | S |
| 7 | **`session.json` is written non-atomically on the UI thread, and errors are ignored.** A crash mid-write truncates it, and `load_session` then quietly falls back to defaults, losing tabs, folds and pane sizes. `order.rs` has the same pattern. | `ui/lib.rs:2589`, `order.rs:36` | S |
| 8 | ✔ **A `---` block in the middle of a note hides its content in the preview.** `ENABLE_YAML_STYLE_METADATA_BLOCKS` accepts metadata blocks anywhere in the note, not just at the top. | `markdown/tree.rs:22`, `:142` | S |
| 9 | **Rename rewrites `[[links]]` inside code samples.** Fence detection is loose: a 4-backtick fence is closed by an inner ```` ``` ````, a closing line with an info string still closes, and fences inside `>` quotes are missed. | `core/lib.rs:184–197` | S |
| 10 | ✔ **`[[Note\|alias]]` in tables gets the target `Note\`.** The link never resolves, and renaming the note silently skips it, breaking the table link. | `core/lib.rs:104`, `:412`, `markdown/tree.rs:528` | S |

## Bugs: data and index

- ✔ Renaming to a name with `(` or `)` writes `[a](Meeting%20(2026).md)`, which `MD_LINK` can't parse, so the link leaves the graph. `rename.rs:337`. S
- ✔ A rename detected by hash keeps the old `title` and FTS title. `indexer/lib.rs:415`. S
- ✔ `# Learn C#` is indexed as "Learn C": a closing `#` is stripped even without a space before it. `core/lib.rs:118`. S
- ✔ Heading text loses inline code, because it is taken from the masked line. `core/lib.rs:199`. S
- ✔ Double-backtick code spans are masked wrongly, which drops tags after them. `core/lib.rs:722`. S
- ✔ `merge3` adds a final newline when theirs deletes the last line of a file that had none. This breaks rule 10. Fuzzed 300k cases with no panics. S
- `sage .` writes `"."` to `last-vault`, and the next start then opens the working directory. Fix: canonicalise the path and check it exists before writing it. `app/main.rs:31`. S
- Symlinked notes are never indexed, and a save replaces the link with a plain file. `indexer:222`, `editor/lib.rs:86`. M
- The temp name `.{name}.sage-tmp` is fixed. Two overlapping writes to one note can collide, for example a tab save and a task write-back. Also, a rename over the file drops its permissions, and in-place writes never `sync_all`. `editor/lib.rs:79`. S
- `Step::Move` in rename has no Windows retry loop, so an antivirus handle forces a full rollback. `rename.rs:398`. S
- A corrupt `index.db` fails every start; it should be deleted and rebuilt, since the index is a cache. Nothing stops two Sage processes on one vault, which would mean two writers. Fix: a lock file. `db/lib.rs:32`. S–M
- `schema.sql:1` says "version 5"; `SCHEMA_VERSION` is 6. S

## Bugs: UI

**Fixed on 2026-11-19**, with the query and preview group below. See PLAN.md §8.

- After a rename, Back/Forward, recent notes and the closed-tab list keep the old path. `ui/lib.rs:2893`. S
- Several errors are set with `self.status =` instead of `notice()`, so the next indexer progress message wipes them almost at once. These are save failure, create, trash, "Cannot open", template and index errors (`:2164`, `:1275`, `:1286`, `:2102`, `:1551`, `:1953`). S
- A template insert, an attachment link or a task link can arrive while a merge is pending. The merge result then overwrites it. `on_attached :2527`, `insert_template :3191`, `task_form.rs:138`. The task link is also not its own undo step. S
- Esc in the palette does not give focus back to the editor. `ui/lib.rs:1729`. S
- Line endings are inconsistent: `insert_task_line` and the template frontmatter hard-code `\n` on CRLF notes. Check what iced's `Content` keeps before fixing. `task_form.rs:32`, `ui/lib.rs:3207`. S
- The live preview find highlight uses alpha 0.35, while `theme::highlight` uses 0.28. Its colours are copied into `live_editor.rs:694,707` instead of coming from a `theme.rs` function, which breaks rule 8. S

## Bugs: query and preview

**Fixed on 2026-11-19**, apart from what PLAN.md §8 lists as unchanged (clicking a `[/]` box; accented letters sorting after `z`).

- ✔ `[due<today]` compares against the literal text "today", because `property()` never calls `day()`. `dsl.rs:406`. S
- ✔ Case-insensitive matching only works for ASCII. `folder = Środa` in a `tasks` block never matches because Rust lowercases the value but SQLite lowercases the path. Fix: compare against keys lowercased in Rust. `sql.rs:92,96,116,137`, `documents.rs:339`. M
- ✔ Kanban boards apply `limit` before grouping and sort done tasks first, so old done cards push out the open ones. `view.rs:418,361`. S–M
- ✔ `v_documents.status` is slugged in SQL, while columns use `sage_core::status_slug`. `To_Do` → `to-do` vs `to_do`, so the card falls into the first column. Fix: store the slug from the indexer and bump the schema. `schema.sql:212`. S–M
- ✔ Quoted numeric properties (`zip: "01234"`) can't be matched as text. `sql.rs:109`. S
- ✔ Sorting by a text property is case-sensitive. `view.rs:294`. S
- ✔ Task states `[/]`, `[>]` and `[-]` are indexed but show no checkbox in the preview. `tree.rs:192`. S–M
- Views with `today` go stale after midnight until the next index event. A timer at midnight that clears cached views would fix it. S
- Only lowercase `none` is accepted (`dsl.rs:357`). Cancelled tasks can never be listed, because `state <> '-'` is hard-coded (`view.rs:384`). S

Safety: **no SQL injection holes found.** All values are parameters, matching uses `instr` or escaped GLOB, and FTS input is quoted. One hardening note: `view.rs:258` relies on the order of anonymous `?` parameters; use `?N` as `documents.rs` does.

## Performance (matters as vaults grow; see PLAN §8 #6)

**Fixed on 2026-11-19, with measurements**: see PLAN.md §8 ("Performance after the review"). Parsing on each key in Split and Live modes was then debounced as well. Still open: `reload_files` after every index change.

1. **Ctrl+Shift+F builds the search corpus from the whole vault on the UI thread.** It also allocates a `String` per word per keystroke. `ui/lib.rs:1851`, `omni.rs:79,229`. M
2. **On Windows, every save in a subfolder triggers a full vault walk.** The directory's modify event goes to `reconcile()`. `indexer:286`. M
3. **Every index event (after every autosave) reloads everything on the UI thread:** all of `v_notes`, the links for every tab, and every view and `tasks` block in hidden tabs too. Fix: refresh the active tab only, and others when they are selected. `ui/lib.rs:1914`, `:2639`. M
4. **Every keystroke processes the whole note several times:** a full text copy for undo even when the step joins the current typing group, then the markdown parse, islands, `core::parse` and counts. Undo keeps 200 full copies. `ui/lib.rs:862`, `:804`, `undo.rs:40`. M
5. **The tree is rebuilt every frame, sorted with `to_lowercase` inside the comparator.** Fix: cache it on change and use `sort_by_cached_key`. `tree.rs:60,263`. M
6. **Live preview lays out the whole note on every layout pass.** `live_editor.rs:289`. M
7. **Notes have no size cap**, so a 200 MB `.md` stalls the indexer and is stored twice. Files with a future mtime are re-read on every pass. `indexer:340,381`. S
8. **Tag subquery runs per task row** without an index on `tags(file_id, line)`. `documents.rs:293`. S

## Quality-of-life ideas (checked as missing)

High value, low effort:
- **Follow the link under the cursor in Source** (Ctrl+click / Alt+Enter). Only Live and Preview can follow links today. S
- **Fuzzy Ctrl+P note search.** It is a plain substring match with at most 6 hits today; `omni::subsequence_score` already exists. S
- **"New note here"** in the folder menu, and use Obsidian's `newFileLocation`. New notes always go to the root today. S
- **Ctrl+W saves then closes** instead of refusing with "Save first". S
- **Tab tooltip or path suffix** when two tabs share a title. S
- **Open attachments (PDF etc.) with the system app** instead of "Sage opens notes only". S
- **Readable line width** in Source, plus optional line numbers. S

Medium:
- **Open in the current tab by default**, Ctrl+click for a new tab, and "Close other tabs". Every open makes a new tab today. M
- **Find and replace**, with case, whole-word and regex options. M
- **`[[Note#heading` and `[[#` completion.** The `headings` table already exists. M
- **Backlinks in Source and Live modes**, plus unlinked mentions. M
- **Tree:** keyboard navigation, a filter box, drag-and-drop to move. M–L
- **Preview gaps:** footnotes (enable the option; today `[^1]:` becomes a bogus link), `%% %%` and `<!-- -->` comments hidden, `^id` block ids hidden, math shown as code. The indexer should also ignore links and tags inside comments. S each
- **Query language:** relative dates (`-7d`, `this week`), several sort keys in `view` blocks, `group_by` for tables and lists, created/cancelled emoji dates. S–M each

Larger: compatibility with Obsidian Tasks plugin syntax inside ```` ```tasks ```` (existing vaults' blocks fail today), OneDrive detection by environment variables and attributes for SharePoint and Teams libraries, a trigram FTS table for CJK and mid-word search, bookmarks, a tag pane, a graph view and spellcheck.

Ranking of the already-known deferred items: the caret-anchored completion popup ranks first, and undo history lost on tab close second (it gets worse once tabs are closed more often).

## Maintainability

- **Split `ui/lib.rs` (5,156 lines)** using the existing `impl App` in other modules pattern (`vault.rs`, `task_form.rs`). In order: `palette.rs` (~550 lines, and its hit builders are testable), `keys.rs` (Command, COMMANDS, BUILT_IN, chords), `resolve.rs` (the merge flow and panel), `tab.rs`, the session, and the platform helpers.
- **Add a `blocking(f, Message::X)` helper.** The `Task::perform(spawn_blocking…)` boilerplate appears 19 times.
- **Add a `Tab::insert_at_cursor`** (checkpoint, step past the frontmatter, paste, `mark_edited`). It replaces 3 copies and fixes the undo and merge bugs above.
- **Add an `open_palette(mode)` and a `close_overlays()`.** About 8 copies today, and they close overlays inconsistently.
- **Process:** the folder is not a git repository, and there is no CI. `git init` plus a GitHub Actions matrix (Windows + Linux: build, test, clippy) would protect the "warning-free on both platforms" rule, which is currently checked by hand.
- `examples/sample-vault/asdf.md` and `asdfg.md` look like scratch notes left behind by testing.
