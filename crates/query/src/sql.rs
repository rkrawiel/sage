//! Compiles a query to a parameterized SQL condition over the index's stable
//! views. Values from a note only ever travel as parameters; column names and
//! operators come from the fixed lists in this file.

use crate::dsl::{DateCmp, DateField, Day, Expr, NoteFilter, TaskFilter, TaskState};
use chrono::NaiveDate;
use rusqlite::types::Value;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Target {
    /// Rows are notes from `v_notes`, aliased `n`.
    Notes,
    /// Rows are tasks, aliased `t`: the columns of `v_tasks` and `document`,
    /// over checkbox lines and task documents alike (see `view::TASKS`).
    Tasks,
}

/// A boolean condition for the rows of `target`, with its parameters in order.
///
/// In a notes query, task filters ask for a task that satisfies the whole
/// expression together with the note's own filters. A note without tasks still
/// gets one row of NULLs from the left join, so `tag:x OR due<today` keeps
/// matching tagged notes that have no tasks at all.
pub(crate) fn compile(expr: &Expr, target: Target, today: NaiveDate) -> (String, Vec<Value>) {
    let mut compiler = Compiler { today, target, params: Vec::new() };
    let id = match target {
        Target::Notes => "n.id",
        Target::Tasks => "t.file_id",
    };
    let cond = compiler.expr(expr, id);
    let cond = if target == Target::Notes && has_task_filter(expr) {
        format!("EXISTS (SELECT 1 FROM (SELECT 1) LEFT JOIN v_tasks t ON t.file_id = n.id WHERE {cond})")
    } else {
        cond
    };
    (cond, compiler.params)
}

/// Whether the query asks for cancelled tasks, which task lists and boards
/// otherwise leave out.
pub(crate) fn asks_for_cancelled(expr: &Expr) -> bool {
    match expr {
        Expr::And(items) | Expr::Or(items) => items.iter().any(asks_for_cancelled),
        Expr::Not(inner) => asks_for_cancelled(inner),
        Expr::Task(TaskFilter::Text { state: TaskState::Cancelled, .. }) => true,
        Expr::Note(_) | Expr::Task(_) => false,
    }
}

fn has_task_filter(expr: &Expr) -> bool {
    match expr {
        Expr::And(items) | Expr::Or(items) => items.iter().any(has_task_filter),
        Expr::Not(inner) => has_task_filter(inner),
        Expr::Note(_) => false,
        Expr::Task(_) => true,
    }
}

struct Compiler {
    today: NaiveDate,
    target: Target,
    params: Vec<Value>,
}

impl Compiler {
    fn param(&mut self, value: impl Into<Value>) {
        self.params.push(value.into());
    }

    fn expr(&mut self, expr: &Expr, id: &str) -> String {
        match expr {
            Expr::And(items) if items.is_empty() => "1".to_owned(),
            Expr::Or(items) if items.is_empty() => "0".to_owned(),
            Expr::And(items) => self.join(items, id, " AND "),
            Expr::Or(items) => self.join(items, id, " OR "),
            Expr::Not(inner) => format!("NOT ({})", self.expr(inner, id)),
            // Every leaf is 0 or 1, never NULL, so NOT negates rows without a
            // task or without a value too.
            Expr::Note(filter) => format!("coalesce(({}), 0)", self.note(filter, id)),
            Expr::Task(filter) => format!("coalesce(({}), 0)", self.task(filter)),
        }
    }

    fn join(&mut self, items: &[Expr], id: &str, op: &str) -> String {
        let parts: Vec<String> = items.iter().map(|item| self.expr(item, id)).collect();
        format!("({})", parts.join(op))
    }

    fn note(&mut self, filter: &NoteFilter, id: &str) -> String {
        match filter {
            NoteFilter::Text { text, phrase } => {
                // The unicode61 tokenizer drops punctuation; a phrase of none
                // would be an FTS5 syntax error, and can match nothing anyway.
                if !text.chars().any(char::is_alphanumeric) {
                    return "0".to_owned();
                }
                let quoted = format!("\"{}\"", text.replace('"', "\"\""));
                self.param(if *phrase { quoted } else { quoted + "*" });
                format!("{id} IN (SELECT rowid FROM fts WHERE fts MATCH ?)")
            }
            NoteFilter::File(name) => {
                self.param(name.clone());
                format!("{id} IN (SELECT id FROM v_notes WHERE instr(lower(name), lower(?)) > 0)")
            }
            NoteFilter::Path(path) => {
                self.param(path.clone());
                format!("{id} IN (SELECT id FROM v_notes WHERE instr(lower(path), lower(?)) > 0)")
            }
            NoteFilter::Tag(tag) => {
                self.param(tag.clone());
                self.param(format!("{}/*", glob_escape(tag)));
                format!("{id} IN (SELECT file_id FROM v_tags WHERE tag = ? OR tag GLOB ?)")
            }
            NoteFilter::Property { key, cmp: None } => {
                self.param(key.clone());
                format!("{id} IN (SELECT file_id FROM v_properties WHERE key = ?)")
            }
            NoteFilter::Property { key, cmp: Some((cmp, value)) } => {
                self.param(key.clone());
                // `[due<today]`: the date words stand for dates, as in task filters.
                let value = match crate::dsl::day(value) {
                    Some(day @ Day::Relative(_)) => self.date(day),
                    _ => value.clone(),
                };
                let test = match value.parse::<f64>() {
                    // A quoted number (`zip: "01234"`) is text, with no value_num.
                    Ok(number) if *cmp == crate::dsl::Cmp::Eq => {
                        self.param(number);
                        self.param(value.clone());
                        "(value_num = ? OR lower(value) = lower(?))".to_owned()
                    }
                    Ok(number) => {
                        self.param(number);
                        format!("value_num {} ?", cmp.sql())
                    }
                    Err(_) if *cmp == crate::dsl::Cmp::Eq => {
                        self.param(value.clone());
                        "lower(value) = lower(?)".to_owned()
                    }
                    // ISO dates compare correctly as text.
                    Err(_) => {
                        self.param(value.clone());
                        format!("value {} ?", cmp.sql())
                    }
                };
                format!("{id} IN (SELECT file_id FROM v_properties WHERE key = ? AND {test})")
            }
        }
    }

