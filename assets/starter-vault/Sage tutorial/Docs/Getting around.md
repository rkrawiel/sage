---
title: Getting around
type: doc
status: stable
area: navigation
aliases: [Navigation, Back and forward, Daily note, Outline, Focus mode]
tags: [docs, docs/navigation]
---
# Getting around

Moving between notes, finding the ones you had open, and seeing the shape of the note in front of you.

Back to the [[Sage guide]].

## Back and forward

`Alt+←` and `Alt+→`, the mouse's side buttons, or **Go back** and **Go forward** in the [[Command palette]] move between notes in the order you opened them, as a browser does. Notes that no longer exist are skipped.

## Recent notes

`Ctrl+P` with nothing typed lists the notes you opened most recently, then the ones changed most recently. Type to search as usual.

## Reopening a closed tab

`Ctrl+Shift+R` opens the tab you closed last. (`Ctrl+Shift+T` is the new task line, see [[Tasks and checkboxes]].)

## Today's daily note

`Ctrl+D`, or **Open today's daily note** in the palette, opens today's note, creating it first if it does not exist.

Sage follows Obsidian's Daily notes settings in `.obsidian/daily-notes.json`: the folder, the date format of the name, and the template the new note starts from. Without them the note is `YYYY-MM-DD.md` in the vault root, such as `2026-11-18.md`. This vault sets the folder to `DailyNotes` and the template to [[Template, daily note]], so its daily notes go there.

To add a line to the daily note without opening it, use a [[Quick Add]] capture entry.

## Outline

`Ctrl+Shift+O`, or **Show outline** in the palette, switches the sidebar between the folder tree and the headings of the active note, indented by level. Clicking a heading puts the cursor on it. Sage remembers which of the two you had.

## Word count

The status bar shows the number of words and characters in the note, or in the selection when there is one.

## Focus mode

`F11`, or **Focus mode** in the palette, hides the folder tree, the tabs and the status bar, and centres the note at a comfortable reading width. `F11` or `Esc` leaves it.

## The tree menu

Right-click a note or folder in the tree for:

| Item | What it does |
|---|---|
| Rename or move | See [[Rename and move]] |
| Delete | Moves it to `.trash`, see [[Deleting notes]] |
| Show in Explorer | Shows it in Explorer. "Show in file manager" outside Windows |
| Copy link | Copies `[[Note]]` for a note, the path for a folder |

## Related

- [[Command palette]]
- [[Keyboard shortcuts]]
