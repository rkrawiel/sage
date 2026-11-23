//! OneDrive, recognised by path and file metadata alone: no Graph API and no
//! calls into Windows. Which folder is a OneDrive root, which files are
//! cloud-only placeholders that reading would download, and which names are
//! the conflict copies OneDrive leaves when two edits collide.

use std::fs::Metadata;
use std::path::{Path, PathBuf};

/// The OneDrive folder `path` lies in: the nearest ancestor (or `path` itself)
/// named `OneDrive` or `OneDrive - <organisation>`, ignoring case.
pub fn root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| {
            dir.file_name().and_then(|n| n.to_str()).is_some_and(|name| {
                let name = name.to_lowercase();
                name == "onedrive" || name.starts_with("onedrive - ")
            })
        })
        .map(Path::to_path_buf)
}

/// NTFS keeps a small file inside its file record: up to about 700 bytes with
/// 1 KB records, more with 4 KB ones. Such a file has no allocated blocks even
/// when it is fully local, so a missing allocation only means "in the cloud"
/// above this size.
#[cfg(unix)]
const RESIDENT_MAX: u64 = 4096;

/// Whether a file is a Files On-Demand placeholder, whose content is still in
/// the cloud.
///
/// On Windows the file says so itself: reading it would recall its data, or it
/// is marked offline. Through WSL it only shows its real size but no allocated
/// blocks. Small local files have no blocks either, because NTFS keeps them
/// inside the file record, so there only files larger than 4 KB count; a small
/// cloud-only file just gets downloaded when read.
pub fn is_placeholder(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const OFFLINE: u32 = 0x0000_1000;
        const RECALL_ON_OPEN: u32 = 0x0004_0000;
        const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
        meta.is_file() && meta.file_attributes() & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.is_file() && meta.len() > RESIDENT_MAX && meta.blocks() == 0
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = meta;
        false
    }
}

/// The file names a conflict copy may stand for, most specific first. OneDrive
/// appends the computer name: `Note-DESKTOP-1A2B3C.md`, `Note-TPT14-G9-ROBERT.md`.
/// A computer name is up to 15 uppercase letters, digits and hyphens, with at
/// least one letter, so `Note-TPT14-G9-ROBERT.md` could be a copy of
/// `Note-TPT14-G9.md`, `Note-TPT14.md` or `Note.md`. Empty for other names.
/// Which original exists is for the caller to check.
pub fn originals_of(file_name: &str) -> Vec<String> {
    let (base, ext) = match file_name.rsplit_once('.') {
        Some((base, ext)) if !base.is_empty() => (base, format!(".{ext}")),
        _ => (file_name, String::new()),
    };
    let mut out: Vec<String> = Vec::new();
    // Every hyphen can start the suffix; the last one leaves the longest stem.
    for (i, _) in base.match_indices('-').rev() {
        let (stem, suffix) = (&base[..i], &base[i + 1..]);
        if stem.is_empty() || !is_copy_suffix(suffix) {
            continue;
        }
        let original = format!("{stem}{ext}");
        if !out.contains(&original) {
            out.push(original);
        }
    }
    out
}

/// `HOST` or `HOST-N`: what OneDrive appends to a conflict copy.
fn is_copy_suffix(suffix: &str) -> bool {
    is_host(suffix)
        || suffix.rsplit_once('-').is_some_and(|(host, n)| {
            !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && is_host(host)
        })
}

/// A Windows computer name as OneDrive writes it: 1 to 15 uppercase letters,
/// digits and hyphens, at least one letter, no hyphen at either end.
fn is_host(s: &str) -> bool {
    (1..=15).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        && s.bytes().any(|b| b.is_ascii_uppercase())
        && !s.starts_with('-')
        && !s.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_by_name() {
        let r = |p: &str| root(Path::new(p)).map(|p| p.to_string_lossy().into_owned());
        assert_eq!(r("/mnt/c/Users/x/OneDrive/Vault/Daily").as_deref(), Some("/mnt/c/Users/x/OneDrive"));
        assert_eq!(
            r("/mnt/c/Users/x/OneDrive - Gravity 9 Solutions Ltd/Notes").as_deref(),
            Some("/mnt/c/Users/x/OneDrive - Gravity 9 Solutions Ltd")
        );
        assert_eq!(r("/mnt/d/onedrive").as_deref(), Some("/mnt/d/onedrive"), "the path itself, any case");
        assert_eq!(r("/mnt/c/Users/x/OneDriveBackup/Vault"), None);
        assert_eq!(r("/home/x/notes"), None);
    }

    #[test]
    fn placeholders_have_size_but_no_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let meta = |name: &str| std::fs::metadata(dir.path().join(name)).unwrap();

        // A sparse file looks exactly like a placeholder: a size, no blocks.
        std::fs::File::create(dir.path().join("cloud.md")).unwrap().set_len(8192).unwrap();
        std::fs::write(dir.path().join("small.md"), "short note").unwrap();
        std::fs::write(dir.path().join("local.md"), vec![b'a'; 8192]).unwrap();

        assert_eq!(is_placeholder(&meta("cloud.md")), cfg!(unix));
        assert!(!is_placeholder(&meta("small.md")));
        assert!(!is_placeholder(&meta("local.md")));
        assert!(!is_placeholder(&std::fs::metadata(dir.path()).unwrap()), "folders never are");
    }

    #[test]
    fn conflict_copy_names() {
        assert_eq!(originals_of("Note-DESKTOP-1A2B3C.md"), ["Note-DESKTOP.md", "Note.md"]);
        assert_eq!(originals_of("Note-TPT14-G9-ROBERT.md"), ["Note-TPT14-G9.md", "Note-TPT14.md", "Note.md"]);
        assert_eq!(originals_of("Plan for Q4-LAPTOP-2.md"), ["Plan for Q4.md"]);
        // A numbered copy of a 15-character computer name only reads as `HOST-N`.
        assert_eq!(
            originals_of("Note-TPT14-G9-ROBERT-2.md"),
            ["Note-TPT14-G9.md", "Note-TPT14.md", "Note.md"]
        );
        assert_eq!(originals_of("a-b-PC.md"), ["a-b.md"]);
        assert!(originals_of("Project-Alpha.md").is_empty(), "mixed case is a name, not a computer");
        assert!(originals_of("Note-2026.md").is_empty(), "a computer name has a letter");
        assert!(originals_of("-DESKTOP.md").is_empty(), "no stem");
        assert!(originals_of("Note-VERYLONGCOMPUTERNAME.md").is_empty(), "over 15 characters");
        assert!(originals_of("Note.md").is_empty());
        // A WhatsApp picture name passes too; callers only act when the
        // original exists next to it.
        assert_eq!(originals_of("IMG-20250220-WA0055.jpg"), ["IMG-20250220.jpg", "IMG.jpg"]);
        assert_eq!(originals_of("Notes-PC"), ["Notes"], "no extension");
    }
}
