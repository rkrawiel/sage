//! Task write-back: checking, unchecking or moving a task line in a note on
//! disk, or setting the `status` of a task document.
//!
//! A write names the line, or the status, as the index saw it. If the note has
//! changed since, the change is made to the text the index read, `base`, and
//! merged with the note as it is now, so an edit elsewhere in the note does
//! not block it.

use crate::{merge, recur, save_guarded};
use sage_core::{FmValue, LineEdit, TaskStatus};
use sage_watcher::Backend;
use chrono::NaiveDate;
use std::path::Path;

/// The lines that replace task line `raw` when it is toggled: the toggled
/// line, and above it the next occurrence when a recurring task is checked.
/// `None` if `raw` is not a task.
pub fn toggled(raw: &str, today: &str) -> Option<Vec<String>> {
    Some(with_next(raw, sage_core::toggle_task(raw, today)?, today))
}

/// The lines that replace task line `raw` when it moves to a Kanban column:
/// `#status/<slug>`, or no status tag for `None`, checked or not. A recurring
/// task that gets checked gets its next occurrence above it.
pub fn moved(raw: &str, slug: Option<&str>, done: bool, today: &str) -> Option<Vec<String>> {
    Some(with_next(raw, sage_core::set_task_column(raw, slug, done, today)?, today))
}

fn with_next(raw: &str, changed: String, today: &str) -> Vec<String> {
    let checked = sage_core::parse(&changed).tasks.first().is_some_and(|t| matches!(t.state, 'x' | 'X'));
    let next = NaiveDate::parse_from_str(today, "%Y-%m-%d").ok().and_then(|day| recur::next_occurrence(raw, day));
    match next {
        Some(next) if checked => vec![next, changed],
        _ => vec![changed],
    }
}

/// Toggles the task on `line` of the note at `path`, but only if that line still
/// reads `raw`, or read `raw` in `base`, the text the index last read: a stale
/// line number must never rewrite the wrong line. Returns the note's new text.
pub fn toggle(path: &Path, line: u32, raw: &str, base: Option<&str>, today: &str, backend: Backend) -> Result<String, String> {
    rewrite(path, line, raw, base, backend, |raw| toggled(raw, today))
}

/// Moves the task on `line` to a Kanban column; see [`moved`]. Guarded like [`toggle`].
#[allow(clippy::too_many_arguments)]
pub fn set_column(
    path: &Path,
    line: u32,
    raw: &str,
    base: Option<&str>,
    slug: Option<&str>,
    done: bool,
    today: &str,
    backend: Backend,
) -> Result<String, String> {
    rewrite(path, line, raw, base, backend, |raw| moved(raw, slug, done, today))
}

fn rewrite(
    path: &Path,
    line: u32,
    raw: &str,
    base: Option<&str>,
    backend: Backend,
    change: impl FnOnce(&str) -> Option<Vec<String>>,
) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let lines = change(raw).ok_or("that line is not a task")?;
    let edit = |src: &str| {
        let ending = if src.contains("\r\n") { "\r\n" } else { "\n" };
        let edit = LineEdit { line, col: 0, len: raw.len() as u32, text: lines.join(ending) };
        sage_core::edit_lines(src, &[edit]).ok_or("the task line could not be edited")
    };
    let updated = if sage_core::line_at(&text, line) == Some(raw) {
        edit(&text)?
    } else {
        let stale = "the note changed since it was indexed, try again in a moment";
        let base = base.filter(|b| sage_core::line_at(b, line) == Some(raw)).ok_or(stale)?;
        merge::rebase(base, &edit(base)?, &text).ok_or("the task line itself changed since it was indexed")?
    };
    save_guarded(path, &text, &updated, backend).map_err(|e| e.to_string())?;
    Ok(updated)
}

