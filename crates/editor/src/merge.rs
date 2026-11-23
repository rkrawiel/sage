//! Three-way merge of note text, for a note that changed on disk while its tab
//! had unsaved edits, and a two-way comparison for OneDrive conflict copies.
//!
//! Lines are the unit. A change made on one side is taken, the same change made
//! on both sides is taken once, and only differing changes to the same lines,
//! or different insertions at the same place, are conflicts for the user.
//! Changes to neighbouring lines do not conflict: a task checked in Obsidian
//! next to the line being typed here merges cleanly.
//!
//! Lines compare without their line endings, so a difference in endings alone
//! is not a change, but every string a merge returns keeps the endings it had.

use similar::{capture_diff_slices, Algorithm, DiffOp};

#[derive(Debug, Clone, PartialEq)]
pub struct Merge {
    pub chunks: Vec<Chunk>,
}

/// A run of the merged text. Every string keeps its line endings.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk {
    /// Lines of `ours` kept as they are: unchanged, changed only here, or
    /// changed the same way on both sides.
    Kept(String),
    /// Lines from `theirs` that replace `replaces` lines of `ours`, which did
    /// not change them.
    Changed { text: String, replaces: usize },
    /// Lines both sides changed, differently. `base` is empty in a two-way diff.
    Conflict { base: String, ours: String, theirs: String },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Choice {
    Ours,
    Theirs,
    /// Ours, then theirs.
    Both,
}

/// Merges `ours` and `theirs`, both edited from `base`.
pub fn merge3(base: &str, ours: &str, theirs: &str) -> Merge {
    let (b, o, t) = (lines(base), lines(ours), lines(theirs));
    let mut changes = hunks(&b, &o, Side::Ours);
    changes.extend(hunks(&b, &t, Side::Theirs));
    // By position in the base; an insertion sorts before a change starting at
    // the same line, so it comes out first.
    changes.sort_by_key(|h| (h.b0, h.b1));

    // Changes that overlap one from the other side, directly or through a
    // chain, form one region. Sorted as they are, a change can only join the
    // region started last.
    let mut regions: Vec<Vec<Hunk>> = Vec::new();
    for hunk in changes {
        match regions.last_mut() {
            Some(region) if region.iter().any(|m| m.overlaps(&hunk)) => region.push(hunk),
            _ => regions.push(vec![hunk]),
        }
    }

    let mut out = Builder::default();
    let (mut bi, mut oi, mut ti) = (0, 0, 0);
    for region in regions {
        let r0 = region.iter().map(|h| h.b0).min().unwrap_or(bi);
        let r1 = region.iter().map(|h| h.b1).max().unwrap_or(bi);
        // Between regions both sides still have the base's lines.
        let same = r0 - bi;
        out.kept(&o[oi..oi + same]);
        oi += same;
        ti += same;

        // How many lines a side has over the region: the base's, give or take
        // what that side's changes in it added or removed.
        let len = |side: Side| {
            region
                .iter()
                .filter(|h| h.side == side)
                .fold((r1 - r0) as isize, |n, h| n + (h.s1 - h.s0) as isize - (h.b1 - h.b0) as isize)
                as usize
        };
        let (on, tn) = (len(Side::Ours), len(Side::Theirs));
        let (ours_part, theirs_part) = (&o[oi..oi + on], &t[ti..ti + tn]);
        let ours_changed = region.iter().any(|h| h.side == Side::Ours);
        let theirs_changed = region.iter().any(|h| h.side == Side::Theirs);
        if !theirs_changed || keys(ours_part).eq(keys(theirs_part)) {
            out.kept(ours_part);
        } else if !ours_changed {
            out.changed(theirs_part, on);
        } else {
            out.conflict(b[r0..r1].concat(), ours_part.concat(), theirs_part.concat());
        }
        bi = r1;
        oi += on;
        ti += tn;
    }
    out.kept(&o[oi..]);
    out.finish()
}

