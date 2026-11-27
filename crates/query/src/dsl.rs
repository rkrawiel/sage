//! The query language of `view` blocks, modelled on Obsidian's search.
//!
//! | Syntax | Matches |
//! |---|---|
//! | `word`, `"a phrase"`, `content:word` | notes whose text contains it; a word also matches as a prefix |
//! | `file:name`, `path:Projects/` | file name or vault path contains, ignoring case |
//! | `tag:#project` | notes with the tag, or one nested under it |
//! | `[status]`, `[status:active]`, `[estimate>3]`, `[due<today]` | frontmatter properties: present, equal, compared; `today`, `tomorrow` and `yesterday` stand for their dates |
//! | `task:text`, `task-todo:text`, `task-done:text`, `task-cancelled:text` | tasks containing text: any, open, done, cancelled (`[-]`); the text may be empty |
//! | `status:in-progress`, `status:none` | tasks by their `#status/` tag |
//! | `due<today`, `scheduled:2026-12-01`, `start>=tomorrow`, `done:yesterday` | task dates; also `due:none`, `due:any` |
//! | `priority>=high`, `priority:none` | lowest, low, medium, high, highest, or 1 to 5 |
//! | `kind:document`, `kind:checkbox` | task documents (`type: task`), or checkbox lines |
//! | `a b`, `a OR b`, `-a`, `( … )` | and, or, not, grouping |
//!
//! Task filters in a notes table match notes that have at least one such
//! checkbox task. Task lists and boards hold task documents as well as
//! checkbox lines; `kind:` tells them apart. They leave cancelled tasks out
//! unless the query asks for them with `task-cancelled:`. `none` and `any`
//! ignore case.

