---
title: "Step 10: Finding things"
type: tutorial
step: 10
status: todo
aliases: [Search tutorial]
tags: [tutorial]
---
# Finding things

A vault grows quickly. Sage has three ways to find something, each for a different question.

## Try it: the palette, `Ctrl+P`

The palette is for opening a note when you know roughly what it is called.

1. Press `Ctrl+P` and type `short`. **Keyboard shortcuts** comes first.
2. Type `three-way` instead. No file is called that, but [[External changes and merging]] contains it, so it shows as a **full-text** hit.
3. Type `>` on its own. Now the palette lists **commands** only. Scroll through them.
4. Type `settings` without the `>`. The Settings command appears after any notes with that name.

## Try it: search everywhere, `Ctrl+Shift+F`

Search everywhere is fuzzy and looks at much more than names: titles, aliases, tags, properties, headings and the text. Try these and see where each match comes from:

| Type | You should find | It matched the note's |
|---|---|---|
| `quick switcher` | [[Command palette]] | alias |
| `ctrl+shift+n` | [[Quick Add]] | `shortcut` property |
| `docs/tasks` | the task pages | tag |
| `resolve panel` | [[External changes and merging]] | heading |
| `fix voice` | [[Voice reconnect]] | title |
| `shortcts` (a typo) | [[Keyboard shortcuts]] | name, despite the typo |

Every word has to match somewhere in a note, but not all in the same place.

> [!info] Brand new
> Search everywhere is the newest part of Sage and is still being built; see [[Search everywhere]] for its current state.

## Try it: find in the note, `Ctrl+F`

Open [[View blocks]], press `Ctrl+F` and type `kanban`. `Enter` steps through the matches in that note only.

## And when you need it again and again

A search you keep repeating belongs in a [[View blocks|view block]]: it is saved in a note and stays up to date. You built some in [[07 Building a dashboard]].

Read more: [[Command palette]], [[Search everywhere]], [[Editing and autocomplete]].

---

← [[09 Rename, move and delete]] · [[Sage guide|Contents]] · Next: [[11 Wrapping up]] →
