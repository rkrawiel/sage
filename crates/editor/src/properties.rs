//! Line-oriented editing of a note's frontmatter properties, for the live
//! preview's property widget. It reads the top-level `key: value` entries the
//! widget can edit (text, booleans, lists) and writes one entry back as lines,
//! leaving every other line of the frontmatter alone. Anything more involved,
//! such as nested maps, block scalars or values with comments, is `Raw`: shown,
//! never rewritten. Lines are the frontmatter's own, without `---` fences or
//! line endings.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A scalar. `quote` is how the file writes it, kept when it is rewritten.
    Text { text: String, quote: Option<char> },
    Bool(bool),
    /// `[a, b]`, or `- a` lines when `block`.
    List { items: Vec<String>, block: bool },
    /// Not editable here: its lines, as written.
    Raw(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    /// The lines the entry takes, within the frontmatter's lines.
    pub lines: Range<usize>,
    pub value: Value,
}

/// The kinds of property Obsidian offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropType {
    Text,
    List,
    Number,
    Checkbox,
    Date,
    DateTime,
}

impl PropType {
    pub const ALL: [PropType; 6] = [PropType::Text, PropType::List, PropType::Number, PropType::Checkbox, PropType::Date, PropType::DateTime];

    pub fn label(self) -> &'static str {
        match self {
            PropType::Text => "Text",
            PropType::List => "List",
            PropType::Number => "Number",
            PropType::Checkbox => "Checkbox",
            PropType::Date => "Date",
            PropType::DateTime => "Date & time",
        }
    }

    /// The name stored in `.sage/settings.json`.
    pub fn id(self) -> &'static str {
        match self {
            PropType::Text => "text",
            PropType::List => "list",
            PropType::Number => "number",
            PropType::Checkbox => "checkbox",
            PropType::Date => "date",
            PropType::DateTime => "datetime",
        }
    }

    /// From either Sage's names or Obsidian's `types.json` ones.
    pub fn from_id(id: &str) -> Option<PropType> {
        Some(match id {
            "text" => PropType::Text,
            "list" | "multitext" | "tags" | "aliases" => PropType::List,
            "number" => PropType::Number,
            "checkbox" => PropType::Checkbox,
            "date" => PropType::Date,
            "datetime" => PropType::DateTime,
            _ => return None,
        })
    }
}

impl std::fmt::Display for PropType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

fn digits(s: &str, n: usize) -> bool {
    s.len() >= n && s.as_bytes()[..n].iter().all(u8::is_ascii_digit)
}

/// `YYYY-MM-DD` at the start of `s`.
fn starts_with_date(s: &str) -> bool {
    let b = s.as_bytes();
    s.len() >= 10 && digits(s, 4) && b[4] == b'-' && digits(&s[5..], 2) && b[7] == b'-' && digits(&s[8..], 2)
}

/// The type `value` shows as: the one chosen for the property's name
/// when the value can be that, else what the value looks like.
pub fn type_of(value: &Value, chosen: Option<PropType>) -> Option<PropType> {
    let fits = |t: PropType| match (t, value) {
        (_, Value::Raw(_)) => false,
        (PropType::Checkbox, v) => matches!(v, Value::Bool(_)),
        (PropType::List, v) => matches!(v, Value::List { .. }),
        (_, v) => matches!(v, Value::Text { .. }),
    };
    if matches!(value, Value::Raw(_)) {
        return None;
    }
    if let Some(t) = chosen.filter(|&t| fits(t)) {
        return Some(t);
    }
    Some(match value {
        Value::Bool(_) => PropType::Checkbox,
        Value::List { .. } => PropType::List,
        Value::Text { text, .. } if !text.is_empty() && starts_with_date(text) => {
            if text.len() > 10 { PropType::DateTime } else { PropType::Date }
        }
        Value::Text { text, .. } if !text.is_empty() && text.parse::<f64>().is_ok() => PropType::Number,
        _ => PropType::Text,
    })
}

