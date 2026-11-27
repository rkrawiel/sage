//! The incremental sync loop: keeps the SQLite index equal to the vault on disk.
//!
//! One thread owns the only write connection and does three things:
//!
//! 1. **Reconcile** on startup and on a timer: walk the vault, compare each file's
//!    `(mtime, size)` with the `files` table, and reparse only what differs. On a
//!    warm cache this is one `stat` per file and no reads.
//! 2. **Apply hints** from inotify between reconciliations (native filesystems).
//! 3. **Report** progress to the UI through a callback.
//!
//! mtime is only a hint. Before reparsing, the content hash is compared, so a
//! touch, a sync client rewriting identical bytes, or a clock jump costs one read
//! and nothing else. The same hash lets a delete + create pair be recognised as a
//! rename, which keeps the file id and every backlink pointing at it.

use anyhow::{Context, Result};
use sage_core::{link_key, name_key, FmValue, ParsedNote};
use sage_watcher::Backend;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Quiet period after the last inotify event before a batch is applied. Editors
/// and sync clients produce bursts (write temp, rename, chmod).
const DEBOUNCE: Duration = Duration::from_millis(150);
/// Longest a batch of events waits for quiet, counted from its first event.
const MAX_DEBOUNCE: Duration = Duration::from_millis(1500);
/// More distinct paths than this in one batch are handled by a full pass.
const MAX_HINTS: usize = 500;
/// Rows per transaction during a large reconcile, so readers see steady progress.
const BATCH: usize = 200;
/// A file modified this recently could change again within the same timestamp
/// tick, so its mtime is not trusted yet (the "racy git" problem).
const RACY_WINDOW_NS: i64 = 2_000_000_000;
/// A note larger than this keeps its row, so it shows in the tree and links
/// find it, but its text is not read: a log or a dump named `.md` would stall
/// every pass and be stored twice, in `texts` and in `fts`.
const MAX_NOTE_BYTES: i64 = 10 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum IndexEvent {
    /// The loop is running. `known_files` is what the cache already held, so the
    /// UI can show the vault before the first reconcile finishes. `onedrive`
    /// says the vault lies in a OneDrive folder.
    Started { backend: Backend, known_files: usize, onedrive: bool },
    Progress { done: usize, total: usize },
    /// Vault-relative paths whose index rows changed. Open tabs use this to
    /// trigger a reload or a three-way merge.
    Changed { paths: Vec<String> },
    Idle { files: usize, took_ms: u64 },
    /// A full pass that began after `IndexerHandle::sync(id)` has finished.
    Synced(u64),
    Error(String),
}

enum Msg {
    Fs(Vec<PathBuf>),
    Rescan,
    Sync(u64),
    Shutdown,
}

/// Controls the indexer thread. Dropping the handle asks it to stop; the thread
/// finishes its current transaction and exits on its own, so a drop never blocks.
pub struct IndexerHandle {
    tx: Sender<Msg>,
}

impl IndexerHandle {
    /// Asks for a full reconciliation now, e.g. when the window regains focus on
    /// a polled vault.
    pub fn rescan(&self) {
        let _ = self.tx.send(Msg::Rescan);
    }

    /// Asks for a full pass and an [`IndexEvent::Synced`] with `id` once it is
    /// done, so the UI can wait until the index has seen the disk as it is now.
    /// On a polled vault the index can otherwise trail it by a whole interval.
    pub fn sync(&self, id: u64) {
        let _ = self.tx.send(Msg::Sync(id));
    }
}

impl Drop for IndexerHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
    }
}

/// Starts the sync loop for `vault`, writing to the index at `db_path`.
pub fn spawn(
    vault: PathBuf,
    db_path: PathBuf,
    mut on_event: impl FnMut(IndexEvent) + Send + 'static,
) -> Result<IndexerHandle> {
    let vault = sage_watcher::canonical(&vault).context("vault path is not accessible")?;
    let mut backend = sage_watcher::detect_backend(&vault);
    let (tx, rx) = mpsc::channel();

    let watch_tx = tx.clone();
    let on_paths = move |paths: Vec<PathBuf>| {
        let msg = if paths.is_empty() { Msg::Rescan } else { Msg::Fs(paths) };
        let _ = watch_tx.send(msg);
    };
    // A watcher that cannot start (inotify's watch limit on a large vault, a
    // drive that refuses change notifications) must not leave the app without
    // an index: polling finds every change, only later.
    let (guard, watch_error) = match sage_watcher::watch(&vault, backend, on_paths.clone()) {
        Ok(guard) => (guard, None),
        Err(e) => {
            backend = Backend::Poll;
            (sage_watcher::watch(&vault, backend, on_paths)?, Some(format!("cannot watch the vault ({e}); polling instead")))
        }
    };

    std::thread::Builder::new()
        .name("sage-indexer".into())
        .spawn(move || {
            let _guard = guard; // keep the OS watcher alive as long as the loop
            let mut indexer = match Indexer::open(vault, &db_path) {
                Ok(indexer) => indexer,
                Err(e) => return on_event(IndexEvent::Error(format!("{e:#}"))),
            };
            on_event(IndexEvent::Started { backend, known_files: indexer.known.len(), onedrive: indexer.onedrive });
            if let Some(error) = watch_error {
                on_event(IndexEvent::Error(error));
            }

            let mut pending = Some(Msg::Rescan); // reconcile once at startup
            // When the last full pass ran. The safety-net pass is timed from it,
            // not from the last message, or a busy vault would never get one.
            let mut last_full = Instant::now();
            loop {
                let wait = backend.rescan_interval().saturating_sub(last_full.elapsed());
                let msg = match pending.take() {
                    Some(msg) => Ok(msg),
                    // Due already: a steady stream of events must not keep postponing it.
                    None if wait.is_zero() => Err(RecvTimeoutError::Timeout),
                    None => rx.recv_timeout(wait),
                };
                // Sync requests answered by the pass this iteration runs.
                let mut synced = Vec::new();
                let mut full = false;
                let mut paths = HashSet::new();
                match msg {
                    Ok(Msg::Fs(first)) => {
                        paths.extend(first);
                        // A process writing several times a second would otherwise
                        // restart the quiet period forever and never be applied.
                        let deadline = Instant::now() + MAX_DEBOUNCE;
                        loop {
                            let wait = DEBOUNCE.min(deadline.saturating_duration_since(Instant::now()));
                            if wait.is_zero() {
                                break;
                            }
                            match rx.recv_timeout(wait) {
                                Ok(Msg::Fs(more)) => paths.extend(more),
                                Ok(Msg::Rescan) => full = true,
                                Ok(Msg::Sync(id)) => {
                                    full = true;
                                    synced.push(id);
                                }
                                Ok(Msg::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                                Err(RecvTimeoutError::Timeout) => break,
                            }
                        }
                    }
                    Ok(Msg::Sync(id)) => {
                        synced.push(id);
                        full = true;
                    }
                    Ok(Msg::Rescan) | Err(RecvTimeoutError::Timeout) => full = true,
                    Ok(Msg::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                }
                // So many paths at once (a checkout, a sync client catching up)
                // are cheaper to find with one walk than one by one.
                full |= paths.len() > MAX_HINTS;
                if full {
                    // Everything queued meanwhile is answered by the one pass about
                    // to start, rather than by a full pass each.
                    loop {
                        match rx.try_recv() {
                            Ok(Msg::Fs(_) | Msg::Rescan) => {}
                            Ok(Msg::Sync(id)) => synced.push(id),
                            Ok(Msg::Shutdown) | Err(TryRecvError::Disconnected) => return,
                            Err(TryRecvError::Empty) => break,
                        }
                    }
                }
                // A bug must not stop the index for the rest of the session with
                // nothing said: it is reported, and the next message tries again.
                let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    if full {
                        indexer.reconcile(&mut on_event)
                    } else {
                        indexer.apply_hints(paths.into_iter().collect(), &mut on_event)
                    }
                }));
                if full {
                    last_full = Instant::now();
                }
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => on_event(IndexEvent::Error(format!("{e:#}"))),
                    Err(panic) => {
                        let what = panic
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown cause".into());
                        on_event(IndexEvent::Error(format!("the indexer failed: {what}")));
                        // The open transaction rolled back as it unwound.
                        indexer.reload_known();
                    }
                }
                // Even after an error: whoever waits must not wait forever.
                for id in synced {
                    on_event(IndexEvent::Synced(id));
                }
            }
        })?;

    Ok(IndexerHandle { tx })
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Stamp {
    id: i64,
    mtime_ns: i64,
    size: i64,
    /// A note stored as a OneDrive placeholder, as `files.cloud`.
    cloud: bool,
}

