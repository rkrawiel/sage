---
title: Command palette
type: doc
status: stable
area: navigation
shortcut: Ctrl+P
aliases: [Palette, Quick switcher]
tags: [docs, docs/navigation]
---
# Command palette

`Ctrl+P` opens the palette: one search box for opening notes, creating notes and running commands. `Esc` closes it.

Back to the [[Sage guide]]. Tried out in [[02 Your first note]] and [[10 Finding things]].

## Opening notes

Type part of a note's name. The palette lists notes by **file name**, as the tree and tabs do, because links find notes by file name. When a note's H1 heading or frontmatter `title` differs from its file name, the palette shows it beside the name as a hint.

Below the name matches come **full-text hits**: notes whose text contains what you typed. `↑` and `↓` move through the list, `Enter` opens.

For fuzzy search across titles, aliases, tags, properties and headings as well, use [[Search everywhere]] (`Ctrl+Shift+F`).

## Creating notes

Type a name that no note has and the palette offers **Create note**. A `/` in the name creates folders, so `Projects/Beta/Kickoff` creates both folders and the note. Characters Windows rejects in file names are dropped.

`Ctrl+N` is the quick way to make an untitled note: it creates `Untitled.md` (then `Untitled 1.md`, …) in the vault root.

## Running commands

Type `>` first to list commands only. Without `>`, commands and [[Quick Add]] entries are found by name too, from the third letter you type, after the notes whose name matches. A Quick Add entry with a shortcut shows it beside its name.

Commands you will meet in this guide:

| Command | What it does | Docs |
|---|---|---|
| Show tasks | The vault-wide task list | [[Tasks and checkboxes]] |
| Insert template | Inserts a note from the templates folder | [[Using templates]] |
| Settings | Opens the settings in place of the note | [[Settings]] |
| Undo last rename | Reverses the newest rename, links included | [[Rename and move]] |
| Rename or move this note's folder | Moves the folder of the active note | [[Rename and move]] |
| Delete note (move to .trash) | Deletes the active note, after asking | [[Deleting notes]] |
| Overwrite file on disk | Keeps your version when a note changed elsewhere | [[External changes and merging]] |
| Open vault folder… | Switches to another vault | [[Getting started]] |
| Go back, Go forward | Moves between notes in the order you opened them | [[Getting around]] |
| Open today's daily note | Opens today's note, creating it first | [[Getting around]] |
| Reopen closed tab | Opens the tab closed last | [[Getting around]] |
| Show outline | Shows the note's headings in place of the tree | [[Getting around]] |
| Focus mode | Only the note, at a comfortable width | [[Getting around]] |

A long list scrolls.

## Related

- [[Keyboard shortcuts]]
- [[Search everywhere]]