/// `value` as a value of type `to`, keeping what can be kept.
pub fn convert(value: &Value, to: PropType) -> Value {
    let plain = |text: String| Value::Text { text, quote: None };
    let as_text = match value {
        Value::Text { text, .. } => text.clone(),
        Value::Bool(b) => b.to_string(),
        Value::List { items, .. } => items.join(", "),
        Value::Raw(_) => return value.clone(),
    };
    match to {
        PropType::Text => plain(as_text),
        PropType::List => match value {
            Value::List { .. } => value.clone(),
            _ => Value::List { items: if as_text.trim().is_empty() { Vec::new() } else { vec![as_text] }, block: false },
        },
        PropType::Checkbox => Value::Bool(as_text.trim().eq_ignore_ascii_case("true")),
        PropType::Number => plain(if as_text.trim().parse::<f64>().is_ok() { as_text.trim().to_owned() } else { String::new() }),
        PropType::Date => plain(if starts_with_date(&as_text) { as_text[..10].to_owned() } else { String::new() }),
        PropType::DateTime => plain(match as_text.trim() {
            t if starts_with_date(t) && t.len() > 10 => t.to_owned(),
            t if starts_with_date(t) => format!("{}T00:00", &t[..10]),
            _ => String::new(),
        }),
    }
}

/// The top-level entries of the frontmatter's `lines`.
pub fn parse(lines: &[String]) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].as_str();
        let Some((key, rest)) = split_key(line) else {
            i += 1;
            continue;
        };
        let mut end = i + 1;
        while end < lines.len() && is_continuation(&lines[end]) {
            end += 1;
        }
        // Trailing blank lines belong to nobody.
        while end > i + 1 && lines[end - 1].trim().is_empty() {
            end -= 1;
        }
        let body = &lines[i + 1..end];
        out.push(Entry { key: key.to_owned(), lines: i..end, value: value_of(rest, body) });
        i = end;
    }
    out
}

/// `key` and what follows the colon, for a line that starts an entry.
fn split_key(line: &str) -> Option<(&str, &str)> {
    if line.starts_with(|c: char| c.is_whitespace() || matches!(c, '#' | '-' | '?' | '[' | '{' | '"' | '\'' | '&' | '*' | '!' | '|' | '>' | '%' | '@')) {
        return None;
    }
    let colon = line.char_indices().find(|&(i, c)| c == ':' && line[i + 1..].is_empty() || c == ':' && line[i + 1..].starts_with([' ', '\t']))?.0;
    let key = line[..colon].trim_end();
    (!key.is_empty()).then(|| (key, &line[colon + 1..]))
}

fn is_continuation(line: &str) -> bool {
    line.is_empty() || line.starts_with([' ', '\t']) || line.starts_with("- ") || line == "-" || line.trim().is_empty()
}

fn value_of(rest: &str, body: &[String]) -> Value {
    let raw = |rest: &str| Value::Raw(std::iter::once(rest.trim()).chain(body.iter().map(|l| l.trim())).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n"));
    let value = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('\t')).unwrap_or(rest);
    if value.trim().is_empty() {
        if body.is_empty() {
            return Value::Text { text: String::new(), quote: None };
        }
        let mut items = Vec::new();
        for line in body {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            let Some(item) = t.strip_prefix("- ").or_else(|| (t == "-").then_some("")) else { return raw(rest) };
            match scalar(item.trim()) {
                Some((text, _)) => items.push(text),
                None => return raw(rest),
            }
        }
        return Value::List { items, block: true };
    }
    if !body.iter().all(|l| l.trim().is_empty()) {
        return raw(rest);
    }
    let v = value.trim_end();
    if v.starts_with('[') {
        return match v.strip_suffix(']').and_then(|inner| split_flow(&inner[1..])) {
            Some(items) => Value::List { items, block: false },
            None => raw(rest),
        };
    }
    match v {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    if v.starts_with(['|', '>', '{', '&', '*', '!', '%', '@', '`']) || (!v.starts_with(['"', '\'']) && v.contains(" #")) {
        return raw(rest);
    }
    match scalar(value) {
        // Only the end is trimmed in the original; spaces typed at the end of a value stay.
        Some((text, quote)) => Value::Text { text, quote },
        None => raw(rest),
    }
}

/// A scalar without its quotes, and the quote it had. `None` when quotes do not close.
fn scalar(s: &str) -> Option<(String, Option<char>)> {
    let mut chars = s.chars();
    match chars.next() {
        Some('"') => {
            let inner = s.strip_suffix('"').filter(|_| s.len() >= 2)?;
            let mut out = String::new();
            let mut it = inner[1..].chars();
            while let Some(c) = it.next() {
                if c == '\\' {
                    match it.next()? {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        other => out.push(other),
                    }
                } else if c == '"' {
                    return None;
                } else {
                    out.push(c);
                }
            }
            Some((out, Some('"')))
        }
        Some('\'') => {
            let inner = s.strip_suffix('\'').filter(|_| s.len() >= 2)?;
            let inner = &inner[1..];
            let text = inner.replace("''", "'");
            (!text.contains('\'') || inner.contains("''")).then_some((text, Some('\'')))
        }
        _ => Some((s.to_owned(), None)),
    }
}

/// The items of a flow sequence's inside, split at commas outside quotes.
fn split_flow(inner: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in inner.chars() {
        match quote {
            Some(q) => {
                current.push(c);
                if q == '"' && c == '\\' && !escaped {
                    escaped = true;
                    continue;
                }
                if c == q && !escaped {
                    quote = None;
                }
                escaped = false;
            }
            None => match c {
                ',' => parts.push(std::mem::take(&mut current)),
                '"' | '\'' => {
                    quote = Some(c);
                    current.push(c);
                }
                '[' | ']' | '{' | '}' => return None,
                _ => current.push(c),
            },
        }
    }
    if quote.is_some() {
        return None;
    }
    parts.push(current);
    let mut items = Vec::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        items.push(scalar(part)?.0);
    }
    Some(items)
}