struct Seen {
    rel: String,
    abs: PathBuf,
    mtime_ns: i64,
    size: i64,
    /// A OneDrive placeholder: reading it would download it.
    placeholder: bool,
}

pub struct Indexer {
    vault: PathBuf,
    conn: Connection,
    /// In-memory mirror of `files (path -> id, mtime, size)`. About 100 bytes per
    /// file, and it turns a warm reconcile into pure `stat` calls.
    known: HashMap<String, Stamp>,
    /// The vault lies in a OneDrive folder, so cloud-only files are left unread.
    onedrive: bool,
}

impl Indexer {
    pub fn open(vault: PathBuf, db_path: &Path) -> Result<Self> {
        let conn = sage_db::open_writer(db_path)?;
        // Every statement run per note is cached; rusqlite keeps only 16.
        conn.set_prepared_statement_cache_capacity(64);
        let known = load_known(&conn)?;
        let onedrive = sage_onedrive::root(&vault).is_some();
        Ok(Self { vault, conn, known, onedrive })
    }

    fn is_placeholder(&self, meta: &std::fs::Metadata) -> bool {
        self.onedrive && sage_onedrive::is_placeholder(meta)
    }

    /// Whether a file stats as the index last stored it. A racy note was stored
    /// with mtime 0, so it never matches and is read again.
    fn unchanged(&self, rel: &str, mtime_ns: i64, size: i64, placeholder: bool) -> bool {
        // Freeing up space or downloading a note changes its blocks, not its
        // stamp; the stat is there anyway, so a note's cloud state is compared too.
        self.known
            .get(rel)
            .is_some_and(|k| k.mtime_ns == mtime_ns && k.size == size && k.cloud == (placeholder && is_note(rel)))
    }

    /// Reads `known` again from what the index really holds, after a write that
    /// rolled back: `known` is updated as rows are written, before the commit.
    fn reload_known(&mut self) {
        if let Ok(known) = load_known(&self.conn) {
            self.known = known;
        }
    }

    /// Full pass: makes the index match the disk. Cheap when little has changed.
    pub fn reconcile(&mut self, on_event: &mut impl FnMut(IndexEvent)) -> Result<()> {
        let started = Instant::now();

        // 1. Walk and stat. No file is opened here.
        let mut on_disk = HashSet::with_capacity(self.known.len());
        let mut dirty = Vec::new();
        let mut unreadable: Vec<String> = Vec::new();
        let vault = self.vault.clone();
        self.walk(&vault, &mut on_disk, &mut dirty, &mut unreadable).context("cannot read the vault folder")?;
        let missing: Vec<String> = self
            .known
            .keys()
            .filter(|path| !on_disk.contains(*path) && !unreadable.iter().any(|dir| is_under(path, dir)))
            .cloned()
            .collect();

        // 2. Apply.
        let changed = self.apply(dirty, missing, on_event)?;
        if !changed.is_empty() {
            on_event(IndexEvent::Changed { paths: changed });
        }
        on_event(IndexEvent::Idle {
            files: self.known.len(),
            took_ms: started.elapsed().as_millis() as u64,
        });
        Ok(())
    }

