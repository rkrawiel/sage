//! The palette flows of the project log, the daily-note triage, imports and
//! the weekly review: which step comes after which, and the writes at the end.
//!
//! The text logic is in `sage_editor::log` and `sage_editor::import`. Every
//! write goes through a guarded save in a blocking task and comes back as a
//! `Message::Written`; a note with unsaved edits in a tab is never written.

use super::*;
use sage_editor::log::{self, Triage};
use sage_editor::{import, quick_add};

/// Where imported and promoted events are made.
const EVENTS: &str = "Events";

/// A loose line of a note, as it is on disk.
#[derive(Debug, Clone)]
pub struct Line {
    pub path: String,
    /// 0-based, from the top of the file.
    pub line: usize,
    /// The whole line, which a write checks before changing it.
    pub raw: String,
    /// The line without its list marker.
    pub text: String,
}

/// What a chosen project is for.
#[derive(Debug, Clone)]
pub enum Then {
    /// A new entry typed next.
    Log,
    /// Text from the clipboard to make an event of.
    Import(String),
    /// A loose line to file, which becomes the entry.
    File(Line),
}

#[derive(Debug, Clone)]
pub enum Step {
    /// A project (by path) or a person (by name) chosen.
    Pick(String),
    /// Write the entry typed.
    Log,
    Line(Line),
    File(Line),
    /// Ask whom the line is for, before making it a task.
    Ask(Line, Triage),
    Make(Line, Triage),
}

fn hint(label: &str) -> Vec<Hit> {
    vec![Hit { label: label.to_owned(), detail: String::new(), action: HitAction::Nothing }]
}

impl App {
    /// The notes that are projects or people, as `(name, path)`, lately
    /// changed first: those in a folder of that name or with that `type`.
    fn kind_notes(&self, project: bool) -> Vec<(String, String)> {
        let Some(reader) = &self.reader else { return Vec::new() };
        let (folder, kind) = if project { ("projects", "project") } else { ("people", "person") };
        let sql = format!(
            "SELECT f.path FROM files f WHERE f.ext = 'md'
               AND (lower(f.parent) = '{folder}' OR lower(f.parent) LIKE '%/{folder}'
                    OR EXISTS (SELECT 1 FROM frontmatter p WHERE p.file_id = f.id AND p.key = 'type' AND lower(trim(p.value)) = '{kind}'))
             ORDER BY f.mtime_ns DESC LIMIT 500"
        );
        let Ok(mut stmt) = reader.prepare_cached(&sql) else { return Vec::new() };
        let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else { return Vec::new() };
        rows.flatten().map(|path| (file_stem(&path).to_owned(), path)).collect()
    }

    /// The palette's hits while it is in one of the flow modes, else `None`.
    pub(super) fn flow_hits(&self) -> Option<Vec<Hit>> {
        let palette = self.palette.as_ref()?;
        let filter = palette.query.trim().to_lowercase();
        let wanted = |s: &str| filter.is_empty() || s.to_lowercase().contains(&filter);
        let hits = match &palette.mode {
            PaletteMode::Project(_) => {
                let hits: Vec<Hit> = palette
                    .kind_notes
                    .iter()
                    .filter(|(name, _)| wanted(name))
                    .map(|(name, path)| Hit { label: name.clone(), detail: path.clone(), action: HitAction::Flow(Step::Pick(path.clone())) })
                    .collect();
                if hits.is_empty() { hint("No project notes: give a note `type: project` or keep it in a Projects folder") } else { hits }
            }
            PaletteMode::Person(..) => {
                let hits: Vec<Hit> = palette
                    .kind_notes
                    .iter()
                    .filter(|(name, _)| wanted(name))
                    .map(|(name, path)| Hit { label: name.clone(), detail: path.clone(), action: HitAction::Flow(Step::Pick(name.clone())) })
                    .collect();
                if hits.is_empty() { hint("No people notes: give a note `type: person` or keep it in a People folder") } else { hits }
            }
            PaletteMode::LogEntry { project, .. } => {
                let (kind, text) = log::split_kind(&palette.query);
                if text.is_empty() {
                    hint("Type what happened, for instance: decision: ship on Friday")
                } else {
                    vec![Hit {
                        label: format!("Log to {}: {kind} · {text}", file_stem(project)),
                        detail: String::new(),
                        action: HitAction::Flow(Step::Log),
                    }]
                }
            }
            PaletteMode::Triage(lines) => lines
                .iter()
                .filter(|l| wanted(&l.text))
                .map(|l| Hit { label: l.text.clone(), detail: format!("line {}", l.line + 1), action: HitAction::Flow(Step::Line(l.clone())) })
                .collect(),
            PaletteMode::TriageAction(line) => {
                let choice = |label: &str, detail: &str, step| Hit { label: label.to_owned(), detail: detail.to_owned(), action: HitAction::Flow(step) };
                vec![
                    choice("File to a project", "adds it to the project's log", Step::File(line.clone())),
                    choice("Make a task", "tracked, with #task", Step::Make(line.clone(), Triage::Task)),
                    choice("Follow up with a person", "#followup, shown on their note", Step::Ask(line.clone(), Triage::Followup)),
                    choice("Waiting for a person", "#waiting, with the day it began", Step::Ask(line.clone(), Triage::Waiting)),
                ]
            }
            _ => return None,
        };
        Some(hits)
    }

