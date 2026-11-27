---
title: "Step 5: Tracking tasks"
type: tutorial
step: 5
status: todo
tags: [tutorial]
---
# Tracking tasks

Any checkbox line in any note is a task. Sage finds them all, so you can write tasks where they come up and still see them in one list.

## Practice tasks

These are real tasks. They show up in the [[Dashboard]], the [[Tasks dashboard]] and "Show tasks".

- [ ] Check this task in the preview #tutorial 📅 2026-11-18
- [ ] Plan the week ahead #tutorial ⏫ 📅 2026-11-19
- [ ] Review the tutorial notes #tutorial 🔁 every week 📅 2026-11-20
- [-] A cancelled task: views and boards leave it out #tutorial

## Try it: check and uncheck

1. In the preview, click the box of **Check this task in the preview**.
2. Look at the source: the line now reads `- [x] … ✅ 2026-11-17`, with today's date.
3. Click again to uncheck it. The `✅` date goes away.

## Try it: a recurring task

Check **Review the tutorial notes**. Because it has `🔁 every week`, Sage inserts the next occurrence above it, due a week later, and checks this one.

## Try it: a new task line

1. Open **My first note** and put the cursor on an empty line.
2. Press `Ctrl+Shift+T`. A form asks for a **title** and a **due date**.
3. Type `Water the plants`, press `Tab`, type `tomorrow`, press `Enter`.
4. Sage writes `- [ ] Water the plants 📅` with tomorrow's date.

## Try it: every task in the vault

Open the [[Command palette]], type `show tasks` and press `Enter`. The task list shows every open task in the vault under **Overdue**, **Today**, **Upcoming** and **No due date**. Click a task's text to jump to its line. `Esc` closes the list.

> [!tip] The emoji are just text
> `📅` marks the due date, `⏫` high priority, `🔁` a repeat. They are the Obsidian Tasks plugin's format, so both apps understand your tasks.

Read more: [[Tasks and checkboxes]].

---

← [[04 Tags and properties]] · [[Sage guide|Contents]] · Next: [[06 Task documents]] →
