---
title: Quick Add
type: doc
status: stable
area: automation
shortcut: Ctrl+Shift+N
aliases: [Capture, Quick capture]
tags: [docs, docs/automation]
---
# Quick Add

Quick Add entries turn something you do often, like starting a meeting note or jotting a line into today's daily note, into one keystroke. You make them in [[Settings]]; they are kept in the vault's `.sage/settings.json`, so they travel with the vault.

Back to the [[Sage guide]]. Practised in [[08 Templates and Quick Add]].

## Running an entry

- the **+** beside the vault's name,
- `Ctrl+Shift+N`,
- the entry's own shortcut, if you gave it one,
- or its name in the [[Command palette]].

When an entry needs a `{{value}}`, the palette asks for it and shows the note that `Enter` will create or add to.

## Template entries

A template entry creates a new note from a template. It has:

| Field | Example |
|---|---|
| template | [[Meeting]] |
| folder | `Meetings` |
| file name pattern | `{{date:YYYY-MM-DD}} {{value}}` |

Running it with the value `Weekly sync` creates `Meetings/2026-11-17 Weekly sync.md` with the template filled in, `{{title}}` being the new note's name, and opens it. A note that already exists is left alone. See [[Using templates]] for the variables.

## Capture entries

A capture entry adds a line to a note. It has:

| Field | Example |
|---|---|
| note | `Daily/{{date:YYYY-MM-DD}}` |
| heading (optional) | `## Log` |
| text | `- {{value}}` |
| where | top or end |
| create the note if missing | yes or no |

With a heading, the text goes under it, at the top of the section or after its last line; if the note lacks the heading, the heading is added at the end. Without one, the text goes at the top of the note after its frontmatter, or at the end. A note with unsaved edits is not written to.

To open today's daily note itself, press `Ctrl+D`, see [[Getting around#Today's daily note]].

## Shortcuts for entries

Each entry has a Shortcut row in the settings. Click it and press the keys. See [[Keyboard shortcuts#Your own shortcuts]] for which keys are allowed.

## Not done yet

Quick Add's macros, prompts for choosing a file, and reordering entries.
