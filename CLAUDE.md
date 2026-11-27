# CLAUDE.md

Guidance for coding agents working in this repository. Read this first, then `PLAN.md` for the design history.

## What this is

**Sage** is a native Rust desktop "second brain" that works directly on an existing **Obsidian vault** (a folder of Markdown files). It has one codebase with two native builds: **Linux** (including WSL2/WSLg) and **Windows** (MSVC). The UI is **iced 0.14**. A disposable **SQLite + FTS5** index sits beside the vault, and a background indexer keeps it equal to the files on disk.

Main features: a folder tree, tabs, source/split/preview/live-preview editing, autocomplete, wikilinks and backlinks, renames that rewrite links, a vault-wide task list, `view` and `tasks` query blocks (tables, Kanban), templates, Quick Add, three-way merge on external edits, OneDrive awareness, and a per-user Windows installer.

> **Naming:** the folder is `claret`, but every crate, binary, data folder and doc uses **`sage`**. "Sage" is a working name. Leftover `claret*` and `libclaret_*` artifacts in `target/` come from an older build; ignore them. Never mix the two names in new code. This folder is **not a git repository**.

## Commands

Run everything from the repo root. Toolchain: stable Rust ≥ 1.88 (`rust-version` in `Cargo.toml`); developed on 1.99. On Windows it needs the MSVC toolchain and the VS C++ build tools, because rusqlite bundles SQLite and is compiled with `cc`.

```sh
cargo build -p sage-app                           # debug build → target/debug/sage(.exe)
cargo build --release -p sage-app                 # release build → target/release/sage(.exe)
cargo run --release -p sage-app -- examples/sample-vault
cargo test --workspace                            # all tests (183 pass on Windows today; one Linux-only test)
cargo test -p sage-editor                         # one crate
cargo test -p sage-editor rename::                # one module's tests (name filter)
cargo test -p sage-query -- --exact view::tests::table_cells_and_sorting   # one test (exact path)
cargo clippy --workspace --all-targets            # 2 pre-existing minor warnings (sage-ui, sage-editor)
```

- **Do NOT run `cargo fmt`.** The code is not rustfmt-formatted: lines run up to about 145 columns and use hand-made layouts, and `cargo fmt --check` reports over 1,000 diffs. Formatting the whole workspace would bury your change in noise. Match the style around you by hand.
- `cargo build` must stay **warning-free**. Every build step in `PLAN.md` was checked against that.
- Windows packaging (installer, `install.ps1`) is covered in `README.md`.

### Running

`sage [vault-folder | note.md]`. Without an argument it opens `$SAGE_VAULT`, then the vault opened last (`<data>/last-vault`), and on Windows it then shows a folder picker. Given a file, it opens the nearest ancestor folder that holds `.obsidian/` or `.sage/` (`crates/app/src/main.rs`).

Under WSL with the source on `/mnt/d`, set `CARGO_TARGET_DIR="$HOME/.cache/sage-target"` so build output stays off 9p (see PLAN §9). `ICED_BACKEND=tiny-skia` forces the software renderer.

## Workspace layout

```
Cargo.toml              workspace: shared versions, path deps (sage-* = { path = ... }), release profile (thin LTO, strip)
PLAN.md                 design doc + per-build-step behaviour spec + verification log (~76 KB)
assets/fonts/           TTFs embedded with include_bytes! (committed); OFL licences in assets/fonts/licenses/
assets/icons/           sage.ico (exe icon via build.rs), sage.png (window icon), sage.svg
examples/sample-vault/  test vault: Dashboard, Tasks, Templates, Tutorial, .sage/settings.json
assets/starter-vault/   what "Create starter vault" writes (embedded via sage_editor::starter::FILES; keep its array length in step)
installer/sage.iss      Inno Setup 6 script → target/installer/Sage-<ver>-setup.exe
scripts/                install.ps1 / uninstall.ps1 (per-user Windows install, HKCU only); fetch-fonts.sh (only to add or refresh a font)
crates/                 ten crates, below
```

### Crates and dependency direction

Dependencies point **strictly downward**. Nothing below `ui` knows that iced exists, so every lower crate is tested headless.

```
app ─→ ui ─→ { indexer, editor, query, markdown } ─→ { db, watcher, core, onedrive }
```

