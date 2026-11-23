//! `sage [vault-path | note.md]`: opens an Obsidian-compatible vault. Given
//! a file, it opens the vault the file is in with that note. Without a path
//! it opens `SAGE_VAULT`, else the vault opened last. On Windows, where it
//! starts from the Start menu without a console, it then asks for a folder,
//! and reports errors in a message box.

#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::Context;
use std::path::{Path, PathBuf};

fn main() {
    report_panics();
    if let Err(error) = run() {
        report(&format!("{error:#}"));
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let last = sage_watcher::data_dir().map(|dir| dir.join("last-vault"));
    let path = std::env::args_os()
        .nth(1)
        .or_else(|| std::env::var_os("SAGE_VAULT"))
        .map(PathBuf::from)
        .or_else(|| {
            let path = std::fs::read_to_string(last.as_ref()?).ok()?;
            Some(PathBuf::from(path.trim())).filter(|p| p.is_dir())
        })
        .or_else(pick_folder)
        .context("usage: sage <vault-path>   (or set SAGE_VAULT)")?;
    let (vault, note) = if path.is_file() { (vault_of(&path), Some(path)) } else { (path, None) };
    if let Some(file) = &last {
        // Remembered for the next start without a path; never needed to run.
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, vault.to_string_lossy().as_bytes());
    }
    sage_ui::run_with(vault, note)
}

/// Writes every panic to `<data>/crash.log`, and on Windows shows it too:
/// there Sage has no console, so a panic would otherwise end it without a
/// word. The default hook still runs, for a terminal under Linux.
fn report_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let log = sage_watcher::data_dir().map(|dir| dir.join("crash.log"));
        let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let entry = format!("--- {secs} (Unix time), thread '{thread}': {info}
{}
", std::backtrace::Backtrace::force_capture());
        let logged = log.as_ref().is_some_and(|log| append(log, &entry).is_ok());
        if cfg!(windows) {
            let details = match (&log, logged) {
                (Some(log), true) => format!("

Details are in {}", log.display()),
                _ => String::new(),
            };
            report(&format!("Sage ran into a problem in its {thread} thread: {info}{details}"));
        }
        default(info);
    }));
}

fn append(file: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::OpenOptions::new().create(true).append(true).open(file)?.write_all(text.as_bytes())
}

/// The vault a file belongs to: the nearest folder above it that has
/// `.obsidian` or `.sage` in it, else the file's own folder.
fn vault_of(file: &Path) -> PathBuf {
    let file = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
    let folder = file.parent().unwrap_or(Path::new(".")).to_path_buf();
    folder
        .ancestors()
        .find(|dir| dir.join(".obsidian").is_dir() || dir.join(".sage").is_dir())
        .map_or(folder.clone(), Path::to_path_buf)
}

#[cfg(windows)]
fn pick_folder() -> Option<PathBuf> {
    rfd::FileDialog::new().set_title("Open a vault").pick_folder()
}

#[cfg(not(windows))]
fn pick_folder() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn report(message: &str) {
    rfd::MessageDialog::new()
        .set_title("Sage")
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .show();
}

#[cfg(not(windows))]
fn report(message: &str) {
    eprintln!("Error: {message}");
}

#[cfg(test)]
mod tests {
    use super::vault_of;
    use std::fs;

    #[test]
    fn nearest_obsidian_folder_is_the_vault() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().join("vault");
        fs::create_dir_all(vault.join(".obsidian")).unwrap();
        fs::create_dir_all(vault.join("Docs/Deep")).unwrap();
        let note = vault.join("Docs/Deep/Note.md");
        fs::write(&note, "# Note").unwrap();
        assert_eq!(vault_of(&note), vault);
    }

    #[test]
    fn sage_folder_marks_a_vault_and_the_nearest_wins() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        let inner = root.path().join("inner");
        fs::create_dir_all(inner.join(".sage")).unwrap();
        let note = inner.join("Note.md");
        fs::write(&note, "").unwrap();
        assert_eq!(vault_of(&note), inner);
    }

    #[test]
    fn without_a_marker_the_files_folder_is_the_vault() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("loose");
        fs::create_dir_all(&folder).unwrap();
        let note = folder.join("Note.md");
        fs::write(&note, "").unwrap();
        assert_eq!(vault_of(&note), folder);
    }

    #[test]
    fn a_file_named_like_a_marker_does_not_count() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".obsidian"), "").unwrap();
        let folder = root.path().join("notes");
        fs::create_dir_all(&folder).unwrap();
        let note = folder.join("Note.md");
        fs::write(&note, "").unwrap();
        assert_eq!(vault_of(&note), folder);
    }
}
