//! The vault-wide task list: open tasks by due date, and what was done today.
//!
//! Read from `v_tasks` and `v_documents`: checkbox lines and task documents
//! (notes with `type: task`). Checking a line rewrites it on disk through
//! `sage_editor::tasks`, which refuses when the line no longer matches;
//! checking a document sets its `status` the same guarded way.

use crate::theme::{self, Tokens};
use crate::{file_stem, plain_links, Message, TaskRef};
use iced::widget::{button, column, container, row, scrollable, space, text, Column};
use iced::{Element, Fill};
use rusqlite::Connection;
use sage_core::TaskStatus;
use std::collections::HashMap;

/// Rows beyond this are not shown; a vault with more open tasks needs filters.
const LIMIT: usize = 1000;

#[derive(Default)]
pub struct TaskList {
    pub rows: Vec<TaskRow>,
    /// `YYYY-MM-DD`, compared with due dates as text.
    pub today: String,
}

pub struct TaskRow {
    pub path: String,
    /// 0 for a document.
    pub line: u32,
    /// The source line, which write-back checks before changing it. Empty
    /// for a document, whose write-back checks `status` instead.
    pub raw: String,
    /// A line's text, or a document's title and tags.
    pub text: String,
    pub due: Option<String>,
    pub done: bool,
    /// A task document rather than a checkbox line.
    pub document: bool,
    /// A document's `status` as the index read it, lowercase with words
    /// joined by `-`. `None` for a line, whose status is in its text.
    pub status: Option<String>,
}

/// Open tasks, soonest due first, then the ones completed today. A document
/// is open unless its status is `done` or `cancelled`, and was completed
/// today when it is `done` and the note changed today, as documents have no
/// completion date. Cancelled tasks (`[-]`, `status: cancelled`) are neither.
pub fn load(db: &Connection, today: &str) -> rusqlite::Result<Vec<TaskRow>> {
    let mut stmt = db.prepare_cached(
        "SELECT path, line, raw, text, due, done, document, status, tags FROM (
             SELECT path, line, raw, text, due, state IN ('x', 'X') AS done, priority,
                    0 AS document, NULL AS status, NULL AS tags
             FROM v_tasks
             WHERE state NOT IN ('x', 'X', '-') OR done_on = ?1
             UNION ALL
             SELECT d.path, 0, '', d.title, d.due, d.status IS 'done', NULL, 1, d.status, d.tags
             FROM v_documents d JOIN v_notes n ON n.id = d.id
             WHERE d.type = 'task'
               AND (d.status IS NULL OR d.status NOT IN ('done', 'cancelled')
                    OR (d.status = 'done' AND date(n.modified, 'unixepoch', 'localtime') = ?1)))
         ORDER BY due IS NULL, due, priority DESC, path COLLATE NOCASE, line
         LIMIT ?2",
    )?;
    let rows = stmt.query_map((today, LIMIT as i64), |r| {
        let text: String = r.get(3)?;
        let document: bool = r.get(6)?;
        let tags: Option<String> = r.get(8)?;
        Ok(TaskRow {
            path: r.get(0)?,
            line: r.get(1)?,
            raw: r.get(2)?,
            text: if document {
                // The tags a line shows in its text.
                let tags = tags.iter().flat_map(|t| t.split(',')).map(|t| format!(" #{t}"));
                std::iter::once(text).chain(tags).collect()
            } else {
                plain_links(&sage_core::task_display(&text))
            },
            due: r.get(4)?,
            done: r.get(5)?,
            document,
            status: r.get(7)?,
        })
    })?;
    rows.collect()
}

/// What a row's checkbox writes to.
pub fn task_ref(row: &TaskRow) -> TaskRef {
    if row.document {
        let status = row.status.clone().unwrap_or_default();
        TaskRef { path: row.path.clone(), line: 0, raw: status, document: true }
    } else {
        TaskRef { path: row.path.clone(), line: row.line, raw: row.raw.clone(), document: false }
    }
}

