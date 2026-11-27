---
title: Properties and frontmatter
type: doc
status: stable
area: organising
aliases: [Frontmatter, YAML, Metadata]
tags: [docs, docs/organising]
---
# Properties

Properties are the YAML block at the very top of a note, between two `---` lines. Obsidian calls them properties; they are also called frontmatter. Every page of this guide has some; this one's are:

```yaml
---
title: Properties and frontmatter
type: doc
status: stable
area: organising
aliases: [Frontmatter, YAML, Metadata]
tags: [docs, docs/organising]
---
```

Back to the [[Sage guide]]. Practised in [[04 Tags and properties]].

## Keys Sage knows

| Key | Effect |
|---|---|
| `title` | Shown as a hint beside the file name in the [[Command palette]]; found by [[Search everywhere]] |
| `aliases` | Other names, offered by `[[` autocomplete and found by search everywhere |
| `tags` | Tags of the whole note, see [[Tags]] |
| `type` | Makes the note a typed document, see [[Typed documents]] |
| `id` | A stable name that `[[links]]` can use, see [[Typed documents]] |
| `status`, `created`, `due` | Read for typed documents |

Any other key is yours to invent. `area` on these pages is one: the table in [[Tags]] shows and sorts by it.

## In the preview

The preview shows the properties at the top of the note, each key beside its values. A `[[link]]` in a value is a real link: it can be followed, appears in backlinks and is rewritten on rename. Lists are joined with commas.

## Querying properties

In a [[View blocks|view block]] query:

| Query | Matches notes |
|---|---|
| `[status]` | that have a `status` property |
| `[status:active]` | whose `status` is `active`, ignoring case |
| `[estimate>3]` | whose `estimate` is more than 3, compared as numbers |
| `[type:doc] -[status:stable]` | docs that are not yet stable |

A notes table can show any property as a column, `columns: [file, status, area]`, and sort by it.

The docs that are not marked stable yet:

```view
type: table
source: "[type:doc] -[status:stable]"
columns: [file, title, status]
```

## Broken YAML

When the frontmatter is not valid YAML, the note's text is still indexed and searchable, but its properties are left out until the YAML is fixed.

## Templates add properties

Inserting a template with properties merges them into the note's frontmatter, see [[Using templates]].