    /// Stats every file under `root`, the vault or a folder in it, without
    /// opening any: files that differ from the index go to `dirty`, every file
    /// seen to `on_disk`. Folders that could not be read go to `unreadable`,
    /// so a USB drive, a network share or a 9p hiccup does not make every note
    /// in them count as deleted; an empty entry stands for the whole vault.
    /// Fails only when `root` itself cannot be read.
    fn walk(
        &self,
        root: &Path,
        on_disk: &mut HashSet<String>,
        dirty: &mut Vec<Seen>,
        unreadable: &mut Vec<String>,
    ) -> Result<(), walkdir::Error> {
        let walker = walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| e.depth() == 0 || !is_hidden(e.file_name()));
        for entry in walker {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) if e.depth() == 0 => return Err(e),
                Err(e) => {
                    unreadable.push(e.path().and_then(|p| self.rel(p)).unwrap_or_default());
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(rel) = self.rel(entry.path()) else { continue };
            let Ok(meta) = entry.metadata() else {
                // Listed but not stat-able right now: keep what the index has.
                on_disk.insert(rel);
                continue;
            };
            let (mtime_ns, size) = stamp_of(&meta);
            let placeholder = self.is_placeholder(&meta);
            if !self.unchanged(&rel, mtime_ns, size, placeholder) {
                dirty.push(Seen { rel: rel.clone(), abs: entry.into_path(), mtime_ns, size, placeholder });
            }
            on_disk.insert(rel);
        }
        Ok(())
    }

    /// Targeted pass for paths reported by inotify.
    pub fn apply_hints(
        &mut self,
        paths: Vec<PathBuf>,
        on_event: &mut impl FnMut(IndexEvent),
    ) -> Result<()> {
        let started = Instant::now();
        let mut dirty = Vec::new();
        let mut missing = Vec::new();
        let mut visited = HashSet::new();
        let mut folders = Vec::new();

        for abs in paths {
            let Some(rel) = self.rel(&abs) else { continue };
            if rel.split('/').any(|part| is_hidden(part.as_ref())) || !visited.insert(rel.clone()) {
                continue;
            }
            match std::fs::symlink_metadata(&abs) {
                Ok(meta) if meta.is_file() => {
                    let (mtime_ns, size) = stamp_of(&meta);
                    let placeholder = self.is_placeholder(&meta);
                    // An event for a file as indexed (a folder's, a chmod, our own
                    // save seen twice) costs a stat, not a read and a hash.
                    if !self.unchanged(&rel, mtime_ns, size, placeholder) {
                        dirty.push(Seen { rel, abs, mtime_ns, size, placeholder });
                    }
                }
                // A folder's own events, such as its time changing whenever a
                // note in it is saved on Windows, say nothing its notes' events
                // do not. A folder the index knows nothing under (made, moved
                // in or renamed) is walked, and only it, not the whole vault.
                Ok(meta) if meta.is_dir() => {
                    if !self.known.keys().any(|path| is_under(path, &rel)) {
                        folders.push(abs);
                    }
                }
                // A symlink or other special file, which a pass never indexes.
                Ok(_) => {}
                Err(_) if self.known.contains_key(&rel) => missing.push(rel),
                // A folder removed or moved away takes what it held along; a
                // path never tracked, such as a temp file come and gone, nothing.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    missing.extend(self.known.keys().filter(|path| is_under(path, &rel)).cloned());
                }
                _ => return self.reconcile(on_event),
            }
        }
        for folder in folders {
            let (mut seen, mut found, mut unreadable) = (HashSet::new(), Vec::new(), Vec::new());
            // Gone again already: a later event or pass catches up.
            if self.walk(&folder, &mut seen, &mut found, &mut unreadable).is_ok() {
                dirty.extend(found.into_iter().filter(|s| visited.insert(s.rel.clone())));
            }
        }

        let changed = self.apply(dirty, missing, on_event)?;
        if !changed.is_empty() {
            on_event(IndexEvent::Changed { paths: changed });
            on_event(IndexEvent::Idle {
                files: self.known.len(),
                took_ms: started.elapsed().as_millis() as u64,
            });
        }
        Ok(())
    }

    /// Writes a set of possibly-changed and missing files to the index.
    /// Returns the vault-relative paths whose rows changed.
    fn apply(
        &mut self,
        dirty: Vec<Seen>,
        missing: Vec<String>,
        on_event: &mut impl FnMut(IndexEvent),
    ) -> Result<Vec<String>> {
        let result = self.write(dirty, missing, on_event);
        if result.is_err() {
            // A failed batch rolled back; without this, its notes would look
            // indexed until they change again.
            self.reload_known();
        }
        result
    }

    fn write(
        &mut self,
        dirty: Vec<Seen>,
        missing: Vec<String>,
        on_event: &mut impl FnMut(IndexEvent),
    ) -> Result<Vec<String>> {
        if dirty.is_empty() && missing.is_empty() {
            return Ok(Vec::new());
        }
        let now_ns = now_ns();
        let total = dirty.len();
        let mut changed = Vec::new();
        // Names whose link resolution may change: new, renamed or removed files.
        let mut relink: HashSet<String> = HashSet::new();
        // Missing paths found again under a new name, which must not be deleted.
        let mut renamed: HashSet<String> = HashSet::new();

        // Content hash of every missing note, to recognise renames.
        let mut gone_by_hash: HashMap<Vec<u8>, String> = HashMap::new();
        for path in &missing {
            let hash: Option<Vec<u8>> = self
                .conn
                .prepare_cached("SELECT hash FROM files WHERE path = ?1")?
                .query_row([path], |r| r.get(0))
                .optional()?
                .flatten();
            if let Some(hash) = hash {
                gone_by_hash.insert(hash, path.clone());
            }
        }

        for (n, chunk) in dirty.chunks(BATCH).enumerate() {
            let tx = self.conn.transaction()?;
            for seen in chunk {
                let prev = self.known.get(&seen.rel).copied();
                // Do not trust an mtime that is still "now": store 0 so the next
                // pass rechecks by hash.
                // A time in the future, from a clock that differs, is not racy:
                // stored as 0 it would be read again on every pass.
                let mtime_ns =
                    if (0..RACY_WINDOW_NS).contains(&(now_ns - seen.mtime_ns)) { 0 } else { seen.mtime_ns };

                if !is_note(&seen.rel) {
                    // Attachment: tracked for ![[embeds]], never read.
                    let (id, _) = upsert_file(&tx, &seen.rel, None, None, mtime_ns, seen.size, false)?;
                    self.known.insert(seen.rel.clone(), Stamp { id, mtime_ns, size: seen.size, cloud: false });
                    if prev.is_none() {
                        relink.insert(name_key(&seen.rel));
                        changed.push(seen.rel.clone());
                    }
                    continue;
                }

                if seen.placeholder {
                    // Cloud-only: reading would download it. A new note gets a
                    // row without content so it shows in the tree; a known one
                    // keeps the rows of what was last read. Stamp 0 makes the
                    // next pass look again, and read it once it is local.
                    let stamp = |id| Stamp { id, mtime_ns: 0, size: seen.size, cloud: true };
                    match prev {
                        Some(prev) => {
                            tx.prepare_cached("UPDATE files SET mtime_ns = 0, size = ?1, cloud = 1 WHERE id = ?2")?
                                .execute(params![seen.size, prev.id])?;
                            self.known.insert(seen.rel.clone(), stamp(prev.id));
                            if !prev.cloud {
                                changed.push(seen.rel.clone());
                            }
                        }
                        None => {
                            let (id, _) = upsert_file(&tx, &seen.rel, None, None, 0, seen.size, true)?;
                            self.known.insert(seen.rel.clone(), stamp(id));
                            relink.insert(name_key(&seen.rel));
                            changed.push(seen.rel.clone());
                        }
                    }
                    continue;
                }

                if seen.size > MAX_NOTE_BYTES {
                    let (id, title) = upsert_file(&tx, &seen.rel, None, None, mtime_ns, seen.size, false)?;
                    write_note(&tx, id, &seen.rel, &title, &ParsedNote::default(), "")?;
                    self.known.insert(seen.rel.clone(), Stamp { id, mtime_ns, size: seen.size, cloud: false });
                    if prev.is_none() {
                        relink.insert(name_key(&seen.rel));
                    }
                    changed.push(seen.rel.clone());
                    on_event(IndexEvent::Error(format!(
                        "{} is over {} MB, so its text is not indexed",
                        seen.rel,
                        MAX_NOTE_BYTES / 1024 / 1024
                    )));
                    continue;
                }

                let bytes = match std::fs::read(&seen.abs) {
                    Ok(bytes) => bytes,
                    // Deleted between the stat and the read; the next pass removes it.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => {
                        on_event(IndexEvent::Error(format!("{}: {e}", seen.rel)));
                        continue;
                    }
                };
                let hash = blake3::hash(&bytes);
                let hash = hash.as_bytes().as_slice();

                if let Some(prev) = prev {
                    let stored: Option<Vec<u8>> = tx
                        .prepare_cached("SELECT hash FROM files WHERE id = ?1")?
                        .query_row([prev.id], |r| r.get(0))
                        .optional()?
                        .flatten();
                    if stored.as_deref() == Some(hash) {
                        // Touched, or downloaded, but identical: refresh the stamp only.
                        tx.prepare_cached("UPDATE files SET mtime_ns = ?1, size = ?2, cloud = 0 WHERE id = ?3")?
                            .execute(params![mtime_ns, seen.size, prev.id])?;
                        self.known
                            .insert(seen.rel.clone(), Stamp { id: prev.id, mtime_ns, size: seen.size, cloud: false });
                        if prev.cloud {
                            changed.push(seen.rel.clone());
                        }
                        continue;
                    }
                } else if let Some(old_path) = gone_by_hash.remove(hash) {
                    // Same bytes under a new path: a rename. Move the row so the
                    // id, and with it every backlink, survives.
                    let old = self.known.remove(&old_path).expect("missing paths are known");
                    // A note titled by its file name takes the new name; reading
                    // the title from the text keeps one set in it.
                    let title = title_of(&seen.rel, Some(&sage_core::parse(&String::from_utf8_lossy(&bytes))));
                    tx.execute(
                        "UPDATE files SET path = ?1, link_key = ?2, name_key = ?3, parent = ?4,
                                          mtime_ns = ?5, size = ?6, cloud = 0, title = ?7
                         WHERE id = ?8",
                        params![
                            seen.rel,
                            link_key(&seen.rel),
                            name_key(&seen.rel),
                            parent_of(&seen.rel),
                            mtime_ns,
                            seen.size,
                            title,
                            old.id
                        ],
                    )?;
                    tx.execute(
                        "UPDATE fts SET path = ?1, title = ?2 WHERE rowid = ?3",
                        params![seen.rel, title, old.id],
                    )?;
                    // Its relative links now start from another folder.
                    let relative: Vec<(i64, String)> = tx
                        .prepare_cached(
                            "SELECT id, target FROM links
                             WHERE src_file_id = ?1 AND (target LIKE './%' OR target LIKE '../%')",
                        )?
                        .query_map([old.id], |r| Ok((r.get(0)?, r.get(1)?)))?
                        .collect::<rusqlite::Result<_>>()?;
                    for (link, target) in relative {
                        let key = sage_core::target_key(&target, &seen.rel);
                        tx.execute(
                            "UPDATE links SET target_key = ?1, target_name = ?2, dst_file_id = NULL WHERE id = ?3",
                            params![key, key.rsplit('/').next().unwrap_or(""), link],
                        )?;
                    }
                    self.known
                        .insert(seen.rel.clone(), Stamp { id: old.id, mtime_ns, size: seen.size, cloud: false });
                    relink.insert(name_key(&old_path));
                    renamed.insert(old_path.clone());
                    relink.insert(name_key(&seen.rel));
                    changed.push(old_path);
                    changed.push(seen.rel.clone());
                    continue;
                }

                let text = String::from_utf8_lossy(&bytes);
                let note = sage_core::parse(&text);
                // Links written as the note's frontmatter id, before and after,
                // may now find another note or none.
                if let Some(prev) = prev {
                    let old: Option<String> =
                        tx.prepare_cached("SELECT id_key FROM files WHERE id = ?1")?.query_row([prev.id], |r| r.get(0))?;
                    relink.extend(old);
                }
                relink.extend(note.id.as_deref().map(link_key));
                let (id, title) = upsert_file(&tx, &seen.rel, Some(&note), Some(hash), mtime_ns, seen.size, false)?;
                write_note(&tx, id, &seen.rel, &title, &note, &text)?;
                self.known.insert(seen.rel.clone(), Stamp { id, mtime_ns, size: seen.size, cloud: false });
                if prev.is_none() {
                    relink.insert(name_key(&seen.rel));
                }
                changed.push(seen.rel.clone());
            }
            tx.commit()?;
            if total > BATCH {
                on_event(IndexEvent::Progress { done: ((n + 1) * BATCH).min(total), total });
            }
        }

        let tx = self.conn.transaction()?;
        for path in missing.iter().filter(|path| !renamed.contains(*path)) {
            if let Some(stamp) = self.known.remove(path) {
                // Child rows cascade; links pointing here fall back to unresolved.
                tx.prepare_cached("DELETE FROM fts WHERE rowid = ?1")?.execute([stamp.id])?;
                tx.prepare_cached("DELETE FROM files WHERE id = ?1")?.execute([stamp.id])?;
                relink.insert(name_key(path));
                changed.push(path.clone());
            }
        }
        // A new or renamed file can become the better match for existing links,
        // so reopen the ones that mention its name, then resolve everything open.
        for name in &relink {
            tx.prepare_cached("UPDATE links SET dst_file_id = NULL WHERE target_name = ?1 AND dst_file_id IS NOT NULL")?
                .execute([name])?;
        }
        resolve_links(&tx)?;
        tx.commit()?;

        // A large pass grows the write-ahead log by its whole size; give the
        // space back now rather than keep it until the app closes. A reader
        // still using the log can make it give up, which is fine.
        if total > BATCH {
            let _ = self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        }

        Ok(changed)
    }

    /// Absolute path to vault-relative path with `/` separators.
    fn rel(&self, abs: &Path) -> Option<String> {
        let rel = abs.strip_prefix(&self.vault).ok()?;
        let rel = rel.to_str()?; // non-UTF-8 names are skipped
        // Vault paths use `/` on every platform, as links do.
        let rel = if cfg!(windows) { rel.replace('\\', "/") } else { rel.to_owned() };
        (!rel.is_empty()).then_some(rel)
    }
}