/// Whether `text` must be quoted to read back as the same text.
fn needs_quote(text: &str) -> bool {
    text.is_empty()
        || text != text.trim_matches(' ')
        || text.starts_with(['[', ']', '{', '}', ',', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`', '#', '?', ':'])
        || text == "-"
        || text.starts_with("- ")
        || text.contains(": ")
        || text.contains(" #")
        || text.ends_with(':')
        || matches!(text, "true" | "false" | "null" | "~" | "True" | "False" | "Null" | "yes" | "no")
}

fn write_scalar(text: &str, quote: Option<char>, empty_ok: bool) -> String {
    if text.is_empty() && empty_ok && quote.is_none() {
        return String::new();
    }
    let quote = quote.or_else(|| (needs_quote(text) || text.contains(['\n', '\t'])).then_some('"'));
    match quote {
        Some('\'') if !text.contains(['\n', '\t']) => format!("'{}'", text.replace('\'', "''")),
        Some(_) => {
            let escaped = text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t");
            format!("\"{escaped}\"")
        }
        None => text.to_owned(),
    }
}

/// The lines that write `key` with `value`.
pub fn write(key: &str, value: &Value) -> Vec<String> {
    match value {
        Value::Text { text, quote } => {
            let scalar = write_scalar(text, *quote, true);
            vec![if scalar.is_empty() { format!("{key}:") } else { format!("{key}: {scalar}") }]
        }
        Value::Bool(b) => vec![format!("{key}: {b}")],
        Value::List { items, block: true } if !items.is_empty() => {
            let mut lines = vec![format!("{key}:")];
            lines.extend(items.iter().map(|i| format!("  - {}", write_scalar(i, None, false))));
            lines
        }
        Value::List { items, .. } => {
            // A comma inside an unquoted item would split it, so such items are quoted.
            let items: Vec<String> = items
                .iter()
                .map(|i| match write_scalar(i, None, false) {
                    s if s.contains(',') && !s.starts_with(['"', '\'']) => write_scalar(i, Some('"'), false),
                    s => s,
                })
                .collect();
            vec![format!("{key}: [{}]", items.join(", "))]
        }
        Value::Raw(_) => Vec::new(),
    }
}

/// The frontmatter's `lines` with entry `index` set to `value`, or `None` when
/// there is no such entry or it cannot be edited.
pub fn set(lines: &[String], index: usize, value: &Value) -> Option<(Range<usize>, Vec<String>)> {
    let entries = parse(lines);
    let entry = entries.get(index)?;
    if matches!(entry.value, Value::Raw(_)) {
        return None;
    }
    let replacement = write(&entry.key, value);
    (replacement != lines[entry.lines.clone()]).then_some((entry.lines.clone(), replacement))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fm(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    fn text(t: &str) -> Value {
        Value::Text { text: t.into(), quote: None }
    }

    #[test]
    fn reads_scalars_booleans_and_lists() {
        let entries = parse(&fm("title: My note\ndone: true\ntags: [a, \"b, c\"]\naliases:\n  - x\n  - 'y'\nempty:\nnested:\n  a: 1\n"));
        let values: Vec<_> = entries.iter().map(|e| (e.key.as_str(), e.value.clone())).collect();
        assert_eq!(values[0], ("title", text("My note")));
        assert_eq!(values[1], ("done", Value::Bool(true)));
        assert_eq!(values[2], ("tags", Value::List { items: vec!["a".into(), "b, c".into()], block: false }));
        assert_eq!(values[3], ("aliases", Value::List { items: vec!["x".into(), "y".into()], block: true }));
        assert_eq!(values[4], ("empty", text("")));
        assert!(matches!(values[5].1, Value::Raw(_)));
        assert_eq!(entries[3].lines, 3..6);
    }

    #[test]
    fn editing_one_entry_leaves_the_others() {
        let lines = fm("title: Old\ntags:\n  - a\ndue: 2024-01-01");
        let (range, new) = set(&lines, 1, &Value::List { items: vec!["a".into(), "b".into()], block: true }).unwrap();
        assert_eq!(range, 1..3);
        assert_eq!(new, ["tags:", "  - a", "  - b"]);
        let (range, new) = set(&lines, 0, &text("New")).unwrap();
        assert_eq!((range, new), (0..1, vec!["title: New".to_owned()]));
        assert!(set(&lines, 0, &text("Old")).is_none(), "nothing changed");
    }

    #[test]
    fn types_follow_the_choice_then_the_value() {
        assert_eq!(type_of(&text("2024-05-01"), None), Some(PropType::Date));
        assert_eq!(type_of(&text("2024-05-01T10:30"), None), Some(PropType::DateTime));
        assert_eq!(type_of(&text("42"), None), Some(PropType::Number));
        assert_eq!(type_of(&text(""), Some(PropType::Date)), Some(PropType::Date));
        assert_eq!(type_of(&text("hello"), Some(PropType::Checkbox)), Some(PropType::Text));
        assert_eq!(type_of(&Value::Raw("a".into()), Some(PropType::Text)), None);
    }

    #[test]
    fn converting_keeps_what_fits() {
        assert_eq!(convert(&text("2024-05-01T10:30"), PropType::Date), text("2024-05-01"));
        assert_eq!(convert(&text("2024-05-01"), PropType::DateTime), text("2024-05-01T00:00"));
        assert_eq!(convert(&text("abc"), PropType::Number), text(""));
        assert_eq!(convert(&text("true"), PropType::Checkbox), Value::Bool(true));
        assert_eq!(convert(&Value::Bool(true), PropType::Text), text("true"));
        assert_eq!(convert(&text("a"), PropType::List), Value::List { items: vec!["a".into()], block: false });
        assert_eq!(convert(&Value::List { items: vec!["a".into(), "b".into()], block: true }, PropType::Text), text("a, b"));
    }

    #[test]
    fn typed_values_read_back_as_typed() {
        for typed in ["plain", "a: b", "trailing ", " leading", "true", "#tag", "it's", "say \"hi\"", "- dash", "-5", "x #y", "ends:", ""] {
            let lines = write("k", &text(typed));
            let back = parse(&lines);
            assert_eq!(back.len(), 1, "{typed:?} as {lines:?}");
            match &back[0].value {
                Value::Text { text, .. } => assert_eq!(text, typed, "{lines:?}"),
                other => panic!("{typed:?} read back as {other:?}"),
            }
        }
    }

    #[test]
    fn list_items_with_commas_and_empty_items_survive() {
        for block in [false, true] {
            let items = vec!["a, b".to_owned(), "c".to_owned(), String::new()];
            let lines = write("k", &Value::List { items: items.clone(), block });
            assert_eq!(parse(&lines)[0].value, Value::List { items, block }, "{lines:?}");
        }
        assert_eq!(write("k", &Value::List { items: vec![], block: true }), ["k: []"]);
    }

    #[test]
    fn quotes_the_file_uses_are_kept() {
        let lines = fm("title: 'It''s'");
        let value = parse(&lines)[0].value.clone();
        assert_eq!(value, Value::Text { text: "It's".into(), quote: Some('\'') });
        assert_eq!(write("title", &Value::Text { text: "Don't".into(), quote: Some('\'') }), ["title: 'Don''t'"]);
    }

    #[test]
    fn comments_and_block_scalars_are_not_rewritten() {
        let lines = fm("a: x # note\nb: |\n  text\n  more\nc: {k: v}");
        let entries = parse(&lines);
        assert!(entries.iter().all(|e| matches!(e.value, Value::Raw(_))), "{entries:?}");
        assert!(set(&lines, 1, &text("y")).is_none());
    }
}
