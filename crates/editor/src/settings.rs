//! Sage's own settings for a vault, in `.sage/settings.json` inside it,
//! the way Obsidian keeps its own in `.obsidian`. They travel with the vault
//! and survive deleting the index. The indexer skips hidden folders.

use crate::chord::Chord;
use crate::save;
use sage_watcher::Backend;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Vault-relative folder of templates. `None` uses Obsidian's setting.
    pub templates_folder: Option<String>,
    /// Vault path of the template Ctrl+N starts a new note from. `None`
    /// makes an empty note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_note_template: Option<String>,
    pub quick_add: Vec<QuickAdd>,
    /// Sage's own shortcuts rebound: command name to keys, such as
    /// `"New note": "Ctrl+Alt+Shift+N"`; empty keys take the shortcut away.
    /// Commands not named keep their usual keys.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub shortcuts: std::collections::BTreeMap<String, String>,
    /// The type chosen for each property name, lowercase, such as
    /// `"due": "date"`. Obsidian's own `types.json` is read as well; these win.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub property_types: std::collections::BTreeMap<String, String>,
}

impl Settings {
    /// The keys that run the built-in command `name`, whose usual keys are
    /// `default`: the vault's own choice if it made one.
    pub fn shortcut_for(&self, name: &str, default: &str) -> Option<Chord> {
        Chord::parse(self.shortcuts.get(name).map_or(default, String::as_str))
    }
}

/// One Quick Add entry, run from the Quick Add list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum QuickAdd {
    /// Creates a note from a template and opens it.
    Template {
        name: String,
        /// Vault path of the template; empty for an empty note.
        template: String,
        /// Vault-relative folder for the new note; empty for the vault root.
        folder: String,
        /// The new note's name, e.g. `{{date:YYYY-MM-DD}} {{value}}`. A
        /// `{{value}}` in it is asked for when the entry runs.
        file_name: String,
        /// Open the note once it is created.
        open: bool,
        /// The keys that run it, such as `Ctrl+Shift+P`; empty for none.
        #[serde(default)]
        shortcut: String,
    },
    /// Adds text to a note: an inbox, or today's daily note.
    Capture {
        name: String,
        /// Vault path of the note, with the same variables as a file name,
        /// e.g. `Daily/{{date:YYYY-MM-DD}}`; `.md` may be left off.
        target: String,
        /// The heading the text goes under; empty for the whole note.
        heading: String,
        /// What is added, e.g. `- {{value}}`.
        format: String,
        /// At the top of the note or section rather than at its end.
        top: bool,
        /// Create the note when it does not exist yet.
        create: bool,
        /// The keys that run it; empty for none.
        #[serde(default)]
        shortcut: String,
    },
}

impl QuickAdd {
    pub fn name(&self) -> &str {
        match self {
            QuickAdd::Template { name, .. } | QuickAdd::Capture { name, .. } => name,
        }
    }

    /// The keys that run this entry, if it has any.
    pub fn shortcut(&self) -> Option<Chord> {
        match self {
            QuickAdd::Template { shortcut, .. } | QuickAdd::Capture { shortcut, .. } => Chord::parse(shortcut),
        }
    }

    pub fn set_shortcut(&mut self, chord: Option<Chord>) {
        let text = chord.map(|c| c.to_string()).unwrap_or_default();
        match self {
            QuickAdd::Template { shortcut, .. } | QuickAdd::Capture { shortcut, .. } => *shortcut = text,
        }
    }

    pub fn new_template() -> Self {
        QuickAdd::Template {
            name: "New note".into(),
            template: String::new(),
            folder: String::new(),
            file_name: "{{value}}".into(),
            open: true,
            shortcut: String::new(),
        }
    }

    pub fn new_capture() -> Self {
        QuickAdd::Capture {
            name: "Capture".into(),
            target: "Inbox".into(),
            heading: String::new(),
            format: "- {{value}}".into(),
            top: false,
            create: true,
            shortcut: String::new(),
        }
    }
}

pub fn path(vault: &Path) -> PathBuf {
    vault.join(".sage").join("settings.json")
}

/// The vault's settings. A missing file gives the defaults; a file that
/// cannot be read is an error, so it is never overwritten with defaults.
pub fn load(vault: &Path) -> Result<Settings, String> {
    match std::fs::read(path(vault)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!(".sage/settings.json: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(format!(".sage/settings.json: {e}")),
    }
}

pub fn store(vault: &Path, settings: &Settings, backend: Backend) -> Result<(), String> {
    let file = path(vault);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    json.push('\n');
    save(&file, &json, backend).map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebound_shortcuts() {
        let mut settings = Settings::default();
        let chord = |keys: &str| Chord::parse(keys);
        assert_eq!(settings.shortcut_for("New note", "Ctrl+N"), chord("Ctrl+N"));
        settings.shortcuts.insert("New note".into(), "Ctrl+Alt+Shift+N".into());
        settings.shortcuts.insert("Close tab".into(), String::new());
        assert_eq!(settings.shortcut_for("New note", "Ctrl+N"), chord("Ctrl+Alt+Shift+N"));
        assert_eq!(settings.shortcut_for("Close tab", "Ctrl+W"), None);
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
        // Nothing rebound: the file stays as it was.
        assert!(!serde_json::to_string(&Settings::default()).unwrap().contains("shortcuts"));
    }

    #[test]
    fn round_trip_and_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), Settings::default(), "no file, defaults");

        let settings = Settings {
            templates_folder: Some("Meta/Templates".into()),
            quick_add: vec![QuickAdd::new_template(), QuickAdd::new_capture()],
            ..Settings::default()
        };
        store(dir.path(), &settings, Backend::Native).unwrap();
        assert_eq!(load(dir.path()).unwrap(), settings);
        let json = std::fs::read_to_string(path(dir.path())).unwrap();
        assert!(json.contains("\"kind\": \"capture\""), "{json}");

        // Keys added later fall back to their defaults; a broken file is an error.
        std::fs::write(path(dir.path()), r#"{"quick_add": []}"#).unwrap();
        assert_eq!(load(dir.path()).unwrap(), Settings::default());
        let older = r#"{"quick_add": [{"kind": "capture", "name": "Inbox", "target": "Inbox", "heading": "", "format": "- {{value}}", "top": false, "create": true}]}"#;
        std::fs::write(path(dir.path()), older).unwrap();
        let mut entry = load(dir.path()).unwrap().quick_add.remove(0);
        assert_eq!(entry.shortcut(), None, "files from before shortcuts load");
        entry.set_shortcut(Chord::parse("shift+ctrl+i"));
        assert_eq!(entry.shortcut().map(|c| c.to_string()).as_deref(), Some("Ctrl+Shift+I"));
        assert!(matches!(&entry, QuickAdd::Capture { shortcut, .. } if shortcut == "Ctrl+Shift+I"), "kept as written for people");
        std::fs::write(path(dir.path()), "{ nope").unwrap();
        assert!(load(dir.path()).is_err());
    }
}