/// Fills in `dst_file_id` for unresolved links, following Obsidian's rule:
/// `[[Note]]` matches by file name anywhere in the vault, `[[folder/Note]]` must
/// match the end of the path, and the shortest path wins when several qualify.
/// `sage_core::resolve` is the same rule for code outside SQL; keep them in step.
/// A bare `[[name]]` that no file name matches then finds the note whose
/// frontmatter `id` it is, so `[[fix-voice-reconnect]]` reaches a task document
/// whatever its file is called.
///
/// Only links that some file could match are written: SQLite rewrites a row
/// set to the NULL it already holds, and a vault keeps many dangling links.
fn resolve_links(tx: &Transaction) -> Result<()> {
    tx.prepare_cached(
        "UPDATE links SET dst_file_id = (
             SELECT f.id FROM files f
             WHERE f.name_key = links.target_name
               AND (links.target_key = links.target_name
                    OR f.link_key = links.target_key
                    OR substr(f.link_key, -length(links.target_key) - 1) = '/' || links.target_key)
             ORDER BY length(f.path), f.path
             LIMIT 1)
         WHERE dst_file_id IS NULL AND target_name <> ''
           AND EXISTS (SELECT 1 FROM files f WHERE f.name_key = links.target_name)",
    )?
    .execute([])?;
    tx.prepare_cached(
        "UPDATE links SET dst_file_id = (
             SELECT f.id FROM files f
             WHERE f.id_key = links.target_key
             ORDER BY length(f.path), f.path
             LIMIT 1)
         WHERE dst_file_id IS NULL AND target_key = target_name AND target_name <> ''
           AND EXISTS (SELECT 1 FROM files f WHERE f.id_key = links.target_key)",
    )?
    .execute([])?;
    Ok(())
}

/// A file's title: the note's frontmatter title or first H1, else its file name.
fn title_of(rel: &str, note: Option<&ParsedNote>) -> String {
    let file_name = rel.rsplit('/').next().unwrap_or(rel);
    note.and_then(|n| n.title.clone()).unwrap_or_else(|| file_name.strip_suffix(".md").unwrap_or(file_name).to_owned())
}

/// Writes a file's row and returns its id and the title stored.
fn upsert_file(
    tx: &Transaction,
    rel: &str,
    note: Option<&ParsedNote>,
    hash: Option<&[u8]>,
    mtime_ns: i64,
    size: i64,
    cloud: bool,
) -> Result<(i64, String)> {
    let title = title_of(rel, note);
    let id_key = note.and_then(|n| n.id.as_deref()).map(link_key);
    // A document's status as a Kanban column names it, with `_` read as `-` as
    // `TaskStatus::parse` reads it. In Rust, so it folds case beyond ASCII.
    let status_key = note
        .and_then(|n| n.frontmatter.iter().find(|e| e.key == "status" && e.ord == 0))
        .and_then(|e| match &e.value {
            FmValue::Text(text) => Some(sage_core::status_slug(&text.replace('_', "-"))),
            _ => None,
        })
        .filter(|slug| !slug.is_empty());
    let id = tx
        .prepare_cached(
            "INSERT INTO files (path, link_key, name_key, parent, ext, title, mtime_ns, size, hash, indexed_at, cloud, id_key, status_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, unixepoch(), ?10, ?11, ?12)
             ON CONFLICT (path) DO UPDATE SET
                 title = excluded.title, mtime_ns = excluded.mtime_ns, size = excluded.size,
                 hash = excluded.hash, indexed_at = excluded.indexed_at, cloud = excluded.cloud,
                 id_key = excluded.id_key, status_key = excluded.status_key
             RETURNING id",
        )?
        .query_row(
            params![rel, link_key(rel), name_key(rel), parent_of(rel), ext_of(rel), title, mtime_ns, size, hash, cloud, id_key, status_key],
            |r| r.get(0),
        )?;
    Ok((id, title))
}

