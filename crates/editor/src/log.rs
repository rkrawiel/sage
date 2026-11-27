//! The project log and the lines that feed it: dated entries with a kind tag,
//! the daily-note inbox that is sorted into them, tasks waiting on a person,
//! and event notes.
//!
//! Everything here is plain Markdown. A log entry is a list item such as
//! `- 2026-11-26 #decision Ship on Friday [[Anna]]`, kept under a `## Log`
//! heading of a project note; the same line anywhere else in the vault that
//! links to a project reaches its timeline too.

use crate::save_guarded;
use chrono::NaiveDate;
use regex::Regex;
use sage_watcher::Backend;
use std::path::Path;
use std::sync::LazyLock;

/// The kinds of entry, as tags. `info` is what an entry without a kind is.
pub const KINDS: [&str; 4] = ["meeting", "email", "decision", "info"];

/// The heading a project's entries are added under.
pub const LOG_HEADING: &str = "Log";

/// Marks a task that waits for somebody else.
pub const WAITING_TAG: &str = "waiting";

/// ASCII digits only: `\d` is Unicode, and a fullwidth digit is three bytes,
/// so the ten bytes cut off after a match would split a character.
static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}").unwrap());
static ADDED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"➕\s*([0-9]{4}-[0-9]{2}-[0-9]{2})").unwrap());

/// An entry's kind, taken from `kind: text`, and the text. Anything else is
/// an `info` entry.
pub fn split_kind(input: &str) -> (&'static str, String) {
    let input = input.trim();
    if let Some((head, rest)) = input.split_once(':') {
        if let Some(kind) = KINDS.iter().find(|k| k.eq_ignore_ascii_case(head.trim())) {
            return (kind, rest.trim().to_owned());
        }
    }
    ("info", input.to_owned())
}

/// The list item that records an event.
pub fn entry(date: &str, kind: &str, text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("- {date} #{kind} {text}")
}

#[derive(Debug, PartialEq, Clone)]
pub struct Entry {
    /// `YYYY-MM-DD`, or empty when the line has none.
    pub date: String,
    pub kind: String,
    /// What is left of the line without its marker, date and kind tag.
    pub text: String,
}

/// The entry a line holds: a list item with a kind tag, and a date at the start.
pub fn parse_entry(line: &str) -> Option<Entry> {
    let body = line.trim_start();
    let body = ["- ", "* ", "+ "].iter().find_map(|m| body.strip_prefix(m)).unwrap_or(body);
    let mut kind = None;
    let mut date = String::new();
    let mut words = Vec::new();
    for word in body.split_whitespace() {
        let tag = word.strip_prefix('#').map(str::to_lowercase);
        match tag.as_deref().and_then(|t| KINDS.iter().find(|k| **k == t)) {
            Some(k) if kind.is_none() => kind = Some((*k).to_owned()),
            _ if words.is_empty() && date.is_empty() && DATE.is_match(word) => date = word.get(..10).unwrap_or_default().to_owned(),
            _ => words.push(word),
        }
    }
    Some(Entry { date, kind: kind?, text: words.join(" ") })
}

/// The date a note's name starts with, as a daily note's does.
pub fn date_in_name(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.get(..10).filter(|_| DATE.is_match(name)).map(str::to_owned)
}

/// Whole days from `then` to `today`, both `YYYY-MM-DD`.
pub fn days_between(then: &str, today: &str) -> Option<i64> {
    let parse = |s: &str| NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok();
    Some((parse(today)? - parse(then)?).num_days())
}

/// How long a task has waited, from its `➕` date.
pub fn waiting_days(line: &str, today: &str) -> Option<i64> {
    days_between(&ADDED.captures(line)?[1], today)
}

/// Line `line` of the note at `rel` with `new`, if it still reads `raw`.
/// Returns the note's new text. Its line ending is kept.
pub fn rewrite_line(vault: &Path, rel: &str, line: usize, raw: &str, new: &str, backend: Backend) -> Result<String, String> {
    let abs = vault.join(rel);
    let text = std::fs::read_to_string(&abs).map_err(|e| format!("{rel}: {e}"))?;
    let mut out = String::with_capacity(text.len() + new.len());
    let mut found = false;
    for (i, piece) in text.split_inclusive('\n').enumerate() {
        let content = piece.trim_end_matches(['\n', '\r']);
        if i == line && content == raw {
            found = true;
            out.push_str(new);
            out.push_str(&piece[content.len()..]);
        } else {
            out.push_str(piece);
        }
    }
    if !found {
        return Err(format!("{rel} changed: the line is not there any more"));
    }
    save_guarded(&abs, &text, &out, backend).map_err(|e| format!("{rel}: {e}"))?;
    Ok(out)
}

