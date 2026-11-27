---
title: External changes, merging and OneDrive
type: doc
status: stable
area: sync
aliases: [Merge, Conflicts, OneDrive, Sync]
tags: [docs, docs/sync]
---
# External changes and merging

Sage is happy to share a vault with Obsidian, a text editor, a sync client or another computer. It watches the vault, keeps its index current, and merges changes into open notes instead of overwriting them.

Back to the [[Sage guide]].

## How the vault is watched

| Where the vault is | How changes are seen | Status bar |
|---|---|---|
| A local Linux or Windows drive | change notifications, plus a full check every 10 minutes | watching |
| `/mnt/c` under WSL, SMB, NFS, network drives | a scan every 45 s, and open notes checked every 2 s | polling |

Coming back to Sage's window starts a full index pass. [[View blocks]] on a polled vault show edits made elsewhere after the next scan; Sage's own writes appear at once.

## Changes to an open note

Every change made elsewhere to a note you have open goes through a **three-way merge** against the text the tab loaded:

- With no local edits, the note reloads and the cursor stays on the same text.
- With local edits, changes to different lines, even neighbouring ones, merge silently and autosave writes the result.
- A save that finds the file changed re-reads it and merges the same way.

## The resolve panel

Only sections that both sides changed differently wait for you. The resolve panel replaces the editor and shows each such section with two lines of context, yours beside theirs. For each section, or for all of them, choose:

- **keep yours**,
- **take theirs**,
- or **keep both**.

Autosave waits until you press **Apply**. **Overwrite file on disk** in the [[Command palette]] keeps your version outright.

## Writes to closed notes

Checking a task from a list, a view or a board, or undoing a rename, changes notes that may not be open. Sage makes the change to the text the index read and merges it with the file, so only an edit to the very same lines blocks it.

## OneDrive

A vault inside a folder named `OneDrive` or `OneDrive - <organisation>` shows **OneDrive** in the status bar.

- **Cloud-only notes** are not downloaded by indexing. They appear dimmed in the tree with ☁, and are read once OneDrive has them locally.
- **Saving** rewrites notes in place, so OneDrive keeps its version history.
- **Conflict copies**: when OneDrive makes `<note>-<COMPUTERNAME>.md` next to `<note>.md`, the original shows a banner. **Compare and merge** opens the resolve panel with every differing section; Apply puts the result in the note and moves the copy to `.trash`. A copy identical to the note goes straight to `.trash`. See [[Deleting notes]].
- **Names** Windows keeps for devices, like `CON` or `NUL`, are avoided when creating or renaming notes, since OneDrive cannot sync them.
