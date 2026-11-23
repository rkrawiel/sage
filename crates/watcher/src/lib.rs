//! Chooses how a vault is watched and delivers raw change hints, and holds the
//! other few things that differ between Linux and Windows: the canonical form
//! of a vault's path, and where per-user data lives.
//!
//! * Native filesystems get kernel notifications through `notify`: inotify on
//!   Linux, change notifications on Windows.
//! * 9p (a Windows drive under WSL2, e.g. `/mnt/c`), SMB/CIFS, NFS, FUSE and
//!   Windows network drives get no watcher at all: notifications there miss
//!   changes made on the other side, so the indexer falls back to periodic
//!   mtime + size scans.
//!
//! The watcher only reports "these paths may have changed". Deciding what really
//! changed is the indexer's job, which is why a missed or duplicated event is
//! harmless.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Kernel notifications, with a slow safety-net rescan.
    Native,
    /// Periodic scan only.
    Poll,
}

impl Backend {
    /// How often the indexer runs a full reconciliation when nothing else wakes it.
    pub fn rescan_interval(self) -> Duration {
        match self {
            Backend::Native => Duration::from_secs(600),
            Backend::Poll => Duration::from_secs(45),
        }
    }

    /// Rename-over is atomic on native filesystems. On 9p it makes the OneDrive
    /// client see a delete plus a create, so files there are rewritten in place.
    pub fn atomic_save(self) -> bool {
        self == Backend::Native
    }

    pub fn label(self) -> &'static str {
        match self {
            Backend::Native if cfg!(windows) => "watching",
            Backend::Native => "inotify",
            Backend::Poll => "polling",
        }
    }
}

/// Picks the backend from the filesystem type that `path` lives on.
#[cfg(target_os = "linux")]
pub fn detect_backend(path: &Path) -> Backend {
    const V9FS: i64 = 0x0102_1997;
    const SMB: i64 = 0x517B;
    const CIFS: i64 = 0xFF53_4D42;
    const SMB2: i64 = 0xFE53_4D42;
    const NFS: i64 = 0x6969;
    const FUSE: i64 = 0x6573_5546;

    match fs_type(path) {
        Some(V9FS | SMB | CIFS | SMB2 | NFS | FUSE) => Backend::Poll,
        Some(_) => Backend::Native,
        // If statfs fails, polling is the choice that cannot miss changes.
        None => Backend::Poll,
    }
}

#[cfg(target_os = "linux")]
fn fs_type(path: &Path) -> Option<i64> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut buf = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: c_path is a valid NUL-terminated string and buf is a valid out pointer.
    let rc = unsafe { libc::statfs(c_path.as_ptr(), buf.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }
    // SAFETY: statfs returned 0, so buf is initialised.
    let fs = unsafe { buf.assume_init() };
    // f_type is signed on some targets; mask so the magic numbers compare equal.
    Some(fs.f_type as i64 & 0xFFFF_FFFF)
}

/// Picks the backend from the kind of drive `path` lives on: a network share,
/// by UNC path or mapped drive letter, is polled; a local drive is watched.
#[cfg(windows)]
pub fn detect_backend(path: &Path) -> Backend {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_REMOTE;
    let root = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => format!("{}:\\", letter as char),
            // \\server\share, however it is written.
            _ => return Backend::Poll,
        },
        _ => return Backend::Poll,
    };
    let wide: Vec<u16> = std::ffi::OsStr::new(&root).encode_wide().chain([0]).collect();
    // SAFETY: wide is a valid NUL-terminated UTF-16 string.
    match unsafe { GetDriveTypeW(wide.as_ptr()) } {
        DRIVE_REMOTE => Backend::Poll,
        _ => Backend::Native,
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn detect_backend(_: &Path) -> Backend {
    Backend::Poll
}

/// The canonical form of `path`, as vault paths are kept. On Windows this is
/// without the `\\?\` prefix that `canonicalize` adds, which other programs
/// and people do not expect, wherever the plain form names the same folder.
pub fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    let path = path.canonicalize()?;
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if let Some(Component::Prefix(prefix)) = path.components().next() {
            if let Prefix::VerbatimDisk(_) = prefix.kind() {
                let plain = PathBuf::from(&path.as_os_str().to_string_lossy()[4..]);
                // Long paths need the prefix, so they keep it.
                if plain.as_os_str().len() < 260 && plain.exists() {
                    return Ok(plain);
                }
            }
        }
    }
    Ok(path)
}

/// Where Sage keeps per-user data that is not part of any vault: the index,
/// the session and the Kanban order. `$XDG_DATA_HOME/sage` or
/// `~/.local/share/sage` on Linux, `%LOCALAPPDATA%\sage` on Windows.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).filter(|p| p.is_absolute());
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    base.map(|b| b.join("sage"))
}

/// Keeps the OS watcher alive. Dropping it stops notifications.
pub struct WatchGuard {
    _watcher: Option<RecommendedWatcher>,
}

/// Starts watching `vault`. `on_paths` is called from the watcher's own thread
/// with batches of absolute paths, so it must be cheap: push into a channel.
///
/// For [`Backend::Poll`] this installs nothing and returns an empty guard.
pub fn watch(
    vault: &Path,
    backend: Backend,
    on_paths: impl Fn(Vec<PathBuf>) + Send + 'static,
) -> notify::Result<WatchGuard> {
    if backend == Backend::Poll {
        return Ok(WatchGuard { _watcher: None });
    }
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        match res {
            // Reads and opens are noise.
            Ok(event) if !matches!(event.kind, EventKind::Access(_)) => on_paths(event.paths),
            Ok(_) => {}
            // Queue overflow or a watch error: an empty batch asks for a rescan.
            Err(_) => on_paths(Vec::new()),
        }
    })?;
    watcher.watch(vault, RecursiveMode::Recursive)?;
    Ok(WatchGuard { _watcher: Some(watcher) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_folders_are_watched() {
        let dir = canonical(&std::env::temp_dir()).unwrap();
        assert_eq!(detect_backend(&dir), Backend::Native);
        assert!(dir.is_absolute());
        assert!(!dir.to_string_lossy().starts_with(r"\\?\"), "{dir:?}");
        assert!(data_dir().is_some_and(|d| d.ends_with("sage")));
    }

    #[cfg(windows)]
    #[test]
    fn network_shares_are_polled() {
        assert_eq!(detect_backend(Path::new(r"\\server\share\vault")), Backend::Poll);
        assert_eq!(detect_backend(Path::new(r"\\?\UNC\server\share\vault")), Backend::Poll);
    }
}
