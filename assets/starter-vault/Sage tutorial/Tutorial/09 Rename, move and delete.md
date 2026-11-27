---
title: "Step 9: Rename, move and delete"
type: tutorial
step: 9
status: todo
tags: [tutorial]
---
# Rename, move and delete

Reorganising a vault is only safe when links follow the notes around. In Sage they do, and every rename can be undone.

## Try it: rename

1. Open **My second note**, the one you created from a dimmed link in [[03 Linking notes]]. It links to **My first note**.
2. Switch to **My first note** and press `F2`. The palette opens on the note's path with the name selected.
3. Type `Practice note` (keep the `Playground/` in front) and press `Enter`.
4. Open **My second note** again: its link now reads `[[Practice note]]`. Every link in the vault was rewritten.

## Try it: undo

1. Read the status bar: it says how to undo the rename.
2. Open the palette and run **Undo last rename**. The note gets its old name back and the link in **My second note** is restored exactly.

## Try it: move

1. With **My first note** open, press `F2` again.
2. Change the path to `Playground/Archive/My first note` and press `Enter`. The missing `Archive` folder is created, the note moves into it, and links follow.
3. Right-click the `Playground` folder in the tree and choose **Rename or move** to move a whole folder the same way. (Skip this one if you like; leave it named `Playground` for the next steps.)

## Try it: delete

1. Right-click **My second note** in the tree and choose **Delete**. Sage asks first; confirm.
2. The note moves to the vault's hidden `.trash` folder, its tab closes, and the link to it in **My first note** is now dimmed.
3. To get it back, move the file out of `.trash` with your file manager. The link lights up again.

> [!warning] Unsaved edits
> Sage refuses to rename or delete a note with unsaved edits. Autosave gets there within seconds, or press `Ctrl+S`.

Read more: [[Rename and move]], [[Deleting notes]].

---

← [[08 Templates and Quick Add]] · [[Sage guide|Contents]] · Next: [[10 Finding things]] →
