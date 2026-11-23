---
title: Rename and move
type: doc
status: stable
area: organising
shortcut: F2
aliases: [Rename, Move note, Undo last rename]
tags: [docs, docs/organising]
---
# Rename and move

Renaming a note in Sage rewrites every link to it, across the whole vault, as one step you can undo.

Back to the [[Sage guide]]. Practised in [[09 Rename, move and delete]].

## Renaming a note

Press `F2`. The palette opens on the note's path with the name selected. Type the new name and press `Enter`.

- **Moving**: type a folder path, like `Archive/Old idea`. Missing folders are created.
- **Case only**: `note` to `Note` works, on Windows too.
- **Device names**: names Windows reserves for devices (`CON`, `AUX`, `NUL`, `COM1`, …) get a `_`, so OneDrive can sync them.

You can also right-click the note in the folder tree and choose **Rename or move**.

## What happens to links

Every link to the note is rewritten in the style it was written in:

- a bare `[[Name]]` stays bare while the name is unique;
- a path stays a path;
- a relative Markdown link stays relative, with `.md` and `%20` kept as written;
- a link elsewhere that the new name would capture is pinned to its own note with a full path.

The renamed note's own relative links follow it. Links by frontmatter `id` are not touched, since they do not use the name: see [[Typed documents]].

Notes with unsaved edits are never rewritten underneath you; Sage asks you to save first. An open note that the rename rewrote follows along and keeps its cursor.

## Undo last rename

The status bar shows the rename and how to undo it for a few seconds. **Undo last rename** in the [[Command palette]] moves the note back and restores every rewritten note exactly, newest rename first. It is refused, with nothing changed, if one of those notes changed since. Renames can be undone until you close Sage.

## Renaming a folder

Right-click a folder in the tree, or run **Rename or move this note's folder** from the palette. Everything in it moves in one rename, and links into, out of and inside it are rewritten by the same rules. A bare link the move would send to another note of the same name gets a path. One undo step, like a note.

## Renames outside Sage

A note renamed or moved in Obsidian or your file manager is recognised by its content and keeps its backlinks in the index, but links in other notes are not rewritten. Rename in Sage to keep links right.

## Related

- [[Deleting notes]]
- [[Links and backlinks]]
