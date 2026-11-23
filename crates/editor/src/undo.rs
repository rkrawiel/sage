//! Undo and redo for a note being edited: the text and cursor as they were
//! before each step. Typing that follows on quickly is one step, as in
//! Obsidian; anything else, or a pause, starts the next.
//!
//! Each step holds the whole text, so the history is capped by steps and by
//! bytes: a long note keeps fewer steps rather than hundreds of copies.

use std::time::{Duration, Instant};

/// Typing closer together than this undoes as one step.
const GROUP: Duration = Duration::from_millis(1000);
/// The most steps kept per note.
const LIMIT: usize = 200;
/// The most text kept for undo per note, in bytes; at least one step is kept
/// whatever its size.
const BUDGET: usize = 32 * 1024 * 1024;

/// The note's text and the cursor's line and byte column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub text: String,
    pub line: usize,
    pub col: usize,
}

/// What kind of edit is about to happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A character typed or deleted: joins the typing just before it.
    Typing,
    /// Anything else: always a step of its own.
    Other,
}

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// When the last typing happened, while more typing would join it.
    typing: Option<Instant>,
}

impl History {
    /// Called just before an edit. `before` gives the text as it is now; it
    /// is only asked for when the edit starts a step, so typing that joins
    /// the step before it does not copy the whole note on every key.
    pub fn record(&mut self, before: impl FnOnce() -> Snapshot, kind: Kind, now: Instant) {
        let joins = kind == Kind::Typing && self.typing.is_some_and(|at| now.duration_since(at) < GROUP);
        if !joins {
            let before = before();
            if self.undo.last() != Some(&before) {
                self.undo.push(before);
                let mut bytes: usize = self.undo.iter().map(|s| s.text.len()).sum();
                while self.undo.len() > LIMIT || (self.undo.len() > 1 && bytes > BUDGET) {
                    bytes -= self.undo.remove(0).text.len();
                }
            }
        }
        self.redo.clear();
        self.typing = (kind == Kind::Typing).then_some(now);
    }

    /// The cursor moved: the next typing is a new step.
    pub fn break_group(&mut self) {
        self.typing = None;
    }

    /// The step to go back to, given the text as it is now; `None` when
    /// there is nothing to undo.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let back = self.undo.pop()?;
        self.redo.push(current);
        self.typing = None;
        Some(back)
    }

    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let forward = self.redo.pop()?;
        self.undo.push(current);
        self.typing = None;
        Some(forward)
    }

    /// Forgets every step, as when the note is read from disk again.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(text: &str) -> Snapshot {
        Snapshot { text: text.into(), line: 0, col: text.len() }
    }

    #[test]
    fn typing_close_together_is_one_step() {
        let mut h = History::default();
        let t = Instant::now();
        h.record(|| snap(""), Kind::Typing, t);
        h.record(|| snap("a"), Kind::Typing, t + Duration::from_millis(300));
        h.record(|| snap("ab"), Kind::Typing, t + Duration::from_millis(600));
        assert_eq!(h.undo(snap("abc")), Some(snap("")));
        assert_eq!(h.undo(snap("")), None);
        assert_eq!(h.redo(snap("")), Some(snap("abc")));
        assert_eq!(h.redo(snap("abc")), None);
    }

    #[test]
    fn a_pause_a_cursor_move_or_another_edit_starts_a_step() {
        let mut h = History::default();
        let t = Instant::now();
        h.record(|| snap(""), Kind::Typing, t);
        h.record(|| snap("a"), Kind::Typing, t + Duration::from_secs(2));
        h.break_group();
        h.record(|| snap("ab"), Kind::Typing, t + Duration::from_millis(2100));
        h.record(|| snap("abc"), Kind::Other, t + Duration::from_millis(2200));
        h.record(|| snap("abc!"), Kind::Typing, t + Duration::from_millis(2300));
        let mut now = snap("abc!?");
        let mut seen = Vec::new();
        while let Some(back) = h.undo(now.clone()) {
            seen.push(back.text.clone());
            now = back;
        }
        assert_eq!(seen, ["abc!", "abc", "ab", "a", ""]);
    }

    #[test]
    fn a_new_edit_forgets_what_was_undone() {
        let mut h = History::default();
        let t = Instant::now();
        h.record(|| snap("a"), Kind::Other, t);
        assert_eq!(h.undo(snap("ab")), Some(snap("a")));
        h.record(|| snap("a"), Kind::Other, t);
        assert_eq!(h.redo(snap("ax")), None);
    }

    #[test]
    fn history_is_capped() {
        let mut h = History::default();
        let t = Instant::now();
        for i in 0..LIMIT + 10 {
            h.record(|| snap(&i.to_string()), Kind::Other, t);
        }
        let mut count = 0;
        let mut now = snap("end");
        while let Some(back) = h.undo(now.clone()) {
            now = back;
            count += 1;
        }
        assert_eq!(count, LIMIT);
        assert_eq!(now, snap("10"));

        // A long note keeps fewer steps, but always the last.
        let mut h = History::default();
        let big = "x".repeat(BUDGET / 4);
        for i in 0..5 {
            h.record(|| snap(&format!("{i}{big}")), Kind::Other, t);
        }
        let mut kept = Vec::new();
        let mut now = snap("end");
        while let Some(back) = h.undo(now.clone()) {
            kept.push(back.text.chars().next().unwrap());
            now = back;
        }
        assert_eq!(kept, ['4', '3', '2']);
    }

    #[test]
    fn typing_that_joins_a_step_takes_no_snapshot() {
        let mut h = History::default();
        let t = Instant::now();
        h.record(|| snap(""), Kind::Typing, t);
        h.record(|| panic!("no copy of the text for a key that joins"), Kind::Typing, t + Duration::from_millis(100));
    }
}
