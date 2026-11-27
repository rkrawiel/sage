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

Choose it in [[Settings]] from the vault's folders. Until you do, Sage uses the folder set in Obsidian's `.obsidian/templates.json`, and `Templates` when there is none. This vault keeps its templates in `Templates`: [[Template, regular note]], [[Template, daily note]], [[Template, meeting note]], [[Template, project]], [[Template, person]] and [[Template, book]].

## Variables

| Variable | Becomes |
|---|---|
| `{{title}}` | the note's name |
| `{{date}}` | today, in the date format from `.obsidian/templates.json` |
| `{{time}}` | now, in the time format from the same file |
| `{{date:YYYY-MM-DD}}` | today in the format given, with Moment.js tokens |
| `{{time:HH:mm}}` | now in the format given |
| `<% tp.date.now("FORMAT") %>`, `<% tp.file.creation_date("FORMAT") %>`, `<% tp.file.title %>` | the commonest Templater tags, read the same way; [[Template, meeting note]] uses one for its `Date` |

The [[Template, daily note]] template asks for the tasks `due <= {{date:YYYY-MM-DD}}`, which becomes something like `due <= 2026-11-17` in the note made that day.

## Templates with properties

A template's frontmatter never lands at the cursor. Its properties go into the note's own frontmatter instead:

- keys the note lacks are added as written;
- list items the note lacks join its lists, so `tags` grow;
- values the note already has stay.

If the cursor is inside the frontmatter, the template's body goes right after it.

## Related

- [[Quick Add]] creates a note from a template in one keystroke.
- [[Properties]] explains frontmatter.
