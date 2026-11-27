---
title: Tasks and checkboxes
type: doc
status: stable
area: tasks
aliases: [Checkbox tasks, Task list, Recurring tasks, Show tasks]
tags: [docs, docs/tasks]
---
# Tasks and checkboxes

Any checkbox line in any note is a task. Sage indexes them all, so you can write tasks where they belong, in a meeting note or a project page, and still see them together.

Back to the [[Sage guide]]. Practised in [[05 Tracking tasks]].

## Writing a task

```markdown
- [ ] Open task
- [x] Done task ✅ 2026-11-15
- [-] Cancelled task
- [ ] Task with a due date 📅 2026-11-30
- [ ] High priority, scheduled and started ⏫ ⏳ 2026-11-20 🛫 2026-11-18
- [ ] Water the plants 🔁 every week 📅 2026-11-20
```

Sage reads the Tasks plugin's emoji format:

| Marker | Meaning |
|---|---|
| `📅 date` | due |
| `⏳ date` | scheduled |
| `🛫 date` | start |
| `✅ date` | done on |
| `🔺 ⏫ 🔼 🔽 ⏬` | priority: highest, high, medium, low, lowest |
| `🔁 every …` | recurrence |
| `(@date)`, `(@date HH:MM)` | reminder, in the Reminder plugin's format: see Reminders below |

Dataview's inline fields, such as `[due:: 2026-11-30]`, are read as well; when Sage writes a date, it writes the emoji form. Dates are `YYYY-MM-DD`.

Task lines inside quotes and callouts (`> - [ ] …`) are tasks too.

## New task line: `Ctrl+Shift+T`

A small form asks for a title, a due date (`YYYY-MM-DD`, `today`, `tomorrow`, or nothing) and when to remind you (`9:30` on the due date or else today, `tomorrow 9:30`, `YYYY-MM-DD HH:MM`, or nothing). `Tab` moves between the fields, `Enter` writes the task, `Esc` cancels. The line, like `- [ ] Title 📅 2026-11-30 (@2026-11-30 09:30)`, goes on the cursor's line when it is blank, else on a new line below it, indented like it when it is a list item. It never goes into the frontmatter.

## Reminders

A task with `(@2026-11-30 09:30)` on its line reminds you then: Sage shows a system notification and a note at the bottom right with **Open**, which goes to the task. `(@2026-11-30)` without a time reminds at the default time, 09:00 unless [[Settings]] say otherwise. This is the Obsidian Reminder plugin's format, so the plugin reads the same lines. A [[Typed documents|task document]] reminds with a `reminder: 2026-11-30 09:30` property.

Only open tasks remind: checking or cancelling one stops it. Sage reminds while it is running; reminders that went off while it was closed are gathered into one notification at the next start. Settings > Tasks can turn the system notifications off, and sets the default time.

## Checking a task

Click the checkbox in the preview. Checking writes `[x]` and `✅ <today>`; unchecking removes both. It works in a view, a `tasks` block, a Kanban board and the task list as well.

Sage only rewrites the line if it still says what the index read, and merges with other edits to the note, so a task checked from a dashboard never overwrites newer work.

## The task list: Show tasks

Run **Show tasks** from the [[Command palette]] for every open task in the vault, grouped under **Overdue**, **Today**, **Upcoming** and **No due date**, then the ones completed today. Click a task's text to open its note on that line. `Esc` closes the list.

[[Typed documents|Task documents]] are in the list too, with their title and tags, their status in brackets when it is neither todo nor done (`[In Progress] Fix voice reconnect bug`), and their `due` date. One is open unless its `status` is `done` or `cancelled`; a done one counts as completed today when its note changed today, since documents have no completion date. Clicking its title opens the note. Its checkbox sets `status: done`, and unchecking puts back the status it had before it was checked in this session, else `todo`. Like a line, the status is only rewritten when the note has no unsaved edits and the status still says what the index read.

## Recurring tasks

Checking a task with `🔁 every …` (or `[repeat:: every …]`) inserts the next occurrence above it. Its dates, a reminder's included, move forward by the same number of days as its due date moves to the next date; with `when done`, from today instead.

Rules Sage understands:

- `every day`, `every 3 days`, `every week`, `every 2 months`, `every year`
- `every other week`
- `every weekday`
- `every Monday`, `every Monday, Friday`, `every week on Tuesday`
- `every month on the 15th`, `every month on the last`

Anything else just checks the task.

## Seeing tasks elsewhere

- [[Tasks blocks]]: a `tasks` code block lists checkbox tasks and task documents with filters.
- [[View blocks]]: `type: tasks` lists tasks matching a query.
- [[Kanban boards]]: tasks as cards in columns.
- [[Typed documents]]: a whole note can be one task, with its own status and due date.
