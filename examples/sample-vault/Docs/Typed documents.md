---
title: Typed documents and task notes
type: doc
status: stable
area: tasks
aliases: [Task documents, Task notes, Document types]
tags: [docs, docs/tasks]
---
# Typed documents

A note whose frontmatter has a `type` is a **typed document** of that type. Nothing else about it changes: it is indexed, linked, renamed and searched like any note. The pages of this guide are `type: doc` and the tutorial steps are `type: tutorial`.

Back to the [[Sage guide]]. Practised in [[06 Task documents]].

## Task documents

`type: task` is the type Sage knows best. A task document is a whole note about one piece of work:

```yaml
---
id: fix-voice-reconnect
type: task
title: Fix voice reconnect bug
status: in-progress     # todo, in-progress, done, cancelled
created: 2026-11-16
due: 2026-11-30
tags: [backend, voice]
---
```

The sample vault has that one in `Tasks/`.

## Links by id

`[[fix-voice-reconnect]]` first looks for a note with that file name, as always, then for the note whose frontmatter `id` it is, ignoring case. So a task note can be renamed freely: links by id are never rewritten and keep finding it.

## How task links and embeds look

A link to a whole task document shows its status and title: [[fix-voice-reconnect]] reads as *[In Progress] Fix voice reconnect bug*. With `|` the alias replaces the title. A link to a heading inside it is a plain link.

An embed shows the title, then Status, Due and Tags, then the note's text:

![[write-release-notes]]

Links, embeds and `tasks` blocks are live: edit the task anywhere and every note that shows it updates.

## New task note: `Ctrl+T`

`Ctrl+T` opens the same form as a new task line: a title and a due date. Enter creates `Tasks/<id>.md` with `id`, `type: task`, the title, `status: todo`, `created` today and the due date, and puts `[[id]]` at the cursor. With no note open, it opens the new note.

The id is the title written as a slug, and the file is named after it so Obsidian finds `[[id]]` too. If the vault already has that id or file name, Sage adds `-2`, `-3`, and so on.

## Listing task documents

- A [[Tasks blocks|tasks block]] lists task documents together with checkbox lines.
- **Show tasks** lists them with the checkbox tasks; see [[Tasks and checkboxes]]. Checking one there sets `status: done`, unchecking puts back the status it had, else `todo`.
- A [[View blocks|view]] of `type: tasks` and a [[Kanban boards|Kanban board]] hold them as tasks: a board puts each in the column its `status` names, and dragging the card rewrites `status`. `kind:document` and `kind:checkbox` in the query pick one kind.
- A [[View blocks|view]] table filters by folder and properties, as in [[Tasks dashboard]]:

```view
type: table
source: "path:Tasks/ [type:task] -[status:done]"
columns: [file, title, status, due, tags]
sort: due
```

## Not done yet

- `[[` autocomplete does not offer ids.
- A task document's status cannot be changed from a link, embed or `tasks` block; check it in Show tasks, a view or a board, or edit its frontmatter.
- The folder for new task notes is always `Tasks/`.