/// The lines of a note nobody has dealt with yet: text that is not a task,
/// links to nothing and carries no tag. A note with an `## Inbox` heading is
/// read under it only. Pairs of the 0-based line and its text.
pub fn untriaged(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let start = sage_core::split_frontmatter(text).2 as usize;
    let headings = sage_core::parse(text).headings;
    let (from, to) = match headings.iter().position(|h| h.text.trim().eq_ignore_ascii_case("inbox")) {
        Some(i) => {
            let h = &headings[i];
            let end = headings[i + 1..].iter().find(|n| n.level <= h.level).map_or(lines.len(), |n| n.line as usize);
            (h.line as usize + 1, end)
        }
        None => (start, lines.len()),
    };
    let mut fenced = false;
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate().take(to.min(lines.len())) {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if i < from || fenced || t.is_empty() || t.starts_with('#') || t.starts_with('|') || t.starts_with('>') || t.starts_with("---") {
            continue;
        }
        let item = ["- ", "* ", "+ "].iter().find_map(|m| t.strip_prefix(m)).unwrap_or(t);
        let checkbox = item.starts_with('[') && item.chars().nth(2) == Some(']');
        let tagged = item.split_whitespace().any(|w| w.len() > 1 && w.starts_with('#'));
        if checkbox || tagged || item.contains("[[") || item.contains("](") {
            continue;
        }
        out.push((i, item.to_owned()));
    }
    out
}

/// What an inbox line becomes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Triage {
    Task,
    Followup,
    Waiting,
}

/// The task line for `text`: tracked, linked to `person` and tagged for the
/// follow-up or the wait. A wait records the day it began.
pub fn as_task(text: &str, how: Triage, person: Option<&str>, today: &str) -> String {
    let mut line = format!("- [ ] {}", text.trim());
    if let Some(person) = person {
        line.push_str(&format!(" [[{person}]]"));
    }
    match how {
        Triage::Task => line.push_str(" #task"),
        Triage::Followup => line.push_str(" #followup"),
        Triage::Waiting => line.push_str(&format!(" #{WAITING_TAG} ➕ {today}")),
    }
    line
}

/// A line marked as dealt with by a link to where it went.
pub fn filed(line: &str, link: &str) -> String {
    format!("{} [[{link}]]", line.trim_end())
}

/// A note for one event: what the timeline links to when a line is too short
/// to hold it.
pub fn event_note(date: &str, kind: &str, title: &str, project: &str, people: &[String], body: &str) -> String {
    let mut out = format!("---\ntype: event\nkind: {kind}\ndate: {date}\nproject: \"[[{project}]]\"\n");
    if !people.is_empty() {
        out.push_str("people:\n");
        for p in people {
            out.push_str(&format!("  - \"[[{p}]]\"\n"));
        }
    }
    out.push_str(&format!("---\n# {title}\n\n"));
    if !body.trim().is_empty() {
        out.push_str(body.trim_end());
        out.push('\n');
    }
    out
}

/// Creates the note at `rel` holding `text`, with its folders. Returns false,
/// changing nothing, when a note is there already.
pub fn create_exclusive(vault: &Path, rel: &str, text: &str) -> Result<bool, String> {
    use std::io::Write;
    let abs = vault.join(rel);
    if abs.exists() {
        return Ok(false);
    }
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&abs).map_err(|e| format!("{rel}: {e}"))?;
    file.write_all(text.as_bytes()).map_err(|e| format!("{rel}: {e}"))?;
    Ok(true)
}

/// The note a line links to first, by the name it is written with.
pub fn first_link(text: &str) -> Option<String> {
    let inner = text.split_once("[[")?.1.split_once("]]")?.0;
    let name = inner.split(['|', '#']).next()?.trim();
    let name = name.rsplit('/').next().unwrap_or(name);
    (!name.is_empty()).then(|| name.to_owned())
}

/// `2026-W48`, the ISO week `today` falls in.
pub fn week_label(today: &str) -> Option<String> {
    use chrono::Datelike;
    let week = NaiveDate::parse_from_str(today, "%Y-%m-%d").ok()?.iso_week();
    Some(format!("{}-W{:02}", week.year(), week.week()))
}

/// The date a week of entries ending `today` starts on.
pub fn week_start(today: &str) -> Option<String> {
    let date = NaiveDate::parse_from_str(today, "%Y-%m-%d").ok()?;
    Some((date - chrono::Duration::days(6)).format("%Y-%m-%d").to_string())
}

