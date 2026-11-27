---
title: "Step 3: Linking notes"
type: tutorial
step: 3
status: todo
tags: [tutorial]
---
# Linking notes

Links connect notes into a web you can walk. In this step you link **My first note** to other notes, follow links, and see backlinks.

## Try it: link with autocomplete

In **My first note** (from [[02 Your first note]]), on a new line:

1. Type `[[Keyb`. A popup suggests **Keyboard shortcuts**. Press `Enter` or `Tab` to accept; Sage writes `[[Keyboard shortcuts]]`.
2. Type `[[Hotkeys`. The same note is suggested, because `Hotkeys` is one of its **aliases**.
3. Type `[[Tags#Nested tags]]` by hand. That links to a heading inside a note.
4. Type `[[Command palette|the palette]]`. The text after `|` is what the preview shows.

## Try it: follow and come back

1. In the preview, click the **Keyboard shortcuts** link. The note opens.
2. Scroll to the bottom of its preview. Under **Linked from** you now see **My first note**. Those are its **backlinks**.
3. Click **My first note** there to go back.

## Try it: link to a note that does not exist

1. Type `[[My second note]]`. In the preview the link is **dimmed**: no note has that name.
2. Click it. Sage creates the note and opens it. Type a line in it, such as `Back to [[My first note]]`.

## Try it: embed

Back in **My first note**, type:

```markdown
![[Keyboard shortcuts#Core shortcuts]]
```

The preview shows that section of the other note right inside yours, framed, with a link to it.

> [!note] Markdown links work too
> `[the guide](../Sage%20guide.md)` is a link as well, and so is a `[[link]]` inside a frontmatter property. Type `](` to get path suggestions.

Read more: [[Links and backlinks]].

---

← [[02 Your first note]] · [[Sage guide|Contents]] · Next: [[04 Tags and properties]] →
