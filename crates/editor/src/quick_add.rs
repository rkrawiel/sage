//! Quick Add, after the Obsidian plugin: a note made from a template in one
//! step, and text captured into a note under a heading.
//!
//! Patterns take the template variables (`{{date}}`, `{{time}}`, their
//! formats, `{{title}}`) and `{{value}}`, which is asked for when the entry
//! runs.

use crate::save_guarded;
use crate::template::{self, Config};
use chrono::NaiveDateTime;
use sage_watcher::Backend;
use regex::{NoExpand, Regex};
use std::path::Path;
use std::sync::LazyLock;

static VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\{\{\s*value\s*\}\}").unwrap());

/// Whether running a pattern needs a value typed first.
pub fn needs_value(pattern: &str) -> bool {
    VALUE.is_match(pattern)
}

/// A pattern with its variables filled in; `{{title}}` is the value too.
/// Text in the value is taken as it is, never as variables.
pub fn fill(pattern: &str, value: &str, now: NaiveDateTime, config: &Config) -> String {
    VALUE.replace_all(&template::expand(pattern, value, now, config), NoExpand(value)).into_owned()
}

/// Creates the note at vault path `rel` from the template at vault path
/// `template`, or empty without one: `{{title}}` is the note's name and
/// `{{value}}` what was typed. Returns false, changing nothing, when the
/// note exists already.
pub fn create_note(
    vault: &Path,
    rel: &str,
    template: &str,
    value: &str,
    now: NaiveDateTime,
    config: &Config,
) -> Result<bool, String> {
    let abs = vault.join(rel);
    if abs.exists() {
        return Ok(false);
    }
    let text = if template.is_empty() {
        String::new()
    } else {
        let body = std::fs::read_to_string(vault.join(template)).map_err(|e| format!("{template}: {e}"))?;
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let title = name.strip_suffix(".md").unwrap_or(name);
        VALUE.replace_all(&template::expand(&body, title, now, config), NoExpand(value)).into_owned()
    };
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&abs).map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Adds `insertion` to the note at vault path `rel` with [`capture`], and
/// returns the note's new text. With `create`, a missing note is created.
pub fn capture_into(
    vault: &Path,
    rel: &str,
    insertion: &str,
    heading: &str,
    top: bool,
    create: bool,
    backend: Backend,
) -> Result<String, String> {
    let abs = vault.join(rel);
    let text = match std::fs::read_to_string(&abs) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
            if let Some(parent) = abs.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            String::new()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(format!("{rel} does not exist")),
        Err(e) => return Err(format!("{rel}: {e}")),
    };
    let updated = capture(&text, insertion, heading, top);
    save_guarded(&abs, &text, &updated, backend).map_err(|e| format!("{rel}: {e}"))?;
    Ok(updated)
}

