//! Domain types and the Markdown scanner used by the indexer.
//!
//! This is a line-oriented scanner, not a CommonMark parser: the index only needs
//! frontmatter, headings, wikilinks, tags and tasks, and scanning lines keeps a
//! reindex of one note cheap. The preview pane uses `pulldown-cmark` separately
//! for the full block tree.
//!
//! All `line` values are 0-based and count from the top of the file, frontmatter
//! included, so they can be used directly for write-back.

use regex::Regex;
use std::borrow::Cow;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq)]
pub enum FmValue {
    Text(String),
    Num(f64),
    Bool(bool),
    Null,
}

/// One scalar from the YAML frontmatter. Lists expand to several entries that
/// share a key and differ in `ord`; nested maps flatten to `outer.inner` keys.
#[derive(Debug, Clone, PartialEq)]
pub struct FmEntry {
    pub key: String,
    pub value: FmValue,
    pub ord: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// Target as written, without heading, block or alias. Empty for `[[#Heading]]`.
    /// Markdown link targets are percent-decoded.
    pub target: String,
    pub heading: Option<String>,
    pub block: Option<String>,
    /// For Markdown links, the link text.
    pub alias: Option<String>,
    /// `![[...]]` or `![...](...)`
    pub embed: bool,
    /// `[text](target)` rather than `[[target]]`.
    pub markdown: bool,
    pub line: u32,
    /// Byte offset of the opening `[[` (or `![[`, `[`) within the line, and byte length.
    pub col: u32,
    pub len: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    /// Without the leading `#`, case preserved.
    pub name: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub line: u32,
    pub indent: u32,
    /// The character inside the brackets: ' ', 'x', or a custom state.
    pub state: char,
    /// Text after the checkbox, metadata included.
    pub text: String,
    /// The full source line, compared before any write-back so a stale index
    /// can never rewrite the wrong line.
    pub raw: String,
    /// Slug from a `#status/<slug>` tag on the line.
    pub status: Option<String>,
    pub due: Option<String>,
    pub scheduled: Option<String>,
    pub start: Option<String>,
    pub done: Option<String>,
    /// 5 highest .. 1 lowest, `None` when unset.
    pub priority: Option<u8>,
    /// When to notify, from the Reminder plugin's `(@2026-12-01 09:30)`:
    /// `YYYY-MM-DD HH:MM`, or `YYYY-MM-DD` for the default time.
    pub reminder: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedNote {
    pub title: Option<String>,
    /// The frontmatter `id`. `[[id]]` finds the note by it when no file has
    /// that name, so a typed document (`type: task`) keeps a stable handle
    /// while its file is renamed.
    pub id: Option<String>,
    pub frontmatter: Vec<FmEntry>,
    pub aliases: Vec<String>,
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
    pub tags: Vec<Tag>,
    pub tasks: Vec<Task>,
    /// Content without the frontmatter block, for full-text indexing.
    pub body: String,
}

static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(!?)\[\[([^\]\|#\^\n]*)(?:#([^\]\|\^\n]*))?(?:\^([^\]\|\n]*))?(?:\|([^\]\n]*))?\]\]")
        .unwrap()
});
/// `[text](dest)` and `![alt](dest)`, with an optional title. Nested brackets in
/// the text and parentheses in the destination are not supported.
static MD_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(!?)\[([^\[\]\n]*)\]\(\s*(<[^<>\n]*>|[^\s()<>]+)(?:\s+(?:"[^"\n]*"|'[^'\n]*'))?\s*\)"#)
        .unwrap()
});
static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)#([\p{L}\p{N}_/\-]+)").unwrap());
/// A checkbox line, also inside a quote or callout (`> - [ ] task`).
static TASK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^((?:\s*>)*\s*)(?:[-*+]|\d+[.)])\s\[(.)\]\s+(.*)$").unwrap());
static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.+?)\s*#*\s*$").unwrap());
/// Dates are ASCII digits only: `\d` is Unicode in Rust's regex, and would
/// take fullwidth or Arabic-Indic digits for a date no other tool can read.
static EMOJI_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(📅|⏳|🛫|✅)\s*([0-9]{4}-[0-9]{2}-[0-9]{2})").unwrap());
static INLINE_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\[\(]([A-Za-z_][\w\-]*)::\s*([^\]\)]+)[\]\)]").unwrap());
/// A completion date in either format, with the space before it.
static DONE_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s*(?:✅\s*[0-9]{4}-[0-9]{2}-[0-9]{2}|[\[\(](?:completion|done)::[^\]\)]*[\]\)])").unwrap()
});
/// Due and completion dates, which a task list shows in columns of their own.
static DATE_META: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s*(?:(?:📅|✅)\s*[0-9]{4}-[0-9]{2}-[0-9]{2}|[\[\(](?:due|completion|done)::[^\]\)]*[\]\)])").unwrap()
});
/// The Obsidian Reminder plugin's `(@2026-12-01)` or `(@2026-12-01 09:30)`.
static REMINDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\(@([0-9]{4}-[0-9]{2}-[0-9]{2})(?:[ T]((?:[01][0-9]|2[0-3]):[0-5][0-9]))?\)").unwrap()
});
static BLOCK_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s\^[A-Za-z0-9\-]+\s*$").unwrap());
static STATUS_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s#status/[\p{L}\p{N}_/\-]+").unwrap());
/// Where a task's trailing metadata begins: an emoji signifier, an inline
/// field, a reminder or a block id. Tags go before it so the Tasks plugin still reads it.
static META_START: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s(?:📅|⏳|🛫|✅|➕|🔺|⏫|🔼|🔽|⏬|🔁|[\[\(][A-Za-z_][\w\-]*::|\(@[0-9]|\^[A-Za-z0-9\-]+\s*$)").unwrap()
});

/// Splits `---` delimited YAML frontmatter off the top of a note.
/// Returns `(yaml, body, number_of_lines_before_body)`.
pub fn split_frontmatter(src: &str) -> (Option<&str>, &str, u32) {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let Some(rest) = src.strip_prefix("---") else {
        return (None, src, 0);
    };
    let Some(rest) = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n')) else {
        return (None, src, 0);
    };
    let mut offset = 0;
    let mut lines = 1;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        lines += 1;
        if trimmed == "---" || trimmed == "..." {
            return (Some(&rest[..offset]), &rest[offset + line.len()..], lines);
        }
        offset += line.len();
    }
    (None, src, 0)
}