/// The `status` of a note as the index reads it into `v_documents`: the
/// first value, trimmed, ASCII lowercase, spaces and underscores as `-`.
pub fn status_of(text: &str) -> Option<String> {
    let (yaml, _, _) = sage_core::split_frontmatter(text);
    let entry = sage_core::frontmatter(yaml?).into_iter().find(|e| e.ord == 0 && e.key.eq_ignore_ascii_case("status"))?;
    let value = match entry.value {
        FmValue::Text(text) => text,
        FmValue::Num(n) => n.to_string(),
        FmValue::Bool(b) => b.to_string(),
        FmValue::Null => return None,
    };
    Some(value.trim_matches(' ').to_ascii_lowercase().replace([' ', '_'], "-"))
}

/// The status a task document's checkbox writes when its status is `status`:
/// `done`, or for a done one the status it had before, `reopen`, else `todo`.
pub fn toggled_status(status: Option<&str>, reopen: Option<&str>) -> String {
    let done = TaskStatus::Done.slug();
    if status != Some(done) {
        return done.to_owned();
    }
    reopen.filter(|s| !s.is_empty() && *s != done).unwrap_or(TaskStatus::Todo.slug()).to_owned()
}

/// `text` with its frontmatter `status` set to `status`, written as the new
/// task form writes it: `status: in-progress`. A `status` key keeps its
/// place; without one, the line goes after `title`, else after `type`, else
/// last. `None` when the note has no frontmatter.
pub fn with_status(text: &str, status: &str) -> Option<String> {
    let (bom, src) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };
    let ending = if src.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let bare = |line: &str| line.trim_end_matches(['\r', '\n']).to_owned();
    if lines.first().map(|l| bare(l)).as_deref() != Some("---") {
        return None;
    }
    let close = (1..lines.len()).find(|&i| matches!(bare(lines[i]).as_str(), "---" | "..."))?;
    // A top-level key, and the end of its entry: the indented or `- ` lines under it.
    let key = |line: &str| -> Option<String> {
        if line.starts_with([' ', '\t', '-', '#']) {
            return None;
        }
        let (key, _) = line.split_once(':')?;
        Some(key.trim().trim_matches(['"', '\'']).to_lowercase())
    };
    let entry_end = |start: usize| (start + 1..close).find(|&i| !lines[i].starts_with([' ', '\t', '-'])).unwrap_or(close);
    let find = |name: &str| (1..close).find(|&i| key(lines[i]).as_deref() == Some(name));

    // A slug YAML would read as something else, `true` or `12`, is quoted.
    let value = match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(status) {
        Ok(serde_yaml_ng::Value::String(s)) if s == status => status.to_owned(),
        _ => format!("\"{}\"", status.replace('\\', "\\\\").replace('"', "\\\"")),
    };
    let (at, end, written) = match find("status") {
        Some(i) => {
            let name = lines[i].split_once(':').map_or("status", |(k, _)| k.trim_end());
            (i, entry_end(i), format!("{name}: {value}{ending}"))
        }
        None => {
            let after = find("title").or_else(|| find("type")).map_or(close, entry_end);
            (after, after, format!("status: {value}{ending}"))
        }
    };
    let mut out = String::with_capacity(text.len() + written.len());
    out.push_str(bom);
    lines[..at].iter().for_each(|l| out.push_str(l));
    out.push_str(&written);
    lines[end..].iter().for_each(|l| out.push_str(l));
    Some(out)
}

