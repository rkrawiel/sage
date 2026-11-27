//! The starter vault: what **Create starter vault** writes into a vault that
//! has no notes yet. It holds the top-level folders of a PARA-style vault,
//! the templates with a Quick Add entry and shortcut each in
//! `.sage/settings.json`, Obsidian's daily-note settings pointing at the
//! daily template, and the Sage tutorial with the guide pages, dashboards and
//! example tasks it links to, all under one `Sage tutorial` folder so the
//! user's own folders stay clean.
//!
//! The files are embedded from `assets/starter-vault/`, so the binary stays
//! self-contained. Nothing is ever overwritten: a file that exists already is
//! left as it is, which also makes it safe to run on a vault that is not
//! empty. That is the one place Sage writes into `.obsidian/`: it creates
//! `daily-notes.json` in a new vault, and never changes one that exists.

use sage_watcher::Backend;
use std::path::Path;

/// Empty top-level folders, as in the vault the starter was modelled on.
pub const FOLDERS: [&str; 9] =
    ["0. MOCs", "1. Projects", "2. Areas", "DailyNotes", "MeetingNotes", "People", "SecondBrain", "Tasks", "Templates"];

/// The note to open once the vault is written: the tutorial's first step.
pub const FIRST_NOTE: &str = "Sage tutorial/Tutorial/01 Welcome.md";

/// Vault-relative paths and their text.
pub const FILES: [(&str, &str); 50] = [
    (".obsidian/daily-notes.json", include_str!("../../../assets/starter-vault/.obsidian/daily-notes.json")),
    (".sage/settings.json", include_str!("../../../assets/starter-vault/.sage/settings.json")),
    ("Sage tutorial/Architecture notes.md", include_str!("../../../assets/starter-vault/Sage tutorial/Architecture notes.md")),
    ("Sage tutorial/Dashboard.md", include_str!("../../../assets/starter-vault/Sage tutorial/Dashboard.md")),
    ("Sage tutorial/Docs/Command palette.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Command palette.md")),
    ("Sage tutorial/Docs/Deleting notes.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Deleting notes.md")),
    ("Sage tutorial/Docs/Editing and autocomplete.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Editing and autocomplete.md")),
    ("Sage tutorial/Docs/External changes and merging.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/External changes and merging.md")),
    ("Sage tutorial/Docs/Getting around.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Getting around.md")),
    ("Sage tutorial/Docs/Getting started.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Getting started.md")),
    ("Sage tutorial/Docs/Kanban boards.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Kanban boards.md")),
    ("Sage tutorial/Docs/Keyboard shortcuts.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Keyboard shortcuts.md")),
    ("Sage tutorial/Docs/Links and backlinks.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Links and backlinks.md")),
    ("Sage tutorial/Docs/Pictures and attachments.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Pictures and attachments.md")),
    ("Sage tutorial/Docs/Preview and formatting.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Preview and formatting.md")),
    ("Sage tutorial/Docs/Properties.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Properties.md")),
    ("Sage tutorial/Docs/Quick Add.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Quick Add.md")),
    ("Sage tutorial/Docs/Rename and move.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Rename and move.md")),
    ("Sage tutorial/Docs/Search everywhere.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Search everywhere.md")),
    ("Sage tutorial/Docs/Settings.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Settings.md")),
    ("Sage tutorial/Docs/Tags.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Tags.md")),
    ("Sage tutorial/Docs/Tasks and checkboxes.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Tasks and checkboxes.md")),
    ("Sage tutorial/Docs/Tasks blocks.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Tasks blocks.md")),
    ("Sage tutorial/Docs/Typed documents.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Typed documents.md")),
    ("Sage tutorial/Docs/Using templates.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/Using templates.md")),
    ("Sage tutorial/Docs/View blocks.md", include_str!("../../../assets/starter-vault/Sage tutorial/Docs/View blocks.md")),
    ("Sage tutorial/Meeting notes.md", include_str!("../../../assets/starter-vault/Sage tutorial/Meeting notes.md")),
    ("Sage tutorial/Projects/Alpha.md", include_str!("../../../assets/starter-vault/Sage tutorial/Projects/Alpha.md")),
    ("Sage tutorial/Sage guide.md", include_str!("../../../assets/starter-vault/Sage tutorial/Sage guide.md")),
    ("Sage tutorial/Tasks dashboard.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tasks dashboard.md")),
    ("Sage tutorial/Tasks/Voice reconnect.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tasks/Voice reconnect.md")),
    ("Sage tutorial/Tasks/finish-the-sage-tutorial.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tasks/finish-the-sage-tutorial.md")),
    ("Sage tutorial/Tasks/write-release-notes.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tasks/write-release-notes.md")),
    ("Sage tutorial/Tutorial/01 Welcome.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tutorial/01 Welcome.md")),
    ("Sage tutorial/Tutorial/02 Your first note.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tutorial/02 Your first note.md")),
    ("Sage tutorial/Tutorial/03 Linking notes.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tutorial/03 Linking notes.md")),
    ("Sage tutorial/Tutorial/04 Tags and properties.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tutorial/04 Tags and properties.md")),
    ("Sage tutorial/Tutorial/05 Tracking tasks.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tutorial/05 Tracking tasks.md")),
    ("Sage tutorial/Tutorial/06 Task documents.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tutorial/06 Task documents.md")),
    ("Sage tutorial/Tutorial/07 Building a dashboard.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tutorial/07 Building a dashboard.md")),
    ("Sage tutorial/Tutorial/08 Templates and Quick Add.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tutorial/08 Templates and Quick Add.md")),
    ("Sage tutorial/Tutorial/09 Rename, move and delete.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tutorial/09 Rename, move and delete.md")),
    ("Sage tutorial/Tutorial/10 Finding things.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tutorial/10 Finding things.md")),
    ("Sage tutorial/Tutorial/11 Wrapping up.md", include_str!("../../../assets/starter-vault/Sage tutorial/Tutorial/11 Wrapping up.md")),
    ("Templates/Template, book.md", include_str!("../../../assets/starter-vault/Templates/Template, book.md")),
    ("Templates/Template, daily note.md", include_str!("../../../assets/starter-vault/Templates/Template, daily note.md")),
    ("Templates/Template, meeting note.md", include_str!("../../../assets/starter-vault/Templates/Template, meeting note.md")),
    ("Templates/Template, person.md", include_str!("../../../assets/starter-vault/Templates/Template, person.md")),
    ("Templates/Template, project.md", include_str!("../../../assets/starter-vault/Templates/Template, project.md")),
    ("Templates/Template, regular note.md", include_str!("../../../assets/starter-vault/Templates/Template, regular note.md")),
];