pub fn parse(src: &str) -> ParsedNote {
    let (yaml, body, body_line) = split_frontmatter(src);
    let mut note = ParsedNote {
        body: body.to_owned(),
        ..ParsedNote::default()
    };

    if let Some(yaml) = yaml {
        parse_frontmatter(yaml, &mut note);
        // Wikilinks in properties (`up: "[[Parent]]"`) are links as well, so
        // backlinks and renames see them. Line 0 is the opening `---`.
        for (i, line) in yaml.lines().enumerate() {
            scan_links(line, 1 + i as u32, false, &mut note.links);
        }
    }

    // The fence we are inside, and how many `>` its quote had: fenced code in a
    // quote or callout also ends where the quote does.
    let mut fence: Option<(Fence, usize)> = None;
    for (i, line) in body.lines().enumerate() {
        let line_no = body_line + i as u32;
        let (depth, inner) = unquote(line);

        if let Some((open, open_depth)) = fence {
            if depth < open_depth {
                fence = None;
            } else {
                if fence_closes(inner, open) {
                    fence = None;
                }
                continue;
            }
        }
        if let Some((open, _)) = fence_open(inner) {
            fence = Some((open, depth));
            continue;
        }

        let masked = mask_inline_code(line);

        if let Some(c) = HEADING.captures(&masked) {
            let level = c[1].len() as u8;
            let text = c[2].to_owned();
            if level == 1 && note.title.is_none() {
                note.title = Some(text.clone());
            }
            note.headings.push(Heading { level, text, line: line_no });
        }

        scan_links(&masked, line_no, true, &mut note.links);

        let mut line_tags = Vec::new();
        for c in TAG.captures_iter(&masked) {
            let name = &c[1];
            // `#123` is an issue number, not a tag.
            if name.chars().all(|ch| ch.is_ascii_digit()) {
                continue;
            }
            line_tags.push(name.to_owned());
        }

        if let Some(c) = TASK.captures(line) {
            let mut task = Task {
                line: line_no,
                indent: c[1].chars().map(|ch| if ch == '\t' { 4 } else { 1 }).sum(),
                state: c[2].chars().next().unwrap_or(' '),
                text: c[3].to_owned(),
                raw: line.to_owned(),
                status: line_tags
                    .iter()
                    .find_map(|t| t.to_lowercase().strip_prefix("status/").map(str::to_owned)),
                due: None,
                scheduled: None,
                start: None,
                done: None,
                priority: None,
                reminder: None,
            };
            parse_task_metadata(&mut task);
            note.tasks.push(task);
        }

        note.tags
            .extend(line_tags.into_iter().map(|name| Tag { name, line: line_no }));
    }

    note
}

/// A code fence: its character and how many of it, such as four backticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fence {
    pub mark: u8,
    pub len: usize,
}

/// The fence a line opens with: three or more backticks or tildes, after
/// any indentation. A backtick fence's language has no backticks.
pub fn fence_open(line: &str) -> Option<(Fence, &str)> {
    let rest = line.trim_start();
    let mark = *rest.as_bytes().first().filter(|&&c| c == b'`' || c == b'~')?;
    let len = rest.bytes().take_while(|&c| c == mark).count();
    let info = &rest[len..];
    (len >= 3 && !(mark == b'`' && info.contains('`'))).then_some((Fence { mark, len }, info))
}

/// Whether a line closes fenced code opened by `open`: the same character,
/// at least as many, and nothing after them.
pub fn fence_closes(line: &str, open: Fence) -> bool {
    fence_open(line).is_some_and(|(f, info)| f.mark == open.mark && f.len >= open.len && info.trim().is_empty())
}