/// Replaces every derived row for one note, and the text they come from.
/// `title` is the one [`upsert_file`] stored, which `fts` repeats.
fn write_note(tx: &Transaction, id: i64, rel: &str, title: &str, note: &ParsedNote, text: &str) -> Result<()> {
    tx.prepare_cached("INSERT OR REPLACE INTO texts (file_id, text) VALUES (?1, ?2)")?.execute(params![id, text])?;
    for table in ["frontmatter", "aliases", "headings", "tags", "tasks"] {
        tx.prepare_cached(&format!("DELETE FROM {table} WHERE file_id = ?1"))?.execute([id])?;
    }
    tx.prepare_cached("DELETE FROM links WHERE src_file_id = ?1")?.execute([id])?;
    tx.prepare_cached("DELETE FROM fts WHERE rowid = ?1")?.execute([id])?;

    let mut stmt = tx.prepare_cached(
        "INSERT OR REPLACE INTO frontmatter (file_id, key, ord, value_type, value, value_num)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for e in &note.frontmatter {
        let (kind, text, num) = match &e.value {
            FmValue::Text(s) => ("text", Some(s.clone()), None),
            FmValue::Num(n) => ("num", Some(n.to_string()), Some(*n)),
            FmValue::Bool(b) => ("bool", Some(b.to_string()), Some(f64::from(u8::from(*b)))),
            FmValue::Null => ("null", None, None),
        };
        stmt.execute(params![id, e.key.to_lowercase(), e.ord, kind, text, num])?;
    }

    let mut stmt =
        tx.prepare_cached("INSERT INTO aliases (file_id, alias, alias_lc) VALUES (?1, ?2, ?3)")?;
    for alias in &note.aliases {
        stmt.execute(params![id, alias, alias.to_lowercase()])?;
    }

    let mut stmt = tx.prepare_cached(
        "INSERT OR REPLACE INTO headings (file_id, line, level, text) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for h in &note.headings {
        stmt.execute(params![id, h.line, h.level, h.text])?;
    }

    let mut stmt = tx.prepare_cached("INSERT INTO tags (file_id, tag, line) VALUES (?1, ?2, ?3)")?;
    for t in &note.tags {
        stmt.execute(params![id, t.name.to_lowercase(), t.line])?;
    }

    let mut stmt = tx.prepare_cached(
        "INSERT INTO links (src_file_id, dst_file_id, target, target_key, target_name,
                            heading, block, alias, is_embed, line, col, len)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?;
    for l in &note.links {
        let key = sage_core::target_key(&l.target, rel);
        // [[#Heading]] points at the note itself.
        let dst = l.target.is_empty().then_some(id);
        stmt.execute(params![
            id, dst, l.target, key, key.rsplit('/').next().unwrap_or(""),
            l.heading, l.block, l.alias, l.embed, l.line, l.col, l.len
        ])?;
    }

    let mut stmt = tx.prepare_cached(
        "INSERT INTO tasks (file_id, line, indent, state, text, raw, status,
                            due, scheduled, start, done_on, priority, reminder)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
    )?;
    for t in &note.tasks {
        stmt.execute(params![
            id, t.line, t.indent, t.state.to_string(), t.text, t.raw, t.status,
            t.due, t.scheduled, t.start, t.done, t.priority, t.reminder
        ])?;
    }

    let tags = note.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(" ");
    tx.prepare_cached("INSERT INTO fts (rowid, title, body, tags, path) VALUES (?1, ?2, ?3, ?4, ?5)")?
        .execute(params![id, title, note.body, tags, rel])?;
    Ok(())
}

/// What the `files` table holds, as the in-memory mirror.
fn load_known(conn: &Connection) -> Result<HashMap<String, Stamp>> {
    let mut stmt = conn.prepare("SELECT path, id, mtime_ns, size, cloud FROM files")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, Stamp { id: r.get(1)?, mtime_ns: r.get(2)?, size: r.get(3)?, cloud: r.get(4)? }))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Whether vault path `path` lies in folder `dir`; an empty `dir` is the vault.
fn is_under(path: &str, dir: &str) -> bool {
    dir.is_empty() || path.strip_prefix(dir).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

fn is_hidden(name: &std::ffi::OsStr) -> bool {
    // .obsidian, .trash, .git, and our own atomic-save temp files.
    name.as_encoded_bytes().first() == Some(&b'.')
}

fn is_note(rel: &str) -> bool {
    ext_of(rel) == "md"
}

fn ext_of(rel: &str) -> String {
    let file_name = rel.rsplit('/').next().unwrap_or(rel);
    match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_lowercase(),
        _ => String::new(),
    }
}

fn parent_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn stamp_of(meta: &std::fs::Metadata) -> (i64, i64) {
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i64);
    (mtime_ns, meta.len() as i64)
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn count(ix: &Indexer, sql: &str) -> i64 {
        ix.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// Backdates a file so its mtime is outside the racy window.
    fn age(path: &Path) {
        let old = SystemTime::now() - Duration::from_secs(60);
        fs::File::options().write(true).open(path).unwrap().set_modified(old).unwrap();
    }

    #[test]
    fn only_marked_checkboxes_are_tracked_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("A.md"), "- [ ] plain\n- [ ] tagged #task\n- [ ] follow #FollowUp\n- [ ] dated 📅 2026-12-01\n- [ ] other #idea\n").unwrap();
        age(&vault.join("A.md"));
        let mut ix = Indexer::open(vault.canonicalize().unwrap(), &dir.path().join("index.db")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM v_tasks"), 5);
        assert_eq!(count(&ix, "SELECT count(*) FROM v_tasks WHERE tracked"), 3);
    }

    #[test]
    fn reminders_of_checkboxes_and_task_documents_are_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("A.md"), "- [ ] call (@2026-12-01 09:30)
