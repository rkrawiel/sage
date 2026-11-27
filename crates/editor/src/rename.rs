//! Renaming or moving a note, or a folder with everything in it, with every
//! link to what moved rewritten, as one step that can be undone.
//!
//! The index says which notes may link to what moves and supplies the file
//! list that links resolve against. The linking notes themselves are re-read
//! and re-parsed: the index can trail the disk (by a whole polling interval on
//! 9p), and patching text by stale positions would corrupt it.

use crate::{merge, save_guarded};
use sage_core::{LineEdit, Link};
use sage_watcher::Backend;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A completed rename, with what is needed to undo it.
#[derive(Debug)]
pub struct Rename {
    pub from: String,
    pub to: String,
    /// A folder moved with everything in it, rather than one note.
    pub folder: bool,
    /// Notes whose text changed, under their path after the rename.
    pub rewrites: Vec<Rewrite>,
    /// How many links were rewritten.
    pub links: usize,
    /// Folders created for the move. Undo removes them again while empty.
    created_dirs: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct Rewrite {
    pub path: String,
    pub before: String,
    pub after: String,
}

impl Rename {
    /// Where the file at `path` went, if the rename moved it.
    pub fn moved(&self, path: &str) -> Option<String> {
        map_path(&self.from, &self.to, self.folder, path)
    }

