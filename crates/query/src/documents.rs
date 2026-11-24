//! Typed documents, notes whose frontmatter names a `type`, and the `tasks`
//! block, which lists tasks of both kinds: task documents (`type: task`), and
//! checkbox lines (`- [ ] …`) in any note, as the Tasks plugin does.
//!
//! ```tasks
//! status != done
//! due < 2026-12-01
//! tag = backend
//! sort due
//! ```
//!
//! One instruction per line, all filters must hold:
//!
//! | Line | Keeps |
//! |---|---|
//! | `status = todo`, `status != done` | by status: todo, in-progress, done, cancelled |
//! | `due < today`, `created >= 2026-11-01` | by date: `=`, `!=`, `<`, `<=`, `>`, `>=`, against a date, `today`, `tomorrow` or `yesterday` |
//! | `due = none`, `due != none` | without, or with, the date |
//! | `tag = backend`, `tag != voice` | with, or without, the tag or one nested under it |
//! | `folder = Projects`, `folder != Archive` | in, or not in, the folder or one inside it |
//! | `sort due`, `sort status desc` | order by due, created, status, title or id; several lines sort by each in turn |
//! | `limit 20` | at most that many, 1 to 1000, default 200 |
//!
//! A checkbox line's status is `done` when checked, `cancelled` for `[-]`,
//! else its `#status/` tag, else `todo`; its tags are those on its line. A
//! document's tags are the note's. Checkbox lines have no `created` date.
//!
//! Like `view` blocks it reads the index's stable views only (`v_documents`,
//! `v_tasks`, `v_tags`), and every value travels as a parameter.

use crate::dsl::{self, Cmp, Day};
use crate::sql::glob_escape;
use chrono::NaiveDate;
use sage_core::TaskStatus;
use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection, OptionalExtension, Row};

/// A typed document, as of the last index pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub path: String,
    /// Lowercase: `task`, `project`, `meeting`, ...
    pub doc_type: String,
    /// The frontmatter `id`, else the file name.
    pub id: String,
    pub title: String,
    /// Lowercase, words joined by `-`, as `v_documents` reads it.
    pub status: Option<String>,
    pub created: Option<String>,
    pub due: Option<String>,
    /// The frontmatter tags, lowercase.
    pub tags: Vec<String>,
}

impl Document {
    pub fn is_task(&self) -> bool {
        self.doc_type == "task"
    }

    /// `None` for a status other than the four a task has.
    pub fn task_status(&self) -> Option<TaskStatus> {
        self.status.as_deref().and_then(TaskStatus::parse)
    }
}

const COLUMNS: &str = "d.path, d.type, d.doc_id, d.title, d.status, d.created, d.due, d.tags";

fn document_row(r: &Row) -> rusqlite::Result<Document> {
    let tags: Option<String> = r.get(7)?;
    Ok(Document {
        path: r.get(0)?,
        doc_type: r.get(1)?,
        id: r.get(2)?,
        title: r.get(3)?,
        status: r.get(4)?,
        created: r.get(5)?,
        due: r.get(6)?,
        tags: tags.map(|t| t.split(',').map(str::to_owned).collect()).unwrap_or_default(),
    })
}