| Crate (package) | Path | Responsibility | Key entry points |
|---|---|---|---|
| `sage-app` | `crates/app` | The `sage` binary: argument and vault resolution, Windows error box, icon (`build.rs`) | `main.rs` → `sage_ui::run_with` |
| `sage-ui` | `crates/ui` | iced shell: state, messages, all widgets, "Sage & Paper" theme | `lib.rs` (`App`, `Message`, `Command`) |
| `sage-indexer` | `crates/indexer` | Sync loop: notify hints + periodic mtime reconcile → SQLite. Owns the **only** write connection | `spawn`, `IndexerHandle`, `IndexEvent` |
| `sage-editor` | `crates/editor` | Toolkit-free buffer logic: guarded save, edits, completion, find, undo, rename + link rewrite, task write-back, recurrence, templates, Quick Add, settings, merge, shortcuts, starter vault | `save_guarded`, modules below |
| `sage-query` | `crates/query` | `view` block DSL → AST → parameterized SQL over `v_*` views; `tasks` blocks over typed documents | `parse_spec`, `run`, `dsl`, `documents` |
| `sage-markdown` | `crates/markdown` | pulldown-cmark → preview block tree with Obsidian extras (wikilinks, embeds, callouts, highlights, properties, view blocks) | `parse`, `references`, `section`, `block` |
| `sage-db` | `crates/db` | Index location, connections, `schema.sql`, `SCHEMA_VERSION`, FTS query building | `open_writer`, `open_reader`, `index_path_for` |
| `sage-watcher` | `crates/watcher` | Platform layer: backend detection (Native vs Poll), notify wrapper, canonical paths, data dir | `detect_backend`, `watch`, `canonical`, `data_dir` |
| `sage-core` | `crates/core` | Domain types + **line-oriented** scanner (frontmatter, headings, links, tags, tasks), link keys and resolution, task line edits | `parse`, `resolve`, `toggle_task`, `edit_lines` |
| `sage-onedrive` | `crates/onedrive` | OneDrive root detection, cloud-only placeholder test, conflict-copy names. No dependencies | `root`, `is_placeholder`, `originals_of` |

### Where things live in `sage-ui`

`crates/ui/src/lib.rs` is about 5,200 lines and holds the whole application state machine. The other files are focused widgets and helpers:

| File | What |
|---|---|
| `lib.rs` | `run_with`, the `Message` enum (~L135), `Command` + the `COMMANDS` palette list + `BUILT_IN` shortcuts (~L335–470), `Session`, `App` struct (~L569), `Tab`, `App::handle` (the big `match`, ~L1218), `App::run(Command)` (~L1789), `on_index`, open/save/merge/rename/trash flows, `view()` and the panel builders (~L3796+), event subscriptions (~L4938+) |
| `theme.rs` | `Tokens` (10 colours, LIGHT/DARK), `RADIUS`, fonts (`SERIF`, `SANS`, `MONO`, `FONT_FILES`), every widget style function |
| `preview.rs` | Renders the `sage-markdown` tree; checkboxes, links, embeds, callouts, properties |
| `views.rs` | Draws `view` results: table, task list, Kanban (drag → messages) |
| `live.rs` / `live_editor.rs` | Live preview: pure syntax-hiding logic / the custom editor widget that shares the tab's `text_editor::Content` |
| `highlight.rs` | Source-pane Markdown highlighter (stateful across lines) |
| `tree.rs` | Sidebar folder tree + its toolbar and context menu |
| `omni.rs` | Search everywhere (`Ctrl+Shift+F`), in-memory fuzzy scoring |
| `suggest.rs` | Autocomplete suggestions read from the index |
| `tasks.rs` | Vault-wide task list panel |
| `task_form.rs` | New-task form (`Ctrl+T` note, `Ctrl+Shift+T` line) |
| `settings.rs` | Settings panel (templates folder, Quick Add entries, shortcuts, theme) |
| `order.rs` | Hand-placed Kanban card ranks (stored next to the index, never in the vault) |
| `store.rs` | Background writer for `session.json` and the Kanban order: atomic, newest write wins, `flush()` before exit |
| `vault.rs` | "Open vault folder…": relaunches the process on the new folder |

## Architecture rules (do not break these)

1. **One writer.** Only the indexer thread writes SQLite (`sage_db::open_writer`). The UI holds a read-only connection (`open_reader`). In WAL mode reads never block. Never write to the DB from the UI or from `sage-editor`.
2. **The UI thread never blocks.** It does no file IO and no long work on the iced thread. Use `Task::perform(async move { tokio::task::spawn_blocking(move || …).await… }, Message::X)` and handle the result in another `Message` variant. Short read-only SQL (sidebar, palette, suggestions) is fine.
3. **The index is a disposable cache.** There are no migrations. If you change `schema.sql`, **or** change which rows the scanner or indexer produce, **bump `SCHEMA_VERSION`** in `crates/db/src/lib.rs`. A mismatch empties and rebuilds the index. Nothing the user cares about may live only in the index.
4. **Where state lives:**
   - In the vault: notes only, plus Sage's per-vault settings in `<vault>/.sage/settings.json` (`sage_editor::settings::Settings`). It travels with the vault.
   - Next to the index, under `<data>/<16-hex blake3 of vault path>/`: `index.db`, `session.json`, Kanban order. `<data>` = `$XDG_DATA_HOME/sage` (`~/.local/share/sage`) on Linux, `%LOCALAPPDATA%\sage` on Windows.
   - Obsidian's own `.obsidian/` settings are **read-only** for us (`sage_editor::obsidian`: attachments folder, indentation, daily notes, templates).
