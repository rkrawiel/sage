---
title: Getting started with Sage
type: doc
status: stable
area: basics
aliases: [Quick start, Introduction]
tags: [docs, docs/basics]
---
# Getting started

Sage is a native Markdown notes app for Linux, WSL and Windows. It works directly on a folder of Markdown files, the same kind of folder Obsidian calls a *vault*, so you can open an existing Obsidian vault and keep using both apps side by side.

Back to the [[Sage guide]]. The hands-on version of this page is [[01 Welcome]].

## Opening a vault

Start Sage with the vault's folder as its argument:

```sh
sage examples/sample-vault
```

Without a path, Sage opens the folder in the `SAGE_VAULT` environment variable, else the vault it opened last. On Windows, started from a shortcut, it asks for a folder the first time and shows errors in a message box.

Given a note instead of a folder, Sage opens the vault the note is in, the nearest folder above it with `.obsidian` or `.sage` in it (else the note's own folder), with that note active.

To switch to another vault later, run **Open vault folder…** from the [[Command palette]] or from the Vault section of [[Settings]]. Sage saves unsaved notes first, then restarts itself on the new folder, so each vault gets its own index, tabs and session.

## Installing on Windows

From a checkout, build Sage and install it for yourself, without admin rights:

```powershell
cargo build --release -p sage-app
powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

This copies `sage.exe` to `%LOCALAPPDATA%\Programs\Sage`, adds **Sage** to the Start menu, adds **Open with Sage** to the right-click menu of folders (opens the folder as a vault), and adds Sage to the **Open with** list of `.md` files. It never makes Sage the default app for `.md` files. Add `-WhatIf` to see every change first, or `-Exe path\to\sage.exe` to install another build. Run it again to update.

To uninstall, use **Settings › Apps › Installed apps › Sage**, or run `%LOCALAPPDATA%\Programs\Sage\uninstall.ps1`. Your vaults and their `.sage` folders are left alone.

A `Sage-<version>-setup.exe` with the same per-user install and an optional desktop icon is built with [Inno Setup](https://jrsoftware.org/isinfo.php): `iscc installer\sage.iss`.

## The window

- **Folder tree** on the left. Folders come first, sorted ignoring case, and start collapsed; opening a note expands the folders above it. Right-click a note or folder for "Rename or move", "Delete", "Show in Explorer" and "Copy link".
- **⚙ and +** beside the vault's name open [[Settings]] and [[Quick Add]].
- **Tabs** across the top. `Ctrl+W` or a middle click closes one.
- **Source and preview panes.** Each tab is in one of three modes: source, split or preview. `Ctrl+E` cycles between them.
- **Dividers** between the tree, source and preview can be dragged. The sizes are remembered.
- **Status bar** at the bottom: index progress, how the vault is watched ("watching" or "polling"), "OneDrive" for a vault inside OneDrive, and messages such as how to undo a rename.

## What Sage remembers

Open tabs, the active tab, each tab's view mode, the theme override, expanded folders, pane sizes and folded callouts are restored the next time you start Sage. They live in `session.json` next to the index, never in your vault.

## The index

Sage keeps an index of the vault in SQLite: names, headings, links, tags, properties, tasks and the full text. It lives outside the vault, under `~/.local/share/sage/` on Linux or `%LOCALAPPDATA%\sage\` on Windows.

> [!info] The index is a cache
> Deleting it is always safe. Sage rebuilds it from your notes on the next start; only Kanban cards you ordered by hand go back to their order by due date.

The index follows changes made outside Sage too, see [[External changes and merging]].

## Light and dark

The theme follows your system. `Ctrl+Shift+L` toggles light and dark by hand, and the Appearance section of [[Settings]] chooses light, dark or system.

## Next

- Learn the [[Command palette]], the one place for notes and commands.
- Skim the [[Keyboard shortcuts]].
- Or start the [[01 Welcome|tutorial]].