    /// Task filters read the row aliased `t`. In a notes query that row can be
    /// the left join's NULLs, so "has no value" checks also ask for a task.
    fn task(&mut self, filter: &TaskFilter) -> String {
        match filter {
            TaskFilter::Text { text, state } => {
                let mut parts = Vec::new();
                if !text.is_empty() {
                    self.param(text.clone());
                    parts.push("instr(lower(t.text), lower(?)) > 0");
                }
                match state {
                    TaskState::Open => parts.push("t.state NOT IN ('x', 'X', '-')"),
                    TaskState::Done => parts.push("t.state IN ('x', 'X')"),
                    TaskState::Cancelled => parts.push("t.state = '-'"),
                    TaskState::Any => {}
                }
                if parts.is_empty() {
                    "t.id IS NOT NULL".to_owned()
                } else {
                    parts.join(" AND ")
                }
            }
            TaskFilter::Status(Some(slug)) => {
                self.param(slug.clone());
                "t.status = ?".to_owned()
            }
            TaskFilter::Status(None) => "t.id IS NOT NULL AND t.status IS NULL".to_owned(),
            TaskFilter::Date { field, cmp } => {
                let column = match field {
                    DateField::Due => "t.due",
                    DateField::Scheduled => "t.scheduled",
                    DateField::Start => "t.start",
                    DateField::Done => "t.done_on",
                };
                match cmp {
                    DateCmp::None => format!("t.id IS NOT NULL AND {column} IS NULL"),
                    DateCmp::Any => format!("{column} IS NOT NULL"),
                    DateCmp::Cmp(cmp, day) => {
                        let date = self.date(*day);
                        self.param(date);
                        format!("{column} {} ?", cmp.sql())
                    }
                }
            }
            TaskFilter::Priority(None) => "t.id IS NOT NULL AND t.priority IS NULL".to_owned(),
            TaskFilter::Priority(Some((cmp, level))) => {
                self.param(i64::from(*level));
                format!("t.priority {} ?", cmp.sql())
            }
            // A notes query joins `v_tasks`, which has checkbox lines only.
            TaskFilter::Document(document) => match (self.target, document) {
                (Target::Tasks, true) => "t.document = 1".to_owned(),
                (Target::Tasks, false) => "t.document = 0".to_owned(),
                (Target::Notes, true) => "0".to_owned(),
                (Target::Notes, false) => "t.id IS NOT NULL".to_owned(),
            },
        }
    }

    fn date(&self, day: Day) -> String {
        day.iso(self.today)
    }
}

/// Makes `*`, `?` and `[` literal in a GLOB pattern.
pub(crate) fn glob_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '*' | '?' | '[' => {
                out.push('[');
                out.push(c);
                out.push(']');
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::parse;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 13).unwrap()
    }

    #[test]
    fn values_travel_as_parameters() {
        let (cond, params) = compile(&parse("file:\"x' OR 1=1 --\" tag:a*b").unwrap(), Target::Notes, today());
        assert!(!cond.contains("OR 1=1"), "{cond}");
        assert_eq!(
            params,
            [
                Value::Text("x' OR 1=1 --".into()),
                Value::Text("a*b".into()),
                Value::Text("a[*]b/*".into()),
            ]
        );
    }

    #[test]
    fn task_filters_in_a_notes_query_share_one_task() {
        let (cond, params) = compile(&parse("tag:x OR due<tomorrow").unwrap(), Target::Notes, today());
        assert!(cond.starts_with("EXISTS (SELECT 1 FROM (SELECT 1) LEFT JOIN v_tasks t ON t.file_id = n.id WHERE "));
        assert_eq!(params.last(), Some(&Value::Text("2026-11-14".into())));

        let (cond, _) = compile(&parse("tag:x").unwrap(), Target::Notes, today());
        assert!(!cond.contains("EXISTS"), "notes-only queries need no join: {cond}");
        let (cond, _) = compile(&parse("due<today").unwrap(), Target::Tasks, today());
        assert!(!cond.contains("EXISTS"), "task rows are already tasks: {cond}");
    }

    #[test]
    fn empty_and_punctuation_queries() {
        assert_eq!(compile(&parse("").unwrap(), Target::Notes, today()).0, "1");
        let (cond, params) = compile(&parse("&").unwrap(), Target::Notes, today());
        assert_eq!((cond.as_str(), params.len()), ("coalesce((0), 0)", 0));
    }
}
