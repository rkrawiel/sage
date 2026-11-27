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
pub mod import;
pub mod log;
pub mod merge;
pub mod new_task;
pub mod obsidian;
pub mod properties;
pub mod quick_add;
pub mod recur;
pub mod reminders;
pub mod rename;
pub mod settings;
pub mod starter;
pub mod table;
pub mod tasks;
pub mod template;
pub mod undo;

use anyhow::{Context, Result};
use sage_watcher::Backend;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
///   version history and can produce conflict copies. The new text is first
///   written to `<data>/rescue`, so a write that fails halfway leaves it
///   recoverable there instead of only in a truncated note.
///
/// A symbolic link is saved through to its target, so the link stays a link.
pub fn save(path: &Path, contents: &str, backend: Backend) -> Result<()> {
    let target;
    let path = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            target = sage_watcher::canonical(path).with_context(|| format!("following link {}", path.display()))?;
            target.as_path()
        }
        _ => path,
    };
    let dir = path.parent().context("note has no parent directory")?;
    let name = path.file_name().context("note has no file name")?.to_string_lossy();
    if !backend.atomic_save() || sage_onedrive::root(path).is_some() {
        // The rescue copy lives outside the vault: a dot-file beside the note
        // would be uploaded and deleted again by OneDrive on every autosave.
        let rescue = sage_watcher::data_dir().map(|d| d.join("rescue")).unwrap_or_else(std::env::temp_dir);
        std::fs::create_dir_all(&rescue).with_context(|| format!("creating {}", rescue.display()))?;
        let tmp = write_temp(&rescue, &name, contents)?;
        let written = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(path)?;
            file.write_all(contents.as_bytes())?;
            file.sync_all()
        })();
        return match written {
            Ok(()) => {
                let _ = std::fs::remove_file(&tmp);
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("writing {}: {e}; the new text is kept in {}", path.display(), tmp.display())),
        };
    }
    let tmp = write_temp(dir, &name, contents)?;
    // Unix gives a new file the default mode; the note keeps its own instead.
    // Windows only knows the read-only flag, and a read-only temp file could
    // not be removed again if the rename fails.
    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    if let Err(e) = rename_over(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("saving {}", path.display()));
    }
    // The rename is only durable once the directory entry is; best effort,
    // since some filesystems refuse to sync a directory.
    #[cfg(unix)]
    if let Some(dir) = path.parent().and_then(|d| std::fs::File::open(d).ok()) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// A new hidden file in `dir` holding `contents` under a name made from the
/// note's `name`, flushed to disk. The name starts with a dot, so the indexer
/// ignores it, and is unique to this process and call, so two saves of one
/// note never write the same file.
fn write_temp(dir: &Path, name: &str, contents: &str) -> Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let tmp = dir.join(format!(".{name}.{}-{}.sage-tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("writing {}", tmp.display()));
    }
    Ok(tmp)
}

/// Renames `from` over `to`. Windows refuses while another program, such as
/// Obsidian, a sync client or a virus scanner, has `to` open for a moment, so
/// there the rename is tried again for about a second. Depending on how the
/// other program opened the file, that shows as access denied or as a sharing
/// or lock violation (OS errors 32 and 33).
pub(crate) fn rename_over(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = if cfg!(windows) { 20 } else { 1 };
    loop {
        match std::fs::rename(from, to) {
            Err(e) if tries > 1 && (e.kind() == std::io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(32 | 33))) => {
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

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn concurrent_saves_use_their_own_temp_files_and_leave_none_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.md");
        std::fs::write(&path, "v0").unwrap();
        let texts: Vec<String> = (1..=8).map(|i| format!("v{i}")).collect();
        std::thread::scope(|s| {
            for text in &texts {
                let path = &path;
                s.spawn(move || save(path, text, Backend::Native).unwrap());
            }
        });
        assert!(texts.contains(&std::fs::read_to_string(&path).unwrap()));
        save(&path, "in place", Backend::Poll).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "in place");
        assert_eq!(names_in(dir.path()), ["n.md"]);
    }

    #[test]
    fn a_failed_rewrite_in_place_keeps_the_text_outside_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        // A folder cannot be opened for writing, so the in-place write fails.
        let path = dir.path().join("n.md");
        std::fs::create_dir(&path).unwrap();
        let err = format!("{:#}", save(&path, "precious", Backend::Poll).unwrap_err());
        let kept = PathBuf::from(err.split("kept in ").nth(1).expect(&err));
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "precious");
        assert!(!kept.starts_with(dir.path()), "{err}");
        std::fs::remove_file(kept).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_symlink_updates_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.md");
        std::fs::write(&real, "v1").unwrap();
        let link = dir.path().join("link.md");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        save(&link, "v2", Backend::Native).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "v2");
    }

    #[cfg(unix)]
    #[test]
    fn an_atomic_save_keeps_the_note_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.md");
        std::fs::write(&path, "v1").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        save(&path, "v2", Backend::Native).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
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
