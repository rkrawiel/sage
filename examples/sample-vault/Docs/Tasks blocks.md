---
title: Tasks blocks
type: doc
status: stable
area: queries
aliases: [Task queries]
tags: [docs, docs/queries, docs/tasks]
---
# Tasks blocks

A fenced code block with the language `tasks` lists tasks of both kinds, as the Obsidian Tasks plugin does: [[Typed documents|task documents]], and checkbox lines from every note (see [[Tasks and checkboxes]]).

Back to the [[Sage guide]]. Practised in [[06 Task documents]].

## Writing one

````markdown
```tasks
status != done
due < 2026-12-01
tag = backend
sort due
```
````

One instruction per line, and every filter must hold.

| Instruction | Meaning |
|---|---|
| `status = todo`, `status != done` | the four statuses: `todo`, `in-progress`, `done`, `cancelled` |
| `due < 2026-12-01`, `created >= yesterday` | compare with `=`, `!=`, `<`, `<=`, `>`, `>=` against a date, `today`, `tomorrow` or `yesterday` |
| `due = none`, `due != none` | a missing or a present date |
| `tag = backend`, `tag != voice` | the tag or one nested under it |
| `folder = Tasks`, `folder != Templates` | the folder or one inside it, ignoring case |
| `sort due`, `sort title desc` | sort by `due`, `created`, `status`, `title` or `id`, then `asc` or `desc`; several `sort` lines sort by each in turn |
| `limit 20` | 1 to 1000, default 200 |

Without a `sort`, tasks go by due date, those without one last. An error names the line it is on.

A checkbox line's status is `done` when checked, `cancelled` for `[-]`, else its `#status/` tag, else `todo`. A task document's tags count anywhere in its note; a checkbox line's tags only on that line.

## In the list

Each line has a checkbox. For a checkbox task it checks the line in its note, with the same guarded write as everywhere. Clicking the text opens the note on that line. Inside an embedded note, a `tasks` block shows as code.

## Example

Open tutorial tasks, by due date:

```tasks
status != done
status != cancelled
tag = tutorial
sort due
```

## Related

- [[View blocks]] for notes tables and Kanban boards.
- [[Tasks dashboard]] in this vault uses both kinds of block.