/// The note at `path` if it is a typed document.
pub fn document(db: &Connection, path: &str) -> rusqlite::Result<Option<Document>> {
    db.prepare_cached(&format!("SELECT {COLUMNS} FROM v_documents d WHERE d.path = ?1"))?
        .query_row([path], document_row)
        .optional()
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskQuery {
    pub filters: Vec<Filter>,
    pub sort: Vec<SortKey>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    Status { status: TaskStatus, equal: bool },
    /// `day` is `None` for `none`: the date is missing, or with `!=` present.
    Date { field: DateField, cmp: DateOp, day: Option<Day> },
    /// Lowercase, without `#`.
    Tag { tag: String, equal: bool },
    /// A vault folder, without slashes around it.
    Folder { folder: String, equal: bool },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DateField {
    Due,
    Created,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DateOp {
    Cmp(Cmp),
    NotEqual,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortField {
    Due,
    Created,
    Status,
    Title,
    Id,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SortKey {
    pub field: SortField,
    pub descending: bool,
}

/// One row of a `tasks` block: a task document, or a checkbox line.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskItem {
    pub path: String,
    /// The document's title, or the line's text without its dates.
    pub title: String,
    /// Lowercase, words joined by `-`.
    pub status: Option<String>,
    pub created: Option<String>,
    pub due: Option<String>,
    pub tags: Vec<String>,
    /// For a checkbox line, its 0-based line and its source text, which a
    /// write-back checks before changing it. `None` for a document.
    pub line: Option<(u32, String)>,
}

impl TaskItem {
    /// `None` for a status other than the four a task has.
    pub fn task_status(&self) -> Option<TaskStatus> {
        self.status.as_deref().and_then(TaskStatus::parse)
    }
}

/// What a `tasks` block lists.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskList {
    pub rows: Vec<TaskItem>,
    /// More matched than the limit.
    pub truncated: bool,
}

const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 1000;
const FIELDS: &str = "status, due, created, tag or folder";
const SORTS: &str = "due, created, status, title or id";

/// Parses the body of a `tasks` block. Errors are sentences for the preview.
pub fn parse_tasks(src: &str) -> Result<TaskQuery, String> {
    let mut query = TaskQuery { filters: Vec::new(), sort: Vec::new(), limit: DEFAULT_LIMIT };
    for (n, line) in src.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        instruction(line, &mut query).map_err(|e| format!("line {}: {e}", n + 1))?;
    }
    Ok(query)
}

fn instruction(line: &str, query: &mut TaskQuery) -> Result<(), String> {
    let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    match word.to_lowercase().as_str() {
        "sort" => {
            let mut words = rest.split_whitespace();
            let field = match words.next().map(str::to_lowercase).as_deref() {
                Some("due") => SortField::Due,
                Some("created") => SortField::Created,
                Some("status") => SortField::Status,
                Some("title") => SortField::Title,
                Some("id") => SortField::Id,
                Some(other) => return Err(format!("cannot sort by `{other}`: use {SORTS}")),
                None => return Err(format!("sort by what? use {SORTS}")),
            };
            let descending = match words.next().map(str::to_lowercase).as_deref() {
                None | Some("asc") => false,
                Some("desc") => true,
                Some(other) => return Err(format!("`{other}` should be asc or desc")),
            };
            if words.next().is_some() {
                return Err("sort takes one field, then asc or desc".into());
            }
            query.sort.push(SortKey { field, descending });
        }
        "limit" => {
            query.limit = rest
                .parse()
                .ok()
                .filter(|n| (1..=MAX_LIMIT).contains(n))
                .ok_or_else(|| format!("limit: a number from 1 to {MAX_LIMIT}"))?;
        }
        _ => query.filters.push(filter(line)?),
    }
    Ok(())
}

/// `field op value`, spaces around the operator optional.
fn filter(line: &str) -> Result<Filter, String> {
    let at = line.find(['=', '!', '<', '>']).ok_or_else(|| {
        format!("`{line}` is not a filter: write a field, an operator and a value, as in `status != done`")
    })?;
    let field = line[..at].trim().to_lowercase();
    let rest = &line[at..];
    let (op, value) = ["!=", "<=", ">=", "=", "<", ">"]
        .into_iter()
        .find_map(|op| rest.strip_prefix(op).map(|value| (op, value.trim())))
        .ok_or_else(|| format!("unknown operator in `{line}`: use =, !=, <, <=, > or >="))?;
    if value.is_empty() {
        return Err(format!("`{field} {op}` needs a value"));
    }
    let equal = match op {
        "=" => Some(true),
        "!=" => Some(false),
        _ => None,
    };
    match field.as_str() {
        "status" => {
            let equal = equal.ok_or("status compares with = or != only")?;
            let status = TaskStatus::parse(value)
                .ok_or_else(|| format!("unknown status `{value}`: use todo, in-progress, done or cancelled"))?;
            Ok(Filter::Status { status, equal })
        }
        "tag" | "tags" => {
            let equal = equal.ok_or("tag compares with = or != only")?;
            Ok(Filter::Tag { tag: value.trim_start_matches('#').to_lowercase(), equal })
        }
        "folder" => {
            let equal = equal.ok_or("folder compares with = or != only")?;
            let folder = value.trim_matches(['/', '"']).to_owned();
            if folder.is_empty() {
                return Err("folder needs a folder name, as in `folder = Projects`".into());
            }
            Ok(Filter::Folder { folder, equal })
        }
        "due" | "created" => {
            let field = if field == "due" { DateField::Due } else { DateField::Created };
            let cmp = match op {
                "!=" => DateOp::NotEqual,
                "=" => DateOp::Cmp(Cmp::Eq),
                "<" => DateOp::Cmp(Cmp::Lt),
                "<=" => DateOp::Cmp(Cmp::Le),
                ">" => DateOp::Cmp(Cmp::Gt),
                _ => DateOp::Cmp(Cmp::Ge),
            };
            if value.eq_ignore_ascii_case("none") {
                if equal.is_none() {
                    return Err("none compares with = or != only".into());
                }
                return Ok(Filter::Date { field, cmp, day: None });
            }
            let day = dsl::day(value)
                .ok_or_else(|| format!("`{value}` is not a date: use YYYY-MM-DD, today, tomorrow, yesterday or none"))?;
            Ok(Filter::Date { field, cmp, day: Some(day) })
        }
        "" => Err(format!("`{line}` names no field: use {FIELDS}")),
        other => Err(format!("unknown field `{other}`: use {FIELDS}")),
    }
}

/// Task documents and checkbox lines side by side, in the columns a `tasks`
/// block filters and sorts by. `line` is NULL for a document.
const ROWS: &str = "
    SELECT d.id AS file_id, d.path, NULL AS line, NULL AS raw, d.title AS text, d.status,
           d.created, d.due, d.tags, d.doc_id
    FROM v_documents d WHERE d.type = 'task'
    UNION ALL
    SELECT t.file_id, t.path, t.line, t.raw, t.text,
           CASE WHEN t.is_done THEN 'done' WHEN t.state = '-' THEN 'cancelled' ELSE coalesce(t.status, 'todo') END,
           NULL, t.due,
           (SELECT group_concat(tag, ',') FROM v_tags g WHERE g.file_id = t.file_id AND g.line = t.line),
           NULL
    FROM v_tasks t WHERE 1@TRACKED@";

/// Runs a `tasks` block. Date words are relative to `today`.
pub fn run_tasks(db: &Connection, query: &TaskQuery, today: NaiveDate) -> rusqlite::Result<TaskList> {
    let mut params: Vec<Value> = Vec::new();
    let mut conds = vec!["1".to_owned()];
    for filter in &query.filters {
        conds.push(match filter {
            // IS and IS NOT treat a missing status as a value of its own, so
            // `status != done` keeps tasks without one.
            Filter::Status { status, equal } => {
                params.push(Value::Text(status.slug().to_owned()));
                format!("x.status {} ?{}", if *equal { "IS" } else { "IS NOT" }, params.len())
            }
            Filter::Date { field, cmp, day } => {
                let column = match field {
                    DateField::Due => "x.due",
                    DateField::Created => "x.created",
                };
                match (cmp, day) {
                    (DateOp::NotEqual, None) => format!("{column} IS NOT NULL"),
                    (_, None) => format!("{column} IS NULL"),
                    (DateOp::NotEqual, Some(day)) => {
                        params.push(Value::Text(day.iso(today)));
                        format!("{column} IS NOT ?{}", params.len())
                    }
                    (DateOp::Cmp(cmp), Some(day)) => {
                        params.push(Value::Text(day.iso(today)));
                        format!("coalesce({column} {} ?{}, 0)", cmp.sql(), params.len())
                    }
                }
            }
            // A document's tags are its note's; a line's are those on it.
            Filter::Tag { tag, equal } => {
                params.push(Value::Text(tag.clone()));
                params.push(Value::Text(format!("{}/*", glob_escape(tag))));
                let (exact, nested) = (params.len() - 1, params.len());
                let not = if *equal { "" } else { "NOT " };
                format!(
                    "{not}EXISTS (SELECT 1 FROM v_tags g WHERE g.file_id = x.file_id \
                     AND (x.line IS NULL OR g.line = x.line) AND (g.tag = ?{exact} OR g.tag GLOB ?{nested}))"
                )
            }
            Filter::Folder { folder, equal } => {
                params.push(Value::Text(format!("{}/", folder.to_lowercase())));
                let n = params.len();
                let not = if *equal { "" } else { "NOT " };
                format!("{not}(substr(lower(x.path), 1, length(?{n})) = ?{n})")
            }
        });
    }

    let mut order: Vec<String> = query
        .sort
        .iter()
        .map(|key| {
            let column = match key.field {
                SortField::Due => "x.due".to_owned(),
                SortField::Created => "x.created".to_owned(),
                SortField::Title => "x.text COLLATE NOCASE".to_owned(),
                SortField::Id => "x.doc_id COLLATE NOCASE".to_owned(),
                // In the order a task goes through them. The slugs are ours, not the note's.
                SortField::Status => {
                    let arms: String = TaskStatus::ALL
                        .iter()
                        .enumerate()
                        .map(|(i, s)| format!(" WHEN '{}' THEN {i}", s.slug()))
                        .collect();
                    format!("CASE x.status{arms} END")
                }
            };
            format!("{column} {} NULLS LAST", if key.descending { "DESC" } else { "ASC" })
        })
        .collect();
    if query.sort.is_empty() {
        order.push("x.due ASC NULLS LAST".into());
    }
    order.push("x.text COLLATE NOCASE, x.path, x.line".into());
    params.push(Value::Integer(query.limit as i64 + 1));

    let rows = ROWS.replace("@TRACKED@", sage_db::tracked_clause());
    let sql = format!(
        "SELECT x.path, x.line, x.raw, x.text, x.status, x.created, x.due, x.tags FROM ({rows}) x \
         WHERE {} ORDER BY {} LIMIT ?{}",
        conds.join(" AND "),
        order.join(", "),
        params.len()
    );
    let mut rows = db
        .prepare(&sql)?
        .query_map(params_from_iter(params), |r| {
            let line: Option<u32> = r.get(1)?;
            let raw: Option<String> = r.get(2)?;
            let text: String = r.get(3)?;
            let tags: Option<String> = r.get(7)?;
            Ok(TaskItem {
                path: r.get(0)?,
                // A line's dates are shown on their own.
                title: if line.is_some() { sage_core::task_display(&text) } else { text },
                status: r.get(4)?,
                created: r.get(5)?,
                due: r.get(6)?,
                tags: tags.map(|t| t.split(',').map(str::to_owned).collect()).unwrap_or_default(),
                line: line.zip(raw),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let truncated = rows.len() > query.limit;
    rows.truncate(query.limit);
    Ok(TaskList { rows, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_filters_sorts_and_limits() {
        let q = parse_tasks("status != done\n\nsort due\nsort status desc\ndue<=tomorrow\ntag = #Backend\nlimit 5").unwrap();
        assert_eq!(
            q.filters,
            [
                Filter::Status { status: TaskStatus::Done, equal: false },
                Filter::Date { field: DateField::Due, cmp: DateOp::Cmp(Cmp::Le), day: Some(Day::Relative(1)) },
                Filter::Tag { tag: "backend".into(), equal: true },
            ]
        );
        assert_eq!(
            q.sort,
            [
                SortKey { field: SortField::Due, descending: false },
                SortKey { field: SortField::Status, descending: true }
            ]
        );
        assert_eq!(q.limit, 5);
        assert_eq!(parse_tasks("status = In Progress").unwrap().filters, [Filter::Status { status: TaskStatus::InProgress, equal: true }]);
        assert_eq!(
            parse_tasks("due = none").unwrap().filters,
            [Filter::Date { field: DateField::Due, cmp: DateOp::Cmp(Cmp::Eq), day: None }]
        );
        assert_eq!(parse_tasks("").unwrap().filters, []);
    }

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

    fn task(id: &str, status: &str, due: Option<&str>, tags: &str) -> String {
        let due = due.map(|d| format!("due: {d}\n")).unwrap_or_default();
        format!("---\nid: {id}\ntype: task\ntitle: Task {id}\nstatus: {status}\ncreated: 2026-11-01\n{due}tags: [{tags}]\n---\nBody of {id}.\n")
    }

    #[test]
    fn runs_against_the_index() {
        let (_dir, db) = indexed(&[
            ("Tasks/a.md", &task("a", "in-progress", Some("2026-11-30"), "backend, voice")),
            ("Tasks/b.md", &task("b", "todo", Some("2026-11-20"), "frontend")),
            ("Tasks/c.md", &task("c", "done", Some("2026-11-10"), "backend")),
            ("Tasks/d.md", &task("d", "todo", None, "backend/db")),
            ("Projects/p.md", "---\ntype: project\ntitle: Voice\nstatus: todo\n---\n"),
            ("Plain.md", "---\nstatus: todo\n---\nNot a document.\n"),
        ]);
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let ids = |src: &str| -> Vec<String> {
            let query = parse_tasks(src).unwrap();
            run_tasks(&db, &query, today).unwrap().rows.into_iter().map(|d| d.title.replace("Task ", "")).collect()
        };
        assert_eq!(ids("status != done\nsort due"), ["b", "a", "d"], "no due date last; projects and plain notes left out");
        assert_eq!(ids("sort due desc"), ["a", "b", "c", "d"]);
        assert_eq!(ids("due < today"), ["c"]);
        assert_eq!(ids("due = none"), ["d"]);
        assert_eq!(ids("tag = backend\nsort id"), ["a", "c", "d"], "nested tags count");
        assert_eq!(ids("tag != backend"), ["b"]);
        assert_eq!(ids("sort status\nsort id"), ["b", "d", "a", "c"]);
        assert_eq!(ids("status = in-progress\ndue >= 2026-11-30\ncreated = 2026-11-01"), ["a"]);
        assert_eq!(ids("folder = tasks\nsort id"), ["a", "b", "c", "d"], "folder ignores case");
        assert_eq!(ids("folder != Tasks"), Vec::<String>::new());
        assert_eq!(ids("folder = TASKS\nsort id"), ["a", "b", "c", "d"]);
        let list = run_tasks(&db, &parse_tasks("limit 2").unwrap(), today).unwrap();
        assert_eq!((list.rows.len(), list.truncated), (2, true));
        assert_eq!(list.rows[0].line, None, "documents have no line");

        let a = document(&db, "Tasks/a.md").unwrap().unwrap();
        assert_eq!(a.title, "Task a");
        assert_eq!(a.task_status(), Some(TaskStatus::InProgress));
        assert_eq!(a.tags, ["backend", "voice"]);
        assert_eq!(a.due.as_deref(), Some("2026-11-30"));
        assert!(document(&db, "Projects/p.md").unwrap().is_some_and(|p| !p.is_task()));
        assert_eq!(document(&db, "Plain.md").unwrap(), None);
    }

    #[test]
    fn a_folder_matches_in_any_case_beyond_ascii() {
        let (_dir, db) = indexed(&[("Środa/a.md", &task("a", "todo", None, "x"))]);
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        for src in ["folder = Środa", "folder = środa", "folder = ŚRODA"] {
            assert_eq!(run_tasks(&db, &parse_tasks(src).unwrap(), today).unwrap().rows.len(), 1, "{src}");
        }
    }

    #[test]
    fn checkbox_lines_are_tasks_too() {
        let (_dir, db) = indexed(&[
            ("Tasks/a.md", &task("a", "todo", Some("2026-11-30"), "backend")),
            (
                "Daily/2026-11-16.md",
                "# Today\n- [ ] Call vendor 📅 2026-11-18 #backend\n- [x] Ship it ✅ 2026-11-15\n\
                 - [-] Dropped idea\n- [ ] Started #status/in-progress #voice\n",
            ),
        ]);
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let rows = |src: &str| run_tasks(&db, &parse_tasks(src).unwrap(), today).unwrap().rows;
        let titles = |src: &str| -> Vec<String> { rows(src).into_iter().map(|r| r.title).collect() };

        let open = rows("status != done\nstatus != cancelled");
        assert_eq!(
            open.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            ["Call vendor #backend", "Task a", "Started #status/in-progress #voice"]
        );
        assert_eq!(open[0].line, Some((1, "- [ ] Call vendor 📅 2026-11-18 #backend".into())), "what a checkbox writes back to");
        assert_eq!(open[0].due.as_deref(), Some("2026-11-18"));
        assert_eq!(open[2].task_status(), Some(TaskStatus::InProgress), "from its #status/ tag");
        assert_eq!(titles("status = done"), ["Ship it"]);
        assert_eq!(titles("status = cancelled"), ["Dropped idea"]);
        assert_eq!(titles("tag = voice"), ["Started #status/in-progress #voice"], "a line's own tags");
        assert_eq!(titles("folder = Daily\ntag = backend"), ["Call vendor #backend"]);
        assert_eq!(titles("created != none"), ["Task a"], "lines have no created date");
    }

    #[test]
    fn errors_name_the_line_and_the_problem() {
        let err = |src: &str| parse_tasks(src).unwrap_err();
        assert_eq!(err("status != done\npriority = high"), "line 2: unknown field `priority`: use status, due, created, tag or folder");
        assert!(err("status = blocked").starts_with("line 1: unknown status `blocked`"));
        assert!(err("status < done").contains("= or != only"));
        assert!(err("due < soon").contains("is not a date"));
        assert!(err("due < none").contains("= or != only"));
        assert!(err("sort priority").contains("cannot sort by `priority`"));
        assert!(err("limit 0").contains("1 to 1000"));
        assert!(err("not done").contains("is not a filter"));
    }
}
