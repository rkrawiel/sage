//! New tasks, from the new-task form: a checkbox line in the Tasks plugin
//! format for the note being edited, or a task document of its own.

use chrono::{Days, NaiveDate, NaiveTime};
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

/// Reads the reminder typed in the form: empty for none, a time (`9:30`)
/// on the due date or else today, a day (`YYYY-MM-DD`, `today`, `tomorrow`)
/// for the default time, or a day and a time. Returns it as the Reminder
/// plugin writes it inside `(@…)`: `2026-12-01 09:30` or `2026-12-01`.
pub fn parse_reminder(text: &str, due: Option<&str>, today: NaiveDate) -> Result<Option<String>, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let error = || format!("`{}` is not a reminder: use 9:30, tomorrow 9:30 or YYYY-MM-DD HH:MM", text.trim());
    let time = |word: &str| NaiveTime::parse_from_str(word, "%H:%M").ok();
    let day = |word: &str| parse_due(word, today).ok().flatten().ok_or_else(error);
    let (day, time) = match words.as_slice() {
        [] => return Ok(None),
        [word] => match time(word) {
            Some(time) => (due.map_or_else(|| today.format("%Y-%m-%d").to_string(), str::to_owned), Some(time)),
            None => (day(word)?, None),
        },
        [on, at] => (day(on)?, Some(time(at).ok_or_else(error)?)),
        _ => return Err(error()),
    };
    Ok(Some(match time {
        Some(time) => format!("{day} {}", time.format("%H:%M")),
        None => day,
    }))
}

/// The title as one line, or an error for the form when there is none.
fn clean_title(title: &str) -> Result<String, String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return Err("a task needs a title".into());
    }
    Ok(title)
}

/// A checkbox line as the Tasks plugin writes it, `- [ ] Title 📅 2026-11-30`,
/// with a reminder as the Reminder plugin writes it, `(@2026-11-30 09:30)`.
pub fn task_line(title: &str, due: Option<&str>, remind: Option<&str>) -> Result<String, String> {
    let mut line = format!("- [ ] {} #task", clean_title(title)?);
    if let Some(due) = due {
        line.push_str(" 📅 ");
        line.push_str(due);
    }
    if let Some(remind) = remind {
        line.push_str(&format!(" (@{remind})"));
    }
    Ok(line)
}

/// The text of a new task document.
pub fn task_note(id: &str, title: &str, due: Option<&str>, remind: Option<&str>, today: &str) -> String {
    // Double quotes keep a title with `:` or `#` a plain string in YAML.
    let quoted = format!("\"{}\"", title.replace('\\', "\\\\").replace('"', "\\\""));
    let due = due.map(|d| format!("due: {d}\n")).unwrap_or_default();
    let remind = remind.map(|r| format!("reminder: {r}\n")).unwrap_or_default();
    format!("---\nid: {id}\ntype: task\ntitle: {quoted}\nstatus: todo\ncreated: {today}\n{due}{remind}tags: []\n---\n")
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
    remind: Option<&str>,
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
        file.write_all(task_note(&id, &title, due, remind, today).as_bytes()).map_err(|e| format!("{rel}: {e}"))?;
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
        assert_eq!(task_line("  Fix   voice\nbug ", Some("2026-11-30"), None).unwrap(), "- [ ] Fix voice bug #task 📅 2026-11-30");
        assert_eq!(task_line("Call", None, None).unwrap(), "- [ ] Call #task");
        assert!(task_line(" ", None, None).is_err());

        // The line is a task with that due date and reminder to the scanner.
        let line = task_line("Call", Some("2026-11-30"), Some("2026-11-30 09:30")).unwrap();
        assert_eq!(line, "- [ ] Call #task 📅 2026-11-30 (@2026-11-30 09:30)");
        let task = sage_core::parse(&line).tasks.remove(0);
        assert_eq!((task.state, task.due.as_deref()), (' ', Some("2026-11-30")));
        assert_eq!(task.reminder.as_deref(), Some("2026-11-30 09:30"));
        assert!(task_note("t", "T", None, Some("2026-11-30 09:30"), "2026-11-16").contains("\nreminder: 2026-11-30 09:30\n"));
    }

    #[test]
    fn reminders_typed_in_the_form() {
        let today = day("2026-11-16");
        let read = |text: &str, due: Option<&str>| parse_reminder(text, due, today);
        assert_eq!(read(" ", None), Ok(None));
        assert_eq!(read("9:30", None), Ok(Some("2026-11-16 09:30".into())), "a bare time is today");
        assert_eq!(read("9:30", Some("2026-11-30")), Ok(Some("2026-11-30 09:30".into())), "or on the due date");
        assert_eq!(read("tomorrow 18:05", None), Ok(Some("2026-11-17 18:05".into())));
        assert_eq!(read("2026-12-01", None), Ok(Some("2026-12-01".into())), "a day alone takes the default time");
        assert_eq!(read("2026-12-01 07:00", Some("2026-11-30")), Ok(Some("2026-12-01 07:00".into())));
        for bad in ["25:00", "soon", "tomorrow noon", "a b c"] {
            assert!(read(bad, None).unwrap_err().contains("not a reminder"), "{bad}");
        }
    }

    #[test]
    fn notes_get_free_ids_and_index_as_task_documents() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let taken = HashSet::from(["fix-voice-bug-2".to_owned()]);
        let make = |title: &str| create_note(vault, title, Some("2026-11-30"), None, "2026-11-16", &taken).unwrap();

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
