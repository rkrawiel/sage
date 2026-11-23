# Sage: implementation plan

A native Rust second-brain app for Linux, WSL and Windows that works directly on an existing Obsidian vault. "Sage" is a working name taken from the design system; rename the crates freely.

## 1. Approved decisions

| Area | Decision |
|---|---|
| Platforms | One codebase, two native builds: Linux (also under WSL) and Windows. Everything is shared but a small platform layer: change detection, canonical paths, the data folder, OneDrive placeholders, saving, opening links, and how the app starts |
| Vault location | ext4 and `/mnt/c` both supported, backend chosen per vault via `statfs`; on Windows any local drive, network drives polled |
| Change detection | Native filesystems: inotify, or Windows change notifications (`notify`), plus a 10 minute safety rescan. 9p, SMB, NFS, FUSE, Windows network drives: mtime + size scan every 45 s |
| Index | SQLite + FTS5 (`rusqlite`, bundled), always under `$XDG_DATA_HOME/sage/<vault-hash>/`, or `%LOCALAPPDATA%\sage\<vault-hash>\` on Windows, disposable and rebuildable |
| Editor | Source pane (monospace, highlighting, autocomplete) plus preview pane. Modes per tab: source, split, preview, and live preview (step 12) |
| Conflicts | Three-way merge against a per-tab base snapshot, prompt only on overlapping hunks |
| Queries | Custom DSL in Obsidian search style, parsed to an AST, compiled to parameterized SQL over stable views. No raw SQL |
| Kanban | Cards are task lines. Column stored as `#status/<slug>`, last column equals `[x]`. The order of cards placed by hand is kept next to the index, never in the vault |
| Task metadata | Tasks emoji format and Dataview inline fields are both parsed, the emoji format is written |
| Rename | In-app renames rewrite all links automatically as one undo step. External renames only reindex |
| Templates | Obsidian core syntax: `{{title}}`, `{{date:FORMAT}}`, `{{time:FORMAT}}` |
| OneDrive | Path based, no Graph API. Root detection, placeholder skipping (files over 4 KB with no allocated blocks, see §8), conflict-copy merge |
| Fonts | Bundled in the binary: Source Serif 4 (headings), Source Sans 3 with its italics (body, UI), JetBrains Mono (editor), Noto Emoji (monochrome) |

One change from what we discussed: Source Sans 3 replaces Inter. Inter's repository ships no static TTF files, and Source Sans is the designed companion of Source Serif, so the pairing is more consistent.

## 2. Workspace layout

```
sage/
├── Cargo.toml                 workspace, shared versions, release profile
├── PLAN.md
├── .gitattributes             forces LF endings, also when edited from Windows
├── scripts/fetch-fonts.sh     downloads the OFL fonts into assets/fonts
├── assets/fonts/              embedded with include_bytes!
├── examples/sample-vault/     five small notes for trying the app
└── crates/
    ├── core/                  domain types, scanner, link rules         [done, tested]
    ├── db/                    schema.sql, connection setup, versioning  [done]
    ├── watcher/               backend detection, notify wrapper, the    [done]
    │                          paths that differ between Linux and Windows
    ├── indexer/               the notify + mtime sync loop              [done, tested]
    ├── editor/                guarded save, autocomplete, find,         [done, tested]
    │                          rename of notes and folders, task
    │                          write-back, recurring tasks, templates,
    │                          three-way merge                           [done, tested]
    ├── query/                 view-block DSL, SQL compiler, views       [done, tested]
    ├── markdown/              preview block tree (pulldown-cmark)       [done, tested]
    ├── onedrive/              root detection, placeholders, conflicts   [done, tested]
    ├── ui/                    Iced shell, theme, preview, merge panel,  [step 6 done]
    │                          Kanban order
    └── app/                   the `sage` binary
```

Dependency direction is strictly downward: `app → ui → {indexer, editor, query, markdown} → {db, watcher, core, onedrive}`. `onedrive` depends on nothing, so it sits at the bottom, where the indexer can use it to skip placeholders. Nothing below `ui` knows Iced exists, so every layer under it is testable headless.

## 3. Runtime architecture

```
            ┌────────────────────── UI thread (Iced) ──────────────────────┐
            │  state, view, short read-only SQL (sidebar, palette, links)  │
            └───────▲──────────────────────────────────────┬───────────────┘
      IndexEvent    │ Subscription                         │ read-only connection
                    │                                      ▼
            ┌───────┴────────── indexer thread ──────┐   ┌──────────────┐
  inotify → │ debounce → apply hints                 │ → │ SQLite (WAL) │
  timer   → │ reconcile: stat walk, hash, reparse    │   │  on ext4     │
            └────────────────────────────────────────┘   └──────────────┘
```

- One writer. The indexer thread owns the only read-write connection, so there is no write contention to reason about.
- WAL mode means the UI's reads never wait for the indexer, including during the first full index.
- The indexer is started by an Iced subscription and stops when the subscription is dropped.

### Startup sequence

1. Open the read-only connection and draw the sidebar from the cached `files` table. No file in the vault has been touched yet.
2. The indexer loads `path → (id, mtime, size)` into memory and emits `Started`.
3. First reconcile: one `stat` per file. Unchanged files are skipped without being opened.
4. Changed files are read, hashed, and reparsed only if the hash differs.

## 4. SQLite schema

Full DDL with comments is in `crates/db/src/schema.sql`. Summary:

| Table | Purpose | Notable columns |
|---|---|---|
| `files` | Every file, notes and attachments | `path`, `link_key`, `name_key`, `mtime_ns`, `size`, `hash`, `cloud` |
| `texts` | Each note's text as last read, which its other rows came from | `text` |
| `frontmatter` | Flattened YAML, one row per scalar | `key`, `ord`, `value`, `value_num` |
| `aliases` | Frontmatter aliases for link autocomplete | `alias_lc` |
| `headings` | For `[[Note#Heading]]` and the outline | `line`, `level`, `text` |
| `tags` | Inline and frontmatter tags, lowercase | `tag`, `line` |
| `links` | Forward links and backlinks in one table | `src_file_id`, `dst_file_id`, `target_key`, `line`, `col`, `len` |
| `tasks` | Checkbox lines with parsed metadata | `state`, `status`, `due`, `priority`, `raw` |
| `fts` | FTS5 over title, body, tags, path | `prefix = '2 3'` for search-as-you-type |
| `v_notes`, `v_tasks`, `v_tags`, `v_properties`, `v_backlinks` | Stable read surface for the DSL and the UI | `v_notes.name`, `v_notes.modified` |

Design points worth knowing:

- **The index is a cache.** A `SCHEMA_VERSION` mismatch deletes and rebuilds it. There are no migrations to write, and no user data can be lost with it.
- **Link resolution follows Obsidian.** `[[Note]]` matches `name_key` anywhere in the vault, `[[folder/Note]]` must match the end of `link_key`, shortest path wins. Unresolved links keep `dst_file_id = NULL` and resolve automatically when the target appears. Deleting a target sets it back to `NULL` through the foreign key. `sage_core::resolve` is the same rule for code outside SQL.
- **Links are wikilinks, Markdown links and property links.** `[text](Note.md)` counts (percent-decoded, URLs skipped), and so does a `[[link]]` inside a frontmatter property. Targets starting with `./` or `../` resolve against the linking note's folder. Schema version 2 added these; the index rebuilds once.
- **Renames keep the row.** The indexer recognises delete + create with an identical hash as a rename and updates `path` in place, so the file id and all backlinks survive.
- **Task write-back is guarded.** `tasks.raw` stores the full source line. A Kanban drag or checkbox click rewrites the line only if the line on disk still equals `raw`.
- **View blocks read the `v_` views and `fts` only.** Schema version 3 added `v_tags`, `v_properties`, and `name` and `modified` on `v_notes`. `modified` falls back to the indexing time while a racy mtime is stored as 0, so a note saved seconds ago does not show 1970.
- **In-app renames re-read the linking notes.** The index says which notes link to the renamed one, but it can trail the disk by a polling interval on 9p, so each of those notes is re-parsed and patched at fresh positions. `links.col` and `links.len` are never trusted for writing.
- **Nested tags** are queried as `tag = 'a/b' OR tag GLOB 'a/b/*'`, both served by one index.
- **Schema version 4** added `files.cloud` (a OneDrive placeholder), `v_notes.cloud`, and `texts`. Task lines inside quotes and callouts (`> - [ ] …`) became tasks in the same version. `texts` roughly doubles the size of the index, which stays a few tens of MB at 10,000 notes; it is the base a write to a closed note merges against (see step 6).
- **Schema version 5** added `files.id_key` and `v_documents`. A bare `[[name]]` that no file name matches finds the note whose frontmatter `id` it is; a note whose `id` changes re-opens the links written as its old and new id, as a rename does for names. `v_documents` lists typed documents (notes with a frontmatter `type`) with `doc_id`, `status` (lowercase, words joined by `-`), `created`, `due` and the frontmatter tags.
- **A detected rename re-resolves relative links.** A note moved unchanged keeps its row, but its `./` and `../` links now start from another folder, so their keys are recomputed. Before version 4 they kept pointing where they pointed from the old folder.
- **Schema version 9** made `tags_file` an index on `(file_id, line)`, so a task line's tags in a `tasks` block are one seek.
- **Schema version 8** added `files.status_key`, which `v_documents.status` reads. The indexer stores a document's `status` with `sage_core::status_slug`, as Kanban columns are named, with `_` read as `-` as `TaskStatus::parse` reads it. SQL did this before, and it differed from the columns: `In  Progress` gave `in--progress`, and capitals beyond ASCII were kept. Every connection also replaces SQLite's ASCII-only `lower()` and `NOCASE` with Unicode case folding. No index uses `NOCASE`. Letters still sort by code point, so `Ł` sorts after `z`.
- **Schema version 7** changed which rows the scanner produces, without changing tables. `[[Note\|alias]]`, as Obsidian writes a link inside a table, links to `Note`. Before, its target was `Note\`, so it never resolved and renames skipped it. Code fences are found as CommonMark finds them: the same character, at least as many, and no info string on the closing line; this also applies inside quotes and callouts, where the block ends with the quote. The scanner and live preview share `sage_core::fence_open` and `fence_closes`. Links, tags and tasks in code samples are no longer indexed, and renames no longer rewrite them.

## 5. The sync loop

Code: `crates/indexer/src/lib.rs`, with `crates/watcher/src/lib.rs` for backend selection.

```
loop:
    wait for: inotify batch | rescan request | timeout(backend interval)
    inotify batch   → debounce 150 ms → apply_hints(paths)
    rescan, timeout → reconcile()