/// How many files were written, and how many were there already and kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    pub created: usize,
    pub kept: usize,
}

/// Writes the starter vault into `vault`, skipping every file that exists.
/// Notes are written with [`crate::save`], so OneDrive and network folders
/// get the in-place policy.
pub fn create(vault: &Path, backend: Backend) -> Result<Written, String> {
    for folder in FOLDERS {
        std::fs::create_dir_all(vault.join(folder)).map_err(|e| format!("{folder}: {e}"))?;
    }
    let mut written = Written { created: 0, kept: 0 };
    for (rel, text) in FILES {
        let abs = vault.join(rel);
        // A dangling link counts as there: writing would go through it.
        if std::fs::symlink_metadata(&abs).is_ok() {
            written.kept += 1;
            continue;
        }
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{rel}: {e}"))?;
        }
        crate::save(&abs, text, backend).map_err(|e| format!("{rel}: {e:#}"))?;
        written.created += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::Chord;

    #[test]
    fn writes_folders_templates_settings_and_the_tutorial() {
        let dir = tempfile::tempdir().unwrap();
        let written = create(dir.path(), Backend::Native).unwrap();
        assert_eq!(written, Written { created: FILES.len(), kept: 0 });
        for folder in FOLDERS {
            assert!(dir.path().join(folder).is_dir(), "{folder}");
        }
        assert!(dir.path().join(FIRST_NOTE).is_file());

        let settings = crate::settings::load(dir.path()).unwrap();
        assert_eq!(settings.templates_folder.as_deref(), Some("Templates"));
        let template = settings.new_note_template.unwrap();
        assert!(dir.path().join(&template).is_file(), "{template}");
        let daily = crate::obsidian::Daily::load(dir.path());
        assert_eq!(daily.folder, "DailyNotes");
        assert!(dir.path().join(&daily.template).is_file(), "{}", daily.template);

        // Every entry's template exists, and its shortcut is usable and its own.
        let mut chords: Vec<Chord> = Vec::new();
        for entry in &settings.quick_add {
            let crate::settings::QuickAdd::Template { name, template, .. } = entry else { panic!("capture entry") };
            assert!(dir.path().join(template).is_file(), "{name}: {template}");
            let chord = entry.shortcut().unwrap_or_else(|| panic!("{name} has no shortcut"));
            assert!(chord.unusable().is_none(), "{name}: {chord}");
            assert!(!chords.contains(&chord), "{name}: {chord} twice");
            chords.push(chord);
        }
        assert_eq!(chords.len(), 4);
    }

    #[test]
    fn existing_files_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let mine = dir.path().join("Templates/Template, book.md");
        std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
        std::fs::write(&mine, "my own book template\r\n").unwrap();

        let written = create(dir.path(), Backend::Native).unwrap();
        assert_eq!(written, Written { created: FILES.len() - 1, kept: 1 });
        assert_eq!(std::fs::read_to_string(&mine).unwrap(), "my own book template\r\n");

        let again = create(dir.path(), Backend::Native).unwrap();
        assert_eq!(again, Written { created: 0, kept: FILES.len() });
    }

    #[test]
    fn the_tutorials_links_resolve_in_the_starter_vault() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        create(&vault, Backend::Native).unwrap();
        let db_path = dir.path().join("index.db");
        let mut ix = sage_indexer::Indexer::open(vault.canonicalize().unwrap(), &db_path).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        drop(ix);
        let db = sage_db::open_reader(&db_path).unwrap();
        let mut stmt = db
            .prepare("SELECT DISTINCT l.target FROM links l WHERE l.dst_file_id IS NULL ORDER BY l.target")
            .unwrap();
        let dangling: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(dangling, Vec::<String>::new());
        let resolved: i64 = db.query_row("SELECT count(*) FROM links WHERE dst_file_id IS NOT NULL", [], |r| r.get(0)).unwrap();
        assert!(resolved > 300, "only {resolved} links resolved");
    }
}
