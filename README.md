<p align="center">
  <img src="assets/icons/sage.png" alt="Sage" width="96">
</p>

<h1 align="center">Sage</h1>

<p align="center">
  A fast, native Markdown notes app that works directly on your existing <strong>Obsidian vault</strong>.<br>
  Written in Rust. One codebase, native builds for <strong>Windows</strong> and <strong>Linux</strong> (including WSL).
</p>

---

## What is Sage?

Sage is a "second brain" desktop app for people who keep their notes as plain Markdown files. You point it at a folder, any Obsidian vault for example, and it gives you a quick native editor with links, tasks, queries and Kanban boards. It has **no import step and no proprietary format**. Your notes stay ordinary `.md` files, so you can keep using Obsidian, a sync client or a text editor on the same folder at the same time.

Behind the scenes Sage keeps a small **SQLite + FTS5 index** outside the vault, so search, backlinks and queries are instant even on large vaults. The index is a cache: you can delete it at any time and Sage rebuilds it.

### Features

- **Editing:** source, split, preview and **live preview** modes per tab; syntax highlighting; autocomplete for `[[links]]`, `#tags` and headings; find in note; undo and redo; formatting keys; smart lists; auto-closing pairs; pasted or dropped pictures saved to the vault's attachments folder.
- **Obsidian-compatible links:** wikilinks, Markdown links, aliases, `[[Note#Heading]]`, `![[embeds]]`, backlinks and outgoing links. **Renaming or moving** a note or folder rewrites every link to it in one undoable step.
- **Tasks:** the Tasks-plugin emoji format and Dataview inline fields, recurring tasks (`🔁 every week`), a vault-wide task list, task documents (notes with `type: task`), and checkboxes you can click in the preview.
- **Queries and boards:** `view` blocks with an Obsidian-search-style query language, shown as tables, task lists or drag-and-drop **Kanban boards** (columns are `#status/…` tags). `tasks` blocks list tasks.
- **Preview:** CommonMark, tables, callouts (foldable), `==highlights==`, frontmatter properties, images, SVG and nested embeds.
- **Getting around:** command palette (`Ctrl+P`), search everywhere (`Ctrl+Shift+F`), back and forward, recent notes, daily note, outline, focus mode, zoom.
- **Templates and Quick Add:** Obsidian template syntax (`{{title}}`, `{{date:YYYY-MM-DD}}`), plus Quick Add entries that create notes from templates or capture text under a heading.
- **Safe with external edits:** saves never overwrite changes made elsewhere. A three-way merge combines them and asks only when the same lines changed on both sides.
- **OneDrive-aware:** skips cloud-only placeholders, rewrites files in place to keep version history, and helps resolve OneDrive conflict copies.
- **Works anywhere:** native filesystem change notifications on local disks, and efficient polling on network drives and `/mnt/c` under WSL.
- **Configurable:** light and dark theme ("Sage & Paper"), rebindable shortcuts, settings stored in the vault (`.sage/settings.json`).
- **Single file:** the binary embeds its fonts and icon. Copy `sage.exe` anywhere and run it.

### Try it

The repository has a sample vault with a hands-on tutorial. Open `examples/sample-vault` and start at **Sage guide**.

---

## Building

### Prerequisites (all platforms)

