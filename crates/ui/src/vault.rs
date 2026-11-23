//! Opening another vault from inside the app: a folder picker, a check that
//! the folder can be used, then Sage started afresh on it.
//!
//! Switching relaunches the process with the folder as its argument instead
//! of rebuilding the state in place: reads, saves and renames still under
//! way name notes by vault path, and must not land in the new vault's tabs.
//! The new process records the folder as the vault opened last, as any start
//! with a path does.

use crate::{App, Message};
use iced::Task;
use std::path::{Path, PathBuf};

impl App {
    /// The palette command and the settings button: asks for a folder.
    pub(crate) fn open_vault(&mut self) -> Task<Message> {
        if let Some(notice) = blocked(&self.conflicts()) {
            self.notice(notice);
            return Task::none();
        }
        Task::perform(
            async { tokio::task::spawn_blocking(pick).await.unwrap_or_else(|e| Err(e.to_string())) },
            Message::VaultPicked,
        )
    }

    /// A folder chosen, or the picker cancelled or missing. Unsaved notes and
    /// settings are saved before the switch, which looks again once they are.
    pub(crate) fn vault_picked(&mut self, picked: Result<Option<PathBuf>, String>) -> Task<Message> {
        let chosen = match picked {
            Ok(Some(chosen)) => chosen,
            Ok(None) => return Task::none(),
            Err(error) => {
                self.notice(format!("No folder picker: {error}"));
                return Task::none();
            }
        };
        let vault = match check(&self.vault, &chosen) {
            Ok(vault) => vault,
            Err(error) => {
                self.notice(error);
                return Task::none();
            }
        };
        let mut saves: Vec<Task<Message>> = (0..self.tabs.len()).map(|i| self.save(i, false)).collect();
        saves.push(self.save_settings());
        Task::batch(saves).chain(Task::done(Message::SwitchVault(vault)))
    }

    /// Starts Sage on `vault` and closes this window, unless a note still
    /// has edits that are not on disk.
    pub(crate) fn switch_vault(&mut self, vault: PathBuf) -> Task<Message> {
        let unsaved: Vec<String> = self.tabs.iter().filter(|t| t.busy()).map(|t| t.title.clone()).collect();
        if let Some(notice) = blocked(&unsaved) {
            self.notice(notice);
            return Task::none();
        }
        self.persist_now();
        let started = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg(&vault).spawn());
        match started {
            Ok(_) => iced::exit(),
            Err(error) => {
                self.notice(format!("Could not open {}: {error}", vault.display()));
                Task::none()
            }
        }
    }

    /// The titles of notes whose disk text changed under unsaved edits: they
    /// cannot be saved until the conflict is resolved.
    fn conflicts(&self) -> Vec<String> {
        self.tabs.iter().filter(|t| t.conflict).map(|t| t.title.clone()).collect()
    }
}

/// Why the vault cannot be switched while these notes are open, if any are.
fn blocked(notes: &[String]) -> Option<String> {
    match notes {
        [] => None,
        [one] => Some(format!("Vault not switched: {one} has edits that are not saved")),
        [first, rest @ ..] => {
            Some(format!("Vault not switched: {first} and {} more have edits that are not saved", rest.len()))
        }
    }
}

/// The chosen folder as Sage opens it, or why it cannot be the vault.
fn check(current: &Path, chosen: &Path) -> Result<PathBuf, String> {
    let shown = chosen.display();
    let vault = sage_watcher::canonical(chosen).map_err(|e| format!("Cannot open {shown}: {e}"))?;
    if !vault.is_dir() {
        return Err(format!("Cannot open {shown}: not a folder"));
    }
    std::fs::read_dir(&vault).map_err(|e| format!("Cannot read {shown}: {e}"))?;
    if vault == current {
        return Err(format!("{shown} is the vault already open"));
    }
    sage_db::index_path_for(&vault).map_err(|e| format!("Cannot open {shown}: {e:#}"))?;
    Ok(vault)
}

/// Shows the system's folder picker; `None` when it was cancelled.
#[cfg(windows)]
fn pick() -> Result<Option<PathBuf>, String> {
    Ok(rfd::FileDialog::new().set_title("Open a vault").pick_folder())
}

/// Shows zenity's or kdialog's folder picker, whichever is installed; `None`
/// when it was cancelled.
#[cfg(not(windows))]
fn pick() -> Result<Option<PathBuf>, String> {
    use std::os::unix::ffi::OsStringExt;
    let pickers: [(&str, &[&str]); 2] = [
        ("zenity", &["--file-selection", "--directory", "--title=Open a vault"]),
        ("kdialog", &["--getexistingdirectory", ".", "--title", "Open a vault"]),
    ];
    for (program, args) in pickers {
        let Ok(output) = std::process::Command::new(program).args(args).output() else { continue };
        if !output.status.success() {
            // Both exit with 1 when cancelled.
            return match output.status.code() {
                Some(1) => Ok(None),
                _ => Err(format!("{program}: {}", String::from_utf8_lossy(&output.stderr).trim())),
            };
        }
        let mut path = output.stdout;
        while path.last() == Some(&b'\n') {
            path.pop();
        }
        return Ok((!path.is_empty()).then(|| PathBuf::from(std::ffi::OsString::from_vec(path))));
    }
    Err("install zenity or kdialog, or start sage with the vault's path".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_readable_folder_is_taken() {
        let (current, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let current = sage_watcher::canonical(current.path()).unwrap();
        let vault = check(&current, other.path()).unwrap();
        assert_eq!(vault, sage_watcher::canonical(other.path()).unwrap());
    }

    #[test]
    fn the_open_vault_a_file_and_a_missing_folder_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let current = sage_watcher::canonical(dir.path()).unwrap();
        assert!(check(&current, dir.path()).unwrap_err().contains("already open"));

        let file = dir.path().join("note.md");
        std::fs::write(&file, "").unwrap();
        assert!(check(Path::new("elsewhere"), &file).unwrap_err().contains("not a folder"));

        let missing = dir.path().join("missing");
        assert!(check(Path::new("elsewhere"), &missing).unwrap_err().starts_with("Cannot open"));
    }

    #[test]
    fn notes_with_unsaved_edits_block_the_switch() {
        assert_eq!(blocked(&[]), None);
        assert_eq!(blocked(&["Inbox".into()]).unwrap(), "Vault not switched: Inbox has edits that are not saved");
        let many = ["Inbox".into(), "Today".into(), "Ideas".into()];
        assert_eq!(blocked(&many).unwrap(), "Vault not switched: Inbox and 2 more have edits that are not saved");
    }
}
