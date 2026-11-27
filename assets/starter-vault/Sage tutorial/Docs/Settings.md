---
title: Settings
type: doc
status: stable
area: basics
aliases: [Preferences, Options, Appearance]
tags: [docs, docs/basics]
---
# Settings

Open the settings with the **⚙** beside the vault's name, or **Settings** in the [[Command palette]]. They show in place of the note; **Done** or `Esc` closes them.

Back to the [[Sage guide]]. Practised in [[08 Templates and Quick Add]] and [[11 Wrapping up]].

## Where they are kept

Changes are saved 1.5 s after the last one, to `.sage/settings.json` inside the vault. That file syncs with the vault and survives deleting the index. If the file cannot be read, Sage reports it and never overwrites it.

The theme is the exception: it is kept with the session on this computer.

## Sections

| Section | What it holds |
|---|---|
| Vault | The open vault's path, and **Open vault folder…** to switch vaults |
| Templates | The templates folder, chosen from the vault's folders. See [[Using templates]] |
| Quick Add | Template and capture entries, each with a shortcut. ↑ and ↓ change their order, which is the order the Quick Add list shows. See [[Quick Add]] |
| Shortcuts | Sage's shortcuts, every Quick Add entry with its keys, and the fixed keys and clicks. Click a shortcut to change its keys. See [[Keyboard shortcuts#Your own shortcuts]] |
| Appearance | Light, dark, or as the system has it. `Ctrl+Shift+L` toggles too |

## Switching vaults

**Open vault folder…** shows the system's folder picker. Cancelling changes nothing. Unsaved notes and settings are saved first; a note in conflict, or one that fails to save, stops the switch. Sage then restarts on the new folder, which becomes the vault it opens next time.

## Related

- [[Getting started]]
