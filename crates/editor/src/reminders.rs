//! Reminders: when an open task should raise a notification. A checkbox line
//! carries one in the Obsidian Reminder plugin's format, `(@2026-12-01 09:30)`,
//! and a task document in its `reminder:` property, `2026-12-01 09:30`. A date
//! without a time goes off at the default time from the settings.
//!
//! The app reads the open tasks with reminders from the index after every
//! change, and once a minute asks which went off since it last looked. Those
//! that went off while Sage was closed come back the same way, from the last
//! time it looked, and are told as one summary. Times are local and naive, as
//! the vault writes them.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rusqlite::Connection;

/// The Reminder plugin's own default for a reminder without a time.
pub const DEFAULT_TIME: &str = "09:00";

/// An open task with a reminder, as the index has it.
#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    pub path: String,
    /// The checkbox line, `None` for a task document.
    pub line: Option<u32>,
    /// The task as a list shows it: without its dates and reminder.
    pub text: String,
    /// As written: `YYYY-MM-DD HH:MM` or `YYYY-MM-DD`.
    pub when: String,
}

/// `HH:MM`, also `H:MM`; `None` for anything else.
pub fn parse_time(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()
}

/// When a reminder written as `when` goes off. A property may use `T`
/// between the date and the time, as Obsidian's date-and-time type does.
pub fn at(when: &str, default_time: NaiveTime) -> Option<NaiveDateTime> {
    let when = when.trim();
    let (day, time) = match when.split_once([' ', 'T']) {
        Some((day, time)) => (day, parse_time(time)?),
        None => (when, default_time),
    };
    Some(NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?.and_time(time))
}

/// The reminders that went off after `after` and up to `until`, earliest first.
pub fn between(reminders: &[Reminder], after: NaiveDateTime, until: NaiveDateTime, default_time: NaiveTime) -> Vec<&Reminder> {
    let mut hits: Vec<(NaiveDateTime, &Reminder)> = reminders
        .iter()
        .filter_map(|r| at(&r.when, default_time).map(|t| (t, r)))
        .filter(|(t, _)| *t > after && *t <= until)
        .collect();
    hits.sort_by_key(|(t, _)| *t);
    hits.into_iter().map(|(_, r)| r).collect()
}

/// Open checkbox tasks and open task documents that have a reminder. A
/// document is open unless its status is `done` or `cancelled`.
pub fn load(db: &Connection) -> rusqlite::Result<Vec<Reminder>> {
    let mut stmt = db.prepare_cached(
        "SELECT path, line, text, reminder FROM v_tasks
         WHERE reminder IS NOT NULL AND state NOT IN ('x', 'X', '-')
         UNION ALL
         SELECT path, NULL, title, trim(reminder) FROM v_documents
         WHERE type = 'task' AND reminder IS NOT NULL
           AND (status IS NULL OR status NOT IN ('done', 'cancelled'))",
    )?;
    let rows = stmt.query_map([], |r| {
        let line: Option<u32> = r.get(1)?;
        let text: String = r.get(2)?;
        Ok(Reminder {
            path: r.get(0)?,
            line,
            text: if line.is_some() { sage_core::task_display(&text) } else { text },
            when: r.get(3)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn nine() -> NaiveTime {
        parse_time(DEFAULT_TIME).unwrap()
    }

    fn reminder(text: &str, when: &str) -> Reminder {
        Reminder { path: "n.md".into(), line: Some(0), text: text.into(), when: when.into() }
    }

    #[test]
    fn a_reminder_goes_off_at_its_time_or_the_default() {
        assert_eq!(at("2026-12-01 18:30", nine()), Some(t("2026-12-01 18:30")));
        assert_eq!(at("2026-12-01T07:05", nine()), Some(t("2026-12-01 07:05")));
        assert_eq!(at("2026-12-01", nine()), Some(t("2026-12-01 09:00")));
        assert_eq!(at("2026-12-01", parse_time("7:15").unwrap()), Some(t("2026-12-01 07:15")));
        assert_eq!(at("2026-12-01 noon", nine()), None);
        assert_eq!(at("soon", nine()), None);
    }

    #[test]
    fn the_window_is_open_before_and_closed_after() {
        let list = [
            reminder("late", "2026-12-01 10:00"),
            reminder("edge", "2026-12-01 09:00"),
            reminder("early", "2026-12-01 08:59"),
            reminder("default", "2026-12-01"),
            reminder("broken", "tomorrow"),
        ];
        let hits = between(&list, t("2026-12-01 08:59"), t("2026-12-01 10:00"), nine());
        let names: Vec<&str> = hits.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(names, ["edge", "default", "late"]);
        assert!(between(&list, t("2026-12-01 10:00"), t("2026-12-01 10:01"), nine()).is_empty());
    }

    #[test]
    fn open_tasks_with_reminders_are_read_from_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::write(
            vault.join("A.md"),
            "- [ ] Call Anna 📅 2026-12-01 (@2026-12-01 09:30)\n- [x] Paid (@2026-12-01)\n- [-] Dropped (@2026-12-01)\n- [ ] Plain\n",
        )
        .unwrap();
        std::fs::write(vault.join("T.md"), "---\ntype: task\ntitle: Renew\nstatus: todo\nreminder: 2026-12-02 08:00\n---\n").unwrap();
        std::fs::write(vault.join("D.md"), "---\ntype: task\ntitle: Old\nstatus: done\nreminder: 2026-12-02\n---\n").unwrap();
        let db_path = dir.path().join("index.db");
        let mut ix = sage_indexer::Indexer::open(vault.canonicalize().unwrap(), &db_path).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        drop(ix);
        let db = sage_db::open_reader(&db_path).unwrap();

        let mut list = load(&db).unwrap();
        list.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            list,
            [
                Reminder { path: "A.md".into(), line: Some(0), text: "Call Anna".into(), when: "2026-12-01 09:30".into() },
                Reminder { path: "T.md".into(), line: None, text: "Renew".into(), when: "2026-12-02 08:00".into() },
            ]
        );
    }
}