use chrono::NaiveDate;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Note(NoteFilter),
    Task(TaskFilter),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NoteFilter {
    /// Full text. `phrase` keeps the words together; a single word matches as a prefix.
    Text { text: String, phrase: bool },
    File(String),
    Path(String),
    /// Lowercase, without `#`.
    Tag(String),
    /// Lowercase key; `None` asks only that the property is present.
    Property { key: String, cmp: Option<(Cmp, String)> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskFilter {
    Text { text: String, state: TaskState },
    /// A `#status/` slug, `None` for tasks without one.
    Status(Option<String>),
    Date { field: DateField, cmp: DateCmp },
    /// 5 highest .. 1 lowest, `None` for tasks without a priority.
    Priority(Option<(Cmp, u8)>),
    /// Task documents when true, checkbox lines when false.
    Document(bool),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskState {
    Any,
    Open,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DateField {
    Due,
    Scheduled,
    Start,
    Done,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DateCmp {
    None,
    Any,
    Cmp(Cmp, Day),
}

/// A date as written: absolute, or relative to the day the view runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Day {
    Date(NaiveDate),
    /// Days from today: `today` is 0, `tomorrow` 1, `yesterday` -1.
    Relative(i64),
}

impl Day {
    /// `YYYY-MM-DD`, the form task dates are stored in.
    pub fn iso(self, today: NaiveDate) -> String {
        let date = match self {
            Day::Date(date) => date,
            Day::Relative(days) => chrono::TimeDelta::try_days(days)
                .and_then(|delta| today.checked_add_signed(delta))
                .unwrap_or(today),
        };
        date.format("%Y-%m-%d").to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmp {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    pub fn sql(self) -> &'static str {
        match self {
            Cmp::Eq => "=",
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub message: String,
    /// Character offset in the query.
    pub at: usize,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (column {})", self.message, self.at + 1)
    }
}

fn error<T>(message: impl Into<String>, at: usize) -> Result<T, Error> {
    Err(Error { message: message.into(), at })
}

/// Parses a query. An empty query matches everything.
pub fn parse(query: &str) -> Result<Expr, Error> {
    let tokens = lex(query)?;
    let mut parser = Parser { tokens, pos: 0, end: query.chars().count(), depth: 0 };
    let expr = parser.or()?;
    match parser.tokens.get(parser.pos) {
        Some((Tok::Close, at)) => error("`)` without a matching `(`", *at),
        Some((_, at)) => error("unexpected input", *at),
        None => Ok(expr),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Open,
    Close,
    Or,
    Not,
    /// A bare word or `key:value`, with quotes around a value removed.
    Term(String),
    Phrase(String),
    /// The inside of `[...]`.
    Property(String),
}

fn lex(query: &str) -> Result<Vec<(Tok, usize)>, Error> {
    let chars: Vec<char> = query.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    // Reads up to the closing quote; `i` points at the opening one.
    let quoted = |i: &mut usize, out: &mut String| -> Result<(), Error> {
        let open = *i;
        *i += 1;
        while *i < chars.len() && chars[*i] != '"' {
            out.push(chars[*i]);
            *i += 1;
        }
        if *i == chars.len() {
            return error("unclosed `\"`", open);
        }
        *i += 1;
        Ok(())
    };
    while i < chars.len() {
        let c = chars[i];
        let at = i;
        match c {
            c if c.is_whitespace() => i += 1,
            '(' => {
                tokens.push((Tok::Open, at));
                i += 1;
            }
            ')' => {
                tokens.push((Tok::Close, at));
                i += 1;
            }
            '-' if chars.get(i + 1).is_some_and(|n| !n.is_whitespace()) => {
                tokens.push((Tok::Not, at));
                i += 1;
            }
            '"' => {
                let mut phrase = String::new();
                quoted(&mut i, &mut phrase)?;
                tokens.push((Tok::Phrase(phrase), at));
            }
            '[' => {
                let mut inner = String::new();
                i += 1;
                while i < chars.len() && chars[i] != ']' {
                    if chars[i] == '"' {
                        quoted(&mut i, &mut inner)?;
                    } else {
                        inner.push(chars[i]);
                        i += 1;
                    }
                }
                if i == chars.len() {
                    return error("unclosed `[`", at);
                }
                i += 1;
                tokens.push((Tok::Property(inner), at));
            }
            _ => {
                let mut term = String::new();
                while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '(' | ')') {
                    // A quoted value: `path:"Daily notes"`.
                    if chars[i] == '"' && !term.is_empty() {
                        quoted(&mut i, &mut term)?;
                    } else {
                        term.push(chars[i]);
                        i += 1;
                    }
                }
                tokens.push((if term == "OR" { Tok::Or } else { Tok::Term(term) }, at));
            }
        }
    }
    Ok(tokens)
}

/// How deep `(` and `-` may nest. The parser and the SQL compiler recurse once
/// per level, and a query of thousands of `(` would otherwise overflow the
/// 1 MB main-thread stack on Windows.
const MAX_DEPTH: usize = 64;

struct Parser {
    tokens: Vec<(Tok, usize)>,
    pos: usize,
    end: usize,
    /// Groups and negations open around the current token.
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|(tok, _)| tok)
    }

    fn at(&self) -> usize {
        self.tokens.get(self.pos).map_or(self.end, |(_, at)| *at)
    }

    fn or(&mut self) -> Result<Expr, Error> {
        let mut terms = vec![self.and()?];
        while self.peek() == Some(&Tok::Or) {
            self.pos += 1;
            if matches!(self.peek(), None | Some(Tok::Close | Tok::Or)) {
                return error("`OR` needs something after it", self.at());
            }
            terms.push(self.and()?);
        }
        Ok(if terms.len() == 1 { terms.remove(0) } else { Expr::Or(terms) })
    }

    fn and(&mut self) -> Result<Expr, Error> {
        let mut terms = Vec::new();
        while !matches!(self.peek(), None | Some(Tok::Close | Tok::Or)) {
            terms.push(self.unary()?);
        }
        if terms.is_empty() && self.peek() == Some(&Tok::Or) {
            return error("`OR` needs something before it", self.at());
        }
        Ok(if terms.len() == 1 { terms.remove(0) } else { Expr::And(terms) })
    }

    fn unary(&mut self) -> Result<Expr, Error> {
        let Some((tok, at)) = self.tokens.get(self.pos).cloned() else {
            return error("unexpected end of the query", self.end);
        };
        self.pos += 1;
        match tok {
            Tok::Not | Tok::Open if self.depth >= MAX_DEPTH => error("groups and negations nest too deeply", at),
            Tok::Not => {
                self.depth += 1;
                let inner = self.unary()?;
                self.depth -= 1;
                Ok(Expr::Not(Box::new(inner)))
            }
            Tok::Open => {
                self.depth += 1;
                let inner = self.or()?;
                self.depth -= 1;
                if self.peek() != Some(&Tok::Close) {
                    return error("`(` without a matching `)`", at);
                }
                self.pos += 1;
                Ok(inner)
            }
            Tok::Phrase(text) if text.trim().is_empty() => error("empty phrase", at),
            Tok::Phrase(text) => Ok(Expr::Note(NoteFilter::Text { text, phrase: true })),
            Tok::Property(inner) => property(&inner, at),
            Tok::Term(term) => term_filter(&term, at),
            Tok::Close | Tok::Or => error("unexpected input", at),
        }
    }
}

/// Splits `key<op>value` at the first operator. `ops` lists the operators the
/// key may use, longest first.
fn split_op<'a>(term: &'a str, ops: &[(&str, Cmp)]) -> Option<(&'a str, Cmp, &'a str)> {
    let first = term.find([':', '<', '>', '='])?;
    let (key, rest) = term.split_at(first);
    ops.iter()
        .find(|(op, _)| rest.starts_with(op))
        .map(|(op, cmp)| (key, *cmp, &rest[op.len()..]))
}

const ALL_OPS: [(&str, Cmp); 6] =
    [("<=", Cmp::Le), (">=", Cmp::Ge), (":", Cmp::Eq), ("=", Cmp::Eq), ("<", Cmp::Lt), (">", Cmp::Gt)];

fn term_filter(term: &str, at: usize) -> Result<Expr, Error> {
    let Some((key, cmp, value)) = split_op(term, &ALL_OPS) else {
        return Ok(Expr::Note(NoteFilter::Text { text: term.to_owned(), phrase: false }));
    };
    let key = key.to_lowercase();
    // The value is the end of the term; with quotes removed this can be one
    // column before the value, at its opening quote.
    let at_value = at + term.chars().count() - value.chars().count();
    let text_only = |what: &str| {
        if cmp != Cmp::Eq {
            return error(format!("`{key}` takes `:`, not a comparison"), at);
        }
        if value.is_empty() && what != "task" {
            return error(format!("`{key}:` needs a value"), at_value);
        }
        Ok(())
    };
    let note = |filter| Ok(Expr::Note(filter));
    let task = |filter| Ok(Expr::Task(filter));
    match key.as_str() {
        "content" => {
            text_only("text")?;
            note(NoteFilter::Text { text: value.to_owned(), phrase: value.contains(' ') })
        }
        "file" => text_only("text").and_then(|()| note(NoteFilter::File(value.to_owned()))),
        "path" => text_only("text").and_then(|()| note(NoteFilter::Path(value.to_owned()))),
        "tag" => {
            text_only("tag")?;
            let tag = value.trim_start_matches('#').to_lowercase();
            if tag.is_empty() {
                return error("`tag:` needs a tag", at_value);
            }
            note(NoteFilter::Tag(tag))
        }
        "task" | "task-todo" | "task-done" | "task-cancelled" => {
            text_only("task")?;
            let state = match key.as_str() {
                "task-todo" => TaskState::Open,
                "task-done" => TaskState::Done,
                "task-cancelled" => TaskState::Cancelled,
                _ => TaskState::Any,
            };
            task(TaskFilter::Text { text: value.to_owned(), state })
        }
        "status" => {
            text_only("status")?;
            let slug = sage_core::status_slug(value);
            task(TaskFilter::Status((!value.eq_ignore_ascii_case("none")).then_some(slug)))
        }
        "due" | "scheduled" | "start" | "done" => {
            let field = match key.as_str() {
                "due" => DateField::Due,
                "scheduled" => DateField::Scheduled,
                "start" => DateField::Start,
                _ => DateField::Done,
            };
            let cmp = match (value.to_lowercase().as_str(), cmp) {
                ("none", Cmp::Eq) => DateCmp::None,
                ("any", Cmp::Eq) => DateCmp::Any,
                (_, cmp) => match day(value) {
                    Some(day) => DateCmp::Cmp(cmp, day),
                    None => {
                        return error(
                            format!("`{value}` is not a date: use YYYY-MM-DD, today, tomorrow or yesterday"),
                            at_value,
                        )
                    }
                },
            };
            task(TaskFilter::Date { field, cmp })
        }
        "priority" => {
            if value.eq_ignore_ascii_case("none") && cmp == Cmp::Eq {
                return task(TaskFilter::Priority(None));
            }
            match priority(value) {
                Some(level) => task(TaskFilter::Priority(Some((cmp, level)))),
                None => error(
                    format!("`{value}` is not a priority: use lowest, low, medium, high, highest or 1 to 5"),
                    at_value,
                ),
            }
        }
        "kind" => {
            text_only("kind")?;
            match value.to_lowercase().as_str() {
                "document" | "documents" => task(TaskFilter::Document(true)),
                "checkbox" | "checkboxes" => task(TaskFilter::Document(false)),
                _ => error(format!("`{value}` is not a kind of task: use document or checkbox"), at_value),
            }
        }
        // `10:30`, `https://...` and other words with a colon in them.
        _ => Ok(Expr::Note(NoteFilter::Text { text: term.to_owned(), phrase: false })),
    }
}

fn property(inner: &str, at: usize) -> Result<Expr, Error> {
    let inner = inner.trim();
    let (key, cmp) = match split_op(inner, &ALL_OPS) {
        Some((key, cmp, value)) => (key, Some((cmp, value.trim().to_owned()))),
        None => (inner, None),
    };
    let key = key.trim().to_lowercase();
    if key.is_empty() {
        return error("a property needs a name, like [status:active]", at);
    }
    if cmp.as_ref().is_some_and(|(_, value)| value.is_empty()) {
        return error(format!("[{key}] needs a value after the operator"), at);
    }
    Ok(Expr::Note(NoteFilter::Property { key, cmp }))
}

pub(crate) fn day(value: &str) -> Option<Day> {
    match value.to_lowercase().as_str() {
        "today" => Some(Day::Relative(0)),
        "tomorrow" => Some(Day::Relative(1)),
        "yesterday" => Some(Day::Relative(-1)),
        value => NaiveDate::parse_from_str(value, "%Y-%m-%d").ok().map(Day::Date),
    }
}

fn priority(value: &str) -> Option<u8> {
    match value.to_lowercase().as_str() {
        "highest" => Some(5),
        "high" => Some(4),
        "medium" => Some(3),
        "low" => Some(2),
        "lowest" => Some(1),
        value => value.parse().ok().filter(|n| (1..=5).contains(n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Expr {
        Expr::Note(NoteFilter::Text { text: t.into(), phrase: false })
    }

    #[test]
    fn obsidian_style_queries() {
        assert_eq!(parse("").unwrap(), Expr::And(vec![]));
        assert_eq!(parse("tag:#Project/Alpha").unwrap(), Expr::Note(NoteFilter::Tag("project/alpha".into())));
        assert_eq!(
            parse("meeting -path:\"Daily notes\" (file:plan OR \"next step\")").unwrap(),
            Expr::And(vec![
                text("meeting"),
                Expr::Not(Box::new(Expr::Note(NoteFilter::Path("Daily notes".into())))),
                Expr::Or(vec![
                    Expr::Note(NoteFilter::File("plan".into())),
                    Expr::Note(NoteFilter::Text { text: "next step".into(), phrase: true }),
                ]),
            ])
        );
        assert_eq!(
            parse("[Status:\"in review\"] [estimate>=3] [draft]").unwrap(),
            Expr::And(vec![
                Expr::Note(NoteFilter::Property { key: "status".into(), cmp: Some((Cmp::Eq, "in review".into())) }),
                Expr::Note(NoteFilter::Property { key: "estimate".into(), cmp: Some((Cmp::Ge, "3".into())) }),
                Expr::Note(NoteFilter::Property { key: "draft".into(), cmp: None }),
            ])
        );
        assert_eq!(text("10:30"), parse("10:30").unwrap(), "a word with a colon in it");
    }

    #[test]
    fn task_filters() {
        assert_eq!(
            parse("task-todo: status:\"In Progress\" due<=tomorrow priority>=high").unwrap(),
            Expr::And(vec![
                Expr::Task(TaskFilter::Text { text: String::new(), state: TaskState::Open }),
                Expr::Task(TaskFilter::Status(Some("in-progress".into()))),
                Expr::Task(TaskFilter::Date { field: DateField::Due, cmp: DateCmp::Cmp(Cmp::Le, Day::Relative(1)) }),
                Expr::Task(TaskFilter::Priority(Some((Cmp::Ge, 4)))),
            ])
        );
        assert_eq!(
            parse("done:2026-11-10 scheduled:none status:none").unwrap(),
            Expr::And(vec![
                Expr::Task(TaskFilter::Date {
                    field: DateField::Done,
                    cmp: DateCmp::Cmp(Cmp::Eq, Day::Date(NaiveDate::from_ymd_opt(2026, 11, 10).unwrap())),
                }),
                Expr::Task(TaskFilter::Date { field: DateField::Scheduled, cmp: DateCmp::None }),
                Expr::Task(TaskFilter::Status(None)),
            ])
        );
    }

    #[test]
    fn grouping_and_negation() {
        let open = Expr::Task(TaskFilter::Text { text: String::new(), state: TaskState::Open });
        assert_eq!(
            parse("-(a OR b) --c").unwrap(),
            Expr::And(vec![
                Expr::Not(Box::new(Expr::Or(vec![text("a"), text("b")]))),
                Expr::Not(Box::new(Expr::Not(Box::new(text("c"))))),
            ])
        );
        assert_eq!(parse("()").unwrap(), Expr::And(vec![]), "an empty group matches everything");
        assert_eq!(parse("well-known").unwrap(), text("well-known"), "`-` negates only at the start of a term");
        assert_eq!(parse("TASK-TODO:").unwrap(), open, "keys ignore case");
        assert_eq!(parse("PRIORITY:High").unwrap(), Expr::Task(TaskFilter::Priority(Some((Cmp::Eq, 4)))));
        assert_eq!(parse("priority:none").unwrap(), Expr::Task(TaskFilter::Priority(None)));
        assert_eq!(parse("kind:Document").unwrap(), Expr::Task(TaskFilter::Document(true)));
        assert_eq!(
            parse("-kind:checkbox").unwrap(),
            Expr::Not(Box::new(Expr::Task(TaskFilter::Document(false))))
        );
        assert_eq!(parse("due:any").unwrap(), Expr::Task(TaskFilter::Date { field: DateField::Due, cmp: DateCmp::Any }));
        assert_eq!(
            parse("[time:10:30]").unwrap(),
            Expr::Note(NoteFilter::Property { key: "time".into(), cmp: Some((Cmp::Eq, "10:30".into())) }),
            "a property splits at its first operator"
        );
    }

    #[test]
    fn errors_point_at_the_problem() {
        let at = |q: &str| parse(q).unwrap_err().at;
        assert_eq!(at("a (b"), 2);
        assert_eq!(at("a b)"), 3);
        assert_eq!(at("due<soon"), 4);
        assert_eq!(at("due<=soon"), 5);
        assert_eq!(at("OR a"), 0);
        assert_eq!(at("tag:"), 4);
        assert_eq!(at("x OR"), 4);
        assert_eq!(at("[ :x]"), 0);
        assert_eq!(at("path:\"open"), 5);
        assert!(parse("file<x").unwrap_err().message.contains("takes `:`"));
        assert_eq!(at("kind:note"), 5);
        assert!(parse("kind:note").unwrap_err().message.contains("use document or checkbox"));
    }

    #[test]
    fn very_deep_nesting_is_an_error_not_a_stack_overflow() {
        for query in [format!("{}a{}", "(".repeat(10_000), ")".repeat(10_000)), format!("{}a", "-".repeat(10_000))] {
            let err = parse(&query).unwrap_err();
            assert_eq!((err.message.as_str(), err.at), ("groups and negations nest too deeply", 64));
        }
        let ok = format!("{}a{}", "(".repeat(64), ")".repeat(64));
        assert_eq!(parse(&ok).unwrap(), text("a"));
    }
}