- [ ] pay (@2026-12-02)
- [ ] plain
").unwrap();
        fs::write(vault.join("T.md"), "---
type: task
status: todo
reminder: 2026-12-03 08:00
---
").unwrap();
        age(&vault.join("A.md"));
        age(&vault.join("T.md"));
        let mut ix = Indexer::open(vault.canonicalize().unwrap(), &dir.path().join("index.db")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM v_tasks WHERE reminder = '2026-12-01 09:30' AND tracked"), 1);
        assert_eq!(count(&ix, "SELECT count(*) FROM v_tasks WHERE reminder = '2026-12-02' AND tracked"), 1);
        assert_eq!(count(&ix, "SELECT count(*) FROM v_tasks WHERE reminder IS NULL AND NOT tracked"), 1);
        assert_eq!(count(&ix, "SELECT count(*) FROM v_documents WHERE reminder = '2026-12-03 08:00'"), 1);
    }

    #[test]
    fn reconcile_is_incremental_and_tracks_renames() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(vault.join("Projects")).unwrap();
        fs::create_dir_all(vault.join(".obsidian")).unwrap();
        fs::write(vault.join(".obsidian/app.json"), "{}").unwrap();
        fs::write(vault.join("Projects/Alpha.md"), "---\ntags: [project/alpha]\n---\n# Alpha\n- [ ] Ship #status/in-progress 📅 2026-12-01\n").unwrap();
        fs::write(vault.join("Home.md"), "See [[Alpha]] and [[Missing]] and ![[pic.png]]\n").unwrap();
        fs::write(vault.join("pic.png"), [0u8; 4]).unwrap();
        for f in ["Projects/Alpha.md", "Home.md", "pic.png"] {
            age(&vault.join(f));
        }

        let mut events = Vec::new();
        let mut ix = Indexer::open(vault.canonicalize().unwrap(), &dir.path().join("index.db")).unwrap();
        ix.reconcile(&mut |e| events.push(e)).unwrap();

        assert_eq!(count(&ix, "SELECT count(*) FROM files"), 3, "hidden dirs are skipped");
        assert_eq!(count(&ix, "SELECT count(*) FROM links WHERE dst_file_id IS NOT NULL"), 2);
        assert_eq!(count(&ix, "SELECT count(*) FROM v_backlinks b JOIN files f ON f.id = b.file_id WHERE f.path = 'Projects/Alpha.md'"), 1);
        assert_eq!(count(&ix, "SELECT count(*) FROM v_tasks WHERE status = 'in-progress' AND due = '2026-12-01'"), 1);
        assert_eq!(count(&ix, "SELECT count(*) FROM fts WHERE fts MATCH 'shi*'"), 1);
        assert_eq!(count(&ix, "SELECT count(*) FROM tags WHERE tag = 'project/alpha' OR tag GLOB 'project/alpha/*'"), 1);

        // Second pass: nothing changed, so nothing is reported as changed.
        events.clear();
        ix.reconcile(&mut |e| events.push(e)).unwrap();
        assert!(events.iter().all(|e| !matches!(e, IndexEvent::Changed { .. })));

        // Rename on disk: the row keeps its id and the backlink survives.
        let alpha_id = ix.known["Projects/Alpha.md"].id;
        fs::rename(vault.join("Projects/Alpha.md"), vault.join("Alpha moved.md")).unwrap();
        ix.reconcile(&mut |e| events.push(e)).unwrap();
        assert_eq!(ix.known["Alpha moved.md"].id, alpha_id);
        assert_eq!(count(&ix, "SELECT count(*) FROM files"), 3);
        // [[Alpha]] no longer matches the new name, so that link is now unresolved.
        assert_eq!(count(&ix, "SELECT count(*) FROM links WHERE target = 'Alpha' AND dst_file_id IS NULL"), 1);

        // Creating the missing target resolves the dangling link.
        fs::write(vault.join("Missing.md"), "now here").unwrap();
        age(&vault.join("Missing.md"));
        ix.reconcile(&mut |e| events.push(e)).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM links WHERE target = 'Missing' AND dst_file_id IS NOT NULL"), 1);

        // Delete: rows and full-text entries go away.
        fs::remove_file(vault.join("Home.md")).unwrap();
        ix.reconcile(&mut |e| events.push(e)).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM links"), 0);
        assert_eq!(count(&ix, "SELECT count(*) FROM fts"), 2);
    }

    #[test]
    fn markdown_and_property_links_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(vault.join("Projects")).unwrap();
        fs::create_dir_all(vault.join("Specs")).unwrap();
        fs::write(
            vault.join("Projects/Alpha.md"),
            "---\nup: \"[[Home]]\"\n---\n[spec](../Specs/My%20Spec.md) [home](Home.md) [web](https://example.com)\n",
        )
        .unwrap();
        fs::write(vault.join("Specs/My Spec.md"), "# Spec\n").unwrap();
        fs::write(vault.join("Home.md"), "# Home\n").unwrap();

        let mut ix = Indexer::open(vault.canonicalize().unwrap(), &dir.path().join("index.db")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM links"), 3, "external links are not indexed");
        let backlinks = |ix: &Indexer, path: &str| {
            count(ix, &format!("SELECT count(*) FROM v_backlinks b JOIN files f ON f.id = b.file_id WHERE f.path = '{path}'"))
        };
        assert_eq!(backlinks(&ix, "Home.md"), 2, "property link and Markdown link");
        assert_eq!(backlinks(&ix, "Specs/My Spec.md"), 1, "relative, percent-encoded link");
        let text: String = ix
            .conn
            .query_row("SELECT t.text FROM texts t JOIN files f ON f.id = t.file_id WHERE f.path = 'Home.md'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(text, "# Home\n", "the text the rows came from");

        // Moved elsewhere unchanged: the row moves, and the relative link now
        // starts from the new folder, where no such note exists.
        fs::create_dir_all(vault.join("Projects/Sub")).unwrap();
        fs::rename(vault.join("Projects/Alpha.md"), vault.join("Projects/Sub/Alpha.md")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(backlinks(&ix, "Specs/My Spec.md"), 0);
        assert_eq!(count(&ix, "SELECT count(*) FROM links WHERE target_key = 'projects/specs/my spec' AND dst_file_id IS NULL"), 1);
        assert_eq!(count(&ix, "SELECT count(*) FROM texts"), 3);
    }

    #[test]
    fn task_documents_link_by_id_and_keep_backlinks_current() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(vault.join("Tasks")).unwrap();
        let task = |status: &str| {
            format!(
                "---\nid: fix-voice-reconnect\ntype: task\ntitle: Fix voice reconnect bug\nstatus: {status}\n\
                 created: 2026-11-16\ndue: 2026-11-30\ntags:\n  - backend\n  - voice\n---\nInvestigate.\n"
            )
        };
        fs::write(vault.join("Tasks/Voice reconnect.md"), task("in-progress")).unwrap();
        fs::write(vault.join("meeting-notes.md"), "![[fix-voice-reconnect]]\nSee [[Fix-Voice-Reconnect]].\n").unwrap();
        fs::write(vault.join("plan.md"), "Nothing yet.\n").unwrap();

        let mut ix = Indexer::open(vault.canonicalize().unwrap(), &dir.path().join("index.db")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        let backlinks = |ix: &Indexer| -> Vec<String> {
            let mut stmt = ix
                .conn
                .prepare(
                    "SELECT DISTINCT b.src_path FROM v_backlinks b JOIN files f ON f.id = b.file_id
                     WHERE f.path LIKE 'Tasks/%' ORDER BY 1",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        assert_eq!(backlinks(&ix), ["meeting-notes.md"], "the embed and the link, matched by id ignoring case");
        let row: (String, String, String, String, String) = ix
            .conn
            .query_row("SELECT type, doc_id, status, due, tags FROM v_documents", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap();
        assert_eq!(row, ("task".into(), "fix-voice-reconnect".into(), "in-progress".into(), "2026-11-30".into(), "backend,voice".into()));

        // Another note starts referencing it: one more backlink.
        fs::write(vault.join("plan.md"), "Blocked on [[fix-voice-reconnect]].\n").unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(backlinks(&ix), ["meeting-notes.md", "plan.md"]);

        // Moved with an edit, so not recognised as a rename: the id still finds it.
        fs::remove_file(vault.join("Tasks/Voice reconnect.md")).unwrap();
        fs::write(vault.join("Tasks/Reconnect.md"), task("In Progress")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM links WHERE dst_file_id IS NULL"), 0);
        assert_eq!(backlinks(&ix), ["meeting-notes.md", "plan.md"]);
        assert_eq!(count(&ix, "SELECT count(*) FROM v_documents WHERE status = 'in-progress'"), 1, "status normalised");

        // A file named like the id wins over the id: file names come first.
        fs::write(vault.join("fix-voice-reconnect.md"), "A note of that name.\n").unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(backlinks(&ix), Vec::<String>::new());

        // Gone again, and the task's id changed: links written as the old id find nothing.
        fs::remove_file(vault.join("fix-voice-reconnect.md")).unwrap();
        fs::write(vault.join("Tasks/Reconnect.md"), task("done").replace("id: fix-voice-reconnect", "id: voice-1")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM links WHERE dst_file_id IS NULL"), 3);
        assert_eq!(backlinks(&ix), Vec::<String>::new());
    }

    // Sparse files stand in for placeholders, which only Unix reads that way.
    #[cfg(unix)]
    #[test]
    fn onedrive_placeholders_are_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("OneDrive").join("Vault");
        fs::create_dir_all(&vault).unwrap();
        // A sparse file has a size and no blocks, as a cloud-only file does.
        fs::File::create(vault.join("Cloud.md")).unwrap().set_len(8192).unwrap();
        fs::write(vault.join("Local.md"), "[[Cloud]] #here").unwrap();
        for f in ["Cloud.md", "Local.md"] {
            age(&vault.join(f));
        }

        let mut ix = Indexer::open(vault.canonicalize().unwrap(), &dir.path().join("index.db")).unwrap();
        assert!(ix.onedrive);
        let mut events = Vec::new();
        ix.reconcile(&mut |e| events.push(e)).unwrap();
        let changed = events.iter().any(|e| matches!(e, IndexEvent::Changed { paths } if paths.contains(&"Cloud.md".to_owned())));
        assert!(changed, "a new placeholder still shows in the tree");
        assert_eq!(count(&ix, "SELECT count(*) FROM files WHERE path = 'Cloud.md' AND hash IS NULL AND mtime_ns = 0"), 1);
        assert_eq!(count(&ix, "SELECT count(*) FROM fts JOIN files f ON f.id = fts.rowid WHERE f.path = 'Cloud.md'"), 0);
        assert_eq!(count(&ix, "SELECT count(*) FROM links WHERE target = 'Cloud' AND dst_file_id IS NOT NULL"), 1);
        let cloud = |ix: &Indexer| count(ix, "SELECT cloud FROM v_notes WHERE path = 'Cloud.md'");
        assert_eq!((cloud(&ix), count(&ix, "SELECT cloud FROM files WHERE path = 'Local.md'")), (1, 0));

        // Still in the cloud: the next pass only looks, and reports nothing.
        events.clear();
        ix.reconcile(&mut |e| events.push(e)).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM tags WHERE tag = 'pulled'"), 0);
        assert!(events.iter().all(|e| !matches!(e, IndexEvent::Changed { .. })));

        // Downloaded: it is read like any note.
        fs::write(vault.join("Cloud.md"), format!("{}\n#pulled", "x".repeat(8192))).unwrap();
        age(&vault.join("Cloud.md"));
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM tags WHERE tag = 'pulled'"), 1);
        assert_eq!(count(&ix, "SELECT count(*) FROM files WHERE path = 'Cloud.md' AND hash IS NOT NULL"), 1);
        assert_eq!(cloud(&ix), 0);

        // Freed up again with new content from elsewhere: the rows read last stay.
        fs::File::create(vault.join("Cloud.md")).unwrap().set_len(9000).unwrap();
        age(&vault.join("Cloud.md"));
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM tags WHERE tag = 'pulled'"), 1);
        assert_eq!(count(&ix, "SELECT count(*) FROM files WHERE path = 'Cloud.md' AND mtime_ns = 0"), 1);
        assert_eq!(cloud(&ix), 1);

        // Freed up with nothing else changed: same stamp, but no blocks, and
        // the change is reported so the tree can show it.
        let big = vault.join("Big.md");
        let when = SystemTime::now() - Duration::from_secs(60);
        fs::write(&big, "y".repeat(9000)).unwrap();
        fs::File::options().write(true).open(&big).unwrap().set_modified(when).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(count(&ix, "SELECT cloud FROM files WHERE path = 'Big.md'"), 0);
        let file = fs::File::create(&big).unwrap();
        file.set_len(9000).unwrap();
        file.set_modified(when).unwrap();
        events.clear();
        ix.reconcile(&mut |e| events.push(e)).unwrap();
        assert_eq!(count(&ix, "SELECT cloud FROM files WHERE path = 'Big.md'"), 1);
        let reported = events.iter().any(|e| matches!(e, IndexEvent::Changed { paths } if paths == &["Big.md".to_owned()]));
        assert!(reported, "{events:?}");

        // Outside OneDrive the same file is just read.
        let other = dir.path().join("Plain");
        fs::create_dir_all(&other).unwrap();
        let ix = Indexer::open(other.canonicalize().unwrap(), &dir.path().join("plain.db")).unwrap();
        assert!(!ix.onedrive);
    }

    #[test]
    fn spawn_picks_up_live_changes() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        let db = dir.path().join("index.db");
        let (tx, rx) = mpsc::channel();
        let handle = spawn(vault.clone(), db.clone(), move |e| {
            let _ = tx.send(e);
        })
        .unwrap();

        let wait_idle = || loop {
            match rx.recv_timeout(Duration::from_secs(10)).expect("indexer went quiet") {
                IndexEvent::Idle { files, .. } => break files,
                IndexEvent::Error(e) => panic!("{e}"),
                _ => {}
            }
        };
        assert_eq!(wait_idle(), 0);

        fs::write(vault.join("New.md"), "# New\n#fresh").unwrap();
        handle.rescan(); // makes the test independent of the watch backend
        assert_eq!(wait_idle(), 1);

        let reader = sage_db::open_reader(&db).unwrap();
        let count = |tag: &str| -> i64 {
            reader.query_row("SELECT count(*) FROM tags WHERE tag = ?1", [tag], |r| r.get(0)).unwrap()
        };
        assert_eq!(count("fresh"), 1);

        // A sync answers once a pass that started after it has finished.
        fs::write(vault.join("Later.md"), "#later").unwrap();
        handle.sync(7);
        loop {
            match rx.recv_timeout(Duration::from_secs(10)).expect("no Synced event") {
                IndexEvent::Synced(id) => break assert_eq!(id, 7),
                IndexEvent::Error(e) => panic!("{e}"),
                _ => {}
            }
        }
        assert_eq!(count("later"), 1);

        // Requests queued together share a pass, and every one is answered.
        for id in [8, 9, 10] {
            handle.sync(id);
        }
        let mut answered = Vec::new();
        while answered.len() < 3 {
            match rx.recv_timeout(Duration::from_secs(10)).expect("a sync went unanswered") {
                IndexEvent::Synced(id) => answered.push(id),
                IndexEvent::Error(e) => panic!("{e}"),
                _ => {}
            }
        }
        assert_eq!(answered, [8, 9, 10]);
        drop(handle);
    }

    #[test]
    fn a_renamed_note_titled_by_its_name_takes_the_new_name() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("Draft.md"), "no heading here\n").unwrap();
        fs::write(vault.join("Headed.md"), "# Kept title\n").unwrap();
        for f in ["Draft.md", "Headed.md"] {
            age(&vault.join(f));
        }
        let mut ix = Indexer::open(vault.canonicalize().unwrap(), &dir.path().join("index.db")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        let draft_id = ix.known["Draft.md"].id;

        fs::rename(vault.join("Draft.md"), vault.join("Final.md")).unwrap();
        fs::rename(vault.join("Headed.md"), vault.join("Moved.md")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(ix.known["Final.md"].id, draft_id, "recognised as a rename");
        let title = |path: &str| -> String {
            ix.conn.query_row("SELECT title FROM files WHERE path = ?1", [path], |r| r.get(0)).unwrap()
        };
        assert_eq!((title("Final.md"), title("Moved.md")), ("Final".into(), "Kept title".into()));
        assert_eq!(count(&ix, "SELECT count(*) FROM fts WHERE fts MATCH 'title:final'"), 1);
        assert_eq!(count(&ix, "SELECT count(*) FROM fts WHERE fts MATCH 'title:draft'"), 0);
    }

    #[test]
    fn writes_leave_dangling_links_and_unchanged_hinted_files_alone() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("Home.md"), "[[Missing]] and [[Other]]\n").unwrap();
        age(&vault.join("Home.md"));
        let vault = vault.canonicalize().unwrap();
        let mut ix = Indexer::open(vault.clone(), &dir.path().join("index.db")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        ix.conn
            .execute_batch(
                "CREATE TEMP TABLE hits (what TEXT);
                 CREATE TEMP TRIGGER link_hit AFTER UPDATE ON links BEGIN INSERT INTO hits VALUES (OLD.target); END;
                 CREATE TEMP TRIGGER file_hit AFTER UPDATE ON files BEGIN INSERT INTO hits VALUES (OLD.path); END;",
            )
            .unwrap();
        let hits = |ix: &Indexer| -> Vec<String> {
            let mut stmt = ix.conn.prepare("SELECT what FROM hits ORDER BY 1").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };

        // A new note resolves the link to it and touches no other link row.
        fs::write(vault.join("Other.md"), "x").unwrap();
        age(&vault.join("Other.md"));
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(hits(&ix), ["Other"]);
        assert_eq!(count(&ix, "SELECT count(*) FROM links WHERE target = 'Missing' AND dst_file_id IS NULL"), 1);

        // A hint for a file the index already holds as it is reads nothing.
        let mut events = Vec::new();
        ix.apply_hints(vec![vault.join("Home.md")], &mut |e| events.push(e)).unwrap();
        assert_eq!(hits(&ix), ["Other"]);
        assert!(events.is_empty(), "{events:?}");
    }

    #[test]
    fn an_unreadable_vault_keeps_its_index() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(vault.join("Projects")).unwrap();
        fs::write(vault.join("Projects/Alpha.md"), "# Alpha
").unwrap();
        age(&vault.join("Projects/Alpha.md"));
        let mut ix = Indexer::open(vault.canonicalize().unwrap(), &dir.path().join("index.db")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();

        // The drive went away: the pass fails rather than deleting every note.
        fs::rename(&vault, dir.path().join("elsewhere")).unwrap();
        assert!(ix.reconcile(&mut |_| {}).is_err());
        assert_eq!(count(&ix, "SELECT count(*) FROM files"), 1);

        assert!(is_under("Projects/Alpha.md", "Projects") && is_under("Projects", "Projects") && is_under("x.md", ""));
        assert!(!is_under("Projects2/x.md", "Projects"));
    }

    #[test]
    fn a_failed_write_is_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("Tagged.md"), "#tag
").unwrap();
        age(&vault.join("Tagged.md"));
        let mut ix = Indexer::open(vault.canonicalize().unwrap(), &dir.path().join("index.db")).unwrap();

        // The batch rolls back, and the cache must not claim the note is indexed.
        ix.conn.execute_batch("CREATE TEMP TRIGGER fail BEFORE INSERT ON tags BEGIN SELECT RAISE(ABORT, 'boom'); END;").unwrap();
        assert!(ix.reconcile(&mut |_| {}).is_err());
        assert_eq!(count(&ix, "SELECT count(*) FROM files"), 0);
        assert!(ix.known.is_empty());

        ix.conn.execute_batch("DROP TRIGGER fail;").unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM tags WHERE tag = 'tag'"), 1);
    }

    #[test]
    fn folder_events_walk_only_new_folders() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(vault.join("Projects")).unwrap();
        fs::write(vault.join("Projects/A.md"), "# A\n").unwrap();
        age(&vault.join("Projects/A.md"));
        let vault = vault.canonicalize().unwrap();
        let mut ix = Indexer::open(vault.clone(), &dir.path().join("index.db")).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        let indexed = |ix: &Indexer| -> Vec<String> {
            let mut paths: Vec<String> = ix.known.keys().cloned().collect();
            paths.sort();
            paths
        };

        // A note saved in a known folder: its folder's event walks nothing,
        // so a note written elsewhere without an event waits for a pass.
        fs::write(vault.join("Unhinted.md"), "x").unwrap();
        ix.apply_hints(vec![vault.join("Projects")], &mut |_| {}).unwrap();
        assert_eq!(indexed(&ix), ["Projects/A.md"]);

        // A folder moved in is walked, and only it.
        fs::create_dir_all(vault.join("Moved/Deep")).unwrap();
        fs::write(vault.join("Moved/Deep/B.md"), "# B\n").unwrap();
        ix.apply_hints(vec![vault.join("Moved")], &mut |_| {}).unwrap();
        assert_eq!(indexed(&ix), ["Moved/Deep/B.md", "Projects/A.md"]);

        // A folder removed takes its notes along; a temp file that came and
        // went changes nothing.
        fs::remove_dir_all(vault.join("Projects")).unwrap();
        ix.apply_hints(vec![vault.join("Projects"), vault.join("gone.tmp")], &mut |_| {}).unwrap();
        assert_eq!(indexed(&ix), ["Moved/Deep/B.md"]);
        assert_eq!(count(&ix, "SELECT count(*) FROM files"), 1);
    }

    #[test]
    fn huge_notes_keep_a_row_without_text_and_future_times_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        let big = "#tag [[Link]]\n".repeat((MAX_NOTE_BYTES as usize) / 14 + 1);
        fs::write(vault.join("Log.md"), &big).unwrap();
        age(&vault.join("Log.md"));
        fs::write(vault.join("Later.md"), "# Later\n").unwrap();
        let future = SystemTime::now() + Duration::from_secs(3600);
        fs::File::options().write(true).open(vault.join("Later.md")).unwrap().set_modified(future).unwrap();

        let mut errors = Vec::new();
        let mut ix = Indexer::open(vault.canonicalize().unwrap(), &dir.path().join("index.db")).unwrap();
        ix.reconcile(&mut |e| if let IndexEvent::Error(e) = e { errors.push(e) }).unwrap();
        assert_eq!(count(&ix, "SELECT count(*) FROM files WHERE path = 'Log.md'"), 1, "in the tree, and links find it");
        assert_eq!(count(&ix, "SELECT count(*) FROM tags"), 0, "its text is not read");
        assert_eq!(count(&ix, "SELECT length(text) FROM texts t JOIN files f ON f.id = t.file_id WHERE f.path = 'Log.md'"), 0);
        assert!(errors.iter().any(|e| e.starts_with("Log.md is over 10 MB")), "{errors:?}");
        assert_ne!(ix.known["Later.md"].mtime_ns, 0, "a future time is stored, so the next pass does not read it again");
    }
}
