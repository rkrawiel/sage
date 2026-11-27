---
title: Preview and formatting
type: doc
status: stable
area: writing
aliases: [Preview, Callouts, Embeds, Markdown]
tags: [docs, docs/writing]
---
# Preview and formatting

The preview pane renders the note as you write it. Switch to it with `Ctrl+E`. This page shows most of what it can draw, so open it in split mode and compare. To write in the formatted note itself, use [[#Live preview]].

Back to the [[Sage guide]].

## Text

**Bold**, *italic*, ~~strikethrough~~, `inline code` and ==highlights== all work, and so do headings, lists, numbered lists, quotes, code blocks, horizontal rules and tables with alignment:

| Left | Centre | Right |
|:---|:---:|---:|
| notes | links | 3 |
| tags | tasks | 12 |

A `#tag` in the text shows in the secondary colour: #docs/writing

A highlight without its closing partner, like ==this, stays as text.

## Properties

The note's frontmatter shows at the top of the preview, each key beside its values. Wikilinks in values are links, `tags` are tags, and lists are joined with commas. See [[Properties]].

## Callouts

> [!note] A note callout
> Callouts are quotes that start with `[!kind]`.

> [!tip] Tips are green
> `tip` and `done` use the success colour.

> [!warning] Warnings stand out
> `question` and `warning` use the warning colour; `bug` and `danger` the error colour; `quote` and `example` the secondary colour; everything else the accent.

> [!example]- A folded callout
> A `-` after the kind starts the callout folded, a `+` starts it open. Click the title to open and close it. Sage remembers what you folded or opened by hand, per note, across restarts.

Titles keep their formatting, so a callout title can hold **bold**, a [[Links and backlinks|link]] or a #docs tag. Tasks inside callouts and quotes are real tasks:

> [!todo] Try it
> - [ ] Fold and unfold the example callout above, then restart Sage and check it stayed folded #tutorial

## Embeds

`![[Note]]` shows another note inside this one, in a frame with a link to it. `![[Note#Heading]]` shows one section and `![[Note#^block]]` one block. Here is the core shortcuts table from another page:

![[Keyboard shortcuts#Core shortcuts]]

Embedded notes show their own embeds, up to three notes deep. A note that embeds one above it says so instead of repeating forever.

Pictures (png, jpg, gif, webp and svg) embed with `![[picture.png]]` or `![alt](picture.png)`, at their natural size up to the page width, or at a width you give: `![[picture.png|300]]`. Pictures from the web are not fetched.

## Live preview

Live preview is one pane where you type and see the note formatted at once, as in Obsidian. `Ctrl+Shift+E` turns it on, and again goes back to the view you had. `Ctrl+E` cycles source, split, preview and live preview, and the [[Command palette]] has "View: live preview" and one command for each other view. Sage remembers the view for the next start.

- Headings show at their size; bold, italic, strikethrough, ==highlights==, inline code, links and tags show formatted.
- The Markdown is hidden except where the cursor is. A line's `#` and `>` show while the cursor is on that line; `**`, `[[ ]]`, a list's `-` and a task's `- [ ]` show while the cursor touches them, so you can edit them. A selection shows the syntax of every line it covers.
- Lists show bullets, tasks a checkbox you can click, quotes a bar, and callouts their title in the callout's colour on a shaded block. A callout without a title shows its kind.
- Clicking a link follows it. To edit a link, move onto it with the arrow keys.
- Tables, embedded notes and pictures, `view` and `tasks` blocks, and the properties show as the preview draws them, with links, checkboxes and Kanban cards that work as they do there. Move the cursor into one, with the arrow keys or by clicking beside a link or checkbox in it, and it shows as written so you can edit it.
- Other code blocks stay as written, in monospace.
- Autocomplete, find, autosave and merging work as in the source view, and the note on disk stays plain Markdown.

Callouts do not fold in live preview. Blocks inside a list or a quote, such as a table in a callout, stay as written.

## Links and checkboxes

- Wikilinks and Markdown links to notes are clickable, see [[Links and backlinks]].
- `http`, `https` and `mailto` links open in your browser or mail app. Other schemes are refused.
- Checkboxes in the preview can be clicked, see [[Tasks and checkboxes]].
- Fenced `view` and `tasks` blocks run queries, see [[View blocks]] and [[Tasks blocks]].
