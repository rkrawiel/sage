---
title: Using templates
type: doc
status: stable
area: automation
aliases: [Templates, Insert template]
tags: [docs, docs/automation]
---
# Using templates

A template is an ordinary note in your templates folder. **Insert template** in the [[Command palette]] lists the notes in that folder and inserts the chosen one at the cursor, filling in its variables. [[Quick Add]] uses the same templates to create whole notes.

Back to the [[Sage guide]]. Practised in [[08 Templates and Quick Add]].

## The templates folder

Choose it in [[Settings]] from the vault's folders. Until you do, Sage uses the folder set in Obsidian's `.obsidian/templates.json`, and `Templates` when there is none. This vault has one template, [[Meeting]].

## Variables

| Variable | Becomes |
|---|---|
| `{{title}}` | the note's name |
| `{{date}}` | today, in the date format from `.obsidian/templates.json` |
| `{{time}}` | now, in the time format from the same file |
| `{{date:YYYY-MM-DD}}` | today in the format given, with Moment.js tokens |
| `{{time:HH:mm}}` | now in the format given |

The [[Meeting]] template's first line is `## {{title}}, {{date:dddd D MMMM YYYY}}`, which becomes something like *Weekly sync, Tuesday 17 November 2026*.

## Templates with properties

A template's frontmatter never lands at the cursor. Its properties go into the note's own frontmatter instead:

- keys the note lacks are added as written;
- list items the note lacks join its lists, so `tags` grow;
- values the note already has stay.

If the cursor is inside the frontmatter, the template's body goes right after it.

## Related

- [[Quick Add]] creates a note from a template in one keystroke.
- [[Properties]] explains frontmatter.