/// Sets the `status` of the task document at `path`, but only if its status
/// still reads `expected` (as [`status_of`] reads it, `None` for none), or
/// read `expected` in `base`, the text the index last read; then the change
/// is made to `base` and merged with the note as it is now. Returns the
/// note's new text.
pub fn set_status(
    path: &Path,
    expected: Option<&str>,
    status: &str,
    base: Option<&str>,
    backend: Backend,
) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let edit = |src: &str| with_status(src, status).ok_or("the note has no frontmatter");
    let updated = if status_of(&text).as_deref() == expected {
        edit(&text)?
    } else {
        let stale = "the note changed since it was indexed, try again in a moment";
        let base = base.filter(|b| status_of(b).as_deref() == expected).ok_or(stale)?;
        merge::rebase(base, &edit(base)?, &text).ok_or("the status itself changed since it was indexed")?
    };
    save_guarded(path, &text, &updated, backend).map_err(|e| e.to_string())?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_only_the_line_it_was_given() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.md");
        std::fs::write(&path, "# T\r\n- [ ] Ship 📅 2026-12-01\r\n- [ ] Other\r\n").unwrap();

        let text = toggle(&path, 1, "- [ ] Ship 📅 2026-12-01", None, "2026-11-13", Backend::Native).unwrap();
        assert_eq!(text, "# T\r\n- [x] Ship 📅 2026-12-01 ✅ 2026-11-13\r\n- [ ] Other\r\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        // The index still thinks line 1 is open: refused, file untouched.
        let stale = toggle(&path, 1, "- [ ] Ship 📅 2026-12-01", None, "2026-11-13", Backend::Native);
        assert!(stale.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        let text = set_column(&path, 2, "- [ ] Other", None, Some("doing"), false, "2026-11-13", Backend::Poll).unwrap();
        assert!(text.ends_with("- [ ] Other #status/doing\r\n"));
    }

    #[test]
    fn merges_with_edits_made_since_the_index_read_the_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.md");
        let base = "# T\n- [ ] Ship\n- [ ] Other\n";
        // Lines were added above the task elsewhere, after the index read it.
        std::fs::write(&path, "# T\nNew intro\nMore\n- [ ] Ship\n- [ ] Other\n").unwrap();

        assert!(toggle(&path, 1, "- [ ] Ship", None, "2026-11-13", Backend::Native).is_err(), "no base, no merge");
        let text = toggle(&path, 1, "- [ ] Ship", Some(base), "2026-11-13", Backend::Native).unwrap();
        assert_eq!(text, "# T\nNew intro\nMore\n- [x] Ship ✅ 2026-11-13\n- [ ] Other\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        // The task line itself was changed elsewhere: refused.
        std::fs::write(&path, "# T\n- [ ] Ship it today\n- [ ] Other\n").unwrap();
        let refused = toggle(&path, 1, "- [ ] Ship", Some(base), "2026-11-13", Backend::Native);
        assert!(refused.unwrap_err().contains("changed"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# T\n- [ ] Ship it today\n- [ ] Other\n");
    }

    #[test]
    fn checking_a_recurring_task_adds_the_next_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.md");
        std::fs::write(&path, "- [ ] Water 🔁 every week 📅 2026-11-13\r\nend\r\n").unwrap();
        let text = toggle(&path, 0, "- [ ] Water 🔁 every week 📅 2026-11-13", None, "2026-11-14", Backend::Native).unwrap();
        assert_eq!(text, "- [ ] Water 🔁 every week 📅 2026-11-20\r\n- [x] Water 🔁 every week 📅 2026-11-13 ✅ 2026-11-14\r\nend\r\n");

        // Unchecking leaves the next one alone; moving to Done checks and recurs.
        let raw = "- [x] Water 🔁 every week 📅 2026-11-13 ✅ 2026-11-14";
        assert_eq!(toggled(raw, "2026-11-14").unwrap(), ["- [ ] Water 🔁 every week 📅 2026-11-13"]);
        assert_eq!(
            moved("- [ ] Pay #status/doing 🔁 every month 📅 2026-11-30", None, true, "2026-11-30").unwrap(),
            ["- [ ] Pay #status/doing 🔁 every month 📅 2026-12-30", "- [x] Pay 🔁 every month 📅 2026-11-30 ✅ 2026-11-30"]
        );
        assert_eq!(moved("- [ ] Pay 🔁 every month", Some("doing"), false, "2026-11-30").unwrap().len(), 1);
    }

    #[test]
    fn rewrites_a_document_status_in_place() {
        let note = "\u{feff}---\r\nid: a\r\ntype: task\r\ntitle: A\r\nStatus: In Progress  \r\ndue: 2026-11-30\r\n---\r\nBody\r\n";
        assert_eq!(status_of(note).as_deref(), Some("in-progress"), "as v_documents reads it");
        assert_eq!(
            with_status(note, "done").unwrap(),
            "\u{feff}---\r\nid: a\r\ntype: task\r\ntitle: A\r\nStatus: done\r\ndue: 2026-11-30\r\n---\r\nBody\r\n",
            "the key keeps its place, its spelling, the BOM and CRLF"
        );
        // A list under the key is replaced as a whole.
        assert_eq!(
            with_status("---\ntype: task\nstatus:\n  - todo\n  - x\ntags: []\n---\n", "doing").unwrap(),
            "---\ntype: task\nstatus: doing\ntags: []\n---\n"
        );
        // A slug YAML would read as another type is quoted.
        assert_eq!(with_status("---\nstatus: todo\n---\n", "true").unwrap(), "---\nstatus: \"true\"\n---\n");
        assert_eq!(with_status("# No frontmatter\n", "done"), None);
    }

    #[test]
    fn adds_a_status_where_the_task_form_writes_it() {
        assert_eq!(
            with_status("---\nid: a\ntype: task\ntitle: \"A: b\"\ncreated: 2026-11-16\ntags: []\n---\n", "done").unwrap(),
            "---\nid: a\ntype: task\ntitle: \"A: b\"\nstatus: done\ncreated: 2026-11-16\ntags: []\n---\n"
        );
        assert_eq!(
            with_status("---\ntype: task\naliases:\n  - x\n---\nBody\n", "todo").unwrap(),
            "---\ntype: task\nstatus: todo\naliases:\n  - x\n---\nBody\n",
            "after type when there is no title"
        );
        assert_eq!(with_status("---\nid: a\n---\n", "todo").unwrap(), "---\nid: a\nstatus: todo\n---\n", "else last");
        assert_eq!(with_status("---\n---\n", "todo").unwrap(), "---\nstatus: todo\n---\n");
        assert_eq!(status_of("---\ntype: task\n---\n"), None);
    }

    #[test]
    fn toggling_a_document_goes_to_done_and_back() {
        assert_eq!(toggled_status(Some("in-progress"), None), "done");
        assert_eq!(toggled_status(None, None), "done");
        assert_eq!(toggled_status(Some("done"), Some("in-progress")), "in-progress");
        assert_eq!(toggled_status(Some("done"), None), "todo");
        assert_eq!(toggled_status(Some("done"), Some("done")), "todo");
    }

    #[test]
    fn sets_a_document_status_only_as_the_index_saw_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.md");
        let base = "---\ntype: task\ntitle: T\n---\nBody\n";
        std::fs::write(&path, base).unwrap();

        // No status yet: it is added.
        let text = set_status(&path, None, "done", Some(base), Backend::Native).unwrap();
        assert_eq!(text, "---\ntype: task\ntitle: T\nstatus: done\n---\nBody\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        // The index still thinks it has none: refused, file untouched.
        assert!(set_status(&path, None, "todo", None, Backend::Native).unwrap_err().contains("changed since"));
        assert!(set_status(&path, None, "todo", Some(base), Backend::Native).unwrap_err().contains("status itself"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        // Edits elsewhere since the index read it are kept.
        std::fs::write(&path, "---\ntype: task\ntitle: T\nstatus: done\n---\nBody\nMore\n").unwrap();
        let text = set_status(&path, Some("done"), "in-progress", Some(&text), Backend::Poll).unwrap();
        assert_eq!(text, "---\ntype: task\ntitle: T\nstatus: in-progress\n---\nBody\nMore\n");
    }
}