/// The weekly review note: what was logged, then queries for what is waiting,
/// what is due for a follow-up and which projects have been left alone.
pub fn review_note(label: &str, activity: &[String]) -> String {
    let mut out = format!("---\ntype: review\n---\n# Weekly review {label}\n\n## Logged this week\n\n");
    if activity.is_empty() {
        out.push_str("Nothing was logged.\n");
    }
    for line in activity {
        out.push_str(&format!("- {line}\n"));
    }
    out.push_str("\n## Waiting for\n\n```view\ntype: tasks\nsource: \"task-todo:#waiting\"\ncolumns: [task, file, due]\n```\n");
    out.push_str("\n## Follow-ups\n\n```view\ntype: tasks\nsource: \"task-todo:#followup\"\ncolumns: [task, file, due]\n```\n");
    out.push_str("\n## Projects left alone longest\n\n```view\ntype: table\nsource: \"path:Projects/\"\ncolumns: [file, status, modified]\nsort: modified\nlimit: 10\n```\n");
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_prefix_becomes_the_tag() {
        assert_eq!(split_kind("decision: ship Friday"), ("decision", "ship Friday".to_owned()));
        assert_eq!(split_kind("Meeting:sync"), ("meeting", "sync".to_owned()));
        assert_eq!(split_kind("note: not a kind"), ("info", "note: not a kind".to_owned()));
        assert_eq!(entry("2026-11-26", "email", "from  Anna"), "- 2026-11-26 #email from Anna");
    }

    #[test]
    fn entries_are_read_back() {
        let e = parse_entry("  - 2026-11-26 #Decision Ship on Friday [[Anna]]").unwrap();
        assert_eq!(e, Entry { date: "2026-11-26".into(), kind: "decision".into(), text: "Ship on Friday [[Anna]]".into() });
        assert_eq!(parse_entry("- #meeting sync").unwrap().date, "");
        assert!(parse_entry("- 2026-11-26 nothing tagged").is_none());
        assert_eq!(date_in_name("Daily/2026-11-13.md").as_deref(), Some("2026-11-13"));
        assert_eq!(date_in_name("Projects/Alpha.md"), None);
    }

    #[test]
    fn non_ascii_digits_are_not_a_date_and_do_not_panic() {
        for date in ["２０２６-11-26", "٢٠٢٦-١١-٢٦"] {
            let e = parse_entry(&format!("- {date} #meeting sync")).unwrap();
            assert_eq!((e.date.as_str(), e.text), ("", format!("{date} sync")));
            assert_eq!(date_in_name(&format!("Daily/{date}.md")), None);
            assert_eq!(waiting_days(&format!("- [ ] x ➕ {date}"), "2026-11-26"), None);
        }
    }

    #[test]
    fn waiting_is_counted_in_days() {
        let line = as_task("Contract", Triage::Waiting, Some("Anna"), "2026-11-20");
        assert_eq!(line, "- [ ] Contract [[Anna]] #waiting ➕ 2026-11-20");
        assert_eq!(waiting_days(&line, "2026-11-26"), Some(6));
        assert_eq!(waiting_days("- [ ] x", "2026-11-26"), None);
        assert_eq!(as_task("Call", Triage::Followup, Some("Bo"), "x"), "- [ ] Call [[Bo]] #followup");
        assert_eq!(as_task("Call", Triage::Task, None, "x"), "- [ ] Call #task");
    }

    #[test]
    fn the_inbox_holds_only_what_is_not_sorted() {
        let text = "---\ntitle: d\n---\n# Day\n- call Anna\n- [ ] a task\n- met [[Bo]]\n- tagged #x\nplain line\n\n```\n- in code\n```\n";
        let got: Vec<_> = untriaged(text).into_iter().map(|(_, t)| t).collect();
        assert_eq!(got, ["call Anna", "plain line"]);
        let with_inbox = "- before\n## Inbox\n- one\n- two\n## Other\n- three\n";
        let got: Vec<_> = untriaged(with_inbox).into_iter().map(|(i, t)| (i, t)).collect();
        assert_eq!(got, [(2, "one".to_owned()), (3, "two".to_owned())]);
    }

    #[test]
    fn rewriting_a_line_keeps_the_rest_and_checks_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("d.md"), "a\r\n- one\r\nb").unwrap();
        let new = rewrite_line(dir.path(), "d.md", 1, "- one", "- [ ] one #task", Backend::Native).unwrap();
        assert_eq!(new, "a\r\n- [ ] one #task\r\nb");
        assert!(rewrite_line(dir.path(), "d.md", 1, "- one", "x", Backend::Native).is_err());
    }

    #[test]
    fn an_event_note_links_its_project_and_people() {
        let n = event_note("2026-11-26", "meeting", "Kick-off", "Alpha", &["Anna".into()], "Notes.");
        assert_eq!(n, "---\ntype: event\nkind: meeting\ndate: 2026-11-26\nproject: \"[[Alpha]]\"\npeople:\n  - \"[[Anna]]\"\n---\n# Kick-off\n\nNotes.\n");
    }

    #[test]
    fn weeks_links_and_reviews() {
        assert_eq!(week_label("2026-11-26").as_deref(), Some("2026-W48"));
        assert_eq!(week_start("2026-11-26").as_deref(), Some("2026-11-20"));
        assert_eq!(first_link("met [[Projects/Alpha|the project]] today").as_deref(), Some("Alpha"));
        assert_eq!(first_link("no link"), None);
        let review = review_note("2026-W48", &["2026-11-25 email: budget — [[Daily]]".into()]);
        assert!(review.contains("- 2026-11-25 email: budget") && review.contains("task-todo:#waiting"));
    }

    #[test]
    fn a_note_is_only_created_once() {
        let dir = tempfile::tempdir().unwrap();
        assert!(create_exclusive(dir.path(), "Events/a.md", "x").unwrap());
        assert!(!create_exclusive(dir.path(), "Events/a.md", "y").unwrap());
        assert_eq!(std::fs::read_to_string(dir.path().join("Events/a.md")).unwrap(), "x");
    }
}