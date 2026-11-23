//! Writes Sage's own small files next to the index (`session.json`, the
//! Kanban order) on a thread of its own, so the UI thread never waits on the
//! disk. Writes go through `sage_editor::save` with a temp file and a rename,
//! so a crash mid-write leaves the previous file whole instead of an empty
//! one that would quietly reset tabs and panes. When several writes to one
//! file queue up, only the newest is written.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

enum Job {
    Write(PathBuf, String),
    /// Answered once every write queued before it is done.
    Flush(Sender<()>),
}

/// Queues `text` to be written to `path`.
pub(crate) fn write_later(path: PathBuf, text: String) {
    send(Job::Write(path, text));
}

/// Waits until every write queued so far is on disk, for when Sage is about
/// to exit and would otherwise cut one off.
pub(crate) fn flush() {
    let (tx, rx) = mpsc::channel();
    send(Job::Flush(tx));
    let _ = rx.recv();
}

fn send(job: Job) {
    static QUEUE: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        let _ = std::thread::Builder::new().name("sage-store".into()).spawn(move || run(rx));
        Mutex::new(tx)
    });
    if let Ok(queue) = queue.lock() {
        let _ = queue.send(job);
    }
}

fn run(rx: Receiver<Job>) {
    while let Ok(first) = rx.recv() {
        let mut jobs = vec![first];
        jobs.extend(rx.try_iter());
        for (i, job) in jobs.iter().enumerate() {
            let Job::Write(path, text) = job else { continue };
            // A newer write to the same file is further on in this batch.
            let newer = jobs[i + 1..].iter().any(|later| matches!(later, Job::Write(p, _) if p == path));
            if !newer {
                // These files fall back to defaults when unreadable, so a failed
                // write costs only the latest state; there is nowhere to report
                // it from this thread.
                let _ = sage_editor::save(path, text, sage_watcher::Backend::Native);
            }
        }
        for job in jobs {
            if let Job::Flush(done) = job {
                let _ = done.send(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_write_wins_and_flush_waits_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session.json");
        for i in 0..50 {
            write_later(file.clone(), format!("{i}"));
        }
        flush();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "49");
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["session.json"], "no temp file is left behind");
    }
}
