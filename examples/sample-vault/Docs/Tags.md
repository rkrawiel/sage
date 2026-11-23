---
title: Tags
type: doc
status: stable
area: organising
aliases: [Hashtags, Nested tags]
tags: [docs, docs/organising]
---
# Tags

Tags group notes across folders. Sage reads them from the text and from the frontmatter, and stores them in lowercase, so `#Docs` and `#docs` are one tag.

Back to the [[Sage guide]]. Practised in [[04 Tags and properties]].

## Writing tags

- **Inline**: `#reading` anywhere in the text. It shows in the secondary colour in the preview.
- **Frontmatter**: a `tags` list, as at the top of this note: `tags: [docs, docs/organising]`. Write them without `#` there.

Typing `#` in the source pane suggests existing tags, prefix matches first, then substring matches, the most used first. See [[Editing and autocomplete]].

## Nested tags

A `/` nests a tag under another: `#docs/organising` is inside `#docs`. Searching for a parent tag finds the children too, so every page of this guide is found by `tag:#docs`, whatever its area.

## Finding tagged notes

| Where | Write |
|---|---|
| [[View blocks]] | `source: "tag:#docs"` (quoted, because YAML reads `#` as a comment) |
| [[Tasks blocks]] | `tag = docs` |
| [[Search everywhere]] | just the tag's name |

In a `tasks` block, `tag = …` matches a task document's tags anywhere in its note, and a checkbox line's tags on that line.

Every page of this guide, by area:

```view
type: table
source: "tag:#docs"
columns: [file, area, status]
sort: area
```

## Status tags

Tags under `#status/` have a special job: they put checkbox tasks in Kanban columns, like `#status/in-progress`. See [[Kanban boards]].

## Related

- [[Properties]], the other kind of metadata.
- [[Links and backlinks]], which links here in three ways.