/// How many `>` quote markers a line starts with, and the text after them.
fn unquote(line: &str) -> (usize, &str) {
    let mut depth = 0;
    let mut rest = line;
    while let Some(after) = rest.trim_start().strip_prefix('>') {
        depth += 1;
        rest = after;
    }
    (depth, rest)
}

/// Wikilinks and Markdown links in one line of text, in order. `line` is 0 in
/// the results. Used by the preview, which renders text outside of `parse`.
pub fn links_in_line(line: &str) -> Vec<Link> {
    let mut links = Vec::new();
    scan_links(&mask_inline_code(line), 0, true, &mut links);
    links
}

/// Appends the links in `line` to `out`. Frontmatter passes `with_markdown =
/// false`: `[text](url)` is not link syntax in YAML.
fn scan_links(line: &str, line_no: u32, with_markdown: bool, out: &mut Vec<Link>) {
    let first = out.len();
    for c in LINK.captures_iter(line) {
        let whole = c.get(0).unwrap();
        // Inside a table Obsidian writes `[[Note\|alias]]`; the backslash only
        // escapes the pipe and is not part of the target, heading or block.
        let opt = |i: usize| {
            c.get(i)
                .map(|m| m.as_str().trim().trim_end_matches('\\').trim_end().to_owned())
                .filter(|s| !s.is_empty())
        };
        out.push(Link {
            target: c[2].trim().trim_end_matches('\\').trim_end().to_owned(),
            heading: opt(3),
            block: opt(4),
            alias: opt(5),
            embed: !c[1].is_empty(),
            markdown: false,
            line: line_no,
            col: whole.start() as u32,
            len: whole.len() as u32,
        });
    }
    if !with_markdown {
        return;
    }
    for c in MD_LINK.captures_iter(line) {
        let whole = c.get(0).unwrap();
        let dest = c[3].strip_prefix('<').and_then(|d| d.strip_suffix('>')).unwrap_or(&c[3]);
        // External links are not part of the vault's link graph.
        if dest.is_empty() || is_url(dest) {
            continue;
        }
        let (path, fragment) = dest.split_once('#').map_or((dest, None), |(p, f)| (p, Some(f)));
        let fragment = fragment.map(percent_decode).filter(|f| !f.is_empty());
        let (heading, block) = match fragment {
            Some(f) if f.starts_with('^') => (None, Some(f[1..].to_owned())),
            f => (f, None),
        };
        let text = c[2].trim();
        out.push(Link {
            target: percent_decode(path).trim().to_owned(),
            heading,
            block,
            alias: (!text.is_empty()).then(|| text.to_owned()),
            embed: !c[1].is_empty(),
            markdown: true,
            line: line_no,
            col: whole.start() as u32,
            len: whole.len() as u32,
        });
    }
    out[first..].sort_by_key(|l| l.col);
}