    /// Opens the palette in a flow mode with `query` typed already. A choice
    /// of project or person reads the notes to choose from here, once: their
    /// query cannot use an index, and every key filters the list again.
    fn open_flow(&mut self, mode: PaletteMode, query: String) -> Task<Message> {
        self.completion = None;
        let kind_notes = match &mode {
            PaletteMode::Project(_) => self.kind_notes(true),
            PaletteMode::Person(..) => self.kind_notes(false),
            _ => Vec::new(),
        };
        self.palette = Some(Palette { mode, query, kind_notes, ..Palette::default() });
        self.search();
        iced::widget::operation::focus(PALETTE_INPUT)
    }

    /// True, with a notice, when `path` is open with changes that are not saved.
    fn busy_note(&mut self, path: &str) -> bool {
        let busy = self.tabs.iter().any(|t| t.path == path && t.busy());
        if busy {
            self.notice(format!("Save {path} first, it has unsaved changes"));
        }
        busy
    }

    pub(super) fn flow_step(&mut self, step: Step, mode: PaletteMode, query: String) -> Task<Message> {
        match (step, mode) {
            (Step::Pick(path), PaletteMode::Project(then)) => match then {
                Then::Log => self.open_flow(PaletteMode::LogEntry { project: path, source: None }, String::new()),
                Then::File(line) => {
                    let text = line.text.clone();
                    self.open_flow(PaletteMode::LogEntry { project: path, source: Some(line) }, text)
                }
                Then::Import(text) => self.import_into(path, text),
            },
            (Step::Log, PaletteMode::LogEntry { project, source }) => self.write_log(project, source, query),
            (Step::Line(line), PaletteMode::Triage(_)) => self.open_flow(PaletteMode::TriageAction(line), String::new()),
            (Step::File(line), _) => self.open_flow(PaletteMode::Project(Then::File(line)), String::new()),
            (Step::Ask(line, how), _) => self.open_flow(PaletteMode::Person(line, how), String::new()),
            (Step::Make(line, how), _) => self.make_task(line, how, None),
            (Step::Pick(person), PaletteMode::Person(line, how)) => self.make_task(line, how, Some(person)),
            _ => Task::none(),
        }
    }

    /// Log to the project open in the active tab, else ask which.
    pub(super) fn start_log(&mut self) -> Task<Message> {
        let active = self.tabs.get(self.active).filter(|t| t.loaded).map(|t| t.path.clone());
        if let Some(path) = active.filter(|p| self.kind_notes(true).iter().any(|(_, project)| project == p)) {
            return self.open_flow(PaletteMode::LogEntry { project: path, source: None }, String::new());
        }
        self.open_flow(PaletteMode::Project(Then::Log), String::new())
    }