    /// Where the file now at `path` was before the rename, if it moved.
    pub fn unmoved(&self, path: &str) -> Option<String> {
        map_path(&self.to, &self.from, self.folder, path)
    }
}

/// `path` after moving `from` to `to`, a note or, with `folder`, a folder.
fn map_path(from: &str, to: &str, folder: bool, path: &str) -> Option<String> {
    if folder {
        path.strip_prefix(from)?.strip_prefix('/').map(|rest| format!("{to}/{rest}"))
    } else {
        (path == from).then(|| to.to_owned())
    }
}

/// Renames the note at vault-relative `from` to `to` and rewrites the links that
/// point at it, keeping each link's style. Links elsewhere that the new name
/// would capture are pinned to their current target with a full path. Notes in
/// `busy` have unsaved edits and are never rewritten underneath the editor.
pub fn rename(
    vault: &Path,
    db: &Connection,
    from: &str,
    to: &str,
    busy: &HashSet<String>,
    backend: Backend,
) -> Result<Rename, String> {
    if from == to {
        return Err("the name did not change".into());
    }
    apply(vault, db, from, to, false, busy, backend)
}

/// Moves the folder at vault-relative `from`, with everything in it, to `to`
/// and rewrites the links into and out of it as [`rename`] does for a note.
pub fn rename_folder(
    vault: &Path,
    db: &Connection,
    from: &str,
    to: &str,
    busy: &HashSet<String>,
    backend: Backend,
) -> Result<Rename, String> {
    if from == to {
        return Err("the name did not change".into());
    }
    if from.is_empty() || to.is_empty() {
        return Err("the vault itself cannot move".into());
    }
    if to.starts_with(&format!("{from}/")) {
        return Err(format!("{from} cannot move into itself"));
    }
    if !vault.join(from).is_dir() {
        return Err(format!("{from} is not a folder"));
    }
    apply(vault, db, from, to, true, busy, backend)
}

fn apply(
    vault: &Path,
    db: &Connection,
    from: &str,
    to: &str,
    folder: bool,
    busy: &HashSet<String>,
    backend: Backend,
) -> Result<Rename, String> {
    let (rewrites, links) = plan(vault, db, from, to, folder).map_err(|e| format!("reading the index: {e}"))?;
    let original = |path: &str| map_path(to, from, folder, path).unwrap_or_else(|| path.to_owned());
    let moving = busy.iter().filter(|path| map_path(from, to, folder, path).is_some());
    if let Some(path) = rewrites.iter().map(|rw| original(&rw.path)).find(|path| busy.contains(path)).or(moving.cloned().next()) {
        return Err(format!("{path} has unsaved changes, save it first"));
    }

    // Notes that move are rewritten in place before they move, so every step
    // can be checked against the disk as it is now.
    let originals: Vec<String> = rewrites.iter().map(|rw| original(&rw.path)).collect();
    let mut steps = Vec::new();
    for (rw, path) in rewrites.iter().zip(&originals) {
        steps.push(Step::Write { path, old: &rw.before, new: &rw.after });
    }
    steps.push(Step::Move { from, to, folder });
    for step in &steps {
        step.check(vault)?;
    }

    let created_dirs = create_parents(vault, to)?;
    if let Err(e) = run(vault, &steps, backend) {
        remove_dirs(&created_dirs);
        return Err(e);
    }
    Ok(Rename { from: from.to_owned(), to: to.to_owned(), folder, rewrites, links, created_dirs })
}

/// Reverses a rename: moves the note or folder back and restores every
/// rewritten note. A note edited since keeps those edits where they do not
/// touch the rewritten links; if they do, nothing changes and the undo is
/// refused. Returns each restored note's path and new text.
pub fn undo(vault: &Path, rename: &Rename, backend: Backend) -> Result<Vec<(String, String)>, String> {
    let Rename { from, to, folder, rewrites, .. } = rename;
    // Per note: its text now, and the text to write back.
    let mut texts = Vec::new();
    for rw in rewrites {
        let disk = std::fs::read_to_string(vault.join(&rw.path)).map_err(|e| format!("{}: {e}", rw.path))?;
        let restored = if disk == rw.after {
            rw.before.clone()
        } else {
            merge::rebase(&rw.after, &rw.before, &disk)
                .ok_or_else(|| format!("{} changed on disk where the rename rewrote it", rw.path))?
        };
        texts.push((disk, restored));
    }

    // Notes that moved are restored before they move back, the others after.
    let mut steps = Vec::new();
    let pairs = || rewrites.iter().zip(&texts);
    for (rw, (disk, restored)) in pairs().filter(|(rw, _)| rename.unmoved(&rw.path).is_some()) {
        steps.push(Step::Write { path: &rw.path, old: disk, new: restored });
    }
    steps.push(Step::Move { from: to, to: from, folder: *folder });
    for (rw, (disk, restored)) in pairs().filter(|(rw, _)| rename.unmoved(&rw.path).is_none()) {
        steps.push(Step::Write { path: &rw.path, old: disk, new: restored });
    }
    for step in &steps {
        step.check(vault)?;
    }

    // The old folder may have been removed since, e.g. by Obsidian.
    let created = create_parents(vault, from)?;
    if let Err(e) = run(vault, &steps, backend) {
        remove_dirs(&created);
        return Err(e);
    }
    remove_dirs(&rename.created_dirs);
    Ok(pairs().map(|(rw, (_, restored))| (rename.unmoved(&rw.path).unwrap_or_else(|| rw.path.clone()), restored.clone())).collect())
}

/// Computes the rewritten text of every note with a link that must change.
fn plan(vault: &Path, db: &Connection, from: &str, to: &str, folder: bool) -> rusqlite::Result<(Vec<Rewrite>, usize)> {
    let moves = |path: &str| map_path(from, to, folder, path);

    // What moves. A note may not be indexed yet.
    let moving: Vec<String> = if folder {
        let mut stmt = db.prepare("SELECT path FROM files WHERE substr(path, 1, length(?1)) = ?1")?;
        let rows = stmt.query_map([format!("{from}/")], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    } else {
        vec![from.to_owned()]
    };
    // Names that get a new holder: a renamed note's new name.
    let mut arriving: HashMap<String, Vec<String>> = HashMap::new();
    for path in &moving {
        let after = moves(path).expect("it moves");
        if sage_core::name_key(&after) != sage_core::name_key(path) {
            arriving.entry(sage_core::name_key(&after)).or_default().push(after);
        }
    }

    // The notes that move, for their own relative links, then every note that
    // links to what moves or to another file with one of its names.
    let mut sources: Vec<String> = moving.iter().filter(|p| is_note(p)).cloned().collect();
    if !folder && !sources.contains(&from.to_owned()) {
        sources.push(from.to_owned());
    }
    let mut names: HashSet<String> = arriving.keys().cloned().collect();
    names.extend(moving.iter().map(|p| sage_core::name_key(p)));
    // Links into what moves, in one query rather than one per moved file.
    let (moved, key) = if folder { ("substr(d.path, 1, length(?1)) = ?1", format!("{from}/")) } else { ("d.path = ?1", from.to_owned()) };
    let mut stmt = db.prepare(&format!(
        "SELECT DISTINCT s.path FROM links l JOIN files s ON s.id = l.src_file_id JOIN files d ON d.id = l.dst_file_id WHERE {moved}"
    ))?;
    let mut linking: Vec<String> = stmt.query_map([key], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut stmt = db.prepare_cached(
        "SELECT DISTINCT s.path FROM links l JOIN files s ON s.id = l.src_file_id
         WHERE l.target_name = ?1 AND l.dst_file_id IS NOT NULL",
    )?;
    for name in &names {
        linking.extend(stmt.query_map([name], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?);
    }
    // A set beside the list, which keeps its order: a big vault can have
    // thousands of linking notes, and a list lookup for each is quadratic.
    let mut seen: HashSet<String> = sources.iter().cloned().collect();
    for path in linking {
        if seen.insert(path.clone()) {
            sources.push(path);
        }
    }

    let mut by_name: HashMap<String, Vec<String>> = HashMap::new();
    let mut rewrites = Vec::new();
    let mut count = 0;
    for src in &sources {
        // Missing, or not UTF-8: nothing here can be rewritten.
        let Ok(text) = std::fs::read_to_string(vault.join(src)) else { continue };
        let src_after = moves(src).unwrap_or_else(|| src.clone());
        let mut edits = Vec::new();
        for link in sage_core::parse(&text).links {
            if link.target.trim().is_empty() {
                continue; // [[#Heading]] stays inside its note
            }
            let key = sage_core::target_key(&link.target, src);
            let name = key.rsplit('/').next().unwrap_or("").to_owned();
            let Some(path) = sage_core::resolve(&key, candidates(db, &mut by_name, &name)?.iter().map(String::as_str))
                .map(str::to_owned)
            else {
                continue;
            };
            // The files with a name after the move, for resolving links then.
            let mut after = |name: &str| -> rusqlite::Result<Vec<String>> {
                let mut paths: Vec<String> = candidates(db, &mut by_name, name)?
                    .iter()
                    .map(|p| moves(p).unwrap_or_else(|| p.clone()))
                    .filter(|p| sage_core::name_key(p) == name)
                    .collect();
                paths.extend(arriving.get(name).into_iter().flatten().cloned());
                Ok(paths)
            };
            let (target, bare_ok) = match moves(&path) {
                // A bare name stays bare while it still finds the file.
                Some(target) => {
                    let name = sage_core::name_key(&target);
                    let bare_ok = sage_core::resolve(&name, after(&name)?.iter().map(String::as_str)) == Some(target.as_str());
                    (target, bare_ok)
                }
                // A moving note's relative links have to follow it.
                None if moves(src).is_some() && is_relative(&link.target) => (path, false),
                // A link the move would capture is pinned to where it points now.
                None => {
                    if sage_core::resolve(&key, after(&name)?.iter().map(String::as_str)) == Some(path.as_str()) {
                        continue;
                    }
                    (path, false)
                }
            };
            edits.extend(retarget(&text, &link, &target, &src_after, bare_ok));
        }
        let Some(after) = sage_core::edit_lines(&text, &edits) else { continue };
        if after != text {
            count += edits.len();
            rewrites.push(Rewrite { path: src_after, before: text, after });
        }
    }
    Ok((rewrites, count))
}

/// The indexed files whose name is `name`, cached in `by_name`.
fn candidates<'a>(db: &Connection, by_name: &'a mut HashMap<String, Vec<String>>, name: &str) -> rusqlite::Result<&'a Vec<String>> {
    if !by_name.contains_key(name) {
        let mut stmt = db.prepare_cached("SELECT path FROM files WHERE name_key = ?1")?;
        let paths = stmt.query_map([name], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
        by_name.insert(name.to_owned(), paths);
    }
    Ok(&by_name[name])
}

fn is_note(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md"))
}

/// The edit that points `link`, found in `src`, at vault path `target`, or
/// `None` if its text would not change.
fn retarget(src: &str, link: &Link, target: &str, src_after: &str, bare_ok: bool) -> Option<LineEdit> {
    let line = sage_core::line_at(src, link.line)?;
    let link_src = line.get(link.col as usize..(link.col + link.len) as usize)?;
    let (range, angle) = sage_core::target_range(link_src, link.markdown)?;
    let raw = &link_src[range.clone()];
    let text = write_target(raw, link.markdown, angle, target, src_after, bare_ok);
    (text != raw).then(|| LineEdit {
        line: link.line,
        col: link.col + range.start as u32,
        len: range.len() as u32,
        text,
    })
}

/// A link target for vault path `target`, in the style of `raw`, the target as
/// written before: relative stays relative, a path stays a path, a bare name
/// stays bare while the new name is unique, `.md` is kept only if it was
/// written, and Markdown links encode spaces unless they use `<...>`.
fn write_target(raw: &str, markdown: bool, angle: bool, target: &str, src_after: &str, bare_ok: bool) -> String {
    let old = if markdown { sage_core::percent_decode(raw) } else { raw.to_owned() };
    let old = old.trim();
    let mut new = if is_relative(old) {
        relative_path(src_after, target)
    } else if bare_ok && !old.contains('/') {
        target.rsplit('/').next().unwrap_or(target).to_owned()
    } else if old.starts_with('/') {
        format!("/{target}")
    } else {
        target.to_owned()
    };
    if target.to_lowercase().ends_with(".md") && !old.to_lowercase().ends_with(".md") {
        new.truncate(new.len() - 3);
    }
    if markdown && !angle {
        new = new.replace(' ', "%20");
    }
    new
}

fn is_relative(target: &str) -> bool {
    let target = target.trim();
    target.starts_with("./") || target.starts_with("../")
}

/// `target` relative to the folder of the note at `src`, always starting with
/// `./` or `../` so that it still reads as relative.
fn relative_path(src: &str, target: &str) -> String {
    let from: Vec<&str> = src.split('/').collect();
    let from = &from[..from.len() - 1];
    let to: Vec<&str> = target.split('/').collect();
    let common = from.iter().zip(&to[..to.len() - 1]).take_while(|(a, b)| a == b).count();
    let mut parts = vec![".."; from.len() - common];
    parts.extend(&to[common..]);
    let path = parts.join("/");
    if path.starts_with("../") { path } else { format!("./{path}") }
}

/// One reversible change on disk.
enum Step<'a> {
    Write { path: &'a str, old: &'a str, new: &'a str },
    /// A note, or with `folder` a folder and everything in it.
    Move { from: &'a str, to: &'a str, folder: bool },
}

impl Step<'_> {
    /// Whether the step can run against the disk as it is now.
    fn check(&self, vault: &Path) -> Result<(), String> {
        match *self {
            Step::Write { path, old, .. } => match std::fs::read(vault.join(path)) {
                Ok(disk) if disk == old.as_bytes() => Ok(()),
                Ok(_) => Err(format!("{path} changed on disk")),
                Err(e) => Err(format!("{path}: {e}")),
            },
            Step::Move { from, to, folder } => {
                let (src, dst) = (vault.join(from), vault.join(to));
                if !(if folder { src.is_dir() } else { src.is_file() }) {
                    Err(format!("{from} no longer exists"))
                } else if occupied(&src, &dst) {
                    Err(format!("{to} already exists"))
                } else {
                    Ok(())
                }
            }
        }
    }

    fn perform(&self, vault: &Path, backend: Backend) -> Result<(), String> {
        match *self {
            Step::Write { path, old, new } => {
                save_guarded(&vault.join(path), old, new, backend).map_err(|e| format!("{path}: {e}"))
            }
            Step::Move { from, to, .. } => {
                // rename(2) replaces an existing target, so check right before.
                self.check(vault)?;
                crate::rename_over(&vault.join(from), &vault.join(to)).map_err(|e| format!("moving {from}: {e}"))
            }
        }
    }

    fn inverse(&self) -> Self {
        match *self {
            Step::Write { path, old, new } => Step::Write { path, old: new, new: old },
            Step::Move { from, to, folder } => Step::Move { from: to, to: from, folder },
        }
    }
}

/// Runs `steps` in order. If one fails, the ones already done are reversed.
/// Steps that cannot be reversed are named in the error, since the vault is
/// then left half-renamed and the user has to finish by hand.
fn run(vault: &Path, steps: &[Step], backend: Backend) -> Result<(), String> {
    for (i, step) in steps.iter().enumerate() {
        if let Err(e) = step.perform(vault, backend) {
            let stuck: Vec<String> = steps[..i].iter().rev().filter_map(|done| done.inverse().perform(vault, backend).err()).collect();
            if stuck.is_empty() {
                return Err(e);
            }
            return Err(format!("{e}; undoing the steps before it also failed, so the vault is partly renamed: {}", stuck.join("; ")));
        }
    }
    Ok(())
}

/// Whether `dst` is taken by a file other than `src`. A case-only rename on a
/// case-insensitive filesystem (NTFS, also through 9p) finds the source itself
/// there.
fn occupied(src: &Path, dst: &Path) -> bool {
    match (std::fs::metadata(src), std::fs::metadata(dst)) {
        (_, Err(_)) => false,
        #[cfg(unix)]
        (Ok(ours), Ok(theirs)) => {
            use std::os::unix::fs::MetadataExt;
            (ours.dev(), ours.ino()) != (theirs.dev(), theirs.ino())
        }
        // Windows gives the name a file really has, in its own case, so two
        // names for one file canonicalize alike.
        #[cfg(windows)]
        (Ok(_), Ok(_)) => !matches!((src.canonicalize(), dst.canonicalize()), (Ok(a), Ok(b)) if a == b),
        _ => true,
    }
}

/// Creates the missing folders above `rel`, outermost first, and returns them.
fn create_parents(vault: &Path, rel: &str) -> Result<Vec<PathBuf>, String> {
    let mut missing = Vec::new();
    let mut dir = vault.join(rel).parent().map(Path::to_path_buf);
    while let Some(d) = dir.filter(|d| !d.exists()) {
        dir = d.parent().map(Path::to_path_buf);
        missing.push(d);
    }
    missing.reverse();
    for (i, d) in missing.iter().enumerate() {
        if let Err(e) = std::fs::create_dir(d) {
            remove_dirs(&missing[..i]);
            return Err(format!("creating {}: {e}", d.display()));
        }
    }
    Ok(missing)
}

/// Removes folders innermost first, leaving any that are no longer empty.
fn remove_dirs(dirs: &[PathBuf]) {
    for d in dirs.iter().rev() {
        let _ = std::fs::remove_dir(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn index(vault: &Path, db: &Path) {
        let mut ix = sage_indexer::Indexer::open(vault.to_path_buf(), db).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
    }

    fn read(vault: &Path, rel: &str) -> String {
        fs::read_to_string(vault.join(rel)).unwrap()
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        vault: PathBuf,
        db_path: PathBuf,
    }

    fn fixture(files: &[(&str, &str)]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        for (rel, text) in files {
            let path = vault.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let vault = vault.canonicalize().unwrap();
        let db_path = dir.path().join("index.db");
        index(&vault, &db_path);
        Fixture { _dir: dir, vault, db_path }
    }

    const HOME: &str = "[[Alpha]] ![[Projects/Alpha#Alpha]] [a](Projects/Alpha.md) [b](<Projects/Alpha.md>) [[Beta]]\n";
    const ALPHA: &str = "# Alpha\nSee [[#Alpha]] and [[Alpha#Alpha|me]] and [up](../Home.md)\n";
    const LOG: &str = "---\nup: \"[[alpha]]\"\n---\n- [ ] Call [[Alpha|the team]]\r\n";

    #[test]
    fn rewrites_every_link_and_undoes_as_one_step() {
        let f = fixture(&[
            ("Home.md", HOME),
            ("Projects/Alpha.md", ALPHA),
            ("Daily/Log.md", LOG),
            ("Other/Beta.md", "# The other Beta\n"),
        ]);
        let db = sage_db::open_reader(&f.db_path).unwrap();

        // The new name is taken by Other/Beta.md, and the vault-root note wins
        // bare [[Beta]] links, so the one that meant Other/Beta.md gets pinned.
        let r = rename(&f.vault, &db, "Projects/Alpha.md", "Beta.md", &HashSet::new(), Backend::Native).unwrap();
        assert!(!f.vault.join("Projects/Alpha.md").exists());
        assert_eq!(read(&f.vault, "Home.md"), "[[Beta]] ![[Beta#Alpha]] [a](Beta.md) [b](<Beta.md>) [[Other/Beta]]\n");
        assert_eq!(read(&f.vault, "Beta.md"), "# Alpha\nSee [[#Alpha]] and [[Beta#Alpha|me]] and [up](./Home.md)\n");
        assert_eq!(read(&f.vault, "Daily/Log.md"), "---\nup: \"[[Beta]]\"\n---\n- [ ] Call [[Beta|the team]]\r\n");
        assert_eq!((r.rewrites.len(), r.links), (3, 9));

        // Every link still resolves to what it meant before.
        index(&f.vault, &f.db_path);
        let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(count("SELECT count(*) FROM links WHERE dst_file_id IS NULL"), 0);
        assert_eq!(count("SELECT count(*) FROM v_backlinks b JOIN files f ON f.id = b.file_id WHERE f.path = 'Other/Beta.md'"), 1);

        undo(&f.vault, &r, Backend::Native).unwrap();
        assert!(!f.vault.join("Beta.md").exists());
        assert_eq!(read(&f.vault, "Projects/Alpha.md"), ALPHA);
        assert_eq!(read(&f.vault, "Home.md"), HOME);
        assert_eq!(read(&f.vault, "Daily/Log.md"), LOG);
    }

    #[test]
    fn moves_into_new_folders_and_refuses_unsafe_steps() {
        let f = fixture(&[("Home.md", HOME), ("Projects/Alpha.md", ALPHA), ("Taken.md", "")]);
        let db = sage_db::open_reader(&f.db_path).unwrap();

        let busy = HashSet::from(["Home.md".to_owned()]);
        let refused = rename(&f.vault, &db, "Projects/Alpha.md", "Journal/2026/Alpha.md", &busy, Backend::Poll);
        assert!(refused.unwrap_err().contains("unsaved"));
        let refused = rename(&f.vault, &db, "Projects/Alpha.md", "Taken.md", &HashSet::new(), Backend::Poll);
        assert!(refused.unwrap_err().contains("already exists"));
        assert_eq!(read(&f.vault, "Home.md"), HOME, "a refused rename changes nothing");

        let r = rename(&f.vault, &db, "Projects/Alpha.md", "Journal/2026/Alpha.md", &HashSet::new(), Backend::Poll)
            .unwrap();
        assert!(read(&f.vault, "Home.md").starts_with("[[Alpha]] ![[Journal/2026/Alpha#Alpha]] [a](Journal/2026/Alpha.md)"));
        assert!(read(&f.vault, "Journal/2026/Alpha.md").ends_with("[up](../../Home.md)\n"));

        // An edit over the rewritten link blocks the undo, which then changes nothing.
        fs::write(f.vault.join("Home.md"), "edited").unwrap();
        assert!(undo(&f.vault, &r, Backend::Poll).unwrap_err().contains("Home.md changed"));
        assert!(f.vault.join("Journal/2026/Alpha.md").exists());

        // An edit elsewhere in the note stays, and the link goes back.
        let after = &r.rewrites.iter().find(|rw| rw.path == "Home.md").unwrap().after;
        fs::write(f.vault.join("Home.md"), format!("# Added on top\n\n{after}")).unwrap();
        let restored = undo(&f.vault, &r, Backend::Poll).unwrap();
        assert_eq!(read(&f.vault, "Projects/Alpha.md"), ALPHA);
        assert_eq!(read(&f.vault, "Home.md"), format!("# Added on top\n\n{HOME}"));
        assert!(restored.contains(&("Home.md".to_owned(), format!("# Added on top\n\n{HOME}"))));
        assert!(restored.iter().any(|(path, _)| path == "Projects/Alpha.md"), "paths are the ones before the rename");
        assert!(!f.vault.join("Journal").exists(), "folders created for the move are removed");
    }

    #[test]
    fn a_rollback_that_fails_is_reported_with_the_original_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "a").unwrap();
        // Undoing the write leaves an empty a.md behind, which then blocks
        // moving b.md back.
        let steps = [
            Step::Move { from: "a.md", to: "b.md", folder: false },
            Step::Write { path: "a.md", old: "", new: "new" },
            Step::Move { from: "missing.md", to: "c.md", folder: false },
        ];
        let err = run(dir.path(), &steps, Backend::Native).unwrap_err();
        assert!(err.starts_with("missing.md no longer exists") && err.contains("a.md already exists"), "{err}");
    }

    #[test]
    fn renames_that_only_change_case() {
        let f = fixture(&[("Home.md", "[[Note]]\n"), ("Note.md", "x\n")]);
        let db = sage_db::open_reader(&f.db_path).unwrap();
        // On NTFS the new name finds the note itself, which must not count as taken.
        let r = rename(&f.vault, &db, "Note.md", "NOTE.md", &HashSet::new(), Backend::Native).unwrap();
        let names: Vec<String> =
            fs::read_dir(&f.vault).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"NOTE.md".to_owned()) && !names.contains(&"Note.md".to_owned()), "{names:?}");
        assert_eq!(read(&f.vault, "Home.md"), "[[NOTE]]\n");
        undo(&f.vault, &r, Backend::Native).unwrap();
        assert_eq!(read(&f.vault, "Home.md"), "[[Note]]\n");
    }

    #[test]
    fn rewrites_links_in_tables_and_leaves_code_samples_alone() {
        let home = "| a |\n|---|\n| [[Note\\|alias]] |\n````md\n```\n[[Note]]\n```\n````\n> ```\n> [[Note]]\n> ```\n";
        let f = fixture(&[("Home.md", home), ("Note.md", "x\n")]);
        let db = sage_db::open_reader(&f.db_path).unwrap();
        rename(&f.vault, &db, "Note.md", "Renamed.md", &HashSet::new(), Backend::Native).unwrap();
        assert_eq!(read(&f.vault, "Home.md"), home.replacen("[[Note\\|", "[[Renamed\\|", 1));
    }

    #[test]
    fn moves_folders_with_their_links() {
        let f = fixture(&[
            ("Home.md", "[[Alpha]] [[Projects/Alpha]] [spec](Projects/Specs/Spec%201.md) ![[Projects/pic.png]] [[Note]]\n"),
            ("Projects/Alpha.md", "[[Spec 1]] [up](../Home.md) [s](./Specs/Spec%201.md) [[Projects/Note]]\n"),
            ("Projects/Specs/Spec 1.md", "[a](../Alpha.md)\n"),
            ("Projects/Note.md", "inside\n"),
            ("Projects/pic.png", "png"),
            ("Projects/.hidden", "kept"),
            ("Other/Deep/Note.md", "outside\n"),
        ]);
        let db = sage_db::open_reader(&f.db_path).unwrap();
        let none = HashSet::new();

        assert!(rename_folder(&f.vault, &db, "Projects", "Projects/Sub", &none, Backend::Native).unwrap_err().contains("into itself"));
        assert!(rename_folder(&f.vault, &db, "Projects", "Other", &none, Backend::Native).unwrap_err().contains("already exists"));
        let busy = HashSet::from(["Projects/Specs/Spec 1.md".to_owned()]);
        assert!(rename_folder(&f.vault, &db, "Projects", "Archive", &busy, Backend::Native).unwrap_err().contains("unsaved"));

        let r = rename_folder(&f.vault, &db, "Projects", "Archive/2026/Work", &none, Backend::Native).unwrap();
        assert!(!f.vault.join("Projects").exists());
        assert_eq!(read(&f.vault, "Archive/2026/Work/.hidden"), "kept", "everything in the folder moves");
        // The bare [[Note]] in Home pointed at Projects/Note.md, the shorter
        // path; after the move Other/Deep/Note.md is shorter, so it is pinned.
        assert_eq!(
            read(&f.vault, "Home.md"),
            "[[Alpha]] [[Archive/2026/Work/Alpha]] [spec](Archive/2026/Work/Specs/Spec%201.md) \
             ![[Archive/2026/Work/pic.png]] [[Archive/2026/Work/Note]]\n"
        );
        assert_eq!(
            read(&f.vault, "Archive/2026/Work/Alpha.md"),
            "[[Spec 1]] [up](../../../Home.md) [s](./Specs/Spec%201.md) [[Archive/2026/Work/Note]]\n"
        );
        assert_eq!(read(&f.vault, "Archive/2026/Work/Specs/Spec 1.md"), "[a](../Alpha.md)\n", "links inside move along");
        assert_eq!(r.moved("Projects/Specs/Spec 1.md").as_deref(), Some("Archive/2026/Work/Specs/Spec 1.md"));
        assert_eq!(r.moved("Projects2/x.md"), None);
        assert_eq!(r.unmoved("Archive/2026/Work/Alpha.md").as_deref(), Some("Projects/Alpha.md"));

        index(&f.vault, &f.db_path);
        let unresolved: i64 =
            db.query_row("SELECT count(*) FROM links WHERE dst_file_id IS NULL", [], |r| r.get(0)).unwrap();
        let names: Vec<String> = db.prepare("SELECT target FROM links WHERE dst_file_id IS NULL").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(unresolved, 0, "{names:?}");

        undo(&f.vault, &r, Backend::Native).unwrap();
        assert_eq!(read(&f.vault, "Projects/Alpha.md"), "[[Spec 1]] [up](../Home.md) [s](./Specs/Spec%201.md) [[Projects/Note]]\n");
        assert!(read(&f.vault, "Home.md").starts_with("[[Alpha]] [[Projects/Alpha]]"));
        assert!(!f.vault.join("Archive").exists());
    }
}