/// The status a task document gets: toggled by its checkbox when `column` is
/// `None`, else moved to a Kanban column, `(slug, last)`. Checking it keeps
/// the status it had in `reopen`, which unchecking it restores.
pub fn document_status(
    reopen: &mut HashMap<String, String>,
    path: &str,
    status: Option<&str>,
    column: Option<&(Option<String>, bool)>,
) -> String {
    let done = TaskStatus::Done.slug();
    let new = match column {
        None => sage_editor::tasks::toggled_status(status, reopen.get(path).map(String::as_str)),
        Some((_, true)) => done.to_owned(),
        Some((slug, false)) => slug.clone().unwrap_or_else(|| TaskStatus::Todo.slug().to_owned()),
    };
    match status.filter(|s| *s != done) {
        Some(old) if new == done => {
            reopen.insert(path.to_owned(), old.to_owned());
        }
        _ if new != done => {
            reopen.remove(path);
        }
        _ => {}
    }
    new
}

/// Updates the rows of the task document at `path` after the app set its
/// status, so the list shows it before the index has caught up.
pub fn patch_document(list: &mut TaskList, path: &str, status: Option<&str>) {
    for row in list.rows.iter_mut().filter(|r| r.document && r.path == path) {
        row.status = status.map(str::to_owned);
        row.done = status == Some(TaskStatus::Done.slug());
    }
}

const SECTIONS: [&str; 5] = ["Overdue", "Today", "Upcoming", "No due date", "Done today"];

fn section(row: &TaskRow, today: &str) -> usize {
    match row.due.as_deref() {
        _ if row.done => 4,
        Some(due) if due < today => 0,
        Some(due) if due == today => 1,
        Some(_) => 2,
        None => 3,
    }
}

pub fn view(list: &TaskList, t: Tokens) -> Element<'_, Message> {
    let mut page = Column::new().spacing(2).padding([28, 36]).max_width(900);
    page = page.push(
        column![
            text("Tasks").size(26).font(theme::SERIF).color(t.text),
            text("Check a task to complete it in its note. Click the text to open the note there.")
                .size(13)
                .color(t.text_2),
        ]
        .spacing(4),
    );
    if list.rows.is_empty() {
        page = page.push(container(text("No open tasks").size(15).color(t.text_2)).padding([16, 0]));
    }

    for (n, title) in SECTIONS.iter().enumerate() {
        let rows: Vec<_> = list.rows.iter().enumerate().filter(|(_, r)| section(r, &list.today) == n).collect();
        if rows.is_empty() {
            continue;
        }
        page = page.push(
            container(text(format!("{title} · {}", rows.len())).size(12).font(theme::SANS_BOLD).color(t.text_2))
                .padding(iced::Padding { top: 20.0, bottom: 4.0, ..iced::Padding::ZERO }),
        );
        for (i, task) in rows {
            page = page.push(row_view(i, task, n == 0, t));
        }
    }
    if list.rows.len() == LIMIT {
        page = page.push(
            container(text(format!("Showing the first {LIMIT} tasks")).size(12).color(t.text_2)).padding([12, 0]),
        );
    }
    scrollable(container(page).width(Fill)).height(Fill).into()
}

