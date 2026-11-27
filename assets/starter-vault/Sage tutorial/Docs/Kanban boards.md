---
title: Kanban boards
type: doc
status: stable
area: queries
aliases: [Kanban, Boards]
tags: [docs, docs/queries, docs/tasks]
---
# Kanban boards

A Kanban board is a [[View blocks|view block]] with `type: kanban`. Its cards are checkbox task lines (see [[Tasks and checkboxes]]) and task documents (see [[Typed documents]]); there is no separate board file to keep in sync.

Back to the [[Sage guide]]. Practised in [[07 Building a dashboard]]. [[Alpha]] has a board for a real project.

## Writing a board

````markdown
```view
type: kanban
source: "tag:#project/alpha"
columns: [Backlog, In Progress, Done]
```
````

- `columns` names the columns, at least two.
- Each column's slug comes from its name: "In Progress" is `in-progress`.
- `group_by` may be left out; when given, it must be `status`.

## Which column a card is in

1. A checked task, or a task document with `status: done`, is in the **last** column.
2. Else the column whose slug its `#status/` tag names, or a document's `status`: `#status/in-progress` and `status: in-progress` are "In Progress".
3. Else the **first** column.

A task document's card shows its title. Cancelled ones (`status: cancelled`) are left out, as `[-]` lines are. Add `kind:checkbox` to the `source` to keep documents off a board.

## Moving cards

Drag a card onto another column and Sage rewrites its line in the note:

- into the last column, the task is checked, gets `✅ <today>` and loses its status tag;
- into any other column, it gets `#status/<slug>` before its Tasks metadata, and is unchecked.

A task document's card gets its `status` property set instead: `status: done` in the last column, `status: <slug>` in any other, written where the [[Typed documents|new task form]] writes it if the note had none. Name the columns after the statuses, `[Todo, In Progress, Done]`, so the documents keep statuses that links and `tasks` blocks know. As with lines, Sage refuses when the note has unsaved edits or its status changed since the index read it.

The board shows the move at once, before the index catches up. Clicking a card without dragging opens its line.

## Ordering cards by hand

Drag a card within its column to put it where you drop it; a rule in the gap shows the place. Dropping on another column places it there too. Cards you placed come first, in your order, then the others by due date and priority. Task document cards are placed the same way.

The order is kept in `kanban-order.json` next to the index, never in the vault. It follows a card across columns and boards. Editing a card's text forgets its place.

## Example

The task documents in `Tasks/`. Drag one to see its `status` change:

```view
type: kanban
source: "path:Tasks/ kind:document"
columns: [Todo, In Progress, Done]
```

The tutorial's practice cards. Drag one to see its line change in [[07 Building a dashboard]]:

```view
type: kanban
source: "path:Tutorial/"
columns: [Backlog, In Progress, Done]
```
