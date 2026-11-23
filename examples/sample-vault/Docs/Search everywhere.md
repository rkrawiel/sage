---
title: Search everywhere
type: doc
status: new
area: navigation
shortcut: Ctrl+Shift+F
aliases: [Omni search, Fuzzy search, Global search]
tags: [docs, docs/navigation, docs/search]
---
# Search everywhere

`Ctrl+Shift+F` opens **search everywhere**, a fuzzy search over the whole vault. Where the [[Command palette]] matches file names and plain text, search everywhere also looks at what is *about* a note:

- the note's file name and path
- its title (frontmatter `title`)
- its aliases
- its tags
- its frontmatter properties
- its headings
- and the note's text

> [!warning] New feature
> Search everywhere is being built right now. The details below describe the intent and may change.

## How matching works

The search is fuzzy, so a small typo still finds the note: `shortcts` should find [[Keyboard shortcuts]]. Every word you type has to match somewhere in a note, but the words may match in different places, one in a tag and another in a heading, for instance.

## Things to try in this vault

| Type | You should find | Because of its |
|---|---|---|
| `quick switcher` | [[Command palette]] | alias |
| `ctrl+shift+n` | [[Quick Add]] | `shortcut` property |
| `docs/tasks` | the task pages | tag |
| `resolve panel` | [[External changes and merging]] | heading |
| `kanban drag` | [[Kanban boards]] | text |
| `fix voice` | [[Voice reconnect]] | title |

The [[10 Finding things]] tutorial step walks through these.

## When to use which search

| Need | Use |
|---|---|
| Open a note whose name you know | `Ctrl+P` |
| Run a command | `Ctrl+P`, or `>` for commands only |
| Find a note by its title, alias, tag or a heading | `Ctrl+Shift+F` |
| Find a word in the open note | `Ctrl+F`, see [[Editing and autocomplete]] |
| A saved, live list of matching notes | a `view` block, see [[View blocks]] |

Back to the [[Sage guide]].