| Requirement | Notes |
|---|---|
| [Rust](https://rustup.rs) **1.88 or newer** (stable) | `rustup update stable` |
| A C compiler | SQLite is compiled from source (bundled `rusqlite`) |

```sh
git clone <this-repo> sage
cd sage
```

The fonts come with the repository (`assets/fonts/`) and are compiled into the binary, so there's nothing else to download.

### Windows (native)

**Requirements:** the Rust **MSVC** toolchain (`x86_64-pc-windows-msvc`) and **Visual Studio Build Tools** with the "Desktop development with C++" workload (the C compiler and the Windows SDK's `rc.exe`, which embeds the app icon).

```powershell
cargo build --release -p sage-app
```

```powershell
.\target\release\sage.exe "C:\Users\me\Documents\MyVault"
```

The result is a single self-contained `target\release\sage.exe` with no console window. Errors appear in a message box. Started without a path, it opens the vault it opened last, or asks for a folder the first time.

### Linux

**Requirements:** a C compiler (for example `sudo apt install build-essential` on Debian/Ubuntu), and a desktop session (X11 or Wayland) to run the app.

```sh
cargo build --release -p sage-app
./target/release/sage ~/Notes/MyVault
```

Optional: `xdg-utils` (`xdg-open`) to open web links in your browser.

### WSL2 (Linux build, shown through WSLg)

Build the Linux version inside your WSL distribution exactly as above. If the source lives on a Windows drive (for example `/mnt/d/...`), keep build output on the Linux filesystem, because writing it over 9p is very slow:

```sh
echo 'export CARGO_TARGET_DIR="$HOME/.cache/sage-target"' >> ~/.bashrc
source ~/.bashrc
cargo build --release -p sage-app      # binary: ~/.cache/sage-target/release/sage
```

Run it from PowerShell on a vault on the Windows side:

```powershell
wsl -d Ubuntu-24.04 -e env ICED_BACKEND=tiny-skia /root/.cache/sage-target/release/sage "/mnt/c/path/to/vault"
```

- `ICED_BACKEND=tiny-skia` selects the software renderer and avoids Mesa warnings when WSL has no GPU driver.
- Install `wslu` (`wslview`) so links open in your Windows browser. Without it Sage falls back to `explorer.exe`.
- If the window drops while it is moved or resized under WSLg, run with `env -u WAYLAND_DISPLAY` to use X11.

> On Windows, the native build is the recommended way to use Sage. The WSL build is useful when your vault lives on the Linux filesystem.

### Debug builds and running from source

```sh
cargo run -p sage-app -- examples/sample-vault             # debug
cargo run --release -p sage-app -- examples/sample-vault   # release (much faster UI)
```

---

## Windows installer

Sage installs **per user**. It needs no admin rights and writes only to `%LOCALAPPDATA%` and `HKCU`. Both methods below install the same things:

- `sage.exe` in `%LOCALAPPDATA%\Programs\Sage`
- a **Sage** entry in the Start menu
- **"Open with Sage"** on folders and on a folder's background (opens the folder as a vault)
- Sage in the **"Open with"** list for `.md` files (never made the default app)
- an entry in **Settings → Apps** for uninstalling

Build the release binary first:

```powershell
cargo build --release -p sage-app
```

### Option A: setup.exe (Inno Setup)

1. Install Inno Setup 6:

   ```powershell
   winget install JRSoftware.InnoSetup
   ```

2. Compile the script from the repository root:

   ```powershell
   iscc installer\sage.iss
   ```

3. The installer is written to `target\installer\Sage-<version>-setup.exe`. The version is read from `sage.exe`, which takes it from `Cargo.toml`. Override it with `iscc /DAppVersion=1.2.3 installer\sage.iss`.

The setup offers an optional desktop icon and has its own uninstaller in Settings → Apps.

### Option B: PowerShell script (no extra tools)

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

- `-WhatIf` lists every change without making it.
- `-Exe <path>` installs a different `sage.exe` (the default is `target\release\sage.exe`).
- Run it again to update.

To uninstall, use Settings → Apps, or run:

```powershell
powershell -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\Programs\Sage\uninstall.ps1"
```

Uninstalling never touches your vaults, their `.sage` folders, or Sage's index cache.

---

## Usage

```
sage [vault-folder | note.md]
```

| Started with | Opens |
|---|---|
| A folder | That folder as the vault |
| A `.md` file | The nearest folder above it that contains `.obsidian/` or `.sage/` (else the file's own folder), with that note open |
| Nothing | `$SAGE_VAULT` if set, else the vault opened last, else (Windows) a folder picker |

Press `Ctrl+P` for the command palette. **Settings → Shortcuts** inside the app lists every shortcut. Common ones:

| Keys | Action |
|---|---|
| `Ctrl+P` | Command palette / open note |
| `Ctrl+Shift+F` | Search everywhere |
| `Ctrl+N` / `Ctrl+Shift+N` | New note / Quick Add |
| `Ctrl+E` / `Ctrl+Shift+E` | Cycle view mode / toggle live preview |
| `Ctrl+T` / `Ctrl+Shift+T` | New task note / new task line |
| `Ctrl+D` | Today's daily note |
| `F2` | Rename or move note (rewrites links) |
| `Ctrl+Shift+L` | Toggle light and dark theme |
| `F11` | Focus mode |

### Where Sage keeps its data

| What | Where | Safe to delete? |
|---|---|---|
| Your notes | Your vault | — |
| Per-vault settings (templates folder, Quick Add, shortcuts) | `<vault>/.sage/settings.json` | Resets those settings |
| Index, session (open tabs, layout), Kanban card order | Linux: `~/.local/share/sage/<hash>/` · Windows: `%LOCALAPPDATA%\sage\<hash>\` | **Yes**: rebuilt automatically (hand-placed card order resets) |

Deleting a note in Sage moves it to the vault's `.trash` folder, as Obsidian does.

---

## Project structure

A Cargo workspace of small crates. Dependencies point strictly downward, and only `ui` knows about the GUI toolkit ([iced](https://iced.rs)):

```
app ─→ ui ─→ { indexer, editor, query, markdown } ─→ { db, watcher, core, onedrive }
```

| Crate | Purpose |
|---|---|
| `crates/app` | The `sage` binary |
| `crates/ui` | iced application shell, panels, theme |
| `crates/indexer` | Background sync loop that keeps the index equal to the vault |
| `crates/editor` | Saving, editing commands, rename with link rewrite, tasks, templates, Quick Add, merge |
| `crates/query` | `view` / `tasks` query language compiled to parameterized SQL |
| `crates/markdown` | Preview block tree (pulldown-cmark plus Obsidian syntax) |
| `crates/db` | SQLite schema and connections |
| `crates/watcher` | Filesystem change detection and the platform layer |
| `crates/core` | Domain types and the fast Markdown scanner |
| `crates/onedrive` | OneDrive roots, placeholders, conflict copies |

See [`PLAN.md`](PLAN.md) for the full design, the behaviour spec and the verification notes, and [`CLAUDE.md`](CLAUDE.md) for contributor and agent guidance.

## Development

```sh
cargo test --workspace                  # run all tests
cargo test -p sage-editor               # one crate
cargo clippy --workspace --all-targets
```

The build should stay free of warnings. The code is not formatted with `rustfmt`, so match the surrounding style instead of running `cargo fmt`.

## License

MIT, as declared in `Cargo.toml`.

The bundled fonts in `assets/fonts/` are under the SIL Open Font License 1.1. Their licences, with each copyright notice, are in [`assets/fonts/licenses/`](assets/fonts/licenses):

| Font | Used for | Copyright |
|---|---|---|
| Source Serif 4 | Headings | Adobe, Reserved Font Name "Source" |
| Source Sans 3 | Body text and UI | Adobe, Reserved Font Name "Source" |
| JetBrains Mono | Editor | The JetBrains Mono Project Authors |
| Noto Emoji | Monochrome emoji | Google LLC |

The fonts are embedded unmodified.
