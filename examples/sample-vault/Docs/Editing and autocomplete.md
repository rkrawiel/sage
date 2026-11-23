---
title: Editing and autocomplete
type: doc
status: stable
area: writing
aliases: [Editor, Autocomplete, Find in note]
tags: [docs, docs/writing]
---
# Editing and autocomplete

Back to the [[Sage guide]]. Practised in [[02 Your first note]] and [[03 Linking notes]].

## The source pane

You write Markdown in the **source pane**, in a monospace font with syntax highlighting. The **preview pane** shows the rendered note, see [[Preview and formatting]]. `Ctrl+E` cycles each tab between source, split and preview.

## Saving

- **Autosave** writes the note 1.5 s after your last keystroke.
- `Ctrl+S` saves at once.
- A tab with unsaved edits does not close; autosave finishes within seconds.
- If the file changed on disk while you were editing, Sage merges the two versions instead of overwriting either, see [[External changes and merging]].

## Autocomplete

Suggestions open while you type and close when you move the cursor away. `↑` and `↓` choose, `Enter` or `Tab` accepts, `Esc` closes. The popup sits at the bottom of the source pane.

| You type | Sage suggests |
|---|---|
| `[[` | notes, attachments and aliases |
| `![[` | notes and attachments to embed |
| `#` | tags: prefix matches first, then substring matches, the most used first |
| `](` | file paths for a Markdown link, with spaces as `%20` |

The inserted link follows Obsidian's *shortest unique* rule: just the note's name while that is unique in the vault, a path when it is not. See [[Links and backlinks]] and [[Tags]].

> [!tip] Aliases in autocomplete
> Typing `[[Hotkeys` suggests [[Keyboard shortcuts]], because `Hotkeys` is one of its aliases. Aliases are frontmatter, see [[Properties]].

## Formatting

These keys work in the source pane and in live preview.

| Key | What it does |
|---|---|
| `Ctrl+B` | `**bold**` |
| `Ctrl+I` | `*italic*` |
| ``Ctrl+` `` | `` `inline code` `` |

The key wraps the selection, or takes the markers off if it already has them, inside the selection or just outside it. With no selection it works on the word under the cursor; with no word it inserts an empty pair with the cursor between, and pressing it again removes the pair. Italic on bold text gives bold italic, `***text***`.

`Ctrl+K` makes a link. The selection becomes `[text]()` with the cursor in the parentheses. A selected URL becomes `[](url)`. A selected `[text](url)` goes back to `text`.

## Lists and quotes

`Enter` continues what the line started:

| Line | Next line |
|---|---|
| `- item`, `* item`, `+ item` | the same bullet |
| `1. item`, `1) item` | `2. `, `2) ` |
| `- [x] task` | `- [ ] `, a new open task |
| `> quote`, `> [!note] callout` | `> ` |

Text after the cursor moves to the new item. `Enter` on an empty item ends the list: a nested item first moves out one level, and an empty quote line ends one level of quote. `Shift+Enter` is a plain line break.

`Tab` and `Shift+Tab` indent and outdent a list item, or every line of a selection over several lines. Elsewhere `Tab` types an indent. One level is a tab, or spaces when Obsidian's `.obsidian/app.json` sets `useTab: false`: then `tabSize` spaces, 4 if it is not set.

## Moving lines

- `Alt+↑` and `Alt+↓` move the line, or the selected lines, up or down.
- `Ctrl+Shift+D` duplicates them below.

## Auto-closing pairs

| You type | You get |
|---|---|
| `(` | `()` with the cursor inside, before a space or at the end of the line |
| `[` | `[]` the same way, so `[[` gives `[[]]` and link suggestions still open |
| a backtick | a pair of backticks, except right after another backtick, so code fences still work |
| `**` | `****` with the cursor in the middle |

A single `*` is left alone, for bullets and italics. Typing a closing character that is already in front of the cursor steps over it. With a selection, typing `(`, `[`, a backtick or `*` wraps the selection instead of replacing it. `Backspace` between an empty pair deletes both sides.

To paste or drop a picture into a note, see [[Pictures and attachments]].

## Find in note

`Ctrl+F` opens a find bar over the source pane, prefilled with the selection when it is on one line.

- Matching is literal and ignores case.
- `Enter` goes to the next match, `Shift+Enter` to the previous one.
- The current match is drawn in the warning colour.
- `Esc` closes the bar and leaves the match selected. Switching tabs closes it too.

To search the whole vault, use the [[Command palette]] or [[Search everywhere]].

## New notes

`Ctrl+N` creates `Untitled.md` in the vault root, then `Untitled 1.md`, and so on. To create a note with a name, or in a folder, type the name in the [[Command palette]] and choose **Create note**.

## New task lines

`Ctrl+Shift+T` asks for a title and a due date and writes a checkbox task at the cursor. See [[Tasks and checkboxes]].
