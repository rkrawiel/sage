//! The new-task form: Ctrl+T creates a task document in `Tasks/` and links it
//! at the cursor, Ctrl+Shift+T adds a checkbox line in the Tasks plugin format
//! at the cursor. Both ask for a title, an optional due date and an optional
//! reminder, written in the Reminder plugin's `(@2026-12-01 09:30)` format.

use crate::theme::{self, Tokens};
use crate::{App, Message, EDITOR};
use sage_editor::new_task;
use iced::widget::{button, column, container, row, space, text, text_input};
use iced::{Element, Fill, Task};
use std::collections::HashSet;

pub const TITLE_INPUT: &str = "task-title-input";
pub const DUE_INPUT: &str = "task-due-input";
pub const REMIND_INPUT: &str = "task-remind-input";
/// The form's fields in the order Tab visits them.
const FIELDS: [&str; 3] = [TITLE_INPUT, DUE_INPUT, REMIND_INPUT];

impl crate::Tab {
    /// Puts a task line on the cursor's line when that is blank, else on a new
    /// line below it, indented like it when it is a list item; never into the
    /// frontmatter. The cursor ends after the task.
    pub(crate) fn insert_task_line(&mut self, line: &str) {
        let fm_lines = sage_core::split_frontmatter(&self.content.text()).2 as usize;
        let last = self.content.line_count().saturating_sub(1);
        let row = self.content.cursor().position.line.max(fm_lines).min(last);
        let current = self.content.line(row).map(|l| l.text.into_owned()).unwrap_or_default();
        let trimmed = current.trim_start();
        let (at, text) = if trimmed.is_empty() {
            (row, line.to_owned())
        } else {
            let list_item = ["- ", "* ", "+ "].iter().any(|m| trimmed.starts_with(m));
            let indent = if list_item { &current[..current.len() - trimmed.len()] } else { "" };
            (row + 1, format!("{current}{}{indent}{line}", self.ending()))
        };
        self.replace_line(row, &text);
        let column = self.content.line(at).map_or(0, |l| l.text.len());
        self.put_cursor(at, column);
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskForm {
    /// A task document of its own, rather than a line in the note.
    pub note: bool,
    /// The tab whose note the task or its link goes into, at its cursor, by
    /// `Tab::id`.
    pub target: Option<u64>,
    pub title: String,
    pub due: String,
    pub remind: String,
    pub error: Option<String>,
    /// The field Tab last moved to, an index into `FIELDS`.
    pub field: usize,
}

impl App {
    /// Opens the form. A line needs a note to go into; a task note does not,
    /// and without one it opens once created.
    pub(crate) fn open_task_form(&mut self, note: bool) -> Task<Message> {
        let target = self.tabs.get(self.active).filter(|t| t.loaded).map(|t| t.id);
        if target.is_none() && !note {
            self.notice("Open a note to add a task line to; Ctrl+T makes a task note".into());
            return Task::none();
        }
        self.palette = None;
        self.completion = None;
        self.task_form = Some(TaskForm { note, target, ..TaskForm::default() });
        iced::widget::operation::focus(TITLE_INPUT)
    }

    /// Tab moves to the next field, from the last back to the first.
    pub(crate) fn task_form_tab(&mut self) -> Task<Message> {
        let Some(form) = &mut self.task_form else { return Task::none() };
        form.field = (form.field + 1) % FIELDS.len();
        iced::widget::operation::focus(FIELDS[form.field])
    }

    pub(crate) fn submit_task_form(&mut self) -> Task<Message> {
        let Some(form) = &mut self.task_form else { return Task::none() };
        let now = sage_editor::now();
        let due = match new_task::parse_due(&form.due, now.date()) {
            Ok(due) => due,
            Err(error) => {
                form.error = Some(error);
                return Task::none();
            }
        };
        let remind = match new_task::parse_reminder(&form.remind, due.as_deref(), now.date()) {
            Ok(remind) => remind,
            Err(error) => {
                form.error = Some(error);
                return Task::none();
            }
        };
        if !form.note {
            return match new_task::task_line(&form.title, due.as_deref(), remind.as_deref()) {
                Ok(line) => {
                    let form = self.task_form.take().unwrap_or_default();
                    self.insert_task_line(form.target, &line)
                }
                Err(error) => {
                    form.error = Some(error);
                    Task::none()
                }
            };
        }
        if form.title.trim().is_empty() {
            form.error = Some("a task needs a title".into());
            return Task::none();
        }
        let form = self.task_form.take().unwrap_or_default();
        let taken = self.taken_ids();
        let vault = self.vault.clone();
        let today = now.date().format("%Y-%m-%d").to_string();
        let tab = form.target;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || new_task::create_note(&vault, &form.title, due.as_deref(), remind.as_deref(), &today, &taken))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::TaskNoteCreated { tab, result },
        )
    }