/// A change the app made to `base`, giving `ours`, carried over to `theirs`,
/// what the file holds now: the merged text, or `None` where the two overlap.
/// For writes to notes no tab holds, whose base is the text last read.
pub fn rebase(base: &str, ours: &str, theirs: &str) -> Option<String> {
    let merge = merge3(base, ours, theirs);
    (merge.conflicts() == 0).then(|| merge.resolve(&[]))
}

/// Compares two versions without a common base, as for a conflict copy: lines
/// they share are kept, every region that differs is a conflict.
pub fn diff2(ours: &str, theirs: &str) -> Merge {
    let (o, t) = (lines(ours), lines(theirs));
    let mut out = Builder::default();
    let mut oi = 0;
    for hunk in hunks(&o, &t, Side::Theirs) {
        out.kept(&o[oi..hunk.b0]);
        out.conflict(String::new(), o[hunk.b0..hunk.b1].concat(), t[hunk.s0..hunk.s1].concat());
        oi = hunk.b1;
    }
    out.kept(&o[oi..]);
    out.finish()
}

impl Merge {
    pub fn conflicts(&self) -> usize {
        self.chunks.iter().filter(|c| matches!(c, Chunk::Conflict { .. })).count()
    }

    /// The merged text, conflict `n` resolved by `choices[n]`. Conflicts
    /// without a choice keep ours.
    pub fn resolve(&self, choices: &[Choice]) -> String {
        // Glue for a last line without a newline that is no longer last.
        let ending = if self.chunks.iter().any(|c| c.texts().any(|t| t.contains("\r\n"))) { "\r\n" } else { "\n" };
        let mut out = String::new();
        let mut n = 0;
        for chunk in &self.chunks {
            match chunk {
                Chunk::Kept(text) | Chunk::Changed { text, .. } => push(&mut out, text, ending),
                Chunk::Conflict { ours, theirs, .. } => {
                    match choices.get(n).copied().unwrap_or(Choice::Ours) {
                        Choice::Ours => push(&mut out, ours, ending),
                        Choice::Theirs => push(&mut out, theirs, ending),
                        Choice::Both => {
                            push(&mut out, ours, ending);
                            push(&mut out, theirs, ending);
                        }
                    }
                    n += 1;
                }
            }
        }
        out
    }

    /// Where line `line` of `ours` ends up in `resolve(choices)`, to keep the
    /// cursor on the same text. A line that was replaced maps to the first
    /// line of what replaced it. A line past the end maps to the last line, as
    /// an editor counts lines: after a final newline there is an empty one.
    pub fn ours_line(&self, line: usize, choices: &[Choice]) -> usize {
        let count = |s: &str| s.split_inclusive('\n').count();
        let (mut ours, mut out, mut n) = (0, 0, 0);
        for chunk in &self.chunks {
            match chunk {
                Chunk::Kept(text) => {
                    let len = count(text);
                    if line < ours + len {
                        return out + line - ours;
                    }
                    ours += len;
                    out += len;
                }
                Chunk::Changed { text, replaces } => {
                    if line < ours + replaces {
                        return out;
                    }
                    ours += replaces;
                    out += count(text);
                }
                Chunk::Conflict { ours: mine, theirs, .. } => {
                    let choice = choices.get(n).copied().unwrap_or(Choice::Ours);
                    n += 1;
                    let (len, other) = (count(mine), count(theirs));
                    if line < ours + len {
                        return if choice == Choice::Theirs { out } else { out + line - ours };
                    }
                    ours += len;
                    out += match choice {
                        Choice::Ours => len,
                        Choice::Theirs => other,
                        Choice::Both => len + other,
                    };
                }
            }
        }
        self.resolve(choices).matches('\n').count()
    }
}

impl Chunk {
    fn texts(&self) -> impl Iterator<Item = &str> {
        let texts: [&str; 3] = match self {
            Chunk::Kept(text) | Chunk::Changed { text, .. } => [text, "", ""],
            Chunk::Conflict { base, ours, theirs } => [base, ours, theirs],
        };
        texts.into_iter()
    }
}

/// Appends `text`, first ending the last line if it has no newline yet.
fn push(out: &mut String, text: &str, ending: &str) {
    if text.is_empty() {
        return;
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(ending);
    }
    out.push_str(text);
}

