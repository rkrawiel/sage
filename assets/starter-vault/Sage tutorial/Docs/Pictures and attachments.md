---
title: Pictures and attachments
type: doc
status: stable
area: writing
aliases: [Attachments, Pasting pictures, Images]
tags: [docs, docs/writing]
---
# Pictures and attachments

Paste a picture or drop a picture file on the window, and Sage saves it into the vault's attachments folder and embeds it at the cursor as `![[name.png]]`, as Obsidian does.

Back to the [[Sage guide]].

## Pasting

`Ctrl+V` with a picture on the clipboard and no text, for example after taking a screenshot, saves the picture as a PNG. It is named like Obsidian's pasted pictures, after the time: `Pasted image 20261118151230.png`.

## Dropping

Drop a picture file from Explorer or a file manager on Sage's window. A copy goes into the attachments folder under the file's own name; when that name is taken, ` 1`, ` 2` and so on are added, as in `photo 1.png`. Only pictures can be dropped.

## Where they go

Sage follows Obsidian's setting "Default location for new attachments", which Obsidian keeps as `attachmentFolderPath` in `.obsidian/app.json`:

| `attachmentFolderPath` | Folder |
|---|---|
| `/` | the vault root |
| `./` | the note's own folder |
| `./assets` | `assets` beside the note |
| `Meta/Files` | `Meta/Files` in the vault |
| not set | `attachments/` in the vault |

The folder is created when it does not exist yet.

## Seeing them

The embed shows the picture in the preview, see [[Preview and formatting]]. In live preview it shows as written for now. `[[` and `![[` suggest attachments by name, see [[Editing and autocomplete#Autocomplete]].

## Related

- [[Links and backlinks]]
- [[Keyboard shortcuts]]