    /// Adds the typed entry to the project's `## Log`, and marks the loose
    /// line it came from, if any, as filed with a link to the project.
    fn write_log(&mut self, project: String, source: Option<Line>, query: String) -> Task<Message> {
        let (kind, text) = log::split_kind(&query);
        if text.is_empty() || self.busy_note(&project) || source.as_ref().is_some_and(|s| self.busy_note(&s.path)) {
            return Task::none();
        }
        let entry = log::entry(&sage_editor::today(), kind, &text);
        let (vault, backend) = (self.vault.clone(), self.backend.unwrap_or(Backend::Poll));
        let (target, name, done) = (project.clone(), file_stem(&project).to_owned(), format!("Logged to {project}"));
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let updated = quick_add::capture_into(&vault, &project, &entry, log::LOG_HEADING, false, false, backend)?;
                    if let Some(s) = source {
                        log::rewrite_line(&vault, &s.path, s.line, &s.raw, &log::filed(&s.raw, &name), backend)?;
                    }
                    Ok(updated)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Written { path: target.clone(), result, done: done.clone(), open: None },
        )
    }

    /// Replaces a loose line with a task, linked to `person` if given.
    fn make_task(&mut self, line: Line, how: Triage, person: Option<String>) -> Task<Message> {
        if self.busy_note(&line.path) {
            return Task::none();
        }
        let indent = &line.raw[..line.raw.len() - line.raw.trim_start().len()];
        let new = format!("{indent}{}", log::as_task(&line.text, how, person.as_deref(), &sage_editor::today()));
        let (vault, backend) = (self.vault.clone(), self.backend.unwrap_or(Backend::Poll));
        let (path, done) = (line.path.clone(), "Made a task of the line".to_owned());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || log::rewrite_line(&vault, &line.path, line.line, &line.raw, &new, backend))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Written { path: path.clone(), result, done: done.clone(), open: None },
        )
    }

    /// Reads today's daily note for the lines nobody has sorted.
    pub(super) fn start_triage(&mut self) -> Task<Message> {
        let vault = self.vault.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let rel = sage_editor::obsidian::Daily::load(&vault).path(sage_editor::now());
                    let text =std::fs::read_to_string(vault.join(&rel)).map_err(|_| "There is no daily note for today yet".to_owned())?;
                    let lines: Vec<&str> = text.lines().collect();
                    Ok(log::untriaged(&text)
                        .into_iter()
                        .map(|(i, item)| Line { path: rel.clone(), line: i, raw: lines[i].to_owned(), text: item })
                        .collect())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::TriageLoaded,
        )
    }

    pub(super) fn triage_loaded(&mut self, result: Result<Vec<Line>, String>) -> Task<Message> {
        match result {
            Ok(lines) if lines.is_empty() => {
                self.notice("Nothing to sort in today's daily note".into());
                Task::none()
            }
            Ok(lines) => self.open_flow(PaletteMode::Triage(lines), String::new()),
            Err(error) => {
                self.notice(error);
                Task::none()
            }
        }
    }

    /// The clipboard's text arrived: ask for the project it belongs to.
    pub(super) fn import_text(&mut self, text: Option<String>) -> Task<Message> {
        match text.filter(|t| !t.trim().is_empty()) {
            Some(text) => self.open_flow(PaletteMode::Project(Then::Import(text)), String::new()),
            None => {
                self.notice("The clipboard holds no text to import".into());
                Task::none()
            }
        }
    }

    /// Makes an event note of `text` and logs it in the project.
    fn import_into(&mut self, project: String, text: String) -> Task<Message> {
        if self.busy_note(&project) {
            return Task::none();
        }
        let people: Vec<String> = self.kind_notes(false).into_iter().map(|(name, _)| name).collect();
        let imported = import::parse(&text, &people, &sage_editor::today());
        let title = sanitize_note_name(&imported.title.replace(['/', '\\'], "-"));
        let stem = format!("{} {title}", imported.date);
        let rel = format!("{EVENTS}/{stem}.md");
        let mut body = imported.body.clone();
        if !imported.tasks.is_empty() {
            body.push_str("\n\n## Actions\n\n");
            for task in &imported.tasks {
                body.push_str(&format!("- [ ] {task} #task\n"));
            }
        }
        let note = log::event_note(&imported.date, imported.kind, &title, file_stem(&project), &imported.people, &body);
        let links: String = imported.people.iter().map(|p| format!(" [[{p}]]")).collect();
        let entry = log::entry(&imported.date, imported.kind, &format!("[[{stem}]]{links}"));
        let (vault, backend) = (self.vault.clone(), self.backend.unwrap_or(Backend::Poll));
        let (target, done, open) = (project.clone(), format!("Imported {rel}, with {} actions", imported.tasks.len()), rel.clone());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    if !log::create_exclusive(&vault, &rel, &note)? {
                        return Err(format!("{rel} exists already"));
                    }
                    quick_add::capture_into(&vault, &project, &entry, log::LOG_HEADING, false, false, backend)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Written { path: target.clone(), result, done: done.clone(), open: Some(open.clone()) },
        )
    }

    /// Turns the log line under the cursor into an event note, leaving a link
    /// to it in the line.
    pub(super) fn promote_log(&mut self) -> Task<Message> {
        let Some(tab) = self.tabs.get(self.active).filter(|t| t.loaded) else { return Task::none() };
        let (path, busy) = (tab.path.clone(), tab.busy());
        let line = tab.content.cursor().position.line;
        let raw = tab.content.line(line).map(|l| l.text.into_owned()).unwrap_or_default();
        if busy {
            self.notice(format!("Save {path} first, it has unsaved changes"));
            return Task::none();
        }
        let Some(entry) = log::parse_entry(&raw) else {
            self.notice("Put the cursor on a log line, such as - 2026-11-26 #meeting Kick-off".into());
            return Task::none();
        };
        let in_project = self.kind_notes(true).iter().any(|(_, p)| *p == path);
        let project = if in_project { Some(file_stem(&path).to_owned()) } else { log::first_link(&entry.text) };
        let Some(project) = project else {
            self.notice("This line links to no project: link one first".into());
            return Task::none();
        };
        let plain = plain_links(&entry.text);
        let title = sanitize_note_name(&plain.chars().take(60).collect::<String>().replace(['/', '\\'], "-"));
        if title.is_empty() || (entry.text.starts_with("[[") && entry.text.ends_with("]]") && entry.text.matches("[[").count() == 1) {
            self.notice("This line is a note already, or has no text to make one of".into());
            return Task::none();
        }
        let date = if entry.date.is_empty() { sage_editor::today() } else { entry.date.clone() };
        let stem = format!("{date} {title}");
        let rel = format!("{EVENTS}/{stem}.md");
        let note = log::event_note(&date, &entry.kind, &title, &project, &[], &entry.text);
        let indent = &raw[..raw.len() - raw.trim_start().len()];
        let extra = if in_project { String::new() } else { format!(" [[{project}]]") };
        let new = format!("{indent}- {date} #{} [[{stem}]]{extra}", entry.kind);
        let (vault, backend) = (self.vault.clone(), self.backend.unwrap_or(Backend::Poll));
        let (target, done, open) = (path.clone(), format!("Made {rel} from the line"), rel.clone());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    if !log::create_exclusive(&vault, &rel, &note)? {
                        return Err(format!("{rel} exists already"));
                    }
                    log::rewrite_line(&vault, &path, line, &raw, &new, backend)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Written { path: target.clone(), result, done: done.clone(), open: Some(open.clone()) },
        )
    }

    /// Opens this week's review, making it first from the week's log entries.
    pub(super) fn weekly_review(&mut self) -> Task<Message> {
        let today = sage_editor::today();
        let (Some(label), Some(from)) = (log::week_label(&today), log::week_start(&today)) else { return Task::none() };
        let rel = format!("Reviews/{label}.md");
        if self.files.iter().any(|f| f.path == rel) {
            return self.open(rel);
        }
        let (vault, db_path) = (self.vault.clone(), self.db_path.clone());
        let (target, open, done) = (rel.clone(), rel.clone(), format!("Made {rel}"));
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    // The week's entries come with the text of every note that
                    // has any: read on a connection of its own, off the UI thread.
                    let activity: Vec<String> = sage_db::open_reader(&db_path)
                        .ok()
                        .and_then(|db| timeline::since(&db, &from).ok())
                        .unwrap_or_default()
                        .iter()
                        .map(|e| format!("{} {}: {} — [[{}]]", e.date, e.kind, e.text, file_stem(&e.path)))
                        .collect();
                    let note = log::review_note(&label, &activity);
                    log::create_exclusive(&vault, &rel, &note).map(|_| String::new())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Written { path: target.clone(), result, done: done.clone(), open: Some(open.clone()) },
        )
    }
}