/// `text` with `insertion` added: under the heading named `heading`, at the
/// top of its section or after its last line, or with no heading at the top
/// of the note, after the frontmatter, or at its end. A heading the note
/// lacks is added at the end. Headings compare by letters and digits,
/// ignoring case; `## Inbox` and `Inbox` name the same one.
pub fn capture(text: &str, insertion: &str, heading: &str, top: bool) -> String {
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(body) => ("\u{feff}", body),
        None => ("", text),
    };
    let mut lines: Vec<String> = body.lines().map(str::to_owned).collect();
    let new: Vec<String> = insertion.lines().map(str::to_owned).collect();
    let blank = |lines: &[String], i: usize| lines[i].trim().is_empty();
    // After the last line with text in `from..to`, but not before `from`.
    let after_last = |lines: &[String], from: usize, to: usize| (from..to).rev().find(|&i| !blank(lines, i)).map_or(from, |i| i + 1);

    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let wanted = norm(heading.trim().trim_start_matches('#'));
    let start = sage_core::split_frontmatter(text).2 as usize;
    let at = if wanted.is_empty() {
        Some(if top { start } else { after_last(&lines, start, lines.len()) })
    } else {
        let headings = sage_core::parse(text).headings;
        headings.iter().position(|h| norm(&h.text) == wanted).map(|i| {
            let h = &headings[i];
            let end = headings[i + 1..].iter().find(|n| n.level <= h.level).map_or(lines.len(), |n| n.line as usize);
            if top { h.line as usize + 1 } else { after_last(&lines, h.line as usize + 1, end) }
        })
    };
    match at {
        Some(at) => {
            lines.splice(at..at, new);
        }
        None => {
            let end = after_last(&lines, 0, lines.len());
            lines.truncate(end);
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(format!("## {}", heading.trim().trim_start_matches('#').trim()));
            lines.extend(new);
        }
    }
    // One line ending at the end, without the blank lines a capture would
    // otherwise leave behind its text.
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let mut out = format!("{bom}{}", lines.join(ending));
    out.push_str(ending);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> NaiveDateTime {
        NaiveDateTime::parse_from_str("2026-11-17 09:05:00", "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn config() -> Config {
        Config { folder: "Templates".into(), date_format: "YYYY-MM-DD".into(), time_format: "HH:mm".into() }
    }

    #[test]
    fn patterns() {
        assert!(needs_value("{{date}} {{ VALUE }}"));
        assert!(!needs_value("Daily/{{date}}"));
        assert_eq!(fill("{{date:YYYY-MM-DD}} {{value}}", "Kickoff", now(), &config()), "2026-11-17 Kickoff");
        assert_eq!(fill("Daily/{{date}}", "", now(), &config()), "Daily/2026-11-17");
        assert_eq!(fill("{{value}}", "a {{date}} $1", now(), &config()), "a {{date}} $1", "the value is taken as it is");
    }

    #[test]
    fn creates_notes_from_templates() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        std::fs::create_dir(vault.join("Templates")).unwrap();
        std::fs::write(vault.join("Templates/Meeting.md"), "# {{title}}\nTopic: {{value}}, {{date}}\n").unwrap();

        let made = create_note(vault, "Meetings/2026-11-17 Kickoff.md", "Templates/Meeting.md", "Kickoff", now(), &config());
        assert_eq!(made, Ok(true));
        assert_eq!(
            std::fs::read_to_string(vault.join("Meetings/2026-11-17 Kickoff.md")).unwrap(),
            "# 2026-11-17 Kickoff\nTopic: Kickoff, 2026-11-17\n"
        );
        // An existing note is left as it is.
        std::fs::write(vault.join("Meetings/2026-11-17 Kickoff.md"), "mine").unwrap();
        assert_eq!(create_note(vault, "Meetings/2026-11-17 Kickoff.md", "Templates/Meeting.md", "x", now(), &config()), Ok(false));
        assert_eq!(std::fs::read_to_string(vault.join("Meetings/2026-11-17 Kickoff.md")).unwrap(), "mine");
        assert_eq!(create_note(vault, "Empty.md", "", "", now(), &config()), Ok(true));
        assert!(create_note(vault, "X.md", "Templates/Missing.md", "", now(), &config()).is_err());
    }

    #[test]
    fn captures_under_headings() {
        let note = "---\ntags: [x]\n---\n# Day\n\n## Inbox\n- old\n\n## Log\n- entry\n";
        assert_eq!(
            capture(note, "- new", "Inbox", false),
            "---\ntags: [x]\n---\n# Day\n\n## Inbox\n- old\n- new\n\n## Log\n- entry\n"
        );
        assert_eq!(
            capture(note, "- new", "## inbox", true),
            "---\ntags: [x]\n---\n# Day\n\n## Inbox\n- new\n- old\n\n## Log\n- entry\n"
        );
        assert_eq!(capture(note, "- new", "Log", false), format!("{note}- new\n"));
        assert_eq!(capture(note, "top", "", true), "---\ntags: [x]\n---\ntop\n# Day\n\n## Inbox\n- old\n\n## Log\n- entry\n");
        assert_eq!(capture("a\r\n\r\n", "b", "", false), "a\r\nb\r\n", "after the last line with text, endings kept");
        assert_eq!(capture("# T\n", "- x", "Ideas", false), "# T\n\n## Ideas\n- x\n", "a missing heading is added");
        assert_eq!(capture("", "- x", "", false), "- x\n");
        assert_eq!(capture("# Day\n## Inbox\n", "- x\n- y", "Inbox", false), "# Day\n## Inbox\n- x\n- y\n");
    }

    #[test]
    fn capture_creates_or_refuses_missing_notes() {
        let dir = tempfile::tempdir().unwrap();
        let text = capture_into(dir.path(), "Daily/2026-11-17.md", "- x", "", false, true, Backend::Native).unwrap();
        assert_eq!(text, "- x\n");
        assert_eq!(std::fs::read_to_string(dir.path().join("Daily/2026-11-17.md")).unwrap(), "- x\n");
        assert!(capture_into(dir.path(), "Inbox.md", "- x", "", false, false, Backend::Native).unwrap_err().contains("does not exist"));
    }
}
