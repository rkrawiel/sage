---
title: "Step 6: Task documents"
type: tutorial
step: 6
status: todo
tags: [tutorial]
---
# Task documents

A checkbox is fine for "buy milk". Bigger work deserves its own note, with a status, a due date and room for details. A note with `type: task` in its properties is a **task document**.

## Look first

- This link points at a task document: [[fix-voice-reconnect]]. The preview shows its **status and title**, not the raw link.
- This one is your tutorial task: [[finish-the-sage-tutorial]].

And this embeds it, showing its status, due date, tags and text:

![[finish-the-sage-tutorial]]

The links use the task's `id` property, not its file name. That is why they keep working even when the file is renamed.

## Try it: create a task note

1. Open **My first note** and put the cursor on an empty line.
2. Press `Ctrl+T`. The same form as before asks for a title and a due date.
3. Type `Try task documents`, `Tab`, `tomorrow`, `Enter`.
4. Sage creates `Tasks/try-task-documents.md` and writes `[[try-task-documents]]` at the cursor.

## Try it: change its status

1. Follow the new link to open the task note.
2. In its properties, change `status: todo` to `status: in-progress`.
3. Go back to **My first note**. The link now reads *[In Progress] Try task documents*.

## Try it: list tasks with a tasks block

A `tasks` block lists task documents and checkbox lines together. This one shows the open tutorial tasks; your new task note is not tagged `tutorial`, so add `tags: [tutorial]` to its properties and watch it appear:

```tasks
status != done
status != cancelled
tag = tutorial
sort due
```

Read more: [[Typed documents]], [[Tasks blocks]].

---

← [[05 Tracking tasks]] · [[Sage guide|Contents]] · Next: [[07 Building a dashboard]] →
