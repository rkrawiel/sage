---
title: Deleting notes
type: doc
status: stable
area: organising
aliases: [Delete, Trash, .trash]
tags: [docs, docs/organising]
---
# Deleting notes

Sage never deletes a note outright. "Deleting" moves it into the vault's hidden `.trash` folder, as Obsidian does, where you can get it back.

Back to the [[Sage guide]]. Practised in [[09 Rename, move and delete]].

## How to delete

- Right-click a note or a folder in the tree and choose **Delete**.
- Run **Delete note (move to .trash)** from the [[Command palette]] for the active note.
- Press `Delete` when no editor or text field has focus, for the active note.

Each asks first. `Esc` closes the menu or the question.

## What happens

- The note, or the folder with everything in it, moves into `.trash` under a free name.
- Its tabs close.
- It disappears from the tree, the index and searches, since `.trash` is hidden.
- Links to it become unresolved and dimmed. See [[Links and backlinks]].

Deleting is refused while a note in it has unsaved edits.

## Getting a note back

Move the file out of `.trash` with your file manager. Sage notices, indexes it again, and links to it resolve once more.

## Related

- [[Rename and move]]
- [[External changes and merging]], for OneDrive conflict copies that Sage moves to `.trash` after merging.
