//! New tasks, from the new-task form: a checkbox line in the Tasks plugin
//! format for the note being edited, or a task document of its own.

use chrono::{Days, NaiveDate};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

/// The folder task documents are created in.
pub const FOLDER: &str = "Tasks";

/// Reads the due date typed in the form: empty for none, `YYYY-MM-DD`,
/// `today` or `tomorrow`. Errors are sentences for the form.
pub fn parse_due(text: &str, today: NaiveDate) -> Result<Option<String>, String> {
    let text = text.trim();
    let day = match text.to_lowercase().as_str() {
        "" => return Ok(None),
        "today" => today,
        "tomorrow" => today + Days::new(1),
        _ => NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .map_err(|_| format!("`{text}` is not a date: use YYYY-MM-DD, today or tomorrow"))?,
    };
    Ok(Some(day.format("%Y-%m-%d").to_string()))
}

/// The title as one line, or an error for the form when there is none.
fn clean_title(title: &str) -> Result<String, String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return Err("a task needs a title".into());
    }
    Ok(title)
}

/// A checkbox line as the Tasks plugin writes it: `- [ ] Title 📅 2026-11-30`.
pub fn task_line(title: &str, due: Option<&str>) -> Result<String, String> {
    let mut line = format!("- [ ] {}", clean_title(title)?);
    if let Some(due) = due {
        line.push_str(" 📅 ");
        line.push_str(due);
    }
    Ok(line)
}

/// The text of a new task document.
pub fn task_note(id: &str, title: &str, due: Option<&str>, today: &str) -> String {
    // Double quotes keep a title with `:` or `#` a plain string in YAML.
    let quoted = format!("\"{}\"", title.replace('\\', "\\\\").replace('"', "\\\""));
    let due = due.map(|d| format!("due: {d}\n")).unwrap_or_default();
    format!("---\nid: {id}\ntype: task\ntitle: {quoted}\nstatus: todo\ncreated: {today}\n{due}tags: []\n---\n")
}

/// Names Windows keeps for devices, which no file may have.
fn is_device_name(name: &str) -> bool {
    matches!(name, "con" | "prn" | "aux" | "nul")
        || (name.len() == 4
            && (name.starts_with("com") || name.starts_with("lpt"))
            && name.as_bytes()[3].is_ascii_digit()
            && name.as_bytes()[3] != b'0')
}

/// Creates a task document for `title` in [`FOLDER`] and returns its vault
/// path and id. The id is the title as a slug, `fix-voice-reconnect`, and so
/// is the file name, so `[[id]]` finds it here and in Obsidian alike. An id in
/// `taken` (lowercase ids and file names the index knows) or a file that
/// exists gets `-2`, `-3`, … instead; an existing file is never overwritten.
pub fn create_note(
    vault: &Path,
    title: &str,
    due: Option<&str>,
    today: &str,
    taken: &HashSet<String>,
) -> Result<(String, String), String> {
    let title = clean_title(title)?;
    let mut base = sage_core::status_slug(&title);
    if base.is_empty() {
        base = "task".into();
    }
    let dir = vault.join(FOLDER);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{FOLDER}: {e}"))?;
    for n in 1..1000 {
        let id = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        if taken.contains(&id) || is_device_name(&id) {
            continue;
        }
        let rel = format!("{FOLDER}/{id}.md");
        let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(vault.join(&rel)) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{rel}: {e}")),
        };
        file.write_all(task_note(&id, &title, due, today).as_bytes()).map_err(|e| format!("{rel}: {e}"))?;
        return Ok((rel, id));
    }
    Err(format!("no free name for \u{201c}{title}\u{201d} in {FOLDER}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn due_dates_and_lines() {
        let today = day("2026-11-16");
        assert_eq!(parse_due("  ", today), Ok(None));
        assert_eq!(parse_due("Tomorrow", today), Ok(Some("2026-11-17".into())));
        assert_eq!(parse_due("2026-11-30", today), Ok(Some("2026-11-30".into())));
        assert!(parse_due("next week", today).unwrap_err().contains("not a date"));
        assert_eq!(task_line("  Fix   voice\nbug ", Some("2026-11-30")).unwrap(), "- [ ] Fix voice bug 📅 2026-11-30");
        assert_eq!(task_line("Call", None).unwrap(), "- [ ] Call");
        assert!(task_line(" ", None).is_err());

        // The line is a task with that due date to the scanner.
        let task = sage_core::parse(&task_line("Call", Some("2026-11-30")).unwrap()).tasks.remove(0);
        assert_eq!((task.state, task.due.as_deref()), (' ', Some("2026-11-30")));
    }

    #[test]
    fn notes_get_free_ids_and_index_as_task_documents() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let taken = HashSet::from(["fix-voice-bug-2".to_owned()]);
        let make = |title: &str| create_note(vault, title, Some("2026-11-30"), "2026-11-16", &taken).unwrap();

        assert_eq!(make("Fix voice: bug #1"), ("Tasks/fix-voice-bug-1.md".into(), "fix-voice-bug-1".into()));
        assert_eq!(make("Fix voice bug"), ("Tasks/fix-voice-bug.md".into(), "fix-voice-bug".into()));
        assert_eq!(make("Fix voice bug"), ("Tasks/fix-voice-bug-3.md".into(), "fix-voice-bug-3".into()), "-2 is taken in the index");
        assert_eq!(make("Con").1, "con-2", "no device names");
        assert_eq!(make("???").1, "task");

        let text = std::fs::read_to_string(vault.join("Tasks/fix-voice-bug-1.md")).unwrap();
        let note = sage_core::parse(&text);
        assert_eq!(note.id.as_deref(), Some("fix-voice-bug-1"));
        assert_eq!(note.title.as_deref(), Some("Fix voice: bug #1"), "quoted, so YAML keeps it");
        let field = |key: &str| note.frontmatter.iter().find(|e| e.key == key).map(|e| e.value.clone());
        assert_eq!(field("type"), Some(sage_core::FmValue::Text("task".into())));
        assert_eq!(field("status"), Some(sage_core::FmValue::Text("todo".into())));
        assert_eq!(field("due"), Some(sage_core::FmValue::Text("2026-11-30".into())));
        assert_eq!(field("created"), Some(sage_core::FmValue::Text("2026-11-16".into())));
    }
}