/// `https:`, `mailto:`, `obsidian:` and other schemes.
fn is_url(dest: &str) -> bool {
    dest.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// Decodes `%XX` escapes, as Markdown link destinations use for spaces.
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(byte) if bytes[i] == b'%' => {
                out.push(byte);
                i += 3;
            }
            _ => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lowercase path without a trailing `.md`: the form link targets and file
/// paths are compared in.
pub fn link_key(path: &str) -> String {
    let lower = path.trim().trim_start_matches('/').to_lowercase();
    lower.strip_suffix(".md").map(str::to_owned).unwrap_or(lower)
}

/// The last segment of [`link_key`]: what a bare `[[Note]]` is matched against.
pub fn name_key(path: &str) -> String {
    let key = link_key(path);
    key.rsplit('/').next().unwrap_or("").to_owned()
}

/// The key a link target written in the note at `src_path` resolves by.
pub fn target_key(target: &str, src_path: &str) -> String {
    link_key(&target_path(target, src_path))
}

/// A link target as a vault path, case kept. Targets starting with `./` or
/// `../` are relative to the folder of the note at `src_path`.
pub fn target_path(target: &str, src_path: &str) -> String {
    let target = target.trim();
    if !(target.starts_with("./") || target.starts_with("../")) {
        return target.to_owned();
    }
    let mut parts: Vec<&str> = match src_path.rsplit_once('/') {
        Some((dir, _)) => dir.split('/').collect(),
        None => Vec::new(),
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// Picks the file a link resolves to among vault-relative `paths`, following
/// Obsidian: `[[Note]]` matches the file name anywhere, `[[folder/Note]]` must
/// match the end of the path, and the shortest path wins. The indexer's
/// `resolve_links` applies the same rule in SQL.
pub fn resolve<'a>(target_key: &str, paths: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let name = target_key.rsplit('/').next().unwrap_or("");
    if name.is_empty() {
        return None;
    }
    let suffix = format!("/{target_key}");
    paths
        .into_iter()
        .filter(|path| {
            let key = link_key(path);
            key.rsplit('/').next() == Some(name)
                && (target_key == name || key == target_key || key.ends_with(&suffix))
        })
        // SQLite's length() counts characters, and its text order is byte order.
        .min_by(|a, b| (a.chars().count(), *a).cmp(&(b.chars().count(), *b)))
}

/// Where the target sits inside a link's source text, e.g. `Note` in
/// `![[Note#H|alias]]` or `My%20Note.md` in `[text](My%20Note.md#h)`, and
/// whether a Markdown destination is wrapped in `<...>`.
pub fn target_range(link_src: &str, markdown: bool) -> Option<(std::ops::Range<usize>, bool)> {
    if !markdown {
        let start = link_src.find("[[")? + 2;
        let end = start + link_src[start..].find(['#', '^', '|', ']'])?;
        // `[[Note\|alias]]` in a table: the backslash stays with the pipe.
        let end = if link_src[..end].ends_with('\\') && link_src[end..].starts_with('|') { end - 1 } else { end };
        return Some((start..end, false));
    }
    let open = link_src.find("](")? + 2;
    let start = open + (link_src[open..].len() - link_src[open..].trim_start().len());
    let angle = link_src[start..].starts_with('<');
    let start = start + usize::from(angle);
    let end = start
        + link_src[start..].find(|c: char| if angle { c == '>' } else { c.is_whitespace() || c == ')' })?;
    let end = link_src[start..end].find('#').map_or(end, |i| start + i);
    Some((start..end, angle))
}

/// Flips a task line between open and done in the Tasks plugin format.
/// Checking writes `[x]` and appends `✅ <today>`, before a trailing block id;
/// unchecking writes `[ ]` and removes the completion date. `None` if `line`
/// is not a task.
pub fn toggle_task(line: &str, today: &str) -> Option<String> {
    let state = TASK.captures(line)?.get(2)?;
    let rest = &line[state.end()..];
    let mut out = line[..state.start()].to_owned();
    if matches!(state.as_str(), "x" | "X") {
        out.push(' ');
        out.push_str(&DONE_DATE.replace_all(rest, ""));
    } else {
        out.push('x');
        let (body, block) = match BLOCK_ID.find(rest) {
            Some(m) => rest.split_at(m.start()),
            None => (rest, ""),
        };
        out.push_str(DONE_DATE.replace_all(body, "").trim_end());
        out.push_str(" ✅ ");
        out.push_str(today);
        out.push_str(block.trim_end());
    }
    Some(out)
}

/// Task text without the due and completion dates, for lists that show those
/// in columns of their own, and without its reminder.
pub fn task_display(text: &str) -> String {
    let text = DATE_META.replace_all(text, "");
    REMINDER.replace_all(&text, "").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Rewrites a task line for a Kanban move. `#status/<slug>` replaces the line's
/// status tag, or goes before its metadata; `None` removes it. `done` checks
/// the task with `✅ <today>` or unchecks it, as [`toggle_task`] does. `None`
/// if `line` is not a task.
pub fn set_task_column(line: &str, slug: Option<&str>, done: bool, today: &str) -> Option<String> {
    let state = TASK.captures(line)?.get(2)?;
    // The checkbox through its closing `]`, then the rest of the line.
    let (head, body) = line.split_at(state.end() + 1);
    let at = STATUS_TAG.find(body).map(|m| m.start());
    let mut body = STATUS_TAG.replace_all(body, "").into_owned();
    if let Some(slug) = slug {
        let at = at
            .or_else(|| META_START.find(&body).map(|m| m.start()))
            .unwrap_or(body.trim_end().len());
        body.insert_str(at, &format!(" #status/{slug}"));
    }
    let moved = format!("{head}{body}");
    if done == matches!(state.as_str(), "x" | "X") {
        Some(moved)
    } else {
        toggle_task(&moved, today)
    }
}

/// The `#status/` slug for a Kanban column name: lowercase, words joined by
/// `-`, characters that cannot appear in a tag dropped. "In Progress" is
/// `in-progress`.
pub fn status_slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || c == '_' {
            slug.push(c);
        } else if (c.is_whitespace() || c == '-') && !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

/// The `status` of a task document, a note with `type: task`. Written in the
/// frontmatter as its slug: `todo`, `in-progress`, `done` or `cancelled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaskStatus {
    Todo,
    InProgress,
    Done,
    Cancelled,
}

impl TaskStatus {
    pub const ALL: [TaskStatus; 4] = [Self::Todo, Self::InProgress, Self::Done, Self::Cancelled];

    /// Reads a status as written, ignoring case, spaces and underscores:
    /// `in-progress`, `In Progress` and `in_progress` are the same.
    pub fn parse(text: &str) -> Option<Self> {
        let slug = status_slug(&text.replace('_', "-"));
        Self::ALL.into_iter().find(|s| s.slug() == slug)
    }

    /// As written in the frontmatter.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in-progress",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    /// As shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Todo => "Todo",
            Self::InProgress => "In Progress",
            Self::Done => "Done",
            Self::Cancelled => "Cancelled",
        }
    }
}