```

- `reconcile()` walks the vault (hidden entries such as `.obsidian` and `.trash` are skipped), compares `(mtime_ns, size)` with the in-memory map, and passes the differences to `apply()`.
- `apply_hints()` stats only the hinted paths. Anything it cannot classify, for example a directory move, falls back to `reconcile()`.
- `apply()` reads, hashes and reparses in transactions of 200 files, then deletes missing rows and re-resolves links.

Edge cases handled:

| Case | Handling |
|---|---|
| File touched, content identical | Hash matches, only the stamp is updated |
| File changes twice within one timestamp tick | mtime newer than 2 s is stored as 0, forcing a hash check on the next pass |
| Rename or move outside the app | Detected by hash, row updated in place |
| inotify queue overflow | Watcher sends an empty batch, which triggers a full reconcile |
| File deleted between `stat` and `read` | Skipped, removed on the next pass |
| Invalid YAML frontmatter | Body is still indexed, frontmatter rows are omitted |
| Non-UTF-8 content | Indexed lossily, never rewritten by the indexer |

Polling cost on `/mnt/c` at 2,000 to 10,000 notes: one `stat` per file per pass. Expect low single-digit seconds per pass over 9p, off the UI thread. `IndexerHandle::rescan()` exists so the UI can trigger a pass on window focus. Open tabs get their own 2 s stat check in step 5.

## 6. Iced shell and the Sage & Paper theme

Code: `crates/ui/src/{lib.rs, theme.rs, highlight.rs, tree.rs, suggest.rs}`. Pinned to iced 0.14.

### Theme

- `theme::Tokens` holds the ten colour tokens for light and dark as constants. Every widget style is a function of `Tokens`, and nothing reads iced's generated palette, which keeps blue, purple, gradients and shadows out by construction.
- `RADIUS = 3.0` is the only radius. Dividers are 1 px containers in the `border` token.
- The theme follows the OS through `iced::system::theme()` and `theme_changes()`. `Ctrl+Shift+L` overrides it manually, which is the fallback for WSLg sessions without a settings portal.

### Behaviour (build step 2)

| Feature | Behaviour |
|---|---|
| Folder tree | Collapsible, folders first, case-insensitive order. Folders start collapsed so large vaults render only what is open. Opening a note expands its ancestors. |
| Names | Tree, tabs and palette show file names, as Obsidian does, because links resolve by file name. The palette adds the H1 or frontmatter title as a hint when it differs. |
| File IO | Reads are async. Saves, creation and trashing run on the blocking pool. The UI thread never touches the disk. |
| Autosave | 1.5 s after the last keystroke. `Ctrl+S` saves immediately. |
| Conflict guard | A save is refused when the file on disk no longer matches the text the tab was loaded from. The tab shows `!` in the error colour and autosave pauses. "Overwrite file on disk" in the palette keeps the local version. The three-way merge in step 5 replaces this manual choice. |
| External changes | An open tab without local edits reloads silently. With local edits it goes into the conflict state above. |
| Autocomplete | `[[` notes, attachments and aliases. `![[` embeds. `#` tags, prefix matches first, then substring, most used first. `](` paths with `%20` encoding. Inserted link text follows Obsidian's "shortest unique" rule. Suggestions open only while typing; moving the cursor closes them. |
| New notes | `Ctrl+N` creates `Untitled.md` (then `Untitled 1.md`, ...) in the vault root. Typing a name in the palette offers "Create note"; `/` creates folders, characters Windows rejects are dropped. |
| Delete | "Move note to .trash" moves the file into the vault's `.trash` folder, as Obsidian does. Recoverable by moving it back. Refused while the note has unsaved edits. |
| Closing | A tab with unsaved edits does not close; autosave finishes within seconds. |
| Session | Open tabs, active tab, view mode, theme override and expanded folders are restored on start. Stored in `session.json` next to the index, never in the vault. |

The completion popup is anchored to the bottom of the source pane rather than at the caret. iced 0.14 does not expose the editor's scroll offset, so a caret-anchored popup would drift once the note scrolls. Revisit if a later iced release exposes it.

### Behaviour (build step 3)

| Feature | Behaviour |
|---|---|
| Rename and move | `F2` opens the palette on the note's path with the name selected. A folder path moves the note; missing folders are created. Every link to it is rewritten in the style it was written in: a bare name stays bare while it is unique, a path stays a path, a relative Markdown link stays relative, `.md` and `%20` are kept as written. A link elsewhere that the new name would capture is pinned to its own target with a full path. The renamed note's relative links follow it. |
| Undo rename | "Undo last rename" in the palette moves the note back and restores every rewritten note exactly, as one step, newest rename first. Refused, with nothing changed, if any of those notes changed since. Kept for the session. |
| Write safety | Notes with unsaved edits are never rewritten underneath the editor; renames and task toggles ask to save first. Every write is guarded against the file changing since it was read, a failed step reverses the ones before it, and a move never replaces an existing file. An open note that the app rewrote follows along and keeps its cursor. |
| Links under the preview | "Linked from" (backlinks) and "Links to" (outgoing). Unresolved targets are dimmed, and following one creates the note. |
| Preview links | Wikilinks and Markdown links to notes are clickable. `[[Note#Heading]]` and `[[Note#^block]]` open the note with the cursor there. A link to a missing note creates it, as in Obsidian. Attachment links stay plain until step 4 renders embeds. |
| Task list | "Show tasks" in the palette (`Ctrl+T` until step 11). Open tasks of the whole vault under Overdue, Today, Upcoming and No due date, then the ones completed today. Checking writes `[x]` and `✅ <date>` into the note, unchecking removes both, and the line is rewritten only if it still equals the indexed `raw`. Clicking the text opens the note on that line. `[-]` counts as cancelled. |
| Templates | "Insert template" in the palette lists the notes in the folder set in `.obsidian/templates.json` (default `Templates`) and inserts the chosen one at the cursor. `{{title}}`, `{{date}}` and `{{time}}` use the formats from the same file; `{{date:FORMAT}}` and `{{time:FORMAT}}` take Moment.js tokens. |
| Find in note | `Ctrl+F` opens a bar over the source pane, prefilled with a one-line selection. Literal and case-insensitive. The current match is drawn in the warning colour, because iced draws the editor's selection only while the editor has focus. `Esc` closes it and leaves the match selected. Switching tabs closes it. |
| Status | Results the user has to read, like a rename and how to undo it, stay in the status bar for 8 s before index progress replaces them. |

| Shortcut | Action |
|---|---|
| `Ctrl+P` | Palette: notes by name, full-text hits, create. Type `>` for commands |
| `Ctrl+Shift+F` | Search everywhere: fuzzy over note names, titles, aliases, tags, properties, headings and text. Every word must match; typos within one edit (two from nine letters) still match. Enter opens the note at the matching line or heading |
| `Ctrl+N` | New note |
| `Ctrl+S` | Save now |
| `Ctrl+E` | Cycle source, split, preview, live preview (step 12) |
| `Ctrl+Shift+E` | Live preview, or back to the mode before it (step 12) |
| `Ctrl+F` | Find in note. `Enter` next, `Shift+Enter` previous |
| `Ctrl+T` | New task note in `Tasks/`, linked at the cursor (step 11; the task list was here before) |
| `Ctrl+Shift+T` | New task line at the cursor (step 11) |
| `F2` | Rename or move note |
| `Ctrl+W` | Close tab |
| `Ctrl+Shift+L` | Toggle light and dark |
| `↑ ↓ Enter Tab Esc` | Navigate and accept suggestions while the popup is open |
| `Esc` | Closes the palette, else the find bar, else the task list |

### Behaviour (build step 4)

| Feature | Behaviour |
|---|---|
| Preview | `pulldown-cmark` block tree from `sage-markdown`, drawn with the tokens: headings, paragraphs, bold, italic, strikethrough, inline code on the surface colour, `#tags` in the secondary colour, lists, quotes, code, tables with alignment, rules. Wikilinks show as Obsidian shows them (`Note > Heading`, or the alias). Internal links open notes; URLs are shown but not followed yet. |
| Checkboxes | A task checkbox in the preview edits that line in the editor, so it works with unsaved edits too; autosave writes it. Checking adds `✅ <date>`, unchecking removes it. |
| Embeds | `![[pic.png]]` and `![alt](pic.png)` show png, jpg, gif and webp, at the width in `![[pic.png\|300]]` or `![alt\|300](…)`, otherwise at natural size up to the page width. `![[Note]]`, `![[Note#Heading]]` and `![[Note#^block]]` show the note, section or block in a frame with a link to it, one level deep: embeds and views inside an embedded note are not expanded. Files are read off the UI thread and reload when the index reports them changed. Pictures from the web are not fetched. |
| View blocks | A fenced block with the language `view` runs a query against the index and shows a table of notes, a task list, or a Kanban board; see "View blocks" below. Results refresh whenever the index changes. A block with an error shows the error in its place. |
| Task lists in views | The checkbox rewrites the line on disk, guarded by its indexed `raw`, and refused while the note has unsaved edits. Clicking the text opens the note on that line. |
| Kanban | Cards are task lines. A card goes in the last column when it is checked, else in the column whose slug its `#status/` tag names, else in the first. Dragging a card onto another column rewrites its line: into the last column it is checked with `✅ <date>` and loses its status tag; into any other column it gets `#status/<slug>`, placed before the Tasks metadata, and is unchecked. Clicking a card without dragging opens its line. The board shows the move at once, before the index catches up. Cards are ordered by due date, then priority. |

### View blocks

````markdown
```view
type: kanban                      # table (notes), tasks, or kanban
source: "tag:#project/alpha"      # a query; empty or missing means everything
columns: [Backlog, In Progress, Done]
sort: due desc                    # table and tasks only
limit: 50                         # 1 to 1000, default 200
```
````

- **table** lists notes. Columns: `file`, `path`, `folder`, `title`, `modified`, `size`, `tags`, `backlinks`, or any frontmatter key. Default `[file, modified]`. Sorting by any of them; a property sorts by its first value.
- **tasks** lists tasks with checkboxes. Columns: `task`, `file`, `path`, `due`, `scheduled`, `start`, `done`, `priority`, `status`. Default `[task, file, due]`, sorted by due date, then priority.
- **kanban** is a board of the matching tasks. `columns` are the column names, at least two; their `#status/` slugs come from the names, so "In Progress" is `in-progress`. `group_by` may only be `status`.
- Cancelled tasks (`[-]`) appear in none of them. Unknown keys, types and columns are reported in the preview.
- A query with `#` or `:` in it has to be quoted, as in the example, because YAML reads `#` as a comment.

The query language follows Obsidian's search:

| Syntax | Matches |
|---|---|
| `word`, `"a phrase"`, `content:word` | notes whose text contains it; a word also matches as a prefix |
| `file:name`, `path:Projects/` | file name or vault path contains, ignoring case (beyond ASCII too: `file:żółw` finds `Żółw.md`) |
| `tag:#project` | notes with the tag, or one nested under it |
| `[status]`, `[status:active]`, `[estimate>3]`, `[due<today]` | frontmatter properties: present, equal (ignoring case), compared as numbers or as text; `today`, `tomorrow` and `yesterday` stand for their dates; `[zip:01234]` also matches the quoted text `"01234"` |
| `task:text`, `task-todo:text`, `task-done:text`, `task-cancelled:text` | tasks containing text: any, open, done, cancelled; the text may be empty. Lists and boards leave cancelled tasks out unless `task-cancelled:` asks for them |
| `status:in-progress`, `status:none` | tasks by their `#status/` tag |
| `due<today`, `scheduled:2026-12-01`, `start>=tomorrow`, `done:yesterday` | task dates, also `due:none` and `due:any` |
| `priority>=high`, `priority:none` | lowest, low, medium, high, highest, or 1 to 5 |
| `a b`, `a OR b`, `-a`, `( … )` | and, or, not, grouping |

`none` and `any` ignore case. Sorting a table by a property ignores case. Views with date words run again when the date changes, without waiting for the index.

In a task list or board, note filters apply to the task's note. A board puts open tasks before done ones when its `limit` cuts the list short, and the most recently done first among those. A card goes to the column whose slug names its status, with `_` and `-` counted alike. In a notes table, task filters match notes that have at least one task satisfying all of them together. The query is parsed to an AST and compiled to parameterized SQL over the stable views; text from a note never becomes SQL.

### Behaviour (build step 5)

| Feature | Behaviour |
|---|---|
| External changes | Every change to an open note made elsewhere goes through a three-way merge against the tab's base text. Without local edits that is a reload, with the cursor kept on the same text instead of jumping to the top. With local edits, changes to different lines merge silently, neighbouring lines included, and autosave writes the result. A save that finds the file changed re-reads it and merges the same way instead of stopping. |
| Resolve panel | Only sections both sides changed differently wait for the user. The panel replaces the editor, shows each section with two lines of context and the other version beside yours, and offers keep yours, take theirs, or keep both, per section or everywhere. Autosave waits until Apply. "Overwrite file on disk" still keeps your version outright. |
| Polled vaults | On 9p, SMB and NFS the open notes are statted every 2 s, so an edit made in Obsidian or Explorer reaches the tab within seconds instead of at the next 45 s scan. Coming back to the window starts a full index pass. A rename first waits for a fresh pass, so links added elsewhere in the last interval are rewritten too. |
| OneDrive | A vault inside a folder named `OneDrive` or `OneDrive - <organisation>` shows "OneDrive" in the status bar. Cloud-only placeholders over 4 KB are never read by the indexer, so indexing does not download the vault; such a note keeps the rows from its last read, or appears in the tree without content, and is read once OneDrive has it locally. |
| Conflict copies | A note named like a copy OneDrive makes, `<note>-<COMPUTERNAME>.md` next to `<note>.md`, shows a banner on the original. "Compare and merge" opens the resolve panel with every differing section; Apply puts the result into the note, which autosave writes, and moves the copy to `.trash`. A copy identical to the note goes straight to `.trash`. |
| Names | New and renamed notes avoid the names Windows keeps for devices (`CON`, `AUX`, `NUL`, `COM1`, ...), which OneDrive cannot sync: `con` becomes `con_`. |

### Behaviour (build step 6)

| Feature | Behaviour |
|---|---|
| Callouts | `> [!kind]± Title` shows as a panel with the title in the kind's colour: accent for note, info and the rest, success for tip and done, warning for question and warning, error for bug and danger, secondary for quote and example. `-` starts folded, `+` starts open, and the title opens and closes it (see step 7 for titles and remembered folding). Tasks inside callouts and quotes are tasks everywhere: indexed, in views, clickable. |
| Highlights | `==text==` on the warning colour, washed out, also across emphasis and links. A mark without a partner stays text. |
| Properties | The frontmatter shows at the top of the preview, key beside values. Wikilinks in values are links, `tags` are tags, lists are joined with commas. Embedded notes show without it. |
| URLs | `http`, `https` and `mailto` links open with `xdg-open`, or under WSL with `wslview`, falling back to `explorer.exe`. Other schemes are refused, since they could start any program. |
| Embeds | SVG files show like pictures, at their own size or the `\|width` given. Embedded notes show their own embeds, three notes deep; deeper ones are a line with their name, and a note that embeds one above it says so instead of repeating. |
| Recurring tasks | Checking a task with `🔁 every …` (or `[repeat:: every …]`) inserts the next occurrence above it, as the Tasks plugin does, from the preview, the task list, views and the Kanban Done column. Its dates move by the same number of days as the due date (else scheduled, else start) moves to its next date, or from today with `when done`; completion date and block id are dropped. Rules: `every [N] day(s)/week(s)/month(s)/year(s)`, `every other …`, `every weekday`, `every Monday[, Friday]`, `every week on …`, `every month on the 15th/last`. Anything else just checks the task. |
| Folder rename | Right-click a folder in the tree, or "Rename or move this note's folder" in the palette. Everything in it moves in one rename, hidden files included, and links into and out of it are rewritten by the note rules. A bare link stays bare while it still finds its note: moving a folder changes path lengths, so a bare link the move would send to another note of the same name gets a path. One undo step, like a note rename. |
| Templates | A template's properties go into the note's frontmatter, never at the cursor: keys the note lacks are added as written, list items it lacks join its lists, and values it has stay. A cursor inside the frontmatter puts the template's body right after it. |
| Kanban order | Dragging a card within its column puts it where it is dropped; a rule in the gap shows the place. Dropping on another column places it there too. Cards placed once come first, in their order, then the others by due date and priority. The order lives in `kanban-order.json` next to the index, and follows a card across columns and boards; editing a card's text forgets its place. |
| Cloud-only notes | In a OneDrive vault, notes whose content is in the cloud are dimmed in the tree with ☁. Downloading or freeing up a note changes its blocks but not its stamp, so each pass compares that too, at no extra cost. |
| Writes to closed notes | Checking a task in the task list, a view or a board, and undoing a rename, used to be refused when the note had changed since the index read it. Now the change is made to the text the index read and merged with the file: an edit elsewhere in the note no longer blocks it, and only an edit to the same lines does. |

### Behaviour (build step 7)

| Feature | Behaviour |
|---|---|
| Callout titles | Titles keep their inline formatting, links and tags; a link in a title follows, a click elsewhere on it folds. A title that reads as more than one paragraph, like `1. Step`, is shown as written. |
| Remembered folding | A foldable callout opened or closed by hand stays so across tabs and restarts, kept in `session.json` per note. A callout is known by its kind, title and how many alike come before it, so edits around it do not lose its state; renaming the title or the note's folder does, renaming the note does not. |
| Delete | Right-click a note or folder in the tree for a menu under it: "Rename or move" and "Delete". "Delete note (move to .trash)" in the palette, and the Delete key when no editor or text field has focus, delete the active note. Each asks first, then moves the note, or the folder with everything in it, into the vault's `.trash` folder under a free name, as Obsidian does. Refused while a note in it has unsaved edits. Its tabs close. Esc closes the menu or the question. |
| Tree toolbar | Small buttons under the vault's name: new note (as `Ctrl+N`), new folder (named in the palette, `/` for nested folders, refused if it exists), the tree's order (file name A to Z or Z to A, modified newest or oldest first; folders stay first, by name), and collapse or expand all folders. The order is kept with the session on this computer. Folders with no notes show too: the folders on disk are listed in the background at start and after every index change, hidden ones left out. |
| Resizable panes | The dividers between the folder tree, the source and the preview can be dragged. The tree keeps at least 140 px and leaves the workspace 360 px; source and preview each keep a tenth of their space. A narrowed window cuts the tree down without forgetting its width. The sizes are kept in `session.json`. |

### Behaviour (build step 8)

| Feature | Behaviour |
|---|---|
| Commands in search | `Ctrl+P` finds commands and Quick Add entries by name without `>` too, from the third letter typed, after the notes whose name matches. `>` still lists commands alone. A long list scrolls. |
| Settings | The ⚙ beside the vault's name, or "Settings" in the palette, shows the settings in place of the note; Done or Esc closes them. They are saved 1.5 s after the last change to `.sage/settings.json` in the vault, which syncs with it and survives deleting the index. A file that cannot be read is reported and never overwritten. |
| Templates folder | Chosen from the vault's folders in the settings, for Insert template and Quick Add. Sage's own choice: Obsidian's `.obsidian/templates.json` is left as it is and used while Sage has none. Date and time formats still come from Obsidian's file. |
| Quick Add | Entries made in the settings, run from the + beside the vault's name, `Ctrl+Shift+N`, or by name in `Ctrl+P`. A template entry has a template, a folder and a file-name pattern such as `{{date:YYYY-MM-DD}} {{value}}`; a capture entry has a note, which may be a pattern like `Daily/{{date:YYYY-MM-DD}}`, an optional heading, a text such as `- {{value}}`, top or end, and whether to create a missing note. `{{value}}` is asked for in the palette, which shows the note that Enter will create or add to. A template entry creates the note with the template filled in, `{{title}}` being the note's name, and opens it; a note that exists is left alone. A capture adds its text under the heading, at the top of the section or after its last line, or adds the heading at the end if the note lacks it; without a heading, at the top after the frontmatter or at the end. A note with unsaved edits is not written to. |

### Behaviour (build step 9)

| Feature | Behaviour |
|---|---|
| Two builds | The same app builds for Linux and natively for Windows. What differs lives in the watcher crate and a few `cfg` blocks: how a drive is watched, the canonical vault path (no `\\?\` on Windows), the data folder, OneDrive placeholders, saving, opening links, and starting up. Vault paths in the index use `/` on both. |
| Windows change detection | Local drives are watched with Windows change notifications and a 10 minute safety rescan; network drives, by UNC path or mapped letter, are polled every 45 s. The status bar says "watching". |
| Windows OneDrive | A cloud-only file says so in its attributes (offline, or recalled on open or on access), so placeholders are known exactly, at any size. |
| Saving | Inside a OneDrive folder a note is rewritten in place on every platform, so OneDrive keeps its version history; elsewhere on a local drive it is written beside and renamed over, which on Windows is tried again for about a second while another program has the note open. |
| Renames | A rename that only changes case, such as `note` to `Note`, works on NTFS, where the new name finds the note itself. |
| Starting | Without a path, `sage` opens `SAGE_VAULT`, else the vault opened last. On Windows, started from a shortcut, it runs without a console, asks for a folder the first time, and shows errors in a message box. Links open with the shell's "open". |
| Tabs | A middle click on a tab closes it. |

### Behaviour (build step 10)

| Feature | Behaviour |
|---|---|
| Quick Add shortcuts | Each entry has a Shortcut row in the settings: click it and press the keys, Esc cancels, Clear removes it. Kept in `.sage/settings.json` as written, e.g. `Ctrl+Shift+P`. A shortcut needs Ctrl, or is a function key, so it never fires while typing. Refused, with the reason, and listening on: Ctrl+Alt with a key but no Shift, which is AltGr on many keyboards and types ą, ś or €; Sage's own shortcuts; Ctrl+A, C, V and X, which edit text; another entry's keys. The palette shows an entry's shortcut beside it. |
| Shortcut matching | Modifiers count exactly. Before, any Ctrl+letter ran its command with Alt held as well, so AltGr+N, typing ń on a Polish keyboard, made a new note, and AltGr+E, ę, switched the view. One table of Sage's shortcuts now drives the keyboard, the palette and the list. |
| Shortcuts list | A Shortcuts section in the settings lists Sage's shortcuts, every Quick Add entry with its keys, settable there too, and the keys and clicks that always do the same. Sage's own shortcuts are listed, not rebindable. |
| Theme | An Appearance section in the settings chooses light, dark, or as the system has it. Kept with the session on this computer, as Ctrl+Shift+L's choice always was. |

### Behaviour (build step 11)

Typed documents. A note whose frontmatter has a `type` is a document of that type; nothing else about it is special, so it is indexed, linked, renamed and searched like any note. Tasks are the first type the preview knows:

```yaml
---
id: fix-voice-reconnect
type: task
title: Fix voice reconnect bug
status: in-progress     # todo, in-progress, done, cancelled
created: 2026-11-16
due: 2026-11-30
tags: [backend, voice]
---
```

| Feature | Behaviour |
|---|---|
| Links by id | `[[fix-voice-reconnect]]` finds the note by its file name, as always, else by its frontmatter `id`, ignoring case. So a task can be renamed freely: links by id are not rewritten and keep finding it. Following one opens the note; "Linked from" lists every note that links to or embeds it, kept up to date by the indexer per changed note. |
| Task links | A link to a whole task document reads `[In Progress] Fix voice reconnect bug`: the status, then the title, or the alias written with `\|`. A link to a heading inside it, or to a document of another type, is a plain link. |
| Task embeds | `![[fix-voice-reconnect]]` shows the title, then Status, Due and Tags, then the note's text. |
| Live | Links, embeds and `tasks` blocks read the index; when it reports a change, they are read again, so editing the task anywhere updates every note that shows it. |
| `tasks` blocks | A fenced block with the language `tasks` lists tasks of both kinds, as the Tasks plugin does: task documents, and checkbox lines from every note. A line has a checkbox that checks it in its note, as in a view, with the guarded write-back; its text opens the note on that line. A line's status is `done` when checked, `cancelled` for `[-]`, else its `#status/` tag, else `todo`. Inside an embedded note the block shows as code, as views do. |
| New task line (`Ctrl+Shift+T`) | A form asks for a title and a due date (`YYYY-MM-DD`, `today`, `tomorrow`, or empty). Enter writes `- [ ] Title 📅 2026-11-30` on the cursor's line when it is blank, else on a new line below it, indented like it when it is a list item; never into the frontmatter. Tab moves between the fields, Esc cancels. |
| New task note (`Ctrl+T`) | The same form creates `Tasks/<id>.md` with `id`, `type: task`, the title, `status: todo`, `created` today and the due date, and puts `[[id]]` at the cursor; with no note open it opens the new one. The id is the title as a slug, and the file is named after it so Obsidian finds `[[id]]` too; an id or file name the vault has gets `-2`, `-3`, … |
| Task overviews | Belong in notes, as Obsidian Bases do: a `tasks` block, or a `view` table such as `source: "path:Tasks/ [type:task] -[status:done]"` with `columns: [file, title, status, due, tags]`, which filters by folder and properties. The sample vault's `Tasks dashboard.md` has both. The vault-wide task list stays in the palette as "Show tasks". |

````markdown
```tasks
status != done
due < 2026-12-01
tag = backend
sort due
```
````

One instruction per line, and every filter must hold. `status = …` and `status != …` take the four statuses; `due` and `created` compare with `=`, `!=`, `<`, `<=`, `>`, `>=` against a date, `today`, `tomorrow` or `yesterday`, and `= none` or `!= none` ask for a missing or present date; `tag = …` and `tag != …` match the tag or one nested under it, a document's anywhere in its note, a line's on that line; `folder = …` and `folder != …` match the folder or one inside it, ignoring case. `sort` takes `due`, `created`, `status`, `title` or `id`, then `asc` or `desc`, and several `sort` lines sort by each in turn; without one, tasks go by due date, those without last. `limit` is 1 to 1000, default 200. Errors name the line. The block is parsed to a small AST and compiled to parameterized SQL over `v_documents`, `v_tasks` and `v_tags`, like view blocks. Schema version 6 added `v_tags.line` for a line's own tags.

How it fits: the Markdown tree already had references as nodes, `Link::Internal` spans for `[[…]]` and `Block::Embed` for `![[…]]`, built from pulldown-cmark's wikilink events. Whether a target is a task is a fact of the vault, not of the syntax, and changes without the referring note changing, so the parser does not decide it: `sage_markdown::references` lists the documents a tree names, the UI looks them up (`Tab::docs`, by link key) and draws a task where it finds one. `Block::TaskQuery` is the one new node. Checkbox tasks (`- [ ]`), the task list and Kanban boards are unchanged and separate.

Not done: `[[` autocomplete does not offer ids; task documents do not appear in the "Show tasks" list or on Kanban boards; changing a task document's status from a link, an embed or a `tasks` block (checkbox lines can be checked there); the folder for task notes is fixed to `Tasks/`.

### Behaviour (build step 12)

Live preview, as Obsidian has it: one pane where you type and see the note formatted. A fourth view mode beside source, split and preview.

| Feature | Behaviour |
|---|---|
| Switching | `Ctrl+Shift+E` turns live preview on, and goes back to the mode before it. `Ctrl+E` cycles source, split, preview, live preview. The palette has "Toggle live preview" and "View: …" for each mode. The mode stays one setting for all tabs, kept in `session.json` as before, so no separate default is needed. Switching to a mode with an editor focuses it. |
| Formatting | Headings at the preview's sizes in the serif, bold, italic, strikethrough, `==highlights==`, inline code on the surface colour, links in the accent, tags in the secondary colour. Wikilinks show their alias, or `Note > Heading`. Body text in Source Sans at the preview's size, at most 688 px wide. |
| Hidden syntax | Block syntax (`#`, `>`, a callout's `[!kind]`) shows while the cursor is on its line; inline syntax (`**`, `` ` ``, `[[ ]]`, `](url)`, a list's `-`, a task's `- [ ]`) while the cursor touches it, ends included. A selection shows everything on the lines it covers. |
| Blocks | Bullets as `•`, tasks as the preview's checkbox, clickable, done tasks muted. Quotes with a bar per level. Callouts on the surface with a bar and their title in the kind's colour; one without a title shows its kind. Code and fences in monospace on the surface, frontmatter in monospace, rules as a hairline. |
| Editing | Typing, deleting, selection, clipboard, word and line selection by double and triple click all go through the tab's `text_editor::Content`, so autosave, completion, find, the find mark, merging, jumps and the preview's checkbox edits work unchanged. Up, Down, Page Up and Page Down move by what is shown, keeping their column; Home and End go to the start and end of the line as written. Clicking a link follows it; clicking a checkbox toggles the task. |

How it fits: iced 0.14's `text_editor` cannot do this. A highlighter sets only colour and font, every line has one height, and no character can be hidden. So `live_editor.rs` is a widget of its own that draws every line of the `Content` as its own paragraph (`Paragraph::with_spans`, which takes size, font and colour per span) and maps columns between the text as written and as shown. With the concrete `iced::Renderer`, a paragraph exposes its cosmic-text buffer, which gives the caret's place and hit testing. What to hide is pure logic in `live.rs`: each line becomes pieces, each shown, hidden, or replaced (`•`, a checkbox); it is unit tested, including that the pieces always cover the line. The widget sends the same actions as the source editor, plus "put the cursor here" for clicks and vertical moves, so nothing in the app's edit path changed. Paragraphs are cached by line text and what is shown, so typing lays out only the edited line, wherever lines moved.

Not done: tables, embeds, pictures, `view` and `tasks` blocks and the properties panel show as written; callouts do not fold; links are not underlined on hover; the caret does not blink; no IME composition window (typed and AltGr characters work, CJK input methods do not); the whole note is laid out when it first shows, which is not measured on long notes.

### Behaviour (build step 13)

Editing and getting around: the keys and small conveniences a writer expects from Obsidian.

| Feature | Behaviour |
|---|---|
| Formatting | `Ctrl+B` bold, `Ctrl+I` italic, ``Ctrl+` `` inline code, in the source pane and live preview. Each wraps the selection, or takes the markers off when it has them, inside the selection or just outside it. No selection: the word under the cursor; no word: an empty pair with the cursor between, which the same key removes again. Italic on bold gives bold italic. |
| Links | `Ctrl+K` turns the selection into `[text]()` with the cursor in the parentheses, a selected URL into `[](url)`, and a selected `[text](url)` back into `text`. |
| Lists and quotes | `Enter` continues `- `, `* `, `+ `, numbered items (`1.` to `2.`, `1)` to `2)`), tasks (the next is `- [ ] `), and quotes and callouts (`> `). Text after the cursor moves to the new item. On an empty item it ends the list, moving a nested item out a level first; an empty quote line ends one level. `Shift+Enter` is a plain line break. |
| Indent | `Tab` and `Shift+Tab` indent and outdent list items and every line of a selection over several lines; elsewhere `Tab` types an indent. One level is a tab, or `tabSize` spaces (default 4) when `.obsidian/app.json` has `useTab: false`. |
| Moving lines | `Alt+↑` and `Alt+↓` move the line or the selected lines; `Ctrl+Shift+D` duplicates them below. |
| Auto-closing pairs | `(` and `[` close before a space or at the end of the line, so `[[` gives `[[]]` and completion still opens. A backtick pairs, except right after another, so fences work. `**` gives `****` with the cursor in the middle; a single `*` does nothing, for bullets and italics. A typed closing character in front of the cursor is stepped over. With a selection, `(`, `[`, a backtick or `*` wraps it. Backspace inside an empty pair deletes both. |
| Pictures | `Ctrl+V` with a picture and no text on the clipboard, or a picture file dropped on the window, saves it and inserts `![[name.png]]` at the cursor. Pasted pictures are named as Obsidian names them, `Pasted image 20261118151230.png`; dropped files keep their name, with ` 1`, ` 2` when it is taken. The folder follows `attachmentFolderPath` in `.obsidian/app.json`: `/` the vault root, `./` the note's folder, `./sub` a folder beside the note, anything else a vault folder; without it, `attachments/`. Created if missing. Only pictures can be dropped. |
| Back and forward | `Alt+←` and `Alt+→`, the mouse's side buttons, and "Go back" and "Go forward" in the palette move through the notes in the order they were opened, skipping notes that no longer exist. |
| Recent notes | `Ctrl+P` with nothing typed lists recently opened notes, then recently changed ones. |
| Reopen closed tab | `Ctrl+Shift+R`. `Ctrl+Shift+T` stays the new task line. |
| Daily note | `Ctrl+D` or "Open today's daily note" opens today's note, creating it first, with the folder, Moment.js date format and template from `.obsidian/daily-notes.json`. Without that file: `YYYY-MM-DD.md` in the vault root. |
| Outline | `Ctrl+Shift+O` or "Show outline" switches the sidebar between the tree and the active note's headings, indented by level. A click puts the cursor on the heading. The choice is kept in `session.json`. |
| Word count | The status bar shows words and characters in the note, or in the selection when there is one. |
| Focus mode | `F11` or "Focus mode" hides the tree, tabs and status bar and centres the note at a reading width. `F11` or `Esc` leaves it. |
| Tree menu | Adds "Show in Explorer" ("Show in file manager" outside Windows) and "Copy link": `[[Note]]` for a note, the path for a folder. |

Code: the edits are pure string logic in `crates/editor/src/edit.rs` (`toggle`, `link`, `enter`, `indent`, `move_lines`, `duplicate`, `type_char`, `backspace`), each returning a `Change`: the lines it replaces, their new text, and where the cursor and selection go. Obsidian's settings are read in `crates/editor/src/obsidian.rs` (`indent_unit`, `attachment_folder`, `pasted_name`, `save_attachment`, `Daily`), with defaults when a file or value is missing, so a vault Obsidian never opened works too. `crates/ui/src/lib.rs` binds the keys for both editors, applies a `Change` to the tab's `text_editor::Content`, and holds the history, recent notes, closed tabs, outline, word count and focus mode.

### Behaviour (build step 14)

| Feature | Behaviour |
|---|---|
| Undo and redo | `Ctrl+Z`, and `Ctrl+Y` or `Ctrl+Shift+Z`, in the source pane and live preview. Each note keeps up to 200 steps of its text and cursor. Typing less than a second apart is one step; moving the cursor, or any other edit (Enter on a list, formatting, a pasted picture, a template, an accepted suggestion, a task checked in the preview), is a step of its own. A new edit forgets what was undone. Text read from disk or merged in starts the history afresh. Code: `crates/editor/src/undo.rs`, `Tab::checkpoint` and `Tab::step_history`. |
| Zoom | `Ctrl+=` (also `Ctrl++` and `Ctrl+Shift+=`), `Ctrl+-` and `Ctrl+0`, or "Zoom in/out" and "Reset zoom" in the palette. Steps of 80% to 200%, applied as iced's scale factor to the whole window, and kept in `session.json`. The window opens at the same size on screen whatever the zoom (checked at 100% and 150%). |
| Rebinding shortcuts | Every shortcut in the built-in table can be changed in Settings › Shortcuts: click it and press the keys, Clear takes it away, Reset brings back the usual keys. Kept per vault in `.sage/settings.json` as `"shortcuts": {"<command name>": "<keys>"}`, empty keys for none. The same checks as Quick Add shortcuts apply, against the keys as rebound. The palette shows the keys as rebound. |
| Quick Add order | ↑ and ↓ on each entry in the settings; the order is the Quick Add list's. |
| Live preview blocks | Tables, embedded notes and pictures, `view` and `tasks` blocks and the properties are drawn as the preview pane draws them, with its links, checkboxes and Kanban drags. A block shows as written while the cursor or a selection is in it, reached with the arrow keys or a click beside its links. Only top-level blocks count, and only when their text parsed alone gives that kind of block. Code: `crates/ui/src/live.rs`, `live_editor.rs`. |
| Task documents in lists and boards | "Show tasks" lists task documents beside checkbox lines, with title, tags, status in brackets unless todo or done, and due date; the title opens the note. A document is open unless its `status` is `done` or `cancelled`; a done one counts as completed today when its note changed today. The checkbox writes `status: done` and remembers the old status for the session; unchecking restores it, else `todo`. `type: tasks` and `kanban` views read `v_tasks` and `v_documents` side by side, so every task filter applies; `kind:document` and `kind:checkbox` pick one. A board places a document by its `status` as it places lines by `#status/` tags; dragging writes `status: done` in the last column and `status: <slug>` elsewhere. Writes go through `sage_editor::tasks::set_status`, with the guards of checkbox write-back. |
| Installer | `scripts/install.ps1` installs for the current user without admin rights: `sage.exe` and `uninstall.ps1` in `%LOCALAPPDATA%\Programs\Sage`, a Start-menu entry, "Open with Sage" on folders and folder backgrounds, Sage in `.md` files' "Open with" list (never the default app), all under HKCU, and an entry in Settings › Apps. `-WhatIf` previews it. `installer/sage.iss` builds the same as `Sage-<version>-setup.exe` with Inno Setup 6. Given a `.md` file, `sage` opens the nearest vault above it (a folder with `.obsidian` or `.sage`, else the file's folder) with that note active. |

Not done:
- Live preview: blocks inside lists, quotes or callouts, and an embed with text on its line, stay as written. There's no table editor and no edit button on a rendered block. A selection that touches a block shows it raw. Rendered blocks are laid out again on every scroll, which hasn't been measured on long notes.
- Task documents: the status restored on unchecking is kept only for the session. `tasks` blocks, links and embeds still cannot change a document's status. Rewriting `status` drops an inline comment on that line.
- Undo: the history is per tab and is lost when the tab closes.
- Installer: `sage.iss` hasn't been compiled. Installing with both the script and `setup.exe` shares registry keys, so uninstalling either removes them.

### Behaviour: switching vaults

| Feature | Behaviour |
|---|---|
| Open vault folder… | In `Ctrl+P` and in Settings under Vault, which also shows the open vault's path. Shows the system folder picker (Windows), else zenity or kdialog. Cancelling changes nothing. A missing, unreadable or non-folder choice, or the vault already open, is refused in the status bar. Unsaved notes and settings are saved first; a note in conflict, or one whose save fails, stops the switch. Sage then starts itself again with the folder as its path and closes, so the new vault opens with its own index, tabs and session, and is the vault opened last. Relaunching, rather than rebuilding the state in place, keeps reads and saves still under way from landing in the new vault's tabs. Code: `crates/ui/src/vault.rs`. |

### Behaviour: editable properties in live preview

| Feature | Behaviour |
|---|---|
| Properties widget | In live preview the frontmatter is a fixed widget, as in Obsidian, not text that turns raw when clicked. Text values are fields, booleans toggle, lists have one field per item with `×` and `+`. Each change rewrites only that entry's lines (`Tab::set_property`, one undo step per change, line endings kept). Nested maps, block scalars and values with comments are shown read-only; moving the cursor into the frontmatter with the keys while the editor is focused still shows it as written. Code: `crates/editor/src/properties.rs` (pure, tested), `preview::property_editor`, `LiveEditor::editable`. Verified by unit tests and a build; **not yet driven by hand** in the running app. |
| Link properties and popups | A property named `links` takes notes like `[[`: typing in a field suggests notes (`suggest::for_context`), picking or Enter writes `[[Note]]`, and a name that matches nothing offers "Create" (following the link creates the note). `[[` typed in any other field suggests too. The suggestions show under the field's row (`PropCompletion`, `preview::property_editor`). In live preview the editor's own suggestions are drawn under the caret by `LiveEditor::popup` (flipped above at the bottom of the pane); source and split mode keep the bottom anchor, as iced hides the caret position and scroll. Link suggestions are labelled with the file's name (the title and folder go in the detail). A list item left empty is removed once it loses focus (`Message::PropertyBlur`, checked after each mouse release with `is_focused`) or on Enter. An "Add property…" field under the rows appends a new property (`Tab::add_property`): a list for `tags`, `aliases`, `cssclasses` and link names, else an empty text field, focused at once. Each row has a type icon (hover for the name; click to list the types inline, as a menu would need an overlay) for Text, List, Number, Checkbox, Date and Date & time as in Obsidian: the type is the one chosen for the name (`property_types` in `.sage/settings.json`, else Obsidian's read-only `types.json`) when the value can be it, else inferred from the value (`properties::type_of`); picking one converts the value (`properties::convert`) and keeps the choice. Number fields take digits only; a × at the end of each row removes the property (`Tab::remove_property`, one undo step); dates are typed as `YYYY-MM-DD`, with no calendar popup yet. A chip takes a light accent wash while hovered (`Message::ChipHover`, `theme::chip`). Outside an edit, list values, tags and wikilinks show as pills in live preview and in the read-only preview (`preview::pill`, `theme::pill`, a light accent wash on hover): a link pill opens its note (a missing one is created, as for any link), a tag pill opens Search everywhere on `#tag` (`Message::SearchTag`), other values are inert. Editing is asked for with the ✎ at the end of the row (`Message::PropertyEdit`, `App.editing_prop`); it focuses the last item and shows the chips with × and +; ✓ or Esc ends it and drops empty items. Add property starts an edit for a new list. Not yet driven by hand. |
| Templates in Quick Add and Ctrl+N | Every `.md` file in the templates folder is a Quick Add entry without configuration (`App::quick_entries`, after the entries in settings), so the palette lists "Quick add: <template>" for each; the file name is asked for and the note opens. The `new_note_template` setting (`.sage/settings.json`, picked in Settings under the templates folder) fills notes made by Ctrl+N and the New note button (`App::create` with no name); notes made by following a link stay empty. Not yet driven by hand. |

## 7. Build order

1. **Headless core (done).** `core`, `db`, `watcher`, `indexer`. Remaining: benchmark against the real vault on ext4 and `/mnt/c`, a `sage-cli` for `index`, `search`, `tasks`.
2. **Shell to daily driver (done).** Collapsible tree, async IO, autosave with the conflict guard, autocomplete, create, move to `.trash`, session restore. Not done and moved: find in note (step 3), rename (step 3, together with link rewrite).
3. **Links, tasks, templates (done).** Rename and move with link rewrite as one undo step, outgoing links panel, global task view with write-back, template folder and insertion, find in note, clickable wikilinks in the preview. Markdown links and property links joined the index for this, so renames reach them. Not done, and moved: clickable checkboxes in the preview (step 4), recurring tasks, folder renames and template properties (step 6).
4. **Query DSL and preview renderer (done).** `pulldown-cmark` block tree, inline formatting, wikilinks, tables, images, `![[embeds]]`, `view` blocks for table and Kanban, drag to rewrite `#status/` tags, plus the clickable preview checkboxes moved here from step 3. Not done, and moved to step 6: callouts, highlights, following URLs, SVG embeds, nested embeds, ordering Kanban cards by hand, properties in the preview. PDF embeds stay undone.
5. **Merge and OneDrive (done).** Three-way merge using the per-tab base text that already exists, open-tab stat polling on 9p, OneDrive root detection, conflict-copy handling, hardening on `/mnt/c` (index pass on focus, rename after a fresh pass, device names). Not done, and moved to step 6: merging writes to closed notes, showing which notes are cloud-only.
6. **The remaining items (done).** Callouts, `==highlights==`, properties in the preview, URLs, SVG and nested embeds, recurring tasks, folder renames, template properties, Kanban order by hand, cloud-only notes in the tree, and merging writes to closed notes against the text the index read. Not done: PDF embeds, which need a native renderer such as pdfium; the `sage-cli` and the benchmark from step 1; inline formatting in callout titles and remembering folded callouts (both step 7).
7. **Callout titles, folding, delete, panes (done).** Inline formatting in callout titles, folding remembered per note across restarts, deleting notes and folders to `.trash` from a tree menu, the palette or the Delete key, and dividers to resize the tree, source and preview.
8. **Settings and Quick Add (done).** Commands found in plain palette search, a settings panel with the templates folder, and Quick Add with template and capture entries, all kept in the vault's `.sage/settings.json`. Not done: Quick Add's macros and its prompts for a choice of files; reordering Quick Add entries.
9. **Windows (done).** A native Windows build from the same code, with the platform layer above, and middle click to close a tab.
10. **Shortcuts and theme (done).** Shortcuts for Quick Add entries, a list of every shortcut, exact matching of modifiers, and the theme in the settings. Not done: rebinding Sage's own shortcuts; PDF embeds, the `sage-cli` and the benchmark; an installer; macOS, for which the platform layer has no backend yet and polls.
11. **Typed documents and task documents (done).** Notes with a frontmatter `type`, links by frontmatter `id`, task links and embeds that show status, due date and tags, `tasks` query blocks over task documents and checkbox lines, and the new-task form on `Ctrl+T` (a task note) and `Ctrl+Shift+T` (a line). Not done: see build step 11.
12. **Live preview (done).** A fourth view mode, one pane that shows the note formatted while you type, with the syntax shown only at the cursor, on a widget of its own over the same editor content. Not done: see build step 12.
13. **Editing and getting around (done).** Formatting keys, `Ctrl+K` links, lists and quotes that continue on Enter, indenting, moving and duplicating lines, auto-closing pairs, pasted and dropped pictures in Obsidian's attachments folder; back and forward, recent notes, reopening a closed tab, the daily note, the outline, word count, focus mode, and "Show in Explorer" and "Copy link" in the tree menu.
14. **Undo, zoom, shortcuts, blocks, task documents, installer (done).** Undo and redo, zoom, rebinding Sage's shortcuts, reordering Quick Add entries, tables, embeds and query blocks in live preview, task documents in the task list and on boards, and a per-user Windows installer with "Open with Sage". Not done: see build step 14.

The app is usable for real work after step 3.

## 8. Verification status and open items

Verified in a clean Ubuntu 24.04 container with Rust 1.91:

- The whole workspace builds without warnings against iced 0.14.0, rusqlite 0.40, notify 8.2.
- 9 unit tests pass across `core`, `indexer`, `editor` and `ui`: scanner coverage, incremental reconcile, rename detection, link re-resolution, deletion, FTS prefix search, the live spawn loop, the save conflict guard, autocomplete contexts, and note-name sanitising.
- The binary was driven on a virtual X display with the software renderer, against the sample vault, and each of these was checked by screenshot or by reading the file on disk afterwards: tree expand and collapse, session restore after restart, link and tag completion inserting the right text, autosave writing to disk, the conflict guard refusing to overwrite an external edit and the overwrite command resolving it, palette note creation in a new folder with name sanitising, editor focus after creating a note, and moving a note to `.trash`.

Step 3 was built with Rust 1.99 on Ubuntu 24.04 under WSL2 and driven the same way, in an Ubuntu 24.04 container on Xvfb:

- The workspace builds without warnings. 21 unit tests pass; the new ones cover Markdown and property links with their positions, link resolution and relative targets, the task toggle, line edits that keep CRLF and a BOM, a rename that rewrites every link style, pins a captured link and undoes byte for byte, refused renames and refused undos, folder cleanup on undo, the guarded task write-back, templates with Moment.js tokens and Obsidian's settings file, and find.
- Checked by screenshot or on disk, against a copy of the sample vault: wikilinks in the preview opening an existing note, creating a missing one and jumping to a heading; "Links to" with an unresolved target dimmed; find with next, previous and close, and `Esc` closing the completion popup before the find bar; `F2` rename with wikilinks and a `%20` Markdown link rewritten on disk, the undo hint staying visible, and undo restoring every file; checking and unchecking a task from the task list; inserting the sample template into a new note; the dark theme.

Step 4 was built and driven the same way:

- The workspace builds without warnings, and 45 unit tests pass. The new ones cover the query language and its errors, the SQL compiler (values travel only as parameters, task filters in a notes table share one task), table cells and sorting, task filters, Kanban grouping, truncation, view spec errors, the Markdown tree (task lines behind CRLF frontmatter and a BOM, tight and loose lists, wikilinks, internal and external links, inline styles and tags, embeds splitting paragraphs and their sizes, tables, view blocks, section and block extraction), and the Kanban line rewrite.
- Checked by screenshot or on disk: the sample Dashboard with a task list, a notes table and an embedded note; inline styles including italics, a table with alignments, nested task lists, a picture embed at `|300`, a section embed, and a view error; a preview checkbox written to disk by autosave; dragging Kanban cards between columns, into Done and back out, with each line checked on disk; split mode and the dark theme.
- The query crate and the Markdown crate were written in parallel by two subagents in separate copies of the workspace, against interfaces fixed beforehand, then merged.

Step 5 was built and driven the same way, with the vault on a Windows-backed `v9fs` mount inside a folder named `OneDrive`:

- The workspace builds without warnings, and 59 unit tests pass. The new ones cover the three-way merge (one-sided, identical, neighbouring and overlapping edits, insertions at the same place, deletion against an edit, CRLF, a missing final newline, empty texts, resolving each choice, cursor mapping), the two-way diff, OneDrive roots, placeholders (a sparse 8 KB file on ext4 looks exactly like one), conflict-copy names, the indexer leaving placeholders unread and reading them once local, and the indexer's sync barrier.
- Checked by screenshot or on disk: "polling · OneDrive" in the status bar; the conflict-copy banner, its resolve panel, the chosen text written to the note and the copy moved to `.trash`; typing here while another program changed a different line, merged on disk with both edits and the cursor kept; the same line changed on both sides, the resolve panel, "Keep both" written to disk; a rename on the polled vault with its links rewritten.
- The merge module and the OneDrive crate with the indexer changes were written in parallel by two subagents, as in step 4.
- Measured on this machine before the OneDrive work: through `/mnt/c`, every local file up to about 700 bytes has `st_blocks == 0`, because NTFS keeps it inside the file record. Of 400 files in a OneDrive folder, 118 small local ones showed that, so the plan's candidate rule (`st_blocks == 0` with any size) would have skipped most short notes. A real cloud-only file (533 KB) showed its true size and no blocks. The rule became "over 4 KB and no blocks"; a smaller cloud-only note is downloaded when read, which is cheap.

Step 6 was built with Rust 1.99 under WSL2 and driven on Xvfb in an Ubuntu 24.04 container, against a copy of the sample vault inside a folder named `OneDrive`, with notes for each feature added:

- The workspace builds without warnings, and 69 unit tests pass. The new ones cover callouts (kinds, titles, folding, CRLF, callouts in lists, the marker only on the first line), highlights (across emphasis and links, unpaired marks, code), properties (lists, links, tags, broken YAML), tasks inside callouts, the recurrence rules and next occurrences, recurring tasks through toggle and Kanban moves, task write-back merged with edits made since indexing and refused on the same line, folder renames (links in, out and inside, bare links pinned where a shorter path would capture them, refusals, undo), undo merging with later edits, template property merging, the cloud flag in the index including a note freed up without a stamp change, stored texts, relative links after a detected rename, and the Kanban order (placing, unplaced cards after, survival across a status change and a restart, renumbering after many halvings).
- Checked by screenshot or on disk: the properties panel, a highlight, four callout kinds with a folded one opened by its title, a recurring task in a callout checked from the preview with its next occurrence written above it, an SVG embed, embeds three notes deep with the fourth as a line and a cycle shown as such, a web link reaching the opener with its query string intact, a card reordered within its column with the drop rule showing and the order on disk, a card dropped at a position in another column with its tag rewritten, a folder moved by right click with a Markdown link rewritten and undone again, a template's properties merged into a note's frontmatter, the ☁ on a sparse note, and the dark theme.
- The opener was a stand-in script recording its arguments; no real browser, `xdg-open` or `wslview` was run.
- The merge for writes to closed notes was not driven in the app: with inotify the index catches up within 150 ms, so the window in which a task list is stale cannot be hit by hand. The unit tests drive it through the same function the app calls.
- The bug in relative links after a detected rename was found by the folder rename test and fixed in the indexer.

Step 7 was built and driven the same way:

- The workspace builds without warnings, and 69 unit tests pass; the callout test now also covers formatted titles with links and tags, titles that are not a paragraph, and keys that count alike callouts and stay put when lines are added above.
- Checked by screenshot or on disk: a folded callout opened by hand still open after a restart and after two lines were added above it, with its key in `session.json`; a title with bold, a link and a tag, folding from a click on its text; the tree dragged to 400 px and the split moved to two thirds, both restored after a restart; the properties panel fitting a narrow preview; the tree menu on a note, the question, and the note in `.trash`; a folder with two notes moved to `.trash` the same way; the Delete key deleting a character in the focused editor, and asking about the active note with focus elsewhere, Esc cancelling.

Step 8 was built and driven the same way:

- The workspace builds without warnings, and 74 unit tests pass. The new ones cover the settings file (round trip, missing keys, a broken file refused), Quick Add patterns (`{{value}}` taken literally, dates), creating notes from templates (variables, an existing note left alone, a missing template), and captures (under a heading at either end, top of the note after the frontmatter, a missing heading added, endings kept, a missing note created or refused).
- Checked by screenshot or on disk: "insert" in `Ctrl+P` finding Insert template without `>`; the ⚙ and + with their tooltips; the settings panel; `Meta/Templates/` chosen as the templates folder, the template list showing only its notes, and Insert template using it after a restart; a template entry and a capture entry made in the panel and saved to `.sage/settings.json`; the template entry run from +, the palette showing the path to create, and the note created in `Projects/` with title, date and value filled in and opened; the capture entry run twice with `Ctrl+Shift+N`, creating today's daily note with `## Log` and adding the second line under it.

Step 9 was built for Linux under WSL and natively on Windows 11 with the MSVC toolchain:

- Both builds compile without warnings, and the 77 unit tests pass on both; one, which fakes OneDrive placeholders with sparse files, runs on Linux only. On Windows the indexer, rename, folder-move and Quick Add tests run against real Windows paths. The new tests cover local folders being watched and network shares polled, canonical paths without `\\?\`, the data folder, notes in OneDrive rewritten in place and others renamed over, and a case-only rename with its undo.
- The Windows `sage.exe` was started on a copy of the test vault: its window opened, titled with the vault's name, the index was built under `%LOCALAPPDATA%\sage\`, and the vault was remembered for the next start. No screenshots were taken on Windows, and nothing in the window was clicked: the folder picker, the message box, opening links, the tab middle click and placeholders on a real OneDrive were not tried there.
- The middle click is a `mouse_area` on each tab; it was not driven on Xvfb either.

Step 10 was built and tested on both platforms, and driven on Xvfb:

- Both builds compile without warnings, and 79 unit tests pass on each; the new ones read and write shortcuts (any order and case, function keys, the plus key, bad text) and check which can type text, and load a settings file from before shortcuts.
- Checked by screenshot or on disk: the theme switched to dark from the settings and kept in the session; recording a shortcut for a Quick Add entry, with Ctrl+S refused as Save note and Ctrl+Alt+A refused as AltGr, then Ctrl+Shift+P taken and saved; the Shortcuts section with Sage's shortcuts, both entries and the fixed keys; Ctrl+Shift+P running the entry and asking for the note's name.

Step 12 was built natively on Windows 11 with the MSVC toolchain and driven there:

- The workspace builds without warnings, and 121 unit tests pass. The new ones cover what live preview hides for a cursor (headings, emphasis nested and unpaired, code, wikilinks with alias and heading, Markdown links, tags, lists and tasks, quotes and callouts, frontmatter and fences, rules, selections across lines), that the pieces cover every line, the column mapping both ways, and, on the real text layout, clicks landing on the right column, a click on a checkbox or link, and up and down stepping past the room above a heading.
- Checked by screenshot, against a copy of the sample vault in both themes: `Ctrl+Shift+E` switching to live preview and the mode kept in `session.json`; headings, emphasis, strikethrough, highlights, inline code, links and tags formatted; the frontmatter and a code block in monospace; bullets, open and done checkboxes, numbered lists, nested quotes, a rule, and five callout kinds; `##` and a callout's `> [!todo]` shown on the cursor's line; a selection showing the syntax it covers; Page Down and Up moving by what is shown, with the view following the caret; typing, the `[[` popup and accepting a suggestion; autosave writing plain Markdown to disk.
- Not driven: mouse clicks and drags, the clipboard, find and the palette commands. The screenshots were taken with `PrintWindow` and keys posted to the window, so that nothing else on the desktop was touched; posted mouse clicks do not reach iced, which loses the pointer when the real one is elsewhere, and `Ctrl` shortcuts need the real keyboard state. The click logic is covered by the unit tests above.

Step 13 was built natively on Windows 11 and driven there the same way:

- The workspace builds without warnings, and 135 unit tests pass. The new ones cover bold, italic and code toggled inside and outside a selection and on the word at the cursor, links made and taken apart, Enter on bullets, numbers, tasks, quotes and empty or nested items, Tab and Shift+Tab, moving and duplicating lines, closing and stepping over pairs, Backspace on an empty pair, and Obsidian's attachment folder, indentation and daily-note settings.
- Checked by screenshot and on disk, against a copy of the sample vault: Enter carrying on `- `, `1.` and `- [x]` items and ending the list on an empty one; Tab indenting with a tab; `(` closing itself, `)` stepping over it, Backspace taking an empty pair, and `[[` giving `[[]]` with the link popup; the word and character counts matching the file; the outline listing the note's headings; F11 entering focus mode, and F11 or Esc leaving it.
- Not driven: Ctrl and Alt shortcuts (formatting, Ctrl+K, Alt+↑ and Alt+↓, Ctrl+Shift+D, Back and Forward, Ctrl+D, Ctrl+Shift+R), Shift+Tab, pasting and dropping pictures, the mouse's side buttons and the tree menu's new entries, for the reasons above. Their text logic is covered by the unit tests.

Fixes after the review in `REVIEW.md` ("Fix first" items 1–10), built natively on Windows 11:

- The workspace builds without warnings, and 168 unit tests pass. The new ones cover:
  - fences closing only on their own kind and length, including in quotes and callouts;
  - escaped pipes in table links, in the scanner, the preview and a rename that also leaves code samples alone;
  - a `---` pair below the top of a note drawn as a rule and a heading, not as properties;
  - a vault folder that cannot be read keeping its index;
  - a failed index write leaving the in-memory mirror equal to the database;
  - the active tab staying put when tabs to its left close;
  - the session and Kanban order writer keeping the newest write and leaving no temp file.
- Behaviour changes:
  - **Closing the window** saves unsaved notes and settings first, then writes the session and exits. A note in conflict, or one whose save failed, stops the close once with a notice, and closing again exits without it. A save already under way is waited for, except on the second close.
  - **`session.json` and the Kanban order** are written on a `sage-store` thread through `sage_editor::save`, so a write is atomic and never blocks the UI thread. Exiting and switching vaults flush them first.
  - **A walk error** no longer counts the notes under that folder as deleted. An unreadable vault root fails the pass and leaves the index alone.
  - **If the watcher cannot start**, the indexer falls back to polling and says so in the status bar. As on any polled vault, saves then rewrite notes in place.
  - **A panic** is appended to `<data>/crash.log` with a backtrace. On Windows, it is also shown in a message box.
- Checked in the app: the debug build was started on a copy of the sample vault, and its window was closed with `CloseMainWindow` (a real `WM_CLOSE`). The process exited at once, and `session.json` was written at that moment. Not driven: closing with unsaved typing, closing with a note in conflict, and the panic box. No keys could be sent to the window, as in step 12.

Fixes for the second group in `REVIEW.md` (the UI, query and preview bugs), built natively on Windows 11:

- The workspace builds without warnings, and 176 unit tests pass. The new ones cover:
  - Unicode `lower()` and `NOCASE`;
  - property filters with date words and quoted numbers;
  - property sorts that ignore case;
  - `none` in any case, and cancelled tasks on request;
  - open cards before old done ones on a limited board;
  - document statuses with underscores, double spaces and non-ASCII capitals finding their columns;
  - `folder =` beyond ASCII;
  - `[/]`, `[-]`, `[>]` and `[?]` drawn as ticked boxes in the preview, as live preview and Obsidian draw them;
  - inserted text taking a CRLF note's line endings.
- UI changes, which are not covered by tests:
  - Back and Forward, the notes opened lately, and the closed tabs follow a rename and its undo.
  - Errors and refusals use `notice()`, so the next index pass no longer wipes them at once. Progress lines still give way.
  - Esc in the palette puts the focus back in the note.
  - Inserting a template, a picture's embed or a new task's link goes through `Tab::insert_at_cursor`. That makes it an undo step of its own, keeps it out of the frontmatter, and gives it the note's line endings. While a merge waits to be resolved, the insert is refused with a notice instead of being lost when the merge lands.
  - A task line added to a CRLF note gets CRLF, and so does a template's frontmatter.
  - The selection and find-match colours in live preview come from `theme::selection` and `theme::find_match`.
- Checked in the app: the debug build started on a copy of the sample vault, built the version 8 index without a crash log, and closed on `WM_CLOSE`. The UI changes above were not driven, for the reasons in step 12.
- Unchanged:
  - Clicking a `[/]` box checks it as done, in the preview as in live preview. Obsidian would uncheck it.
  - Accented letters sort after `z`; ordering by language would need ICU.

Performance after the review (the performance group in `REVIEW.md`), built natively on Windows 11:

- **Measurements.** Timings came from temporary release-build tests on generated notes; the tests were removed afterwards. Per call:

  | What | Before | After |
  |---|---|---|
  | One key in the source pane, 200 KB note (markdown 16.6 ms, outline 7.9 ms, islands 4.8 ms) | about 30 ms | about 0.4 ms (word count only) |
  | The tree, 10,000 notes in 500 folders, root open | 4.0 ms | 0.6 ms |
  | The tree, same vault, two folders open | 4.5 ms | 0.85 ms |
  | Live preview layout pass with nothing changed, 7,000 lines | 5.6 ms | 0.22 ms |

  A copy of the text for undo costs 0.02 ms at 200 KB, so undo was never the CPU cost.
- **Changes:**
  - **Parsing as you type.** The preview, live preview's islands and the outline are parsed again only for what is on screen (`Tab::catch_up`, run from `sync_previews`). Source mode parses nothing per key, and the outline is parsed only while its panel is open.
  - **Undo.** History takes its snapshot only when an edit starts a step. It also keeps at most 32 MB of text per note besides the 200 steps, and always keeps the last step.
  - **Tree.** It builds nodes only down to the first closed folder, and sorts with `sort_by_cached_key`.
  - **Live preview.** A layout whose text, cursor, width, theme and island heights are all unchanged is skipped. Scrolling, the autosave clock and index passes all asked for one. Typing still lays out every line, because a fence or a callout changes how the lines after it show.
  - **Index changes.** After an index change, only the active tab runs its views and `tasks` blocks again and reads its links. Other tabs catch up when they are shown (`links_stale`, `stale`).
  - **Search everywhere.** It reads its corpus on a background thread with its own reader. The palette opens at once with "Reading the vault…", and what is typed meanwhile is searched once the corpus arrives. Each keystroke no longer makes a `String` for every word in the vault.
  - **Indexer folder events.** An event on a folder the index knows notes under is ignored; on Windows a folder's time changes with every save in it. A folder the index knows nothing under is walked, and only that folder. A folder that is gone takes its notes along. Symlinks no longer start a full pass. Before, any of these walked the whole vault.
  - **Indexer limits.** A note over 10 MB keeps its row with no text, and the status bar says so. A file with a modified time in the future is no longer re-read on every pass.
- **Tests.** The workspace builds without warnings, and 180 unit tests pass. The new ones cover:
  - folder events walking only new folders, and removed folders taking their notes along;
  - huge notes, and future modified times;
  - undo skipping the snapshot for joined typing, and its byte budget;
  - nothing being built inside a closed folder.
- **Checked in the app:** the debug build started on a copy of the sample vault, built the version 9 index, picked up a note written from outside while it ran, and closed without a crash log.
- **Debounce in Split, Preview and Live** (added after the above):
  - A note whose last preview parse took under 8 ms is still parsed on every key, so short notes stay live.
  - A slower note's preview and outline wait for a 250 ms pause in typing, and are parsed at least every 750 ms (`parse_waits`). While a parse is waiting, a 100 ms timer runs, and only then.
  - Live preview's islands, which say which lines are drawn as blocks, are still parsed on every key: if they trailed the text, a block would cover the wrong lines. At 200 KB that is about 5 ms a key in Live mode, against about 30 ms before; Split and Preview do no parsing per key while typing goes on.
  - While a preview waits, a click on one of its checkboxes is ignored, since its line may have moved. Live preview finds its checkboxes in the text itself, so its clicks still count.
  - A unit test covers when a parse waits. Typing was not driven in the app; the debug build opened a note with views and closed cleanly.
- **Not done:**
  - `reload_files` still reads all of `v_notes` after every index change.
  - The first live preview layout of a 7,000-line note takes about 225 ms.

Clicks that selected text (reported by hand, 2026-11-19): a plain click could select from the old cursor, and moving the pointer could go on selecting.

- **Live preview.** It kept Shift as last reported. Shift let go while the window had no focus (Win+Shift+S, Alt+Shift+Tab) stayed down, so every click extended the selection. Its drag also ended only if the release reached it past the rendered blocks, and a checkbox or board can capture the release. `live_editor::State::track` now sees every event first. A release ends the drag, and losing focus forgets the drag and the modifiers.
- **Source pane.** iced's editor keeps dragging until it sees the release itself, and does nothing when the window loses focus. The app now tracks the left button from the editor's click to the global release listener or a focus loss, and drops editor drags while the button is up.
- **Tests.** A unit test drives `State::track` through both sequences. The fix was not driven in the app, which cannot be clicked from here (see step 12).
- **Still reported after that: the real cause.** iced's `Content::move_to` sets a selection but never clears one, and it reports even an empty selection as one. In live preview, a click followed by the slightest movement of the pointer published an empty selection at the click's place. After that, and after any real selection, every click selected from the old anchor. All cursor moves that ask for no selection now go through `place`, which clears it with a motion first. A drag back to where it began no longer leaves an empty selection. A unit test shows iced keeping the anchor and `place` clearing it, for an empty selection and a real one. Not driven in the app either.

Not verified, test on your machine first:

1. **Pass time on a real vault on `/mnt/c`.** Polling was confirmed on a Windows-backed mount, but no large vault was measured.
2. **OneDrive offline.** Reading a cloud-only note under 4 KB while offline fails, and the index reports the error on each pass. A placeholder renamed outside Sage has no hash to match, so it is indexed as new and its rows are lost until it is downloaded.
3. **mtime precision over 9p.** If Windows-side timestamps arrive truncated, the racy window may need to widen. The hash check keeps the index correct either way.
4. **Conflict copies made on other machines.** Detection expects OneDrive's `<note>-<COMPUTERNAME>.md` with an uppercase computer name next to the original; no real copy existed on this machine to compare with.
5. **GPU rendering and OS theme detection.** Only the software renderer was exercised, and the container has no desktop portal.
6. **Scale.** No real vault of 10,000 notes has been run; the timings in "Performance after the review" come from release-build tests on generated notes. The task list shows at most 1,000 tasks.
7. **Views on a polled vault.** Views query the index, so on `/mnt/c` they show edits made outside Sage only after the next scan or when the window regains focus. Sage's own task writes appear at once.
8. **Large pictures.** Embedded pictures are decoded by the renderer on first draw; a very large photo may stall the first frame that shows it.
9. **Text in SVG files.** The SVG renderer draws text with the system's fonts. The test container had none, so a label in the test SVG was not drawn; a desktop with fonts installed should show it.
10. **Opening links under WSL.** `wslview` comes with the `wslu` package, which recent Ubuntu images for WSL no longer install; without it Sage falls back to `explorer.exe`, which was not tried.
11. **Dragging dividers under WSLg.** The dividers follow the pointer while the button is held; this was driven on Xvfb only. WSLg's Wayland connection has been seen to drop while the window is moved or resized, which `env -u WAYLAND_DISPLAY` avoids by running on X11.

## 9. Running it

```sh
cargo run --release -p sage-app -- examples/sample-vault
cargo run --release -p sage-app -- /path/to/vault
```

Without a path, `sage` opens `SAGE_VAULT`, else the vault it opened last.

On Windows, build and run it natively; it needs the Rust MSVC toolchain and Visual Studio's C++ build tools, for SQLite:

```powershell
cargo build --release -p sage-app
.\target\release\sage.exe "C:\Users\me\OneDrive\Vault"
```

`sage.exe` is a single file with the fonts inside; it can be copied anywhere and pinned to the taskbar. Started without a path, as from a shortcut, it opens the vault it opened last, or asks for a folder the first time; errors appear in a message box, since it runs without a console.

Sage's settings for a vault, the templates folder and Quick Add entries, live in the vault's hidden `.sage/settings.json`. The index, session and Kanban order live under `~/.local/share/sage/`, or `%LOCALAPPDATA%\sage\` on Windows. Deleting that folder is always safe, though cards placed by hand go back to their order by due date. Links open with `xdg-open` (package `xdg-utils`), or under WSL with `wslview` (package `wslu`).

On Windows it runs through WSL2, with WSLg showing the window. From PowerShell:

```powershell
wsl -d Ubuntu-24.04 -e env ICED_BACKEND=tiny-skia /root/.cache/sage-target/release/sage "/mnt/c/path/to/vault"
```

`ICED_BACKEND=tiny-skia` picks the software renderer. Without a GPU-backed Vulkan driver in WSL the app renders on the CPU either way, and this skips Mesa's warnings about the GPU path.

### Keeping the source on a Windows drive (D:\)

From WSL, `D:\` is `/mnt/d`, served over the slow 9p bridge. Keeping the source there is fine, but put Cargo's build output on the Linux filesystem, or every build writes tens of thousands of small files over 9p:

```sh
echo 'export CARGO_TARGET_DIR="$HOME/.cache/sage-target"' >> ~/.bashrc
source ~/.bashrc
cd /mnt/d/claret
```

The binary then lands in `~/.cache/sage-target/release/sage`. `.gitattributes` keeps line endings LF if the repository is also touched from Windows.
