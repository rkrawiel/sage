//! Buffer-side logic that does not depend on the UI toolkit: the save policy
//! with its conflict guard, autocomplete context detection, find in note, task
//! write-back with recurring tasks, templates, Quick Add, the vault's Sage
//! settings, renames of notes and folders with link rewriting, and the
//! three-way merge.

pub mod chord;
pub mod complete;
pub mod dates;
pub mod edit;
pub mod find;
pub mod merge;
pub mod new_task;
pub mod obsidian;
pub mod properties;
pub mod quick_add;
pub mod recur;
pub mod rename;
pub mod settings;
pub mod tasks;
pub mod template;
pub mod undo;

use anyhow::{Context, Result};
use sage_watcher::Backend;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum SaveError {
    /// The file on disk no longer matches what the tab was loaded from, so
    /// writing would discard someone else's change.
    Conflict,
    Io(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Conflict => f.write_str("the file changed on disk since it was opened"),
            SaveError::Io(e) => f.write_str(e),
        }
    }
}

/// The local date and time, for templates and completion dates.
pub fn now() -> chrono::NaiveDateTime {
    chrono::Local::now().naive_local()
}

/// Today as `YYYY-MM-DD`, the date format tasks use.
pub fn today() -> String {
    now().format("%Y-%m-%d").to_string()
}

/// Saves only if the file on disk still equals `base`, the text the tab was
/// loaded from or last saved. A missing file counts as unchanged when `base` is
/// empty, so new notes can be written.
///
/// There is a short window between the check and the write. It is acceptable
/// for a single user; the three-way merge in build step 5 closes the gap in
/// practice by re-checking on every external change.
pub fn save_guarded(path: &Path, base: &str, contents: &str, backend: Backend) -> Result<(), SaveError> {
    match std::fs::read(path) {
        Ok(disk) if disk != base.as_bytes() => return Err(SaveError::Conflict),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(SaveError::Io(e.to_string())),
        Err(_) if !base.is_empty() => return Err(SaveError::Conflict),
        _ => {}
    }
    save(path, contents, backend).map_err(|e| SaveError::Io(format!("{e:#}")))
}

/// Writes a note to disk using the policy that suits the vault's filesystem.
///
/// * Native filesystems: write a hidden temp file next to the target, fsync,
///   rename over. A crash leaves either the old or the new note, never half.
///   The temp name starts with a dot, so the indexer ignores it.
/// * 9p / network filesystems, and any OneDrive folder: rewrite in place. A
///   rename there reaches the OneDrive client as delete + create, which loses
///   version history and can produce conflict copies.
pub fn save(path: &Path, contents: &str, backend: Backend) -> Result<()> {
    if !backend.atomic_save() || sage_onedrive::root(path).is_some() {
        return std::fs::write(path, contents)
            .with_context(|| format!("writing {}", path.display()));
    }
    let dir = path.parent().context("note has no parent directory")?;
    let name = path.file_name().context("note has no file name")?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.sage-tmp"));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        drop(file);
        rename_over(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.with_context(|| format!("saving {}", path.display()))
}

/// Renames `from` over `to`. Windows refuses while another program, such as
/// Obsidian, a sync client or a virus scanner, has `to` open for a moment, so
/// there the rename is tried again for about a second.
fn rename_over(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = if cfg!(windows) { 20 } else { 1 };
    loop {
        match std::fs::rename(from, to) {
            Err(e) if tries > 1 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                tries -= 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_in_onedrive_are_rewritten_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("OneDrive").join("Vault");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("n.md");
        std::fs::write(&path, "v1").unwrap();
        // A hard link sees a rewrite in place, and loses touch on a rename over.
        let link = dir.path().join("link.md");
        std::fs::hard_link(&path, &link).unwrap();
        save(&path, "v2", Backend::Native).unwrap();
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "v2");

        let plain = dir.path().join("plain.md");
        std::fs::write(&plain, "v1").unwrap();
        std::fs::hard_link(&plain, dir.path().join("plain-link.md")).unwrap();
        save(&plain, "v2", Backend::Native).unwrap();
        assert_eq!(std::fs::read_to_string(&plain).unwrap(), "v2");
        assert_eq!(std::fs::read_to_string(dir.path().join("plain-link.md")).unwrap(), "v1", "renamed over");
    }

    #[test]
    fn guard_refuses_to_overwrite_external_changes() {
        let dir = std::env::temp_dir().join(format!("sage-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("n.md");
        std::fs::write(&path, "v1").unwrap();

        assert_eq!(save_guarded(&path, "v1", "v2", Backend::Native), Ok(()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "v2");

        std::fs::write(&path, "changed elsewhere").unwrap();
        assert_eq!(save_guarded(&path, "v2", "v3", Backend::Poll), Err(SaveError::Conflict));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed elsewhere");

        let fresh = dir.join("new.md");
        assert_eq!(save_guarded(&fresh, "", "hello", Backend::Native), Ok(()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
