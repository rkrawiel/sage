---
title: "Step 8: Templates and Quick Add"
type: tutorial
step: 8
status: todo
tags: [tutorial]
---
# Templates and Quick Add

Templates save you from typing the same structure twice. Quick Add goes one step further and turns "make a meeting note" or "jot this down in today's daily note" into a single key press.

## Try it: insert a template

1. Create `Playground/Weekly sync` with `Ctrl+P` and **Create note**.
2. Open the palette again and type `insert`. **Insert template** shows up even without `>`, because commands are found by name from the third letter.
3. Choose **Template, meeting note**. The [[Template, meeting note]] template has only properties and headings: its properties go into the note's frontmatter, with `Date` set to today, and its headings are inserted at the cursor.

## Try it: open the settings

Click the **⚙** beside the vault's name (or run **Settings** from the palette). Under **Templates**, the templates folder is chosen from the vault's folders. Leave it on `Templates`.

## Try it: a capture entry

Still in the settings, under **Quick Add**, add a **capture** entry:

| Field | Value |
|---|---|
| Name | `Log to daily note` |
| Note | `DailyNotes/{{date:YYYY-MM-DD}}` |
| Heading | `## Log` |
| Text | `- {{value}}` |
| Where | end |
| Create the note if missing | yes |

Close the settings with **Done** or `Esc`, then:

1. Press `Ctrl+Shift+N` and choose **Log to daily note**.
2. The palette asks for the value and shows the note it will add to. Type `Finished step 8 of the tutorial` and press `Enter`.
3. Open today's note in the `DailyNotes` folder: it has a `## Log` heading with your line under it. Run the entry again and the second line goes under the first.

## Try it: a template entry and a shortcut

1. Add a **template** entry: name `New meeting`, template **Template, meeting note**, folder `Playground`, file name `{{date:YYYY-MM-DD}} {{value}}`.
2. Click its **Shortcut** row and press `Ctrl+Shift+M`. Try `Ctrl+S` first if you are curious: it is refused, because Save uses it.
3. Close the settings, press `Ctrl+Shift+M`, type `Planning` and press `Enter`. Sage creates `Playground/<today> Planning.md` from the template and opens it.

Quick Add entries are saved in the vault's `.sage/settings.json`, so they travel with the vault. You will delete these practice entries in [[11 Wrapping up]].

Read more: [[Using templates]], [[Quick Add]], [[Settings]].

---

← [[07 Building a dashboard]] · [[Sage guide|Contents]] · Next: [[09 Rename, move and delete]] →