/// Byte ranges of the `#tags` in a run of text, `#` included.
pub fn tag_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    TAG.captures_iter(text)
        .filter_map(|c| c.get(1))
        .filter(|name| !name.as_str().chars().all(|ch| ch.is_ascii_digit()))
        .map(|name| name.start() - 1..name.end())
        .collect()
}

/// One replacement inside a line, positioned the way [`parse`] reports links:
/// `line` counts from the top of the file and `col` is a byte offset.
#[derive(Debug, Clone, PartialEq)]
pub struct LineEdit {
    pub line: u32,
    pub col: u32,
    pub len: u32,
    pub text: String,
}

/// Splits `src` into lines as [`parse`] numbers them: a leading BOM belongs to
/// no line, and each line is `(content, ending)`.
fn split_lines(src: &str) -> (&str, impl Iterator<Item = (&str, &str)>) {
    let (bom, body) = match src.strip_prefix('\u{feff}') {
        Some(body) => ("\u{feff}", body),
        None => ("", src),
    };
    let lines = body.split_inclusive('\n').map(|line| {
        let content = line.strip_suffix('\n').map_or(line, |l| l.strip_suffix('\r').unwrap_or(l));
        line.split_at(content.len())
    });
    (bom, lines)
}

/// The text of line `n` as [`parse`] sees it, without BOM and line ending.
pub fn line_at(src: &str, n: u32) -> Option<&str> {
    split_lines(src).1.nth(n as usize).map(|(content, _)| content)
}

/// Applies `edits` to `src`, keeping line endings and a leading BOM. Returns
/// `None` if an edit overlaps another or falls outside its line.
pub fn edit_lines(src: &str, edits: &[LineEdit]) -> Option<String> {
    let mut edits: Vec<&LineEdit> = edits.iter().collect();
    edits.sort_by_key(|e| (e.line, e.col));
    let mut pending = edits.into_iter().peekable();

    let (bom, lines) = split_lines(src);
    let mut out = String::with_capacity(src.len() + 64);
    out.push_str(bom);
    for (n, (content, ending)) in lines.enumerate() {
        let mut at = 0;
        while let Some(edit) = pending.next_if(|e| e.line as usize == n) {
            let (start, end) = (edit.col as usize, (edit.col + edit.len) as usize);
            if start < at || end > content.len() || !content.is_char_boundary(start) || !content.is_char_boundary(end) {
                return None;
            }
            out.push_str(&content[at..start]);
            out.push_str(&edit.text);
            at = end;
        }
        out.push_str(&content[at..]);
        out.push_str(ending);
    }
    pending.next().is_none().then_some(out)
}

/// Reads both the Tasks-plugin emoji format and Dataview inline fields.
/// The app only ever writes the emoji format.
fn parse_task_metadata(task: &mut Task) {
    task.reminder = REMINDER.captures(&task.text).map(|c| match c.get(2) {
        Some(time) => format!("{} {}", &c[1], time.as_str()),
        None => c[1].to_owned(),
    });
    for c in EMOJI_DATE.captures_iter(&task.text) {
        let date = Some(c[2].to_owned());
        match &c[1] {
            "📅" => task.due = date,
            "⏳" => task.scheduled = date,
            "🛫" => task.start = date,
            _ => task.done = date,
        }
    }
    for (emoji, level) in [("🔺", 5), ("⏫", 4), ("🔼", 3), ("🔽", 2), ("⏬", 1)] {
        if task.text.contains(emoji) {
            task.priority = Some(level);
            break;
        }
    }
    for c in INLINE_FIELD.captures_iter(&task.text) {
        let value = c[2].trim().to_owned();
        let slot = match c[1].to_lowercase().as_str() {
            "due" => &mut task.due,
            "scheduled" => &mut task.scheduled,
            "start" => &mut task.start,
            "completion" | "done" => &mut task.done,
            "priority" => {
                if task.priority.is_none() {
                    task.priority = match value.to_lowercase().as_str() {
                        "highest" => Some(5),
                        "high" => Some(4),
                        "medium" => Some(3),
                        "low" => Some(2),
                        "lowest" => Some(1),
                        _ => None,
                    };
                }
                continue;
            }
            _ => continue,
        };
        // The emoji form wins when a line carries both.
        slot.get_or_insert(value);
    }
}

/// The entries of a frontmatter block's YAML, as [`parse`] reads them. Empty
/// for broken YAML.
pub fn frontmatter(yaml: &str) -> Vec<FmEntry> {
    let mut note = ParsedNote::default();
    parse_frontmatter(yaml, &mut note);
    note.frontmatter
}

