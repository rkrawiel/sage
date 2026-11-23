---
title: View blocks and the query language
type: doc
status: stable
area: queries
aliases: [Views, Queries, Query language, DSL]
tags: [docs, docs/queries]
---
# View blocks

A fenced code block with the language `view` runs a query against the index and shows the result in the preview: a table of notes, a list of tasks or a [[Kanban boards|Kanban board]]. It refreshes whenever the index changes, so a view is a live dashboard written in plain text. [[Dashboard]] and [[Tasks dashboard]] are built from them.

Back to the [[Sage guide]]. Practised in [[07 Building a dashboard]].

## Writing a view

````markdown
```view
type: table                 # table (notes), tasks, or kanban
source: "path:Docs/"        # a query; empty or missing means everything
columns: [file, title, status]
sort: file                  # table and tasks only; add desc to reverse
limit: 50                   # 1 to 1000, default 200
```
````

And here is that block for real, listing this guide:

```view
type: table
source: "path:Docs/"
columns: [file, title, status]
sort: file
```

> [!warning] Quote queries with `#` or `:`
> YAML reads `#` as the start of a comment, so write `source: "tag:#docs"` with the quotes.

A block with a mistake shows the error in its place: unknown keys, types and columns are reported.

## Types and columns

| Type | Shows | Columns | Default |
|---|---|---|---|
| `table` | notes | `file`, `path`, `folder`, `title`, `modified`, `size`, `tags`, `backlinks`, or any frontmatter key | `[file, modified]` |
| `tasks` | tasks with checkboxes | `task`, `file`, `path`, `due`, `scheduled`, `start`, `done`, `priority`, `status` | `[task, file, due]`, by due date then priority |
| `kanban` | a board of tasks | `columns` are the board's column names | see [[Kanban boards]] |

A table sorts by any of its columns; a property sorts by its first value. Cancelled tasks (`[-]`, or `status: cancelled`) appear in none of the types.

## Task documents in task lists and boards

A `tasks` list and a `kanban` board hold [[Typed documents|task documents]] (notes with `type: task`) as well as checkbox lines. A document's `task` is its title; its `status` and `due` are its properties; it has no priority, scheduled, start or done date. Its checkbox sets `status: done`, and unchecking puts back the status it had before, else `todo`. Add `kind:checkbox` to a query to leave documents out, or `kind:document` for documents only.

## The query language

The `source` query follows Obsidian's search syntax.

| Syntax | Matches |
|---|---|
| `word`, `"a phrase"`, `content:word` | notes whose text contains it; a word also matches as a prefix |
| `file:name`, `path:Projects/` | file name or vault path contains it, ignoring case |
| `tag:#project` | notes with the tag, or one nested under it |
| `[status]`, `[status:active]`, `[estimate>3]` | frontmatter properties: present, equal (ignoring case), compared |
| `task:text`, `task-todo:text`, `task-done:text` | tasks containing text: any, open, done; the text may be empty |
| `status:in-progress`, `status:none` | tasks by their `#status/` tag |
| `due<today`, `scheduled:2026-12-01`, `start>=tomorrow`, `done:yesterday` | task dates; also `due:none` and `due:any` |
| `priority>=high`, `priority:none` | lowest, low, medium, high, highest, or 1 to 5 |
| `kind:document`, `kind:checkbox` | task documents (`type: task`), or checkbox lines |
| `a b`, `a OR b`, `-a`, `( … )` | and, or, not, grouping |

In a task list or a board, note filters (like `path:` or `tag:`) apply to the task's note. Task filters read a task document as a task: `task:` its title, `task-todo:` and `task-done:` its `status` (done is `done`), `status:` its `status` property, `due` its `due` property. In a notes table, task filters match notes that have at least one checkbox task meeting all of them together, so there `kind:document` matches nothing.

## Examples

Open tasks from the tutorial, soonest first:

```view
type: tasks
source: "path:Tutorial/ task-todo:"
columns: [task, file, due, priority]
```

More to copy:

| Want | `source` |
|---|---|
| Open tasks due this week or earlier | `"task-todo: due<=2026-11-23"` |
| Overdue high-priority tasks | `"due<today priority>=high"` |
| Notes in Projects tagged alpha | `"path:Projects/ tag:#project/alpha"` |
| Tutorial steps not finished yet | `"path:Tutorial/ -[status:done]"` |
| Notes mentioning Kanban or boards | `"kanban OR board"` |
| Open task documents, no checkbox lines | `"kind:document task-todo:"` |
| Checkbox tasks only, as before task documents | `"kind:checkbox"` |

## Related

- [[Tasks blocks]] use a simpler, line-based language for task overviews.
- [[Properties]] explains the `[key:value]` filters.
- [[Search everywhere]] is for finding a note once, not keeping a list.
