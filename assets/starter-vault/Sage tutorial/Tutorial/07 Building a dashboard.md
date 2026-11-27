---
title: "Step 7: Building a dashboard"
type: tutorial
step: 7
status: todo
tags: [tutorial]
---
# Building a dashboard

A dashboard in Sage is a note with view blocks in it. They are queries that the preview turns into tables, task lists and boards, and they stay live as your notes change.

## Try it: drag cards on a board

These tasks are the cards of the board below:

- [ ] Sketch a dashboard layout #tutorial #status/in-progress 📅 2026-11-21
- [ ] Add a notes table #tutorial #status/backlog 📅 2026-11-24
- [ ] Add a task list #tutorial
- [x] Read about view blocks #tutorial ✅ 2026-11-17

```view
type: kanban
source: "path:Tutorial/07"
columns: [Backlog, In Progress, Done]
```

1. Drag **Add a notes table** from Backlog to **In Progress**. In the source, the line gains `#status/in-progress`.
2. Drag it to **Done**. The line is checked, gets today's `✅` date, and loses the status tag.
3. Drag it back to **Backlog**. It is unchecked again.
4. Drag a card up or down within its column. It stays where you drop it.

## Try it: build your own

Make a note `Playground/My dashboard` (`Ctrl+P`, type the name, **Create note**) and paste these blocks into it, one at a time, watching the preview.

Open tasks, soonest first:

````markdown
```view
type: tasks
source: "task-todo:"
columns: [task, file, due, priority]
```
````

Every note in the playground:

````markdown
```view
type: table
source: "path:Playground/"
columns: [file, title, modified]
sort: modified desc
```
````

Tasks overdue or due this week:

````markdown
```view
type: tasks
source: "task-todo: due<=2026-11-23"
```
````

Then change something: edit a query, add a column such as `tags` or `backlinks`, or make a typo in a column name and see the error the block shows in its place.

> [!example] Look at the real ones
> [[Dashboard]] and [[Tasks dashboard]] are dashboards built this way, and [[Alpha]] has a Kanban board for a project.

Read more: [[View blocks]], [[Kanban boards]].

---

← [[06 Task documents]] · [[Sage guide|Contents]] · Next: [[08 Templates and Quick Add]] →