fn row_view(i: usize, task: &TaskRow, overdue: bool, t: Tokens) -> Element<'_, Message> {
    let done = task.done;
    // A document's open and done show in the box, other statuses in words
    // before its title, as a task link reads: `[In Progress] Title`.
    let label = task.status.as_deref().filter(|_| task.document).and_then(|status| match TaskStatus::parse(status) {
        Some(TaskStatus::Todo | TaskStatus::Done) => None,
        Some(known) => Some(known.label().to_owned()),
        None => Some(status.to_owned()),
    });
    let shown = match label {
        Some(label) => format!("[{label}] {}", task.text),
        None => task.text.clone(),
    };
    let open = if task.document {
        Message::Open(task.path.clone())
    } else {
        Message::OpenAt(task.path.clone(), task.line as usize)
    };
    row![
        button(container(space()).width(13).height(13).style(move |_| theme::checkbox(t, done)))
            .padding([6, 6])
            .style(move |_, status| theme::nav_button(t, false, status))
            .on_press(Message::ToggleTask(i)),
        button(text(shown).size(15).color(if done { t.text_2 } else { t.text }))
            .width(Fill)
            .padding([4, 6])
            .style(move |_, status| theme::nav_button(t, false, status))
            .on_press(open),
        text(file_stem(&task.path)).size(12).color(t.text_2).wrapping(text::Wrapping::None),
        text(task.due.as_deref().unwrap_or_default())
            .size(12)
            .color(if overdue { t.error } else { t.text_2 })
            .width(76),
    ]
    .spacing(8)
    .align_y(iced::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed(files: &[(&str, &str)]) -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        for (rel, text) in files {
            let path = vault.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let db_path = dir.path().join("index.db");
        let mut ix = sage_indexer::Indexer::open(vault.canonicalize().unwrap(), &db_path).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        drop(ix);
        let db = sage_db::open_reader(&db_path).unwrap();
        (dir, db)
    }

    fn doc(title: &str, status: Option<&str>, due: Option<&str>) -> String {
        let status = status.map(|s| format!("status: {s}\n")).unwrap_or_default();
        let due = due.map(|d| format!("due: {d}\n")).unwrap_or_default();
        format!("---\ntype: task\ntitle: {title}\n{status}{due}tags: [backend, voice]\n---\nBody\n")
    }

    #[test]
    fn documents_are_in_the_task_list() {
        let today = sage_editor::today();
        let (_dir, db) = indexed(&[
            ("Tasks/a.md", &doc("Fix reconnect", Some("in-progress"), Some("2026-11-20"))),
            ("Tasks/b.md", &doc("No status", None, None)),
            // Written just now, so done today.
            ("Tasks/c.md", &doc("Shipped", Some("done"), Some("2026-11-01"))),
            ("Tasks/d.md", &doc("Dropped", Some("cancelled"), None)),
            ("Notes/p.md", "---\ntype: project\ntitle: Project\nstatus: todo\n---\n"),
            ("Daily.md", "- [ ] Call vendor 📅 2026-11-19\n- [-] Cancelled line\n"),
        ]);
        let rows = load(&db, &today).unwrap();
        let shown: Vec<(&str, bool, bool)> = rows.iter().map(|r| (r.text.as_str(), r.document, r.done)).collect();
        assert_eq!(
            shown,
            [
                ("Shipped #backend #voice", true, true),
                ("Call vendor", false, false),
                ("Fix reconnect #backend #voice", true, false),
                ("No status #backend #voice", true, false),
            ]
        );
        let a = &rows[2];
        assert_eq!((a.path.as_str(), a.status.as_deref(), a.due.as_deref()), ("Tasks/a.md", Some("in-progress"), Some("2026-11-20")));
        assert_eq!(section(&rows[0], &today), 4, "done today");

        // What the checkbox writes to: the document's status as indexed.
        let r = task_ref(a);
        assert_eq!((r.line, r.raw.as_str(), r.document), (0, "in-progress", true));
        assert_eq!(task_ref(&rows[3]).raw, "", "no status yet");
        assert!(!task_ref(&rows[1]).document);

        // Done a day later: no longer listed.
        assert_eq!(load(&db, "2999-01-01").unwrap().iter().filter(|r| r.done).count(), 0);
    }

    #[test]
    fn a_document_checkbox_remembers_the_status_to_reopen() {
        let mut reopen = HashMap::new();
        let toggle = |reopen: &mut HashMap<String, String>, status| document_status(reopen, "t.md", status, None);
        assert_eq!(toggle(&mut reopen, Some("in-progress")), "done");
        assert_eq!(toggle(&mut reopen, Some("done")), "in-progress");
        assert!(reopen.is_empty());
        assert_eq!(toggle(&mut reopen, Some("done")), "todo", "nothing remembered");
        assert_eq!(toggle(&mut reopen, None), "done");
        assert_eq!(toggle(&mut reopen, Some("done")), "todo");

        // Kanban moves: the last column is done, others their slug.
        assert_eq!(document_status(&mut reopen, "t.md", Some("todo"), Some(&(None, true))), "done");
        assert_eq!(reopen["t.md"], "todo");
        assert_eq!(document_status(&mut reopen, "t.md", Some("done"), Some(&(Some("doing".into()), false))), "doing");
        assert!(reopen.is_empty());

        let mut list = TaskList {
            rows: vec![TaskRow {
                path: "t.md".into(),
                line: 0,
                raw: String::new(),
                text: "T".into(),
                due: None,
                done: false,
                document: true,
                status: Some("todo".into()),
            }],
            today: String::new(),
        };
        patch_document(&mut list, "t.md", Some("done"));
        assert!(list.rows[0].done);
        assert_eq!(list.rows[0].status.as_deref(), Some("done"));
    }
}