/// Lines with their endings; the last one may have none.
fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// What a line is compared by: its text without the line ending.
fn key(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

fn keys<'a>(lines: &'a [&'a str]) -> impl Iterator<Item = &'a str> {
    lines.iter().map(|l| key(l))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Side {
    Ours,
    Theirs,
}

/// One side's change: base lines `b0..b1` became that side's lines `s0..s1`.
/// An empty base range is an insertion before base line `b0`.
#[derive(Debug, Clone, Copy)]
struct Hunk {
    b0: usize,
    b1: usize,
    s0: usize,
    s1: usize,
    side: Side,
}

impl Hunk {
    /// Whether two changes from different sides touch the same base lines:
    /// their ranges share a line, an insertion falls strictly inside the
    /// other's range, or both insert at the same place. Changes that only
    /// meet at a boundary do not overlap.
    fn overlaps(&self, other: &Hunk) -> bool {
        if self.side == other.side {
            return false;
        }
        match (self.b0 == self.b1, other.b0 == other.b1) {
            (true, true) => self.b0 == other.b0,
            (true, false) => other.b0 < self.b0 && self.b0 < other.b1,
            (false, true) => self.b0 < other.b0 && other.b0 < self.b1,
            (false, false) => self.b0 < other.b1 && other.b0 < self.b1,
        }
    }
}

/// The changes from `old` to `new`, adjacent edit operations joined into one.
fn hunks(old: &[&str], new: &[&str], side: Side) -> Vec<Hunk> {
    let (old_keys, new_keys): (Vec<&str>, Vec<&str>) = (keys(old).collect(), keys(new).collect());
    // Patience anchors on lines that occur once on each side, such as headings
    // and sentences. Notes repeat blank lines and list markers a lot, and
    // Myers happily aligns those, which splits one edit into scattered pieces.
    let mut out: Vec<Hunk> = Vec::new();
    for op in capture_diff_slices(Algorithm::Patience, &old_keys, &new_keys) {
        let (b0, b1, s0, s1) = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete { old_index, old_len, new_index } => (old_index, old_index + old_len, new_index, new_index),
            DiffOp::Insert { old_index, new_index, new_len } => (old_index, old_index, new_index, new_index + new_len),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                (old_index, old_index + old_len, new_index, new_index + new_len)
            }
        };
        match out.last_mut() {
            Some(last) if last.b1 == b0 && last.s1 == s0 => {
                last.b1 = b1;
                last.s1 = s1;
            }
            _ => out.push(Hunk { b0, b1, s0, s1, side }),
        }
    }
    out
}

/// Collects chunks, joining neighbours of the same kind.
#[derive(Default)]
struct Builder {
    chunks: Vec<Chunk>,
}

impl Builder {
    fn kept(&mut self, lines: &[&str]) {
        if lines.is_empty() {
            return;
        }
        match self.chunks.last_mut() {
            Some(Chunk::Kept(text)) => text.push_str(&lines.concat()),
            _ => self.chunks.push(Chunk::Kept(lines.concat())),
        }
    }

    /// An empty `lines` is still a change: theirs deleted what ours kept.
    fn changed(&mut self, lines: &[&str], replaces: usize) {
        if lines.is_empty() && replaces == 0 {
            return;
        }
        match self.chunks.last_mut() {
            Some(Chunk::Changed { text, replaces: n }) => {
                text.push_str(&lines.concat());
                *n += replaces;
            }
            _ => self.chunks.push(Chunk::Changed { text: lines.concat(), replaces }),
        }
    }

    fn conflict(&mut self, base: String, ours: String, theirs: String) {
        self.chunks.push(Chunk::Conflict { base, ours, theirs });
    }

    fn finish(self) -> Merge {
        Merge { chunks: self.chunks }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(text: &str) -> Chunk {
        Chunk::Kept(text.into())
    }

    fn changed(text: &str, replaces: usize) -> Chunk {
        Chunk::Changed { text: text.into(), replaces }
    }

    fn conflict(base: &str, ours: &str, theirs: &str) -> Chunk {
        Chunk::Conflict { base: base.into(), ours: ours.into(), theirs: theirs.into() }
    }

    /// The clean result, after checking there was nothing to decide.
    fn clean(base: &str, ours: &str, theirs: &str) -> String {
        let merge = merge3(base, ours, theirs);
        assert_eq!(merge.conflicts(), 0, "{merge:?}");
        merge.resolve(&[])
    }

    #[test]
    fn one_sided_and_identical_edits() {
        let base = "a\nb\nc\n";
        assert_eq!(merge3(base, base, base).chunks, [kept(base)]);
        assert_eq!(merge3(base, "a\nB\nc\n", base).chunks, [kept("a\nB\nc\n")]);
        assert_eq!(merge3(base, base, "a\nB\nc\n").chunks, [kept("a\n"), changed("B\n", 1), kept("c\n")]);
        assert_eq!(merge3(base, "a\nB\nc\n", "a\nB\nc\n").chunks, [kept("a\nB\nc\n")], "the same edit, taken once");
        assert_eq!(merge3(base, base, "a\nc\n").chunks, [kept("a\n"), changed("", 1), kept("c\n")]);
    }

    #[test]
    fn edits_to_different_and_neighbouring_lines_merge() {
        let base = "1\n2\n3\n4\n5\n";
        assert_eq!(clean(base, "one\n2\n3\n4\n5\n", "1\n2\n3\n4\nfive\n"), "one\n2\n3\n4\nfive\n");
        // Unlike git, lines next to each other do not conflict.
        assert_eq!(clean(base, "1\nX\n3\n4\n5\n", "1\n2\nY\n4\n5\n"), "1\nX\nY\n4\n5\n");
        assert_eq!(clean(base, "1\n2\n2b\n3\n4\n5\n", "1\n2\nY\n4\n5\n"), "1\n2\n2b\nY\n4\n5\n");
    }

    #[test]
    fn overlapping_edits_conflict() {
        let merge = merge3("a\nb\nc\n", "a\nB1\nc\n", "a\nB2\nc\n");
        assert_eq!(merge.chunks, [kept("a\n"), conflict("b\n", "B1\n", "B2\n"), kept("c\n")]);

        // The region grows to cover every change that overlaps another.
        let merge = merge3("a\nb\nc\nd\ne\n", "a\nB\nC\nd\ne\n", "a\nb\nX\nY\ne\n");
        assert_eq!(merge.chunks, [kept("a\n"), conflict("b\nc\nd\n", "B\nC\nd\n", "b\nX\nY\n"), kept("e\n")]);

        // Deleting a line the other side edited.
        let merge = merge3("a\nb\nc\n", "a\nc\n", "a\nB\nc\n");
        assert_eq!(merge.chunks, [kept("a\n"), conflict("b\n", "", "B\n"), kept("c\n")]);
    }

    #[test]
    fn insertions_at_the_same_place() {
        let merge = merge3("a\nb\n", "a\nX\nb\n", "a\nY\nb\n");
        assert_eq!(merge.chunks, [kept("a\n"), conflict("", "X\n", "Y\n"), kept("b\n")]);
        assert_eq!(clean("a\nb\n", "a\nX\nb\n", "a\nX\nb\n"), "a\nX\nb\n");
        // An insertion next to the other side's edit is not a conflict.
        assert_eq!(clean("a\nb\n", "a\nX\nb\n", "a\nB\n"), "a\nX\nB\n");
    }

    #[test]
    fn line_endings_survive() {
        assert_eq!(clean("a\r\nb\r\nc\r\n", "A\r\nb\r\nc\r\n", "a\r\nb\r\nC\r\n"), "A\r\nb\r\nC\r\n");
        // Endings alone are not a change.
        assert_eq!(clean("a\nb\n", "a\r\nb\r\n", "a\nB\n"), "a\r\nB\n");
        // A last line without a newline gets one when more lines follow it.
        assert_eq!(clean("a\nb", "a\nb\nc", "x\nb"), "x\nb\nc");
        assert_eq!(clean("a\nb", "a\nB", "a\nb\nc"), "a\nB\nc");
        assert_eq!(clean("a\r\nb", "a\r\nb", "a\r\nb\r\nc"), "a\r\nb\r\nc");
    }

    #[test]
    fn empty_texts() {
        assert_eq!(merge3("", "", "").chunks, []);
        assert_eq!(merge3("", "x\n", "y\n").chunks, [conflict("", "x\n", "y\n")]);
        assert_eq!(clean("", "x\n", ""), "x\n");
        assert_eq!(clean("a\n", "", "a\n"), "");
        assert_eq!(clean("a\n", "a\n", ""), "");
    }

    #[test]
    fn resolving_conflicts() {
        let merge = merge3("a\nb\nc\n", "a\nB1\nc\n", "a\nB2\nc\n");
        assert_eq!(merge.resolve(&[Choice::Ours]), "a\nB1\nc\n");
        assert_eq!(merge.resolve(&[Choice::Theirs]), "a\nB2\nc\n");
        assert_eq!(merge.resolve(&[Choice::Both]), "a\nB1\nB2\nc\n");
        assert_eq!(merge.resolve(&[]), "a\nB1\nc\n", "an undecided conflict keeps ours");
        // Both sides' last lines, neither with a newline.
        assert_eq!(merge3("a\nb", "a\nX", "a\nY").resolve(&[Choice::Both]), "a\nX\nY");
    }

    #[test]
    fn cursor_lines_follow_the_text() {
        // Theirs inserted three lines above: the cursor moves down with its line.
        let merge = merge3("a\nb\nc\n", "a\nb\nC\n", "n1\nn2\nn3\na\nb\nc\n");
        assert_eq!(merge.ours_line(2, &[]), 5);

        // A line theirs replaced lands on the first line of the replacement.
        let merge = merge3("a\nb\nc\n", "a\nb\nc\n", "a\nX\nY\nZ\nc\n");
        assert_eq!((merge.ours_line(1, &[]), merge.ours_line(2, &[])), (1, 4));

        // In and after a conflict, by choice.
        let merge = merge3("a\nb\nc\n", "a\nB1\nB1b\nc\n", "a\nB2\nc\n");
        let at = |line, choice| merge.ours_line(line, &[choice]);
        assert_eq!((at(2, Choice::Ours), at(2, Choice::Both), at(2, Choice::Theirs)), (2, 2, 1));
        assert_eq!((at(3, Choice::Ours), at(3, Choice::Theirs), at(3, Choice::Both)), (3, 2, 4));
        assert_eq!(at(99, Choice::Ours), 4, "past the end: the empty line after the last newline");
    }

    #[test]
    fn two_way_diff() {
        let merge = diff2("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!(merge.chunks, [kept("a\n"), conflict("", "b\n", "B\n"), kept("c\n"), conflict("", "", "d\n")]);
        assert_eq!(merge.conflicts(), 2);
        assert_eq!(merge.resolve(&[Choice::Theirs, Choice::Both]), "a\nB\nc\nd\n");
        assert_eq!(diff2("same\n", "same\n").chunks, [kept("same\n")]);
    }

    #[test]
    fn a_note_edited_in_two_places() {
        let base = "# Alpha\n\n- [ ] Draft the runbook\n- [ ] Review policies\n\nNotes so far.\n";
        // Here: a new paragraph typed at the end. There: a task checked near the top.
        let ours = format!("{base}\nThe cutover moves to Friday.\n");
        let theirs = base.replace("- [ ] Review policies", "- [x] Review policies ✅ 2026-11-13");
        let merged = clean(base, &ours, &theirs);
        assert_eq!(
            merged,
            "# Alpha\n\n- [ ] Draft the runbook\n- [x] Review policies ✅ 2026-11-13\n\nNotes so far.\n\nThe cutover moves to Friday.\n"
        );
        // The cursor at the end of the new paragraph stays there.
        let merge = merge3(base, &ours, &theirs);
        assert_eq!(merge.ours_line(7, &[]), 7);
    }
}