    /// Ids and file names the index knows, lowercase, which a new task's id
    /// must not repeat.
    fn taken_ids(&self) -> HashSet<String> {
        let Some(reader) = &self.reader else { return HashSet::new() };
        let Ok(mut stmt) = reader.prepare("SELECT id_key FROM files WHERE id_key IS NOT NULL UNION SELECT name_key FROM files")
        else {
            return HashSet::new();
        };
        stmt.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// The task note exists: add a checkbox linking it at the cursor of the
    /// note the form was opened from, or open it when there was none.
    pub(crate) fn on_task_note_created(&mut self, tab: Option<u64>, result: Result<(String, String), String>) -> Task<Message> {
        let (path, id) = match result {
            Ok(created) => created,
            Err(error) => {
                self.notice(format!("No task note created: {error}"));
                return Task::none();
            }
        };
        self.notice(format!("Created {path}"));
        match tab.and_then(|tab| self.tabs.iter().position(|t| t.id == tab && t.loaded)) {
            // Resolving the merge would replace the text, and the link with it.
            Some(i) if self.tabs[i].merge.is_some() => {
                self.notice(format!("Created {path}; not linked while {} has changes to resolve", self.tabs[i].path));
                Task::none()
            }
            Some(i) => {
                self.tabs[i].insert_task_line(&format!("- [ ] [[{id}]]"));
                self.focus_editor(i)
            }
            None => self.open(path),
        }
    }

    /// Puts a task line into the note in tab `target`; see [`crate::Tab::insert_task_line`].
    fn insert_task_line(&mut self, target: Option<u64>, line: &str) -> Task<Message> {
        let Some(i) = self.tabs.iter().position(|t| Some(t.id) == target && t.loaded) else {
            self.notice("The note is not open any more; no task added".into());
            return Task::none();
        };
        // Resolving the merge would replace the text, and the task with it.
        if self.tabs[i].merge.is_some() {
            self.notice(format!("Resolve the changes to {} before adding a task", self.tabs[i].path));
            return Task::none();
        }
        self.tabs[i].insert_task_line(line);
        self.focus_editor(i)
    }

    fn focus_editor(&mut self, i: usize) -> Task<Message> {
        if i != self.active {
            return Task::none();
        }
        self.refresh_find(false);
        iced::widget::operation::focus(EDITOR)
    }

    pub(crate) fn task_form_overlay<'a>(&'a self, t: Tokens, form: &'a TaskForm) -> Element<'a, Message> {
        let heading = if form.note { "New task note" } else { "New task line" };
        let detail = match (&form.target, form.note) {
            (_, false) => "Added at the cursor as a task line.".to_owned(),
            (Some(_), true) => format!("Created in {}/ and added at the cursor as a checkbox; checking it closes the note.", new_task::FOLDER),
            (None, true) => format!("Created in {}/ and opened.", new_task::FOLDER),
        };
        let field = |label: &'a str, id: &'static str, placeholder: &'a str, value: &'a str, on_input: fn(String) -> Message| {
            column![
                text(label).size(12).color(t.text_2),
                text_input(placeholder, value)
                    .id(id)
                    .on_input(on_input)
                    .on_submit(Message::TaskFormSubmit)
                    .size(15)
                    .padding(8)
                    .style(move |_, s| theme::input(t, s)),
            ]
            .spacing(4)
        };
        let mut body = column![
            text(heading).size(20).font(theme::SERIF).color(t.text),
            text(detail).size(13).color(t.text_2),
            field("Title", TITLE_INPUT, "What needs doing", &form.title, Message::TaskFormTitle),
            field("Due", DUE_INPUT, "YYYY-MM-DD, today or tomorrow; empty for none", &form.due, Message::TaskFormDue),
            field("Remind at", REMIND_INPUT, "9:30, tomorrow 9:30 or YYYY-MM-DD HH:MM; empty for none", &form.remind, Message::TaskFormRemind),
        ]
        .spacing(12);
        if let Some(error) = &form.error {
            body = body.push(text(error).size(13).color(t.error));
        }
        let choice = |label: &'a str, current: bool, message: Message| {
            button(text(label).size(14))
                .padding([6, 14])
                .style(move |_, status| theme::nav_button(t, current, status))
                .on_press(message)
        };
        body = body.push(
            row![
                text("Enter creates · Tab moves · Esc cancels").size(12).color(t.text_2),
                space().width(Fill),
                choice("Cancel", false, Message::Key(crate::Shortcut::Escape)),
                choice("Create", true, Message::TaskFormSubmit),
            ]
            .spacing(8)
            .align_y(iced::Center),
        );
        let panel = container(body).padding(18).width(480).style(move |_| theme::panel(t));
        iced::widget::opaque(
            container(panel)
                .width(Fill)
                .height(Fill)
                .align_x(iced::Center)
                .padding(iced::Padding { top: 120.0, ..iced::Padding::ZERO })
                .style(move |_| theme::scrim(t)),
        )
    }
}

#[cfg(test)]
mod tests {
    fn tab(text: &str, line: usize, column: usize) -> crate::Tab {
        let mut tab = crate::Tab::placeholder("n.md".into(), "n".into());
        tab.set_text(text.into());
        tab.put_cursor(line, column);
        tab
    }

    #[test]
    fn task_lines_go_below_the_cursor_line_and_never_into_frontmatter() {
        let line = "- [ ] Call 📅 2026-11-30";
        let cases = [
            ("# Plan
Intro
", 1, 2, "# Plan
Intro
- [ ] Call 📅 2026-11-30
"),
            ("# Plan

End
", 1, 0, "# Plan
- [ ] Call 📅 2026-11-30
End
"),
            ("  - [ ] Other
", 0, 3, "  - [ ] Other
  - [ ] Call 📅 2026-11-30
"),
            ("---
title: x
---
Body", 1, 0, "---
title: x
---
Body
- [ ] Call 📅 2026-11-30"),
        ];
        for (text, row, column, expected) in cases {
            let mut tab = tab(text, row, column);
            tab.insert_task_line(line);
            assert_eq!(tab.content.text(), expected, "{text:?}");
            assert!(tab.dirty, "an edit autosave writes");
            let cursor = tab.content.cursor().position;
            let at = tab.content.line(cursor.line).unwrap().text.into_owned();
            assert!(at.ends_with(line), "{at:?}");
            assert_eq!(cursor.column, at.len(), "the cursor ends after the task");
        }
    }
}
