//! System notifications, for task reminders. Windows shows a toast through
//! WinRT; elsewhere `notify-send` (libnotify) is asked to, when it is
//! installed. Both block briefly, so callers run them off the UI thread. A
//! failure comes back as a sentence for the status bar; the reminder has
//! shown in Sage's own toast by then.

/// The AppUserModelID the installer and `install.ps1` register for Sage's
/// toasts, under `HKCU\Software\Classes\AppUserModelId`.
#[cfg(windows)]
const APP_ID: &str = "Sage.Sage";

/// Shows a notification with `title` and `body`.
#[cfg(windows)]
pub fn show(title: &str, body: &str) -> Result<(), String> {
    use tauri_winrt_notification::Toast;
    // Without the registration, Windows drops toasts from an unknown app id
    // silently, so a build that was never installed borrows PowerShell's.
    let app_id = if registered() { APP_ID } else { Toast::POWERSHELL_APP_ID };
    Toast::new(app_id).title(title).text1(body).show().map_err(|e| format!("Notification failed: {e}"))
}

/// Whether Sage's app id is registered for the current user.
#[cfg(windows)]
fn registered() -> bool {
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ};
    let key: Vec<u16> = format!(r"Software\Classes\AppUserModelId\{APP_ID}").encode_utf16().chain(Some(0)).collect();
    let mut handle: HKEY = std::ptr::null_mut();
    // SAFETY: `key` is NUL-terminated UTF-16 that outlives the call, and
    // `handle` is a valid out pointer; an opened key is closed at once.
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut handle) != 0 {
            return false;
        }
        RegCloseKey(handle);
    }
    true
}

/// Shows a notification with `title` and `body`.
#[cfg(not(windows))]
pub fn show(title: &str, body: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};
    // `--` keeps a task that starts with `-` from reading as an option.
    let status = Command::new("notify-send")
        .args(["--app-name=Sage", "--", title, body])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Notification failed: notify-send: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("Notification failed: notify-send {status}")) }
}
