---
title: Keyboard shortcuts
type: doc
status: stable
area: reference
aliases: [Shortcuts, Hotkeys, Keybindings]
tags: [docs, docs/reference]
---
# Keyboard shortcuts

Every shortcut Sage has, in one place. The same list is in the Shortcuts section of [[Settings]], together with your [[Quick Add]] shortcuts.

Back to the [[Sage guide]].

## Core shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+P` | [[Command palette]]: notes by name, full-text hits, create. Type `>` for commands |
| `Ctrl+Shift+F` | [[Search everywhere]]: fuzzy search over names, titles, aliases, tags, properties, headings and text |
| `Ctrl+N` | New note (`Untitled.md` in the vault root) |
| `Ctrl+Shift+N` | [[Quick Add]] |
| `Ctrl+S` | Save now (notes also autosave 1.5 s after the last keystroke) |
| `Ctrl+E` | Cycle source, split, preview and live preview |
| `Ctrl+Shift+E` | Live preview on, or back to the view before it, see [[Preview and formatting#Live preview]] |
| `Ctrl+F` | Find in note. `Enter` next, `Shift+Enter` previous |
| `Ctrl+T` | New task note in `Tasks/`, linked at the cursor, see [[Typed documents]] |
| `Ctrl+Shift+T` | New task line at the cursor, see [[Tasks and checkboxes]] |
| `F2` | Rename or move the note, see [[Rename and move]] |
| `Ctrl+W` | Close tab |
| `Ctrl+Shift+R` | Reopen the last closed tab, see [[Getting around]] |
| `Alt+←` and `Alt+→` | Back and forward through the notes you opened, see [[Getting around]] |
| `Ctrl+D` | Open today's daily note, creating it first, see [[Getting around#Today's daily note]] |
| `Ctrl+Shift+O` | Switch the sidebar between the folder tree and the note's outline |
| `F11` | Focus mode on and off. `Esc` also leaves it |
| `Ctrl+Shift+L` | Toggle light and dark |
| `Ctrl+=` and `Ctrl+-` | Zoom in and out, from 80% to 200%. `Ctrl+0` goes back to 100%. Kept for next time |

## Editing

These work in the source pane and in live preview. See [[Editing and autocomplete]].

| Shortcut | Action |
|---|---|
| `Ctrl+Z` | Undo. Typing close together undoes as one step |
| `Ctrl+Y` or `Ctrl+Shift+Z` | Redo |
| `Ctrl+B` | Bold on or off |
| `Ctrl+I` | Italic on or off |
| ``Ctrl+` `` | Inline code on or off |
| `Ctrl+K` | Make the selection a link, or a link back into text |
| `Enter` | New line that continues the list, numbered list, task or quote. On an empty item, ends the list |
| `Shift+Enter` | Plain line break |
| `Tab` and `Shift+Tab` | Indent and outdent list items and selected lines |
| `Alt+↑` and `Alt+↓` | Move the line, or the selected lines, up or down |
| `Ctrl+Shift+D` | Duplicate the line, or the selected lines, below |
| `Ctrl+V` | With a picture on the clipboard and no text: save it and embed it, see [[Pictures and attachments]] |

## Keys and clicks that always do the same

| Key or click | Action |
|---|---|
| `↑ ↓ Enter Tab Esc` | Move through and accept suggestions while the autocomplete popup is open |
| `Esc` | Closes the palette, else the find bar, else the task list |
| `Delete` | With no editor or text field focused: delete the active note, after asking. See [[Deleting notes]] |
| Middle click on a tab | Close the tab |
| Right click in the folder tree | Menu with "Rename or move", "Delete", "Show in Explorer" and "Copy link" |
| Mouse side buttons | Back and forward, as `Alt+←` and `Alt+→` |
| Dropping a picture file on the window | Save it and embed it, see [[Pictures and attachments]] |

## Your own shortcuts

In the Shortcuts section of [[Settings]], click any of Sage's core shortcuts and press the keys you want instead. Clear takes the shortcut away, and Reset gives back the usual keys. Your choices are kept in the vault's `.sage/settings.json`, so they travel with the vault.

[[Quick Add]] entries can each get a shortcut too: click the entry's Shortcut row and press the keys. `Esc` cancels, Clear removes it. A shortcut is saved as written, for example `Ctrl+Shift+P`.

A shortcut is refused, with the reason, when it:

- has no `Ctrl` and is not a function key, because it would fire while you type;
- is `Ctrl+Alt` with a key but no `Shift`, which is AltGr on many keyboards and types letters such as ą, ś or €;
- already belongs to one of Sage's own shortcuts;
- is one of the editing keys above, or `Ctrl+A`, `Ctrl+C` or `Ctrl+X`;
- belongs to another Quick Add entry.

Modifiers count exactly: `Ctrl+E` does not also fire on `Ctrl+Alt+E`, so AltGr letters never trigger commands.
