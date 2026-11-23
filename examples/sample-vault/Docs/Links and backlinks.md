---
title: Links and backlinks
type: doc
status: stable
area: linking
aliases: [Wikilinks, Backlinks]
related: "[[Tags]]"
tags: [docs, docs/linking]
---
# Links and backlinks

Links are what turn a folder of notes into a second brain. Sage reads the same link syntax as Obsidian and keeps every link up to date when notes are renamed.

Back to the [[Sage guide]]. Practised in [[03 Linking notes]].

## Kinds of link

| Syntax | Links to |
|---|---|
| `[[Note]]` | the note named `Note`, anywhere in the vault |
| `[[folder/Note]]` | a note whose path ends in `folder/Note` |
| `[[Note#Heading]]` | a heading inside the note |
| `[[Note#^block]]` | a block marked `^block` |
| `[[Note\|shown text]]` | the note, shown with other text |
| `[text](Note.md)` | a Markdown link; spaces written as `%20` |
| `[text](./Note.md)` | a Markdown link relative to this note's folder; `../` goes up |
| `related: "[[Note]]"` | a link inside a frontmatter property |
| `![[Note]]` | an embed, see [[Preview and formatting]] |

All of them count as links: they show up as backlinks and are rewritten on rename. This note links to [[Tags]] three ways, as a wikilink here, through its `related` property, and as a relative Markdown link to the [tags page](./Tags.md).

Examples that work in this vault:

- [[Keyboard shortcuts#Core shortcuts]] jumps to a heading.
- [[Command palette|the palette]] shows other text.
- [[Docs/Settings]] uses a path.

## How a link finds its note

1. `[[Note]]` matches a note **file name** anywhere in the vault. When several notes share the name, the shortest path wins.
2. `[[folder/Note]]` must match the end of the note's path.
3. A bare `[[name]]` that matches no file name finds the note whose frontmatter `id` it is. Task documents use this, see [[Typed documents]].

A link to a note that does not exist yet stays **unresolved**. It is dimmed, and following it creates the note, as in Obsidian. It resolves by itself once the note appears.

## Following links

In the preview, click a link to open its note. A heading or block link opens the note with the cursor there. URLs open in your browser.

## Backlinks

Under the preview, two lists show the note's connections:

- **Linked from**: every note that links to or embeds this one (the backlinks).
- **Links to**: the notes this one links to, with unresolved targets dimmed.

Open [[Tags]] and look at its "Linked from" list: this page is there.

## Links survive renames

Renaming or moving a note in Sage rewrites every link to it, in the style it was written in. See [[Rename and move]].
