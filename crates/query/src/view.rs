//! A `view` block: its YAML spec, and running it against the index.
//!
//! ```view
//! type: kanban                     # table (notes), tasks, or kanban
//! source: "tag:#project/alpha"     # a query, see `dsl`; empty means everything
//! columns: [Backlog, In Progress, Done]
//! sort: due desc                   # table and tasks only
//! limit: 50
//! ```

use crate::dsl::{self, Expr};
use crate::sql::{self, Target};
use chrono::NaiveDate;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{params_from_iter, Connection, Row};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewKind {
    /// Notes, one row each.
    Table,
    /// Tasks, with checkboxes.
    Tasks,
    /// Tasks in columns by `#status/` tag, or a task document's `status`;
    /// the last column holds the done ones.
    Kanban,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewSpec {
    pub kind: ViewKind,
    pub query: Expr,
    /// Table and task columns, or the Kanban columns' names.
    pub columns: Vec<String>,
    pub sort: Option<Sort>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sort {
    pub field: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Table(Table),
    Tasks(Tasks),
    Board(Board),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<NoteRow>,
    /// More rows matched than `limit`.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteRow {
    pub path: String,
    /// One per column, as display text.
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tasks {
    pub columns: Vec<String>,
    pub rows: Vec<TaskRow>,
    pub truncated: bool,
}

/// A checkbox line, or a task document (a note with `type: task`).
#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub path: String,
    /// The note's file name without `.md`.
    pub note: String,
    /// 0 for a document.
    pub line: u32,
    /// The source line, which write-back checks before changing it. Empty
    /// for a document, whose write-back checks its `status` instead.
    pub raw: String,
    /// The task text without due and completion dates, or the document's title.
    pub text: String,
    /// A line is checked; a document's status is `done`.
    pub done: bool,
    /// A line's `#status/` slug, or a document's `status`, lowercase with
    /// words joined by `-`.
    pub status: Option<String>,
    pub due: Option<String>,
    pub scheduled: Option<String>,
    pub start: Option<String>,
    pub done_on: Option<String>,
    pub priority: Option<u8>,
    /// A task document rather than a checkbox line.
    pub document: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Board {
    pub columns: Vec<BoardColumn>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoardColumn {
    pub name: String,
    /// The `#status/` slug, or document `status`, that places a card here.
    /// The last column is the done one: lines there are checked and carry no
    /// status tag, documents have `status: done`.
    pub slug: String,
    pub cards: Vec<TaskRow>,
}

const KEYS: &str = "type, source, columns, sort, limit or group_by";
const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 1000;
const TASK_COLUMNS: [&str; 9] = ["task", "file", "path", "due", "scheduled", "start", "done", "priority", "status"];

/// Parses the YAML body of a `view` block. Errors are sentences for the preview.
pub fn parse_spec(src: &str) -> Result<ViewSpec, String> {
    use serde_yaml_ng::Value as Yaml;
    let map = match serde_yaml_ng::from_str::<Yaml>(src) {
        Ok(Yaml::Mapping(map)) => map,
        Ok(Yaml::Null) => serde_yaml_ng::Mapping::new(),
        Ok(_) => return Err("a view is written as `key: value` lines".into()),
        Err(e) => return Err(format!("not valid YAML: {e}")),
    };

    let mut kind = None;
    let mut source = String::new();
    let mut columns = None;
    let mut sort = None;
    let mut limit = DEFAULT_LIMIT;
    let mut group_by = None;
    for (key, value) in &map {
        let Some(key) = key.as_str() else {
            return Err(format!("keys are words: use {KEYS}"));
        };
        let text = |value: &Yaml| match value {
            Yaml::String(s) => Ok(s.trim().to_owned()),
            Yaml::Number(n) => Ok(n.to_string()),
            Yaml::Bool(b) => Ok(b.to_string()),
            Yaml::Null => Ok(String::new()),
            _ => Err(format!("{key}: expected text")),
        };
        match key.to_lowercase().as_str() {
            "type" => kind = Some(text(value)?),
            "source" => source = text(value)?,
            "columns" => {
                let names = match value {
                    Yaml::Sequence(items) => items.iter().map(&text).collect::<Result<Vec<_>, _>>()?,
                    value => text(value)?.split(',').map(|s| s.trim().to_owned()).collect(),
                };
                columns = Some(names.into_iter().filter(|name| !name.is_empty()).collect::<Vec<_>>());
            }
            "sort" => sort = Some(text(value)?),
            "limit" => {
                limit = value
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| (1..=MAX_LIMIT).contains(n))
                    .ok_or_else(|| format!("limit: a number from 1 to {MAX_LIMIT}"))?;
            }
            "group_by" => group_by = Some(text(value)?),
            other => return Err(format!("unknown key `{other}`: use {KEYS}")),
        }
    }

    let kind = match kind.as_deref().map(str::to_lowercase).as_deref() {
        Some("table") => ViewKind::Table,
        Some("tasks") => ViewKind::Tasks,
        Some("kanban") => ViewKind::Kanban,
        Some(other) => return Err(format!("unknown type `{other}`: use table, tasks or kanban")),
        None => return Err("a view needs a type: table, tasks or kanban".into()),
    };
    let query = dsl::parse(&source).map_err(|e| format!("source: {e}"))?;
    let sort = sort.filter(|s| !s.is_empty()).map(|s| parse_sort(&s)).transpose()?;
    if group_by.is_some() && kind != ViewKind::Kanban {
        return Err("group_by applies to kanban views only".into());
    }

    let columns = match kind {
        ViewKind::Table => {
            let columns = columns.unwrap_or_else(|| vec!["file".into(), "modified".into()]);
            // Anything that is not a built-in column is a frontmatter key.
            columns.iter().map(|c| c.to_lowercase()).collect()
        }
        ViewKind::Tasks => {
            let columns = columns.unwrap_or_else(|| vec!["task".into(), "file".into(), "due".into()]);
            let columns: Vec<String> = columns.iter().map(|c| c.to_lowercase()).collect();
            if let Some(bad) = columns.iter().find(|c| !TASK_COLUMNS.contains(&c.as_str())) {
                return Err(format!("unknown task column `{bad}`: use {}", TASK_COLUMNS.join(", ")));
            }
            if let Some(sort) = &sort {
                if !TASK_COLUMNS.contains(&sort.field.as_str()) {
                    return Err(format!("tasks cannot be sorted by `{}`: use {}", sort.field, TASK_COLUMNS.join(", ")));
                }
            }
            columns
        }
        ViewKind::Kanban => {
            if let Some(group) = group_by.filter(|g| !g.eq_ignore_ascii_case("status")) {
                return Err(format!("kanban groups by status only, not `{group}`"));
            }
            if sort.is_some() {
                return Err("sort does not apply to kanban: cards go by due date and priority".into());
            }
            let columns = columns.unwrap_or_else(|| vec!["Todo".into(), "Doing".into(), "Done".into()]);
            if columns.len() < 2 {
                return Err("a kanban needs at least two columns".into());
            }
            let slugs: Vec<String> = columns.iter().map(|c| sage_core::status_slug(c)).collect();
            if let Some(i) = slugs.iter().position(String::is_empty) {
                return Err(format!("the column `{}` has no letters or digits for a #status/ tag", columns[i]));
            }
            if let Some(i) = (1..slugs.len()).find(|&i| slugs[..i].contains(&slugs[i])) {
                return Err(format!("the columns `{}` share the tag #status/{}", columns[i], slugs[i]));
            }
            columns
        }
    };
    Ok(ViewSpec { kind, query, columns, sort, limit })
}

fn parse_sort(src: &str) -> Result<Sort, String> {
    let mut words = src.split_whitespace();
    let field = words.next().unwrap_or_default().to_lowercase();
    let descending = match words.next().map(str::to_lowercase).as_deref() {
        None | Some("asc") => false,
        Some("desc") => true,
        Some(other) => return Err(format!("sort: `{other}` should be asc or desc")),
    };
    if words.next().is_some() {
        return Err("sort: one field, then asc or desc".into());
    }
    Ok(Sort { field, descending })
}

/// Runs a view against the index. Date words in the query are relative to `today`.
pub fn run(db: &Connection, spec: &ViewSpec, today: NaiveDate) -> rusqlite::Result<Outcome> {
    match spec.kind {
        ViewKind::Table => table(db, spec, today).map(Outcome::Table),
        ViewKind::Tasks => {
            let (rows, truncated) = task_rows(db, &spec.query, spec.sort.as_ref(), false, spec.limit, today)?;
            Ok(Outcome::Tasks(Tasks { columns: spec.columns.clone(), rows, truncated }))
        }
        ViewKind::Kanban => board(db, spec, today).map(Outcome::Board),
    }
}

const BACKLINKS: &str =
    "(SELECT count(DISTINCT src_file_id) FROM v_backlinks WHERE file_id = n.id AND src_file_id <> n.id)";

fn table(db: &Connection, spec: &ViewSpec, today: NaiveDate) -> rusqlite::Result<Table> {
    // Parameters in the order they appear in the SQL text.
    let mut params: Vec<Value> = Vec::new();
    let mut select = vec!["n.path".to_owned()];
    for column in &spec.columns {
        select.push(match column.as_str() {
            "file" => "n.name".to_owned(),
            "path" => "n.path".to_owned(),
            "folder" => "n.folder".to_owned(),
            "title" => "n.title".to_owned(),
            "modified" => "strftime('%Y-%m-%d %H:%M', n.modified, 'unixepoch', 'localtime')".to_owned(),
            "size" => "n.size".to_owned(),
            "tags" => "(SELECT group_concat(tag, ' ' ORDER BY tag) \
                       FROM (SELECT DISTINCT '#' || tag AS tag FROM v_tags WHERE file_id = n.id))"
                .to_owned(),
            "backlinks" => BACKLINKS.to_owned(),
            key => {
                params.push(Value::Text(key.to_owned()));
                "(SELECT group_concat(value, ', ' ORDER BY ord) FROM v_properties WHERE file_id = n.id AND key = ?)"
                    .to_owned()
            }
        });
    }
    let (cond, cond_params) = sql::compile(&spec.query, Target::Notes, today);
    params.extend(cond_params);
    let order = match &spec.sort {
        None => "n.name COLLATE NOCASE, n.path".to_owned(),
        Some(sort) => {
            let key = match sort.field.as_str() {
                "file" => "n.name COLLATE NOCASE",
                "path" => "n.path COLLATE NOCASE",
                "folder" => "n.folder COLLATE NOCASE",
                "title" => "n.title COLLATE NOCASE",
                "modified" => "n.modified",
                "size" => "n.size",
                "backlinks" => BACKLINKS,
                key => {
                    params.push(Value::Text(key.to_owned()));
                    "(SELECT coalesce(value_num, value) FROM v_properties \
                      WHERE file_id = n.id AND key = ? ORDER BY ord LIMIT 1) COLLATE NOCASE"
                }
            };
            format!("{key} {} NULLS LAST, n.path", direction(sort))
        }
    };
    params.push(Value::Integer(spec.limit as i64 + 1));

    let sql = format!("SELECT {} FROM v_notes n WHERE {cond} ORDER BY {order} LIMIT ?", select.join(", "));
    let mut stmt = db.prepare(&sql)?;
    let width = spec.columns.len();
    let mut rows = stmt
        .query_map(params_from_iter(params), |r| {
            let cells = (1..=width).map(|i| cell(r, i)).collect::<rusqlite::Result<_>>()?;
            Ok(NoteRow { path: r.get(0)?, cells })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let truncated = rows.len() > spec.limit;
    rows.truncate(spec.limit);
    Ok(Table { columns: spec.columns.clone(), rows, truncated })
}

/// A table cell as display text; NULL is empty.
fn cell(row: &Row, i: usize) -> rusqlite::Result<String> {
    Ok(match row.get_ref(i)? {
        ValueRef::Null | ValueRef::Blob(_) => String::new(),
        ValueRef::Integer(n) => n.to_string(),
        ValueRef::Real(x) => x.to_string(),
        ValueRef::Text(text) => String::from_utf8_lossy(text).into_owned(),
    })
}

fn direction(sort: &Sort) -> &'static str {
    if sort.descending {
        "DESC"
    } else {
        "ASC"
    }
}

/// Checkbox lines and task documents side by side, in the columns of
/// `v_tasks` that task filters read. A document's state is `x` when its status
/// is `done`, `-` when `cancelled`, else open; its id is negative so it never
/// meets a line's, and it has no line, priority, or scheduled, start and done
/// dates.
const TASKS: &str = "(
    SELECT id, file_id, path, line, raw, text, state, status, due, scheduled, start, done_on, priority,
           0 AS document
    FROM v_tasks WHERE 1@TRACKED@
    UNION ALL
    SELECT -d.id, d.id, d.path, 0, '', d.title,
           CASE d.status WHEN 'done' THEN 'x' WHEN 'cancelled' THEN '-' ELSE ' ' END,
           d.status, d.due, NULL, NULL, NULL, NULL, 1
    FROM v_documents d WHERE d.type = 'task')";

/// Tasks matching `query`, checkbox lines and task documents, with whether
/// more than `limit` matched. Cancelled ones are left out unless the query
/// asks for them. `open_first` puts open tasks before done ones, and the most
/// recently done first among those, so old done tasks never crowd open cards
/// off a board.
fn task_rows(
    db: &Connection,
    query: &Expr,
    sort: Option<&Sort>,
    open_first: bool,
    limit: usize,
    today: NaiveDate,
) -> rusqlite::Result<(Vec<TaskRow>, bool)> {
    let (cond, mut params) = sql::compile(query, Target::Tasks, today);
    let cancelled = if sql::asks_for_cancelled(query) { "" } else { " AND t.state <> '-'" };
    let order = match sort {
        None if open_first => "t.state IN ('x', 'X'), CASE WHEN t.state IN ('x', 'X') THEN t.done_on END DESC NULLS LAST, \
                               t.due ASC NULLS LAST, t.priority DESC NULLS LAST, t.path COLLATE NOCASE, t.line"
            .to_owned(),
        None => "t.due ASC NULLS LAST, t.priority DESC NULLS LAST, t.path COLLATE NOCASE, t.line".to_owned(),
        Some(sort) => {
            let key = match sort.field.as_str() {
                "due" => "t.due",
                "scheduled" => "t.scheduled",
                "start" => "t.start",
                "done" => "t.done_on",
                "priority" => "t.priority",
                "file" => "n.name COLLATE NOCASE",
                "path" => "t.path COLLATE NOCASE",
                "status" => "t.status",
                // parse_spec only lets task columns through.
                _ => "t.text COLLATE NOCASE",
            };
            format!("{key} {} NULLS LAST, t.path COLLATE NOCASE, t.line", direction(sort))
        }
    };
    params.push(Value::Integer(limit as i64 + 1));

    let tasks = TASKS.replace("@TRACKED@", sage_db::tracked_clause());
    let sql = format!(
        "SELECT t.path, n.name, t.line, t.raw, t.text, t.state, t.status,
                t.due, t.scheduled, t.start, t.done_on, t.priority, t.document
         FROM {tasks} t JOIN v_notes n ON n.id = t.file_id
         WHERE {cond}{cancelled}
         ORDER BY {order} LIMIT ?"
    );
    let mut stmt = db.prepare(&sql)?;
    let mut rows = stmt
        .query_map(params_from_iter(params), |r| {
            let text: String = r.get(4)?;
            let state: String = r.get(5)?;
            let document: bool = r.get(12)?;
            Ok(TaskRow {
                path: r.get(0)?,
                note: r.get(1)?,
                line: r.get(2)?,
                raw: r.get(3)?,
                text: if document { text } else { sage_core::task_display(&text) },
                done: matches!(state.as_str(), "x" | "X"),
                status: r.get(6)?,
                due: r.get(7)?,
                scheduled: r.get(8)?,
                start: r.get(9)?,
                done_on: r.get(10)?,
                priority: r.get(11)?,
                document,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let truncated = rows.len() > limit;
    rows.truncate(limit);
    Ok((rows, truncated))
}

/// Whether a column's slug names a card's status. `_` and `-` count alike,
/// as `TaskStatus::parse` reads them, so `in_progress` lands in "In Progress".
pub fn same_status(slug: &str, status: &str) -> bool {
    slug.len() == status.len() && slug.chars().zip(status.chars()).all(|(a, b)| a == b || (a == '_' || a == '-') && (b == '_' || b == '-'))
}

/// Done cards go to the last column; open ones to the column their status
/// names, else the first. A document's status is its `status` property, as a
/// line's is its `#status/` tag.
fn board(db: &Connection, spec: &ViewSpec, today: NaiveDate) -> rusqlite::Result<Board> {
    let (rows, truncated) = task_rows(db, &spec.query, None, true, spec.limit, today)?;
    let mut columns: Vec<BoardColumn> = spec
        .columns
        .iter()
        .map(|name| BoardColumn { name: name.clone(), slug: sage_core::status_slug(name), cards: Vec::new() })
        .collect();
    let last = columns.len() - 1;
    for row in rows {
        let at = if row.done {
            last
        } else {
            row.status.as_deref().and_then(|s| columns.iter().position(|c| same_status(&c.slug, s))).unwrap_or(0)
        };
        columns[at].cards.push(row);
    }
    Ok(Board { columns, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Connection,
    }

    fn fixture(files: &[(&str, &str)]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        for (rel, text) in files {
            let path = vault.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let vault = vault.canonicalize().unwrap();
        let db_path = dir.path().join("index.db");
        let mut ix = sage_indexer::Indexer::open(vault, Path::new(&db_path)).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        drop(ix);
        let db = sage_db::open_reader(&db_path).unwrap();
        Fixture { _dir: dir, db }
    }

    const ALPHA: &str = "---\ntags: [project/alpha]\nstatus: active\nestimate: 12.5\n---\n# Alpha\n\
        Migration plan for the billing service. See [[Beta]].\n\
        - [ ] Draft the cutover runbook #status/in-progress 📅 2026-11-01 ⏫\n\
        - [ ] Review policies #status/backlog\n\
        - [x] Inventory consumers ✅ 2026-11-10\n\
        - [ ] Mystery task #status/weird\n\
        - [-] Cancelled thing\n";
    const BETA: &str = "---\ntags: [project/beta, work]\nstatus: paused\nestimate: 3\nowners: [Ann, Bob]\n---\n# Beta\n\
        Billing details and the next step for Alpha. #project/alpha/sub\n\
        - [ ] Call vendor 📅 2026-11-20 🔼\n";
    const DAILY: &str = "# 2026-11-13\nMeeting notes about [[Alpha]] and the cutover.\n- [ ] Buy milk [due:: 2026-11-12] 🔺\n";
    const READING: &str = "---\ntags: [work]\n---\nNo tasks here.\n";

    fn vault() -> Fixture {
        fixture(&[
            ("Projects/Alpha.md", ALPHA),
            ("Projects/Beta.md", BETA),
            ("Daily/2026-11-13.md", DAILY),
            ("Reading.md", READING),
        ])
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 13).unwrap()
    }

    fn spec(yaml: &str) -> ViewSpec {
        parse_spec(yaml).unwrap_or_else(|e| panic!("{e}"))
    }

    /// File names of the notes a table query returns.
    fn notes(f: &Fixture, source: &str) -> Vec<String> {
        let yaml = format!("type: table\ncolumns: [file]\nsource: '{}'", source.replace('\'', "''"));
        match run(&f.db, &spec(&yaml), today()).unwrap() {
            Outcome::Table(table) => table.rows.into_iter().map(|r| r.cells[0].clone()).collect(),
            other => panic!("{other:?}"),
        }
    }

    /// Task texts a tasks query returns, in order.
    fn tasks(f: &Fixture, yaml: &str) -> Vec<String> {
        match run(&f.db, &spec(&format!("type: tasks\n{yaml}")), today()).unwrap() {
            Outcome::Tasks(tasks) => tasks.rows.into_iter().map(|r| r.text).collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn note_filters() {
        let f = vault();
        assert_eq!(notes(&f, "tag:#project/alpha"), ["Alpha", "Beta"], "nested tags count");
        assert_eq!(notes(&f, "tag:project/beta"), ["Beta"]);
        assert_eq!(notes(&f, "[status:ACTIVE]"), ["Alpha"]);
        assert_eq!(notes(&f, "[estimate>5]"), ["Alpha"]);
        assert_eq!(notes(&f, "[estimate<=3]"), ["Beta"]);
        assert_eq!(notes(&f, "[owners:bob]"), ["Beta"], "any value of a list");
        assert_eq!(notes(&f, "[status]"), ["Alpha", "Beta"]);
        assert_eq!(notes(&f, "file:alp"), ["Alpha"]);
        assert_eq!(notes(&f, "path:daily/"), ["2026-11-13"]);
        assert_eq!(notes(&f, "bill"), ["Alpha", "Beta"], "a word matches as a prefix");
        assert_eq!(notes(&f, "\"next step\""), ["Beta"]);
        assert!(notes(&f, "\"step next\"").is_empty(), "a phrase keeps its order");
        assert_eq!(notes(&f, "content:cutover"), ["2026-11-13", "Alpha"]);
    }

    #[test]
    fn combining() {
        let f = vault();
        assert_eq!(notes(&f, "tag:#project/beta OR path:Daily"), ["2026-11-13", "Beta"]);
        assert_eq!(notes(&f, "tag:#project/alpha -tag:#work"), ["Alpha"]);
        assert_eq!(notes(&f, "(file:alpha OR file:beta) -[status:paused]"), ["Alpha"]);
        assert_eq!(notes(&f, ""), ["2026-11-13", "Alpha", "Beta", "Reading"], "empty means everything");
        assert!(notes(&f, "file:\"x' OR 1=1 --\"").is_empty(), "quotes are data, not SQL");
        assert!(notes(&f, "[status:\"a'b\"]").is_empty());
        assert!(notes(&f, "&").is_empty(), "punctuation alone matches nothing");
    }

    #[test]
    fn task_filters() {
        let f = vault();
        assert_eq!(
            tasks(&f, "source: 'task-todo:'"),
            [
                "Draft the cutover runbook #status/in-progress ⏫",
                "Buy milk 🔺",
                "Call vendor 🔼",
                "Review policies #status/backlog",
                "Mystery task #status/weird",
            ],
            "by due date, undated last; cancelled tasks never show"
        );
        assert_eq!(tasks(&f, "source: 'status:\"In Progress\"'"), ["Draft the cutover runbook #status/in-progress ⏫"]);
        assert_eq!(
            tasks(&f, "source: 'due<today'"),
            ["Draft the cutover runbook #status/in-progress ⏫", "Buy milk 🔺"]
        );
        assert_eq!(
            tasks(&f, "source: 'priority>=high'\nsort: priority desc"),
            ["Buy milk 🔺", "Draft the cutover runbook #status/in-progress ⏫"]
        );
        assert_eq!(tasks(&f, "source: 'task-done:'"), ["Inventory consumers"]);
        assert_eq!(tasks(&f, "source: 'status:none task-todo:'"), ["Buy milk 🔺", "Call vendor 🔼"]);
        assert_eq!(tasks(&f, "source: 'task:VENDOR'"), ["Call vendor 🔼"]);
        assert_eq!(tasks(&f, "source: 'tag:#work task-todo:'"), ["Call vendor 🔼"], "note filters apply to the task's note");
    }

    #[test]
    fn task_filters_in_a_notes_table_ask_for_one_matching_task() {
        let f = vault();
        assert_eq!(notes(&f, "due<today"), ["2026-11-13", "Alpha"]);
        assert_eq!(notes(&f, "tag:#work OR status:in-progress"), ["Alpha", "Beta", "Reading"], "a note without tasks still matches");
        assert_eq!(notes(&f, "status:backlog due:any"), Vec::<String>::new(), "both on the same task");
        assert_eq!(notes(&f, "status:in-progress due:any"), ["Alpha"]);
    }

    #[test]
    fn table_cells_and_sorting() {
        let f = vault();
        let spec = spec("type: table\nsource: 'path:Projects'\ncolumns: [file, status, estimate, owners, tags, backlinks, modified, folder]\nsort: estimate desc");
        let Outcome::Table(table) = run(&f.db, &spec, today()).unwrap() else { panic!() };
        assert_eq!(table.rows.len(), 2);
        let alpha = &table.rows[0];
        assert_eq!(alpha.path, "Projects/Alpha.md");
        assert_eq!(alpha.cells[..4], ["Alpha", "active", "12.5", ""]);
        assert_eq!(alpha.cells[4], "#project/alpha #status/backlog #status/in-progress #status/weird");
        assert_eq!(alpha.cells[5], "1", "only the daily note links here");
        assert!(alpha.cells[6].len() == 16 && alpha.cells[6].starts_with("20"), "{}", alpha.cells[6]);
        assert_eq!(alpha.cells[7], "Projects");
        assert_eq!(table.rows[1].cells[..4], ["Beta", "paused", "3", "Ann, Bob"]);

        let spec = parse_spec("type: table\nsort: estimate").unwrap();
        let Outcome::Table(table) = run(&f.db, &spec, today()).unwrap() else { panic!() };
        let order: Vec<&str> = table.rows.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(order[..2], ["Projects/Beta.md", "Projects/Alpha.md"], "missing values sort last");
    }

    #[test]
    fn kanban_columns() {
        let f = vault();
        let spec = spec("type: kanban\nsource: \"tag:#project/alpha\"\ngroup_by: status\ncolumns: [Backlog, In Progress, Done]");
        let Outcome::Board(board) = run(&f.db, &spec, today()).unwrap() else { panic!() };
        let cards: Vec<(&str, &str, Vec<&str>)> = board
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.slug.as_str(), c.cards.iter().map(|card| card.text.as_str()).collect()))
            .collect();
        assert_eq!(
            cards,
            [
                (
                    "Backlog",
                    "backlog",
                    vec!["Call vendor 🔼", "Review policies #status/backlog", "Mystery task #status/weird"]
                ),
                ("In Progress", "in-progress", vec!["Draft the cutover runbook #status/in-progress ⏫"]),
                ("Done", "done", vec!["Inventory consumers"]),
            ]
        );
        let card = &board.columns[2].cards[0];
        assert_eq!((card.path.as_str(), card.note.as_str(), card.done), ("Projects/Alpha.md", "Alpha", true));
        assert_eq!(card.raw, "- [x] Inventory consumers ✅ 2026-11-10");
        assert_eq!(card.line, 9, "lines count from the top, frontmatter included");
        assert!(!board.truncated);
    }

    fn task_doc(id: &str, status: Option<&str>, due: Option<&str>) -> String {
        let status = status.map(|s| format!("status: {s}\n")).unwrap_or_default();
        let due = due.map(|d| format!("due: {d}\n")).unwrap_or_default();
        format!("---\nid: {id}\ntype: task\ntitle: Doc {id}\n{status}{due}tags: [backend]\n---\nBody.\n")
    }

    fn with_documents() -> Fixture {
        fixture(&[
            ("Tasks/a.md", &task_doc("a", Some("in-progress"), Some("2026-11-20"))),
            ("Tasks/b.md", &task_doc("b", Some("Todo"), None)),
            ("Tasks/c.md", &task_doc("c", Some("done"), Some("2026-11-01"))),
            ("Tasks/d.md", &task_doc("d", None, Some("2026-11-02"))),
            ("Tasks/e.md", &task_doc("e", Some("cancelled"), None)),
            ("Tasks/f.md", &task_doc("f", Some("blocked"), None)),
            ("Projects/p.md", "---\ntype: project\ntitle: Not a task\nstatus: todo\n---\n"),
            ("Board.md", "# Board\n- [ ] Line task #status/in-progress #backend\n- [x] Done line\n"),
        ])
    }

    #[test]
    fn task_documents_are_tasks_in_lists() {
        let f = with_documents();
        assert_eq!(
            tasks(&f, ""),
            ["Doc c", "Doc d", "Doc a", "Line task #status/in-progress #backend", "Done line", "Doc b", "Doc f"],
            "by due date; cancelled documents and other types left out"
        );
        assert_eq!(tasks(&f, "source: 'kind:document task-todo:'\nsort: file"), ["Doc a", "Doc b", "Doc d", "Doc f"]);
        assert_eq!(tasks(&f, "source: '-kind:document'"), ["Line task #status/in-progress #backend", "Done line"]);
        assert_eq!(tasks(&f, "source: 'kind:checkbox task-done:'"), ["Done line"]);
        assert_eq!(tasks(&f, "source: 'task-done:'"), ["Doc c", "Done line"]);
        assert_eq!(tasks(&f, "source: 'status:in-progress'"), ["Doc a", "Line task #status/in-progress #backend"]);
        assert_eq!(tasks(&f, "source: 'status:none'"), ["Doc d", "Done line"]);
        assert_eq!(tasks(&f, "source: 'path:Tasks/ due<2026-11-15'"), ["Doc c", "Doc d"]);
        assert_eq!(tasks(&f, "source: 'task:\"doc b\"'"), ["Doc b"], "the title is the text");

        let Outcome::Tasks(list) = run(&f.db, &spec("type: tasks\nsource: 'file:a'"), today()).unwrap() else { panic!() };
        let a = &list.rows[0];
        assert!(a.document && !a.done);
        assert_eq!((a.path.as_str(), a.note.as_str(), a.line, a.raw.as_str()), ("Tasks/a.md", "a", 0, ""));
        assert_eq!((a.status.as_deref(), a.due.as_deref()), (Some("in-progress"), Some("2026-11-20")));

        // A notes table is unchanged: its task filters ask for checkbox lines.
        assert_eq!(notes(&f, "task-todo:"), ["Board"]);
        assert!(notes(&f, "kind:document").is_empty());
    }

    #[test]
    fn a_board_groups_documents_by_status() {
        let f = with_documents();
        let spec = spec("type: kanban\nsource: 'tag:#backend'\ncolumns: [Todo, In Progress, Done]");
        let Outcome::Board(board) = run(&f.db, &spec, today()).unwrap() else { panic!() };
        let cards: Vec<Vec<&str>> =
            board.columns.iter().map(|c| c.cards.iter().map(|card| card.text.as_str()).collect()).collect();
        assert_eq!(
            cards,
            [
                // `status: Todo` names the first column; no status and an
                // unknown one fall to the first column, as a line's tag does.
                vec!["Doc d", "Doc b", "Doc f"],
                vec!["Doc a", "Line task #status/in-progress #backend"],
                vec!["Doc c", "Done line"],
            ]
        );
        let done = &board.columns[2].cards[0];
        assert!(done.document && done.done);
    }

    #[test]
    fn properties_with_date_words_quoted_numbers_and_any_case() {
        let f = fixture(&[
            ("Old.md", "---\ndue: 2026-11-01\nzip: \"01234\"\nstate: banana\n---\n"),
            ("New.md", "---\ndue: 2026-12-01\nzip: 1234\nstate: Apple\n---\n"),
            ("Żółw.md", "---\nstate: active\n---\n"),
            ("Środa/Plan.md", "# Plan\n"),
        ]);
        assert_eq!(notes(&f, "[due<today]"), ["Old"], "today is 2026-11-13");
        assert_eq!(notes(&f, "[due>=TOMORROW]"), ["New"]);
        assert_eq!(notes(&f, "[zip:01234]"), ["New", "Old"], "the number, and the quoted text");
        assert_eq!(notes(&f, "file:ŻÓŁW"), ["Żółw"], "case folds beyond ASCII");
        assert_eq!(notes(&f, "path:środa/"), ["Plan"]);
        let yaml = "type: table\ncolumns: [file]\nsource: '[state]'\nsort: state";
        let Outcome::Table(table) = run(&f.db, &spec(yaml), today()).unwrap() else { panic!() };
        let order: Vec<&str> = table.rows.iter().map(|r| r.cells[0].as_str()).collect();
        assert_eq!(order, ["Żółw", "New", "Old"], "active, Apple, banana: case does not sort");
    }

    #[test]
    fn none_ignores_case_and_cancelled_tasks_can_be_asked_for() {
        let f = vault();
        assert_eq!(tasks(&f, "source: 'due:None task-todo:'\nsort: task"), tasks(&f, "source: 'due:none task-todo:'\nsort: task"));
        assert_eq!(tasks(&f, "source: 'status:NONE'"), tasks(&f, "source: 'status:none'"));
        assert_eq!(tasks(&f, "source: 'task-cancelled:'"), ["Cancelled thing"]);
        assert!(!tasks(&f, "").contains(&"Cancelled thing".to_owned()), "left out unless asked for");
    }

    #[test]
    fn open_cards_come_before_old_done_ones_on_a_board() {
        let f = fixture(&[(
            "Board.md",
            "- [x] Long done 📅 2020-01-01 ✅ 2020-01-02\n- [x] Done lately 📅 2021-01-01 ✅ 2026-11-12\n- [ ] Still open 📅 2026-12-01\n",
        )]);
        let Outcome::Board(board) = run(&f.db, &spec("type: kanban\nlimit: 2"), today()).unwrap() else { panic!() };
        let cards: Vec<Vec<&str>> = board.columns.iter().map(|c| c.cards.iter().map(|c| c.text.as_str()).collect()).collect();
        assert_eq!(cards, [vec!["Still open"], vec![], vec!["Done lately"]]);
        assert!(board.truncated);
    }

    #[test]
    fn document_statuses_find_their_columns_however_written() {
        let f = fixture(&[
            ("Tasks/a.md", &task_doc("a", Some("To_Do"), None)),
            ("Tasks/b.md", &task_doc("b", Some("In  Progress"), None)),
            ("Tasks/c.md", &task_doc("c", Some("in_progress"), None)),
            ("Tasks/d.md", &task_doc("d", Some("Wartet auf Ärger"), None)),
            ("Board.md", "- [ ] Line #status/to_do\n"),
        ]);
        let spec = spec("type: kanban\nsource: ''\ncolumns: [Backlog, To_Do, In Progress, Wartet auf ärger, Done]");
        let Outcome::Board(board) = run(&f.db, &spec, today()).unwrap() else { panic!() };
        let cards: Vec<Vec<&str>> = board.columns.iter().map(|c| c.cards.iter().map(|c| c.text.as_str()).collect()).collect();
        assert_eq!(cards, [vec![], vec!["Line #status/to_do", "Doc a"], vec!["Doc b", "Doc c"], vec!["Doc d"], vec![]]);
        assert!(same_status("in-progress", "in_progress") && !same_status("in-progress", "in-progres"));
    }

    #[test]
    fn limits() {
        let f = vault();
        let Outcome::Table(table) = run(&f.db, &spec("type: table\nlimit: 1"), today()).unwrap() else { panic!() };
        assert_eq!((table.rows.len(), table.truncated), (1, true));
        let Outcome::Board(board) = run(&f.db, &spec("type: kanban\nlimit: 2"), today()).unwrap() else { panic!() };
        assert_eq!(board.columns.iter().map(|c| c.cards.len()).sum::<usize>(), 2);
        assert!(board.truncated);
        let Outcome::Tasks(all) = run(&f.db, &spec("type: tasks\nlimit: 1000"), today()).unwrap() else { panic!() };
        assert_eq!((all.rows.len(), all.truncated), (6, false));
    }

    #[test]
    fn spec_defaults_and_errors() {
        let table = spec("type: Table");
        assert_eq!((table.columns.as_slice(), table.limit, &table.query), (&["file".to_owned(), "modified".to_owned()][..], 200, &Expr::And(vec![])));
        assert_eq!(spec("type: tasks").columns, ["task", "file", "due"]);
        assert_eq!(spec("type: kanban").columns, ["Todo", "Doing", "Done"]);
        assert_eq!(spec("type: table\ncolumns: file, Status").columns, ["file", "status"]);
        assert_eq!(spec("type: tasks\nsort: due DESC").sort, Some(Sort { field: "due".into(), descending: true }));

        let error = |yaml: &str| parse_spec(yaml).unwrap_err();
        assert_eq!(error(""), "a view needs a type: table, tasks or kanban");
        assert_eq!(error("type: board"), "unknown type `board`: use table, tasks or kanban");
        assert_eq!(error("type: table\ncolums: [file]"), "unknown key `colums`: use type, source, columns, sort, limit or group_by");
        assert_eq!(error("type: table\nsource: 'a (b'"), "source: `(` without a matching `)` (column 3)");
        assert_eq!(error("type: kanban\ncolumns: [Only]"), "a kanban needs at least two columns");
        assert_eq!(error("type: kanban\ncolumns: [Done, done]"), "the columns `done` share the tag #status/done");
        assert_eq!(error("type: kanban\nsort: due"), "sort does not apply to kanban: cards go by due date and priority");
        assert_eq!(error("type: kanban\ngroup_by: priority"), "kanban groups by status only, not `priority`");
        assert_eq!(error("type: table\ngroup_by: status"), "group_by applies to kanban views only");
        assert!(error("type: tasks\ncolumns: [task, owner]").starts_with("unknown task column `owner`"));
        assert!(error("type: tasks\nsort: owner").starts_with("tasks cannot be sorted by `owner`"));
        assert_eq!(error("type: table\nlimit: 0"), "limit: a number from 1 to 1000");
        assert_eq!(error("type: table\nsort: file sideways"), "sort: `sideways` should be asc or desc");
        assert!(error("type: [table").starts_with("not valid YAML"));
        assert_eq!(error("- a list"), "a view is written as `key: value` lines");
    }
}