5. **Never clobber user edits.** Saves go through `sage_editor::save_guarded` (it refuses if the disk differs from the tab's base text). External changes three-way merge (`sage_editor::merge`) and prompt only on overlapping hunks. Task write-back checks that the line still equals `tasks.raw`, or merges against the indexed `texts` base.
6. **Saving policy.** On native filesystems: write a temp file + fsync + rename (with a Windows retry loop). On 9p, network drives and **any OneDrive folder**: rewrite in place. Use `sage_editor::save`, never `std::fs::write` directly on notes.
7. **No raw SQL from notes.** Query values travel only as SQL parameters. Columns and operators come from fixed lists in `crates/query/src/sql.rs`. View blocks read only the `v_*` views and `fts`.
8. **Theme by construction.** Every colour comes from `theme::Tokens`, and every widget is styled with a `theme.rs` function. Never use iced's generated palette. The only radius is `RADIUS = 3.0`. No blue or purple, gradients or shadows.
9. **Obsidian compatibility is the spec.** Link resolution (shortest path wins, `[[folder/Note]]` suffix match, aliases, frontmatter `id`), templates (`{{title}}`, `{{date:FORMAT}}` with Moment.js tokens), the Tasks plugin emoji format (written; Dataview inline fields are also read), callouts, and `#status/<slug>` Kanban columns all follow Obsidian's behaviour. When unsure, do what Obsidian does.
10. **Positions.** `sage-core` line numbers are **0-based from the top of the file, frontmatter included**. Editor columns are **byte offsets** within a line (matching iced's cursor). Preserve CRLF, BOMs and missing final newlines when editing text (see `core::edit_lines`, `merge`).
11. **Paths.** Inside the app, notes are named by **vault-relative paths with `/` separators**, even on Windows. Canonicalise vault roots with `sage_watcher::canonical` (it strips `\\?\`).

## Platform-specific code

Keep platform differences inside the platform layer. These files have `cfg(windows)`, `cfg(target_os = "linux")` or `cfg(unix)` branches:

- `watcher/src/lib.rs`: `statfs` (Linux) or `GetDriveType` (Windows) to pick the backend; data dir; canonical paths.
- `editor/src/lib.rs`: rename retry on Windows. `editor/src/rename.rs`: case-only renames.
- `ui/src/lib.rs`: `open_link` (`xdg-open`/`wslview`/`explorer.exe` vs `ShellExecuteW`), `reveal` in the file manager.
- `ui/src/vault.rs`: folder picker (`rfd` on Windows).
- `ui/src/notify.rs`: task-reminder notifications (WinRT toast via `tauri-winrt-notification` vs `notify-send`).
- `app/src/main.rs`: `windows_subsystem = "windows"` (no console), `rfd` error box and folder picker.
- `onedrive`, `indexer`: one placeholder test that uses sparse files is Linux-only.

If you add a platform branch, give the other platform a working fallback and keep both builds compiling.

## How to implement common changes

### A new command, palette entry or shortcut (UI)
1. Add a variant to `enum Command` in `crates/ui/src/lib.rs`.
2. Add `("Human name", Command::X)` to `COMMANDS` and **bump the array length**. The name is also the key users rebind under `shortcuts` in `.sage/settings.json`, so don't rename existing entries casually.
3. Optional default keys: add `("Ctrl+…", Command::X)` to `BUILT_IN` (bump the length). Check for clashes with `BUILT_IN`, `EDITING` and `FIXED`.
4. Handle it in `App::run(&mut self, command)`. Return a `Task<Message>` for async work.

### A new async operation
Add `Message::XDone(Result<…, String>)`, start it with `Task::perform` + `spawn_blocking` (see `App::save` ~L2110 or `App::read`), and handle the result in `App::handle`. Errors go to the status bar as strings and must not panic.

### Pure text or editing logic
Put it in `sage-editor` (or `sage-core` if the indexer needs it too) as pure functions over `&str` and lines, returning a value such as `edit::Change`. Unit-test it there. Then wire it into the UI. Logic that is new and testable should not go into `ui/src/lib.rs`.

### Index or schema changes
Edit `crates/db/src/schema.sql`, update the scanner (`core::parse`) and/or indexer insert code (`crates/indexer/src/lib.rs`), **bump `SCHEMA_VERSION`**, and expose new data to queries through a `v_*` view, not raw tables. Add an indexer test that uses `tempfile`.

### Query language (`view` / `tasks` blocks)
Syntax table: `crates/query/src/dsl.rs` (module docs). Spec keys (`type`, `source`, `columns`, `sort`, `limit`): `view.rs`. SQL compilation: `sql.rs`, which adds columns and operators to its whitelists. `tasks` blocks: `documents.rs`. Rendering: `ui/src/views.rs`.

### Preview rendering
Parse in `crates/markdown/src/tree.rs`, add a `Block`/`Span` variant in `lib.rs`, and test in `crates/markdown/src/tests.rs`. Draw it in `ui/src/preview.rs`. If live preview should show it too, update `ui/src/live.rs`.

### Vault settings
Add a field to `sage_editor::settings::Settings` with `#[serde(default)]`-compatible handling, so that old `settings.json` files still load (there is a test for this). Surface it in `ui/src/settings.rs`.

## Testing conventions

- Unit tests live in `#[cfg(test)] mod tests` at the bottom of each file. The exception is `crates/markdown/src/tests.rs`. There is no `tests/` directory.
- File-touching tests use `tempfile::tempdir()` and build tiny vaults inline. Indexer, query and editor tests spin up a real `sage_db` + `sage_indexer` (dev-dependencies) against a temp vault.
- Test names are descriptive sentences (`guard_refuses_to_overwrite_external_changes`, `nearest_obsidian_folder_is_the_vault`).
- Cover edge cases the codebase already cares about: CRLF, a BOM, a missing final newline, frontmatter offsets, Unicode byte columns, Windows paths.
- The UI is mostly verified by running the app on `examples/sample-vault` (use a **copy** so the sample is not modified). `PLAN.md` §8 logs what was driven by hand. Pure halves (`live.rs`, `omni.rs`, `order.rs`, `tree.rs`) have unit tests.

## Code style

- Rust 2021, `anyhow::Result` for fallible functions across crates. UI-facing results are often `Result<T, String>` so they can travel in `Message`.
- **Comments explain why, in full sentences, in British spelling** ("recognised", "behaviour"). Every module starts with a `//!` doc that says what it owns and its constraints. Keep it accurate when you change the module.
- Prefer small free functions and plain data over traits and abstractions. There is almost no trait-based indirection. Keep it that way.
- Dependencies are declared per crate with explicit versions. Internal crates use `sage-x.workspace = true`. Keep heavy features off (for example `image` has only png/jpeg/gif/webp, `chrono` has no default features).
- Fonts and icons are embedded with `include_bytes!`. The binary is a single self-contained file.

## PLAN.md

`PLAN.md` is the source of truth for decisions and behaviour:
- §1 approved decisions · §2 layout · §3 runtime architecture · §4 SQLite schema notes and version history · §5 sync loop and edge cases
- §6 detailed behaviour per build step (2–14): read the relevant step before you change a feature
- §7 build order and what was deferred · §8 verification log and **known unverified items** · §9 running it

When you finish a feature, update the matching section of `PLAN.md`: what was done, what was deferred, and how it was verified.

## Gotchas

- Fonts are committed in `assets/fonts/` and embedded with `include_bytes!` (`theme::FONT_FILES`). To add one: put the `.ttf` there (or add it to `scripts/fetch-fonts.sh`), add its licence to `assets/fonts/licenses/`, extend `FONT_FILES` and its length, and list it in the README's license table. Only use fonts whose licence allows embedding (for example OFL), and never modify a font with a Reserved Font Name such as Adobe's "Source".
- `.gitattributes` forces LF line endings. Keep that in new files, including on Windows.
- Hidden entries (`.obsidian`, `.trash`, `.sage`, `.`-prefixed temp files) are skipped by the indexer. Deleting a note moves it to `<vault>/.trash`, never hard-deletes.
- In polled vaults (9p `/mnt/c`, network drives) the index trails the disk by up to 45 s. Renames re-read the linking notes from disk and never patch by stale indexed positions (`links.col`/`len` are not trusted for writes).
- OneDrive placeholders (over 4 KB, no allocated blocks) are not read by the indexer, because reading would download them.
- `sage-app/build.rs` needs the Windows SDK's `rc.exe` to embed the icon when the target is Windows.
