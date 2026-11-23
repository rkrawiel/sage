//! Obsidian's own settings that Sage follows, read from the vault's
//! `.obsidian` folder: where attachments go, how lists are indented, and
//! where the daily note lives. Missing files and values fall back to the
//! defaults, so a vault Obsidian never opened works too.

use crate::template;
use chrono::NaiveDateTime;
use std::path::Path;

fn read(vault: &Path, file: &str) -> serde_json::Value {
    std::fs::read(vault.join(".obsidian").join(file))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn text<'a>(json: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    json.get(key).and_then(|v| v.as_str()).map(str::trim)
}

/// The property types Obsidian recorded in `types.json`, by lowercase name.
pub fn property_types(vault: &Path) -> std::collections::BTreeMap<String, String> {
    read(vault, "types.json")
        .get("types")
        .and_then(|t| t.as_object())
        .map(|o| o.iter().filter_map(|(k, v)| Some((k.to_lowercase(), v.as_str()?.to_owned()))).collect())
        .unwrap_or_default()
}

/// One level of list indentation: a tab, as Obsidian writes by default, or
/// spaces when `app.json` turns tabs off.
pub fn indent_unit(vault: &Path) -> String {
    let app = read(vault, "app.json");
    let tabs = app.get("useTab").and_then(|v| v.as_bool()).unwrap_or(true);
    let size = app.get("tabSize").and_then(|v| v.as_u64()).unwrap_or(4).clamp(1, 8) as usize;
    if tabs { "\t".into() } else { " ".repeat(size) }
}

/// The vault folder a picture pasted or dropped into the note at `note` goes
/// to, `""` for the vault's root. Follows Obsidian's "Default location for
/// new attachments": `/` the root, `./` the note's folder, `./sub` a folder
/// beside the note, anything else a folder in the vault. Without the setting,
/// `attachments`.
pub fn attachment_folder(vault: &Path, note: &str) -> String {
    folder_for(text(&read(vault, "app.json"), "attachmentFolderPath"), note)
}

fn folder_for(setting: Option<&str>, note: &str) -> String {
    let here = note.rsplit_once('/').map_or("", |(dir, _)| dir);
    let join = |a: &str, b: &str| match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_owned(),
        (_, true) => a.to_owned(),
        _ => format!("{a}/{b}"),
    };
    match setting {
        None => "attachments".into(),
        Some("" | "/") => String::new(),
        Some(rel) if rel == "." || rel.starts_with("./") => join(here, rel.trim_start_matches('.').trim_matches('/')),
        Some(folder) => folder.trim_matches('/').to_owned(),
    }
}

/// What Obsidian calls a pasted picture: `Pasted image 20261118151230.png`.
pub fn pasted_name(now: NaiveDateTime) -> String {
    format!("Pasted image {}.png", now.format("%Y%m%d%H%M%S"))
}

/// Writes `bytes` as `name` in vault folder `folder`, which is created when
/// missing, under a free name: `name`, else `name 1`, `name 2`…. Returns the
/// file's vault path.
pub fn save_attachment(vault: &Path, folder: &str, name: &str, bytes: &[u8]) -> Result<String, String> {
    let dir = vault.join(folder);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{folder}: {e}"))?;
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    for n in 0.. {
        let file = if n == 0 { format!("{stem}{ext}") } else { format!("{stem} {n}{ext}") };
        use std::io::Write;
        match std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join(&file)) {
            Ok(mut out) => {
                out.write_all(bytes).map_err(|e| format!("{file}: {e}"))?;
                return Ok(if folder.is_empty() { file } else { format!("{folder}/{file}") });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{file}: {e}")),
        }
    }
    unreachable!("some name is always free")
}

/// The Daily notes plugin's settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Daily {
    /// Vault folder, `""` for the root.
    pub folder: String,
    /// Moment.js date format of the note's name.
    pub format: String,
    /// Vault path of the template, with `.md`; empty for none.
    pub template: String,
}

impl Daily {
    /// Reads `.obsidian/daily-notes.json`.
    pub fn load(vault: &Path) -> Self {
        let json = read(vault, "daily-notes.json");
        let get = |key: &str| text(&json, key).filter(|s| !s.is_empty()).map(|s| s.trim_matches('/').to_owned());
        let template = get("template").map(|t| if t.ends_with(".md") { t } else { format!("{t}.md") });
        Self {
            folder: get("folder").unwrap_or_default(),
            format: get("format").unwrap_or_else(|| "YYYY-MM-DD".into()),
            template: template.unwrap_or_default(),
        }
    }

    /// The vault path of the daily note for `now`.
    pub fn path(&self, now: NaiveDateTime) -> String {
        let name = template::moment(&self.format, now);
        if self.folder.is_empty() { format!("{name}.md") } else { format!("{}/{name}.md", self.folder) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 11, 18).unwrap().and_hms_opt(15, 12, 30).unwrap()
    }

    #[test]
    fn attachment_folders_follow_obsidian() {
        assert_eq!(folder_for(None, "Projects/Alpha.md"), "attachments");
        assert_eq!(folder_for(Some("/"), "Projects/Alpha.md"), "");
        assert_eq!(folder_for(Some("./"), "Projects/Alpha.md"), "Projects");
        assert_eq!(folder_for(Some("./"), "Alpha.md"), "");
        assert_eq!(folder_for(Some("./assets"), "Projects/Alpha.md"), "Projects/assets");
        assert_eq!(folder_for(Some("./assets"), "Alpha.md"), "assets");
        assert_eq!(folder_for(Some("Meta/Files/"), "Projects/Alpha.md"), "Meta/Files");
    }

    #[test]
    fn settings_files() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        assert_eq!(indent_unit(vault), "\t");
        assert_eq!(attachment_folder(vault, "A.md"), "attachments");
        let daily = Daily::load(vault);
        assert_eq!(daily.path(now()), "2026-11-18.md");
        assert_eq!(pasted_name(now()), "Pasted image 20261118151230.png");

        std::fs::create_dir(vault.join(".obsidian")).unwrap();
        std::fs::write(vault.join(".obsidian/app.json"), r#"{"useTab": false, "tabSize": 2, "attachmentFolderPath": "./img"}"#).unwrap();
        std::fs::write(
            vault.join(".obsidian/daily-notes.json"),
            r#"{"folder": "/Journal/", "format": "YYYY/MM-DD ddd", "template": "Templates/Day"}"#,
        )
        .unwrap();
        assert_eq!(indent_unit(vault), "  ");
        assert_eq!(attachment_folder(vault, "Notes/A.md"), "Notes/img");
        let daily = Daily::load(vault);
        assert_eq!(daily.template, "Templates/Day.md");
        assert_eq!(daily.path(now()), "Journal/2026/11-18 Wed.md");
    }

    #[test]
    fn attachments_get_free_names() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(save_attachment(dir.path(), "attachments", "a.png", b"1").unwrap(), "attachments/a.png");
        assert_eq!(save_attachment(dir.path(), "attachments", "a.png", b"2").unwrap(), "attachments/a 1.png");
        assert_eq!(save_attachment(dir.path(), "", "a.png", b"3").unwrap(), "a.png");
        assert_eq!(std::fs::read(dir.path().join("attachments/a 1.png")).unwrap(), b"2");
    }
}