fn parse_frontmatter(yaml: &str, note: &mut ParsedNote) {
    use serde_yaml_ng::Value;
    let Ok(Value::Mapping(map)) = serde_yaml_ng::from_str::<Value>(yaml) else {
        // Broken YAML must never stop a note from being indexed.
        return;
    };

    fn scalar(v: &Value) -> Option<FmValue> {
        Some(match v {
            Value::Null => FmValue::Null,
            Value::Bool(b) => FmValue::Bool(*b),
            Value::Number(n) => FmValue::Num(n.as_f64()?),
            Value::String(s) => FmValue::Text(s.clone()),
            _ => return None,
        })
    }
    fn flatten(prefix: &str, v: &Value, out: &mut Vec<FmEntry>) {
        match v {
            Value::Sequence(items) => {
                for (ord, item) in items.iter().enumerate() {
                    if let Some(value) = scalar(item) {
                        out.push(FmEntry { key: prefix.to_owned(), value, ord: ord as u32 });
                    }
                }
            }
            Value::Mapping(map) => {
                for (k, v) in map {
                    if let Some(k) = k.as_str() {
                        flatten(&format!("{prefix}.{k}"), v, out);
                    }
                }
            }
            other => {
                if let Some(value) = scalar(other) {
                    out.push(FmEntry { key: prefix.to_owned(), value, ord: 0 });
                }
            }
        }
    }

    for (k, v) in &map {
        if let Some(k) = k.as_str() {
            flatten(k, v, &mut note.frontmatter);
        }
    }

    for entry in &note.frontmatter {
        let FmValue::Text(text) = &entry.value else { continue };
        match entry.key.to_lowercase().as_str() {
            "title" => note.title = Some(text.clone()),
            "id" if entry.ord == 0 && !text.trim().is_empty() => note.id = Some(text.trim().to_owned()),
            "aliases" | "alias" => note.aliases.push(text.clone()),
            // `tags: a, b` and `tags: [a, b]` are both valid in Obsidian.
            "tags" | "tag" => {
                for name in text.split([',', ' ']).filter(|s| !s.is_empty()) {
                    note.tags.push(Tag {
                        name: name.trim_start_matches('#').to_owned(),
                        line: 0,
                    });
                }
            }
            _ => {}
        }
    }
}

