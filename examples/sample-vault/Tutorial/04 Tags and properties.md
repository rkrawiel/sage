---
title: "Step 4: Tags and properties"
type: tutorial
step: 4
status: todo
tags: [tutorial]
---
# Tags and properties

Folders put a note in one place. Tags and properties describe it, so it can turn up in many places: views, searches, dashboards.

## Try it: add properties

At the very top of **My first note**, before the `# My first note` heading, add:

```yaml
---
title: My practice note
status: draft
aliases: [Practice note]
tags: [playground]
---
```

Look at the preview: the properties appear as a panel at the top, with `playground` drawn as a tag.

Now open the [[Command palette]] with `Ctrl+P` and type `my first`. Beside the file name, the palette shows the title *My practice note* as a hint, because it differs from the name.

## Try it: inline and nested tags

1. Somewhere in the text, type `#play`. The popup suggests `#playground`, since that tag now exists. Accept it.
2. Type `#playground/ideas`. A `/` makes a **nested tag**: it belongs to `#playground` too.

## Try it: properties feed views

This table lists the tutorial's steps with their `status` property. It is a **view block**, a live query over the index:

```view
type: table
source: "path:Tutorial/"
columns: [file, title, status]
sort: file
```

1. Scroll to the top of this note's source and change `status: todo` to `status: done`.
2. Watch this step's row in the table change. Mark [[01 Welcome]], [[02 Your first note]] and [[03 Linking notes]] done too, if you finished them.

The same table, built differently, is on the [[Sage guide]] page.

## Try it: your own query

Paste this into **My first note**. It lists every note tagged `playground`:

````markdown
```view
type: table
source: "tag:#playground"
columns: [file, title, status, tags]
```
````

Paste only the inner block, from the first three backticks to the last three. The quotes around `tag:#playground` matter: without them, YAML would treat `#playground` as a comment.

Read more: [[Tags]], [[Properties]], [[View blocks]].

---

← [[03 Linking notes]] · [[Sage guide|Contents]] · Next: [[05 Tracking tasks]] →