/// Blanks out `inline code` spans with spaces so tags and links inside them are
/// ignored while byte columns stay valid.
fn mask_inline_code(line: &str) -> Cow<'_, str> {
    if !line.contains('`') {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for ch in line.chars() {
        if ch == '`' {
            in_code = !in_code;
            out.push(' ');
        } else if in_code {
            out.extend(std::iter::repeat_n(' ', ch.len_utf8()));
        } else {
            out.push(ch);
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntitle: Alpha plan\naliases: [Alpha, Project A]\ntags: [project/alpha, work]\nestimate: 12.5\n---\n# Heading one\nSee [[Other Note#Section|other]] and ![[img.png]] #inline-tag `#not-a-tag [[nope]]`\n```\n- [ ] not a task #nor-this\n```\n- [ ] Ship it #status/in-progress 📅 2026-12-01 ⏫\n  - [x] Done one [due:: 2026-11-20] ✅ 2026-11-13\nIssue #123 is not a tag\n";

    #[test]
    fn parses_everything() {
        let n = parse(NOTE);
        assert_eq!(n.title.as_deref(), Some("Alpha plan"));
        assert_eq!(n.aliases, ["Alpha", "Project A"]);
        assert!(n
            .frontmatter
            .iter()
            .any(|e| e.key == "estimate" && e.value == FmValue::Num(12.5)));
        assert_eq!(n.headings.len(), 1);
        assert_eq!(n.headings[0].line, 6);

        assert_eq!(n.links.len(), 2);
        assert_eq!(n.links[0].target, "Other Note");
        assert_eq!(n.links[0].heading.as_deref(), Some("Section"));
        assert_eq!(n.links[0].alias.as_deref(), Some("other"));
        assert!(n.links[1].embed);

        let tags: Vec<_> = n.tags.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(tags, ["project/alpha", "work", "inline-tag", "status/in-progress"]);

        assert_eq!(n.tasks.len(), 2);
        let t = &n.tasks[0];
        assert_eq!((t.line, t.state), (11, ' '));
        assert_eq!(t.status.as_deref(), Some("in-progress"));
        assert_eq!(t.due.as_deref(), Some("2026-12-01"));
        assert_eq!(t.priority, Some(4));
        assert_eq!(NOTE.lines().nth(11).unwrap(), t.raw);
        let t = &n.tasks[1];
        assert_eq!((t.state, t.indent), ('x', 2));
        assert_eq!(t.due.as_deref(), Some("2026-11-20"));
        assert_eq!(t.done.as_deref(), Some("2026-11-13"));
    }

    #[test]
    fn task_dates_in_non_ascii_digits_are_not_read() {
        let n = parse("- [ ] a 📅 ２０２６-11-26\n- [x] b ✅ ٢٠٢٦-١١-٢٦\n");
        assert_eq!((n.tasks[0].due.as_deref(), n.tasks[1].done.as_deref()), (None, None));
        assert_eq!(task_display("a 📅 ２０２６-11-26"), "a 📅 ２０２６-11-26");
    }

    #[test]
    fn no_frontmatter_and_broken_yaml() {
        assert_eq!(parse("# T\n[[A]]").links[0].line, 1);
        let n = parse("---\n: : [\n---\nbody #x");
        assert_eq!(n.tags[0].name, "x");
    }

    #[test]
    fn markdown_and_property_links() {
        let src = "\u{feff}---\nup: \"[[Parent]]\"\nsite: \"[docs](https://example.com)\"\n---\n\
                   See [the spec](Specs/My%20Spec.md#Scope%20one \"title\") and ![diagram](<img/a b.png>)\r\n\
                   [ext](https://x.y) [rel](../Other.md#^blk) [[Wiki]] `[code](x.md)`\n";
        let n = parse(src);
        let found: Vec<_> = n.links.iter().map(|l| (l.line, l.target.as_str(), l.markdown, l.embed)).collect();
        assert_eq!(
            found,
            [
                (1, "Parent", false, false),
                (4, "Specs/My Spec.md", true, false),
                (4, "img/a b.png", true, true),
                (5, "../Other.md", true, false),
                (5, "Wiki", false, false),
            ]
        );
        let spec = &n.links[1];
        assert_eq!((spec.heading.as_deref(), spec.alias.as_deref()), (Some("Scope one"), Some("the spec")));
        assert_eq!(n.links[3].block.as_deref(), Some("blk"));

        // Positions point at the link source, so a rewrite can patch it in place.
        let line = line_at(src, 4).unwrap();
        let link_src = &line[spec.col as usize..(spec.col + spec.len) as usize];
        assert_eq!(link_src, "[the spec](Specs/My%20Spec.md#Scope%20one \"title\")");
        let (range, angle) = target_range(link_src, true).unwrap();
        assert_eq!((&link_src[range], angle), ("Specs/My%20Spec.md", false));
        let (range, angle) = target_range("![diagram](<img/a b.png>)", true).unwrap();
        assert_eq!((&"![diagram](<img/a b.png>)"[range], angle), ("img/a b.png", true));
        let (range, _) = target_range("![[Note#H|alias]]", false).unwrap();
        assert_eq!(&"![[Note#H|alias]]"[range], "Note");
    }

    #[test]
    fn link_resolution() {
        assert_eq!(target_key("../Other.md", "Projects/Alpha.md"), "other");
        assert_eq!(target_key("./sub/X.md", "Projects/Alpha.md"), "projects/sub/x");
        assert_eq!(target_key("Folder/Note.md", "a/b.md"), "folder/note");
        assert_eq!(target_path("../New Note.md", "Projects/Alpha.md"), "New Note.md");

        let files = ["Note.md", "a/Note.md", "b/folder/Note.md", "pic.png"];
        let r = |key: &str| resolve(key, files);
        assert_eq!(r("note"), Some("Note.md"), "shortest path wins");
        assert_eq!(r("folder/note"), Some("b/folder/Note.md"));
        assert_eq!(r("a/note"), Some("a/Note.md"));
        assert_eq!(r("older/note"), None, "a path matches whole segments");
        assert_eq!(r("pic.png"), Some("pic.png"));
        assert_eq!(r(""), None);
    }

    #[test]
    fn task_toggle() {
        let today = "2026-11-13";
        assert_eq!(
            toggle_task("- [ ] Ship it 📅 2026-12-01", today).unwrap(),
            "- [x] Ship it 📅 2026-12-01 ✅ 2026-11-13"
        );
        assert_eq!(toggle_task("  * [x] Done ✅ 2026-11-10 ^abc", today).unwrap(), "  * [ ] Done ^abc");
        assert_eq!(toggle_task("1. [ ] Numbered ^id-1", today).unwrap(), "1. [x] Numbered ✅ 2026-11-13 ^id-1");
        assert_eq!(toggle_task("- [x] Done [completion:: 2026-11-10]", today).unwrap(), "- [ ] Done");
        assert_eq!(toggle_task("> > - [ ] In a callout", today).unwrap(), "> > - [x] In a callout ✅ 2026-11-13");
        assert_eq!(parse("> [!todo]\n> - [ ] Quoted #status/doing\n").tasks[0].status.as_deref(), Some("doing"));
        assert!(toggle_task("- not a task", today).is_none());
        assert_eq!(task_display("Ship #status/x 📅 2026-12-01 ⏫ ✅ 2026-11-13"), "Ship #status/x ⏫");
    }

    #[test]
    fn reminders_in_the_reminder_plugins_format() {
        let tasks = parse(
            "- [ ] Call Anna 📅 2026-12-01 (@2026-12-01 09:30) #task
- [ ] Pay (@2026-12-02)
             - [ ] Odd (@2026-12-02 25:00)
- [ ] Mail (@ann) [due:: 2026-12-03]
",
        )
        .tasks;
        let reminders: Vec<_> = tasks.iter().map(|t| t.reminder.as_deref()).collect();
        assert_eq!(reminders, [Some("2026-12-01 09:30"), Some("2026-12-02"), None, None]);
        assert_eq!(tasks[3].due.as_deref(), Some("2026-12-03"));
        assert_eq!(task_display("Call Anna 📅 2026-12-01 (@2026-12-01 09:30) #task"), "Call Anna #task");
        // The status tag goes before the reminder, as before any other metadata.
        assert_eq!(
            set_task_column("- [ ] Call (@2026-12-01 09:30)", Some("doing"), false, "2026-11-13").unwrap(),
            "- [ ] Call #status/doing (@2026-12-01 09:30)"
        );
    }

    #[test]
    fn kanban_moves() {
        let today = "2026-11-13";
        let draft = "- [ ] Draft the runbook #status/in-progress 📅 2026-12-01 ⏫";
        assert_eq!(
            set_task_column(draft, Some("backlog"), false, today).unwrap(),
            "- [ ] Draft the runbook #status/backlog 📅 2026-12-01 ⏫"
        );
        // The last column is the checkbox: the status tag goes, the date comes.
        assert_eq!(
            set_task_column(draft, None, true, today).unwrap(),
            "- [x] Draft the runbook 📅 2026-12-01 ⏫ ✅ 2026-11-13"
        );
        assert_eq!(
            set_task_column("- [x] Inventory ✅ 2026-11-10", Some("in-progress"), false, today).unwrap(),
            "- [ ] Inventory #status/in-progress"
        );
        assert_eq!(set_task_column("  * [ ] Plain", Some("doing"), false, today).unwrap(), "  * [ ] Plain #status/doing");
        assert_eq!(set_task_column("- [ ] Ref ^blk", Some("doing"), false, today).unwrap(), "- [ ] Ref #status/doing ^blk");
        assert_eq!(
            set_task_column("- [/] #Status/Old first", Some("new"), false, today).unwrap(),
            "- [/] #status/new first",
            "custom open states stay"
        );
        assert!(set_task_column("plain line", None, true, today).is_none());

        assert_eq!(status_slug("In Progress"), "in-progress");
        assert_eq!(status_slug("  Waiting on — others! "), "waiting-on-others");
        assert_eq!(status_slug("Ready_2"), "ready_2");
        assert_eq!(tag_ranges("a #tag, b#no #12 #x/y-z"), [2..6, 17..23]);
    }

    #[test]
    fn line_edits_keep_endings_and_bom() {
        let src = "\u{feff}a [[X]] b [[X]]\r\nsecond\nlast";
        let edit = |line, col, len, text: &str| LineEdit { line, col, len, text: text.to_owned() };
        let out = edit_lines(src, &[edit(0, 12, 1, "Yy"), edit(0, 4, 1, "Z"), edit(2, 0, 4, "end")]);
        assert_eq!(out.as_deref(), Some("\u{feff}a [[Z]] b [[Yy]]\r\nsecond\nend"));
        assert_eq!(line_at(src, 0), Some("a [[X]] b [[X]]"));
        assert_eq!(line_at(src, 1), Some("second"));
        assert!(edit_lines(src, &[edit(1, 4, 9, "x")]).is_none(), "past the end of the line");
        assert!(edit_lines(src, &[edit(7, 0, 0, "x")]).is_none(), "no such line");
    }

    #[test]
    fn code_fences_close_only_on_their_own_kind_and_length() {
        let targets = |src: &str| parse(src).links.into_iter().map(|l| l.target).collect::<Vec<_>>();
        // A shorter fence shown inside a longer one does not end it.
        assert_eq!(targets("````md\n```\n[[A]]\n```\n[[B]]\n````\n[[C]]\n"), ["C"]);
        // A line with a language does not close a fence, and tildes do not close backticks.
        assert_eq!(targets("```\n```rust\n[[A]]\n~~~\n[[B]]\n```\n[[C]]\n"), ["C"]);
        // Fenced code in a quote or callout, which also ends where the quote does.
        assert_eq!(targets("> [!example]\n> ```\n> [[A]] #nope\n> ```\n> [[B]]\n"), ["B"]);
        assert_eq!(targets("> ```\n> [[A]]\n[[B]]\n"), ["B"]);
        let note = parse("> ```\n> - [ ] not a task #nope\n> ```\n");
        assert!(note.tasks.is_empty() && note.tags.is_empty());
    }

    #[test]
    fn escaped_pipe_in_a_table_link() {
        let src = "| a | b |\n|---|---|\n| [[Note\\|alias]] | [[Other#Head\\|x]] |\n";
        let links = parse(src).links;
        assert_eq!(links[0].target, "Note");
        assert_eq!(links[0].alias.as_deref(), Some("alias"));
        assert_eq!((links[1].target.as_str(), links[1].heading.as_deref()), ("Other", Some("Head")));
        // A rename replaces the target and keeps the backslash for the table.
        let (range, _) = target_range("[[Note\\|alias]]", false).unwrap();
        assert_eq!(&"[[Note\\|alias]]"[range], "Note");
        assert_eq!(&"[[Note|alias]]"[target_range("[[Note|alias]]", false).unwrap().0], "Note");
    }
}
