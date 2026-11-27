//! Live preview: one editable pane that shows the note formatted, as
//! Obsidian's Live Preview does. This is the pure half. It splits every line
//! into pieces and decides which Markdown syntax to hide, given where the
//! cursor is: block syntax (`#`, `>`) shows while the cursor is on its line,
//! inline syntax (`**`, `[[ ]]`, list markers) while the cursor touches it.
//! `live_editor.rs` draws the pieces and edits the text, which stays plain
//! Markdown throughout.

use iced::advanced::graphics::text::cosmic_text;
use std::ops::Range;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

/// The names a link can use to reach a note: every path suffix without its
/// extension, and every alias, in lowercase. Set by the app from the index so
/// that links to notes that do not exist can be drawn differently.
static KNOWN: RwLock<Option<HashSet<String>>> = RwLock::new(None);
/// Counts the changes to `KNOWN`, so the pane knows to lay its lines out again
/// when a link's note appears or goes, with the text and cursor unchanged.
static KNOWN_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Which `KNOWN` the lines would be worked out against now.
pub fn known_generation() -> u64 {
    KNOWN_GENERATION.load(Ordering::Relaxed)
}

pub fn set_known(paths: impl Iterator<Item = String>, aliases: Vec<String>) {
    let mut names: HashSet<String> = aliases.into_iter().collect();
    for path in paths {
        let path = path.to_lowercase();
        let path = path.strip_suffix(".md").unwrap_or(&path);
        let mut rest = path;
        loop {
            names.insert(rest.to_owned());
            match rest.split_once('/') {
                Some((_, tail)) => rest = tail,
                None => break,
            }
        }
    }
    // Every pass of the index sets it; only a real change lays the pane out again.
    if let Ok(mut known) = KNOWN.write() {
        if known.as_ref() != Some(&names) {
            *known = Some(names);
            KNOWN_GENERATION.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Whether `target`, the inside of a `[[link]]`, names no note. Unknown until
/// the index has been read, and never for the note itself (`[[#Heading]]`) or
/// for an attachment.
fn is_missing(target: &str) -> bool {
    let name = target.split(['|', '#']).next().unwrap_or("").trim().to_lowercase();
    let name = name.trim_end_matches('/');
    if name.is_empty() || name.rsplit_once('.').is_some_and(|(_, ext)| ext != "md" && ext.len() <= 5) {
        return false;
    }
    let name = name.strip_suffix(".md").unwrap_or(name);
    match KNOWN.read() {
        Ok(known) => known.as_ref().is_some_and(|k| !k.contains(name)),
        Err(_) => false,
    }
}

/// Stands in for a task's `- [ ]` while the widget draws a box over it; as
/// wide as the box.
pub const CHECKBOX: &str = "\u{2003}";

/// The block a line starts in, carried from line to line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Body,
    Frontmatter,
    /// Inside fenced code, opened by this fence.
    Fence(Fence),
}

// Fences are found the same way as the indexer finds them.
pub(crate) use sage_core::{fence_closes, fence_open, Fence};

/// What a line is, which sets its font and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Heading(u8),
    /// A line opening or closing fenced code.
    Fence,
    Code,
    Frontmatter,
    Rule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    pub highlight: bool,
    pub code: bool,
    pub link: bool,
    /// A link to a note that does not exist; Ctrl+click creates it.
    pub missing: bool,
    pub tag: bool,
    /// Markdown syntax, shown because the cursor is on it.
    pub syntax: bool,
}

/// How a piece of the line is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Show {
    /// The text as written.
    Text,
    /// Nothing: syntax away from the cursor.
    Hidden,
    /// Other text, such as `•` for a list marker.
    Replace(String),
    /// A task's checkbox, drawn by the widget; `true` once done.
    Checkbox(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// Byte range in the line.
    pub range: Range<usize>,
    pub show: Show,
    pub style: Style,
    /// The whole link this piece belongs to, as written, while the link's
    /// syntax is hidden. Clicking the piece follows the link.
    pub link: Option<Range<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    /// How many `>` the line starts with.
    pub quotes: u8,
    /// The kind of callout the line is in, lowercase, such as `note`.
    pub callout: Option<String>,
    /// The callout's first line, which holds its title.
    pub title: bool,
    /// A task marked done; its text is muted.
    pub done: bool,
    /// Cover the line from start to end, in order.
    pub pieces: Vec<Piece>,
}

impl Line {
    fn new(kind: Kind) -> Self {
        Self { kind, quotes: 0, callout: None, title: false, done: false, pieces: Vec::new() }
    }
}

/// The cursor and the selection's other end, ordered: `from <= to`, each
/// `(line, byte column)`. Equal ends are a plain cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    pub from: (usize, usize),
    pub to: (usize, usize),
}

impl Caret {
    pub fn new(a: (usize, usize), b: (usize, usize)) -> Self {
        Self { from: a.min(b), to: a.max(b) }
    }

    /// The part of `line` (of `len` bytes) the cursor or selection covers.
    fn on(&self, line: usize, len: usize) -> Option<Range<usize>> {
        if line < self.from.0 || line > self.to.0 {
            return None;
        }
        let start = if line == self.from.0 { self.from.1 } else { 0 };
        let end = if line == self.to.0 { self.to.1 } else { len };
        Some(start..end)
    }
}

/// What one line passes on to the next: the block it ends in and the callout
/// it is part of. It never depends on the cursor, so a line can be worked out
/// again on its own, from what the line before passed on, when only the
/// cursor moved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Carry {
    state: State,
    callout: Option<String>,
}

/// Every line of the note, with what to show of it.
#[cfg(test)]
pub fn lines<S: AsRef<str>>(text: &[S], caret: Option<Caret>) -> Vec<Line> {
    let mut carry = Carry::default();
    text.iter().enumerate().map(|(n, line)| next_line(line.as_ref(), n, &mut carry, caret)).collect()
}

/// Line `n` of the note, `text`, with what to show of it. `carry` is what the
/// line before passed on (the default for the first line), and becomes what
/// this line passes on.
pub fn next_line(text: &str, n: usize, carry: &mut Carry, caret: Option<Caret>) -> Line {
    let at = caret.and_then(|c| c.on(n, text.len()));
    line(text, n == 0, &mut carry.state, &mut carry.callout, at.as_ref())
}

/// Whether the cursor or selection touches a span, ends included: a cursor
/// right after `**bold**` still shows its stars, as in Obsidian.
fn touches(at: Option<&Range<usize>>, span: &Range<usize>) -> bool {
    at.is_some_and(|at| at.start <= span.end && at.end >= span.start)
}

fn line(text: &str, first: bool, state: &mut State, callout: &mut Option<String>, at: Option<&Range<usize>>) -> Line {
    let whole = |kind| {
        let style = Style { syntax: kind != Kind::Code, code: matches!(kind, Kind::Code | Kind::Fence), ..Style::default() };
        let mut line = Line::new(kind);
        line.pieces.push(Piece { range: 0..text.len(), show: Show::Text, style, link: None });
        line
    };
    let fence = fence_open(text).map(|(f, _)| f);
    match *state {
        State::Frontmatter => {
            if matches!(text.trim_end(), "---" | "...") {
                *state = State::Body;
            }
            return whole(Kind::Frontmatter);
        }
        State::Fence(open) => {
            if fence_closes(text, open) {
                *state = State::Body;
                return whole(Kind::Fence);
            }
            return whole(Kind::Code);
        }
        State::Body if first && text.trim_start_matches('\u{feff}').trim_end() == "---" => {
            *state = State::Frontmatter;
            return whole(Kind::Frontmatter);
        }
        State::Body => {}
    }
    if let Some(open) = fence {
        *state = State::Fence(open);
        *callout = None;
        return whole(Kind::Fence);
    }

    let mut line = Line::new(Kind::Text);
    let on_line = at.is_some();
    let block = |out: &mut Vec<Piece>, range: Range<usize>| {
        let show = if on_line { Show::Text } else { Show::Hidden };
        push(out, range, show, Style { syntax: true, ..Style::default() }, None);
    };

    // Quote markers, `>` nested any number of times.
    let mut pos = 0;
    loop {
        let rest = &text[pos..];
        let indent = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        if !rest[indent..].starts_with('>') {
            break;
        }
        pos += indent + 1;
        if text[pos..].starts_with(' ') {
            pos += 1;
        }
        line.quotes += 1;
    }
    if line.quotes == 0 {
        *callout = None;
    } else {
        block(&mut line.pieces, 0..pos);
        // A callout's first line: `> [!kind]`, maybe `+` or `-`, then a title.
        let rest = &text[pos..];
        if let Some(close) = rest.strip_prefix("[!").and_then(|r| r.find(']')) {
            let kind = rest[2..close + 2].to_lowercase();
            let mut end = pos + close + 3;
            if text[end..].starts_with(['+', '-']) {
                end += 1;
            }
            let title_empty = text[end..].trim().is_empty();
            if text[end..].starts_with(' ') {
                end += 1;
            }
            if title_empty && !on_line {
                // No title of its own: the kind, capitalised, as Obsidian shows.
                let mut label = kind.clone();
                if let Some(first) = label.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                push(&mut line.pieces, pos..end, Show::Replace(label), Style::default(), None);
            } else {
                block(&mut line.pieces, pos..end);
            }
            *callout = Some(kind);
            line.title = true;
            pos = end;
        }
        line.callout = callout.clone();
    }

    let rest = &text[pos..];
    let hashes = rest.len() - rest.trim_start_matches('#').len();
    if (1..=6).contains(&hashes) && (rest.len() == hashes || rest[hashes..].starts_with(' ')) {
        line.kind = Kind::Heading(hashes as u8);
        let end = (pos + hashes + 1).min(text.len());
        block(&mut line.pieces, pos..end);
        pos = end;
    } else if line.quotes == 0 && is_rule(rest) {
        line.kind = Kind::Rule;
        block(&mut line.pieces, pos..text.len());
        return line;
    } else if let Some(marker) = list_marker(rest) {
        let indent = rest.len() - rest.trim_start().len();
        push(&mut line.pieces, pos..pos + indent, Show::Text, Style::default(), None);
        let start = pos + indent;
        let syntax = Style { syntax: true, ..Style::default() };
        match marker {
            Marker::Task(len, done) => {
                let range = start..start + len;
                let show = if touches(at, &range) { Show::Text } else { Show::Checkbox(done) };
                push(&mut line.pieces, range, show, syntax, None);
                line.done = done;
                pos = start + len;
            }
            Marker::Bullet => {
                let range = start..start + 1;
                let show = if touches(at, &range) { Show::Text } else { Show::Replace("•".into()) };
                push(&mut line.pieces, range, show, syntax, None);
                pos = start + 1;
            }
            Marker::Number(len) => {
                push(&mut line.pieces, start..start + len, Show::Text, syntax, None);
                pos = start + len;
            }
        }
    }
    inline(text, pos..text.len(), Style::default(), at, None, &mut line.pieces);
    line
}

/// `---`, `***` or `___`, three or more, spaces allowed between.
fn is_rule(text: &str) -> bool {
    let mut marks = text.chars().filter(|c| !c.is_whitespace());
    let first = marks.next();
    let count = marks.try_fold(1, |n, c| (Some(c) == first).then_some(n + 1));
    matches!(first, Some('-' | '*' | '_')) && count.is_some_and(|n| n >= 3)
}

enum Marker {
    /// `- [ ]` and its length; `true` once done.
    Task(usize, bool),
    /// `-`, `*` or `+`.
    Bullet,
    /// `1.` or `1)` and its length.
    Number(usize),
}

/// The list marker a line (after any quote markers) starts with.
fn list_marker(text: &str) -> Option<Marker> {
    let b = text.trim_start().as_bytes();
    let spaced = |at: usize| b.get(at).is_none_or(|&c| c == b' ' || c == b'\t');
    if matches!(b.first(), Some(b'-' | b'*' | b'+')) && spaced(1) {
        if b.len() >= 5 && b[1] == b' ' && b[2] == b'[' && b[4] == b']' && b[3].is_ascii() && spaced(5) {
            return Some(Marker::Task(5, b[3] != b' '));
        }
        return Some(Marker::Bullet);
    }
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if (1..=9).contains(&digits) && matches!(b.get(digits), Some(b'.' | b')')) && spaced(digits + 1) {
        return Some(Marker::Number(digits + 1));
    }
    None
}

/// Adds a piece, joining it to the last one when both show text alike.
fn push(out: &mut Vec<Piece>, range: Range<usize>, show: Show, style: Style, link: Option<Range<usize>>) {
    if range.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut() {
        if show == Show::Text && last.show == Show::Text && last.style == style && last.link == link && last.range.end == range.start {
            last.range.end = range.end;
            return;
        }
    }
    out.push(Piece { range, show, style, link });
}

/// The inline syntax in `range` of the line: emphasis, strikethrough,
/// highlights, code, links and tags.
fn inline(text: &str, range: Range<usize>, style: Style, at: Option<&Range<usize>>, link: Option<Range<usize>>, out: &mut Vec<Piece>) {
    let b = text.as_bytes();
    let end = range.end;
    let mut i = range.start;
    let mut plain = i;
    let syntax = Style { syntax: true, ..style };

    // A span with its syntax at both ends, the inside drawn by `inner`.
    let span = |out: &mut Vec<Piece>,
                    plain: &mut usize,
                    open: Range<usize>,
                    close: Range<usize>,
                    inner: &mut dyn FnMut(&mut Vec<Piece>, bool)| {
        push(out, *plain..open.start, Show::Text, style, link.clone());
        let shown = touches(at, &(open.start..close.end));
        let show = if shown { Show::Text } else { Show::Hidden };
        push(out, open, show.clone(), syntax, None);
        inner(out, shown);
        push(out, close.clone(), show, syntax, None);
        *plain = close.end;
    };

    while i < end {
        match b[i] {
            b'\\' if i + 1 < end && b[i + 1].is_ascii_punctuation() => i += 2,
            b'`' => {
                let n = run(b, i, end);
                match find_run(b, i + n, end, b'`', n) {
                    Some(j) => {
                        let code = Style { code: true, ..style };
                        span(out, &mut plain, i..i + n, j..j + n, &mut |out, _| {
                            push(out, i + n..j, Show::Text, code, None);
                        });
                        i = j + n;
                    }
                    None => i += n,
                }
            }
            b'[' | b'!' if text[i..end].starts_with("[[") || text[i..end].starts_with("![[") => {
                let inner = if b[i] == b'!' { i + 3 } else { i + 2 };
                let Some(close) = text[inner..end].find("]]").map(|n| inner + n) else {
                    i = inner;
                    continue;
                };
                let whole = i..close + 2;
                let linked = Style { link: true, missing: is_missing(&text[inner..close]), ..style };
                span(out, &mut plain, i..inner, close..close + 2, &mut |out, shown| {
                    let target = &text[inner..close];
                    if shown {
                        push(out, inner..close, Show::Text, linked, None);
                    } else if let Some(pipe) = target.find('|') {
                        // `[[Note|alias]]` shows the alias.
                        push(out, inner..inner + pipe + 1, Show::Hidden, linked, None);
                        push(out, inner + pipe + 1..close, Show::Text, linked, Some(whole.clone()));
                    } else if let Some(hash) = target.find('#') {
                        // `[[Note#Heading]]` shows `Note > Heading`.
                        let show = if hash == 0 { Show::Hidden } else { Show::Replace(" > ".into()) };
                        push(out, inner..inner + hash, Show::Text, linked, Some(whole.clone()));
                        push(out, inner + hash..inner + hash + 1, show, linked, Some(whole.clone()));
                        push(out, inner + hash + 1..close, Show::Text, linked, Some(whole.clone()));
                    } else {
                        push(out, inner..close, Show::Text, linked, Some(whole.clone()));
                    }
                });
                i = close + 2;
            }
            b'[' | b'!' if b[i] == b'[' || b.get(i + 1) == Some(&b'[') => {
                // `[text](url)` or `![alt](picture)`.
                let open = if b[i] == b'!' { i + 2 } else { i + 1 };
                let found = text[open..end].find(']').map(|n| open + n).and_then(|close| {
                    let paren = text[close..end].strip_prefix("](")?;
                    paren.find(')').map(|n| (close, close + 2 + n + 1))
                });
                let Some((close, stop)) = found else {
                    i = open;
                    continue;
                };
                let whole = i..stop;
                let linked = Style { link: true, ..style };
                span(out, &mut plain, i..open, close..stop, &mut |out, shown| {
                    let follow = (!shown).then(|| whole.clone());
                    inline(text, open..close, linked, at, follow, out);
                });
                i = stop;
            }
            c @ (b'*' | b'_' | b'~' | b'=') => {
                let r = run(b, i, end);
                let n = match c {
                    b'~' | b'=' if r == 2 => 2,
                    b'*' | b'_' => r.min(3),
                    _ => {
                        i += r;
                        continue;
                    }
                };
                let opens = b.get(i + n).is_some_and(|c| !c.is_ascii_whitespace())
                    && (c != b'_' || i == 0 || !b[i - 1].is_ascii_alphanumeric());
                let close = if opens { find_closer(b, i + n, end, c, n) } else { None };
                let Some(j) = close else {
                    i += r;
                    continue;
                };
                let mut inner_style = style;
                match (c, n) {
                    (b'~', _) => inner_style.strike = true,
                    (b'=', _) => inner_style.highlight = true,
                    (_, 1) => inner_style.emphasis = true,
                    (_, 2) => inner_style.strong = true,
                    _ => (inner_style.strong, inner_style.emphasis) = (true, true),
                }
                let follow = link.clone();
                span(out, &mut plain, i..i + n, j..j + n, &mut |out, _| {
                    inline(text, i + n..j, inner_style, at, follow.clone(), out);
                });
                i = j + n;
            }
            b'#' if i == 0 || b[i - 1].is_ascii_whitespace() => {
                let len = text[i + 1..end]
                    .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '/')))
                    .unwrap_or(end - i - 1);
                let name = &text[i + 1..i + 1 + len];
                if !name.is_empty() && !name.chars().all(|c| c.is_ascii_digit()) {
                    push(out, plain..i, Show::Text, style, link.clone());
                    push(out, i..i + 1 + len, Show::Text, Style { tag: true, ..style }, link.clone());
                    plain = i + 1 + len;
                }
                i += 1 + len;
            }
            _ => i += 1,
        }
    }
    push(out, plain..end, Show::Text, style, link);
}

/// How many times the byte at `i` repeats from there.
fn run(b: &[u8], i: usize, end: usize) -> usize {
    b[i..end].iter().take_while(|&&c| c == b[i]).count()
}

/// The next run of exactly `n` of `c`.
fn find_run(b: &[u8], mut j: usize, end: usize, c: u8, n: usize) -> Option<usize> {
    while j < end {
        if b[j] == c {
            let r = run(b, j, end);
            if r == n {
                return Some(j);
            }
            j += r;
        } else {
            j += 1;
        }
    }
    None
}

/// The run that closes emphasis opened before `from`: exactly as long, after
/// some text, not after a space; for `_`, not inside a word.
fn find_closer(b: &[u8], from: usize, end: usize, c: u8, n: usize) -> Option<usize> {
    let mut j = from;
    while let Some(at) = find_run(b, j, end, c, n) {
        let after_text = at > from && !b[at - 1].is_ascii_whitespace();
        let word_end = c != b'_' || b.get(at + n).is_none_or(|c| !c.is_ascii_alphanumeric());
        if after_text && word_end {
            return Some(at);
        }
        j = at + n;
    }
    None
}

/// A block that live preview draws as the preview pane does while the cursor
/// is outside it: the properties, a table, an embed or picture, a `view` or
/// `tasks` block. `blocks` is its text parsed on its own, which gives the same
/// view sources and embed targets the preview fetched results for.
#[derive(Debug, Clone, PartialEq)]
pub struct Island {
    pub lines: Range<usize>,
    pub blocks: Vec<sage_markdown::Block>,
    /// The lines `blocks` were parsed from, joined, so the next edit can tell
    /// whether they changed.
    source: String,
}

/// The note's islands, in order. Only blocks at the top level count: a table
/// inside a quote or a list stays text. An island of `previous`, the islands
/// before the edit, whose lines did not change is kept rather than parsed
/// again, wherever it moved.
pub fn islands(text: &str, mut previous: Vec<Island>) -> Vec<Island> {
    use sage_markdown::Block;
    // Split where the editor splits, lone `\r` and `\n\r` included, so an
    // island's lines are the lines the pane shows; `str::lines` would differ.
    let lines: Vec<&str> = cosmic_text::LineIter::new(text).map(|(range, _)| &text[range]).collect();
    let mut out = Vec::new();
    let mut keep = |range: Range<usize>, fits: fn(&Block) -> bool| {
        let source = lines[range.clone()].join("\n");
        if let Some(i) = previous.iter().position(|island| island.source == source && island.blocks.first().is_some_and(fits)) {
            out.push(Island { lines: range, ..previous.swap_remove(i) });
            return;
        }
        let blocks = sage_markdown::parse(&source);
        if blocks.first().is_some_and(fits) {
            out.push(Island { lines: range, blocks, source });
        }
    };
    let mut i = 0;
    // A byte-order mark may sit before the opening fence.
    if lines.first().is_some_and(|l| l.trim_start_matches('\u{feff}').trim_end() == "---") {
        match lines.iter().skip(1).position(|l| matches!(l.trim_end(), "---" | "...")) {
            Some(n) => {
                keep(0..n + 2, |b| matches!(b, Block::Properties(_)));
                i = n + 2;
            }
            None => return out,
        }
    }
    while i < lines.len() {
        let line = lines[i];
        if let Some((open, info)) = fence_open(line) {
            // Only an unindented block is an island; any fence is skipped.
            let language = info.split_whitespace().next().unwrap_or("").to_lowercase();
            let language = if line.starts_with(['`', '~']) { language } else { String::new() };
            let Some(n) = lines[i + 1..].iter().position(|l| fence_closes(l, open)) else { break };
            let end = i + 1 + n + 1;
            if language == "view" || language == "tasks" {
                keep(i..end, |b| matches!(b, Block::View { .. } | Block::TaskQuery { .. }));
            }
            i = end;
        } else if line.starts_with('|') && lines.get(i + 1).is_some_and(|l| is_delimiter_row(l)) {
            let rows = lines[i + 2..].iter().take_while(|l| !l.trim().is_empty() && l.contains('|')).count();
            keep(i..i + 2 + rows, |b| matches!(b, Block::Table { .. }));
            i += 2 + rows;
        } else {
            let t = line.trim_end();
            if (t.starts_with("![[") && t.ends_with("]]")) || (t.starts_with("![") && t.ends_with(')') && t.contains("](")) {
                keep(i..i + 1, |b| matches!(b, Block::Embed(_)));
            }
            i += 1;
        }
    }
    out
}

/// A table's second line, such as `|:---|:---:|---:|`.
fn is_delimiter_row(line: &str) -> bool {
    let cells: Vec<&str> = line.trim().trim_matches('|').split('|').map(str::trim).collect();
    !cells.is_empty()
        && cells.iter().all(|c| {
            let dashes = c.trim_start_matches(':').trim_end_matches(':');
            !dashes.is_empty() && dashes.chars().all(|ch| ch == '-')
        })
}

/// Whether the cursor or selection (lines `from..=to`) is inside an island,
/// which then shows as written.
pub fn touches_island(lines: &Range<usize>, caret: Caret) -> bool {
    caret.from.0 < lines.end && caret.to.0 >= lines.start
}

/// A piece's place in the line as written and as shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    pub source: Range<usize>,
    pub shown: Range<usize>,
    /// Shows the text as written, so columns inside map one to one.
    pub verbatim: bool,
}

/// The text a line shows, and where each piece of it came from.
pub fn shown(line: &Line, text: &str) -> (String, Vec<Seg>) {
    let mut out = String::new();
    let mut segs = Vec::with_capacity(line.pieces.len());
    for piece in &line.pieces {
        let start = out.len();
        match &piece.show {
            Show::Text => out.push_str(&text[piece.range.clone()]),
            Show::Hidden => {}
            Show::Replace(s) => out.push_str(s),
            Show::Checkbox(_) => out.push_str(CHECKBOX),
        }
        segs.push(Seg { source: piece.range.clone(), shown: start..out.len(), verbatim: piece.show == Show::Text });
    }
    (out, segs)
}

/// Where a column as written shows. Hidden syntax collapses to a point.
pub fn to_shown(segs: &[Seg], column: usize) -> usize {
    for seg in segs {
        if column < seg.source.end {
            return if seg.verbatim {
                seg.shown.start + column.saturating_sub(seg.source.start)
            } else if column <= seg.source.start {
                seg.shown.start
            } else {
                seg.shown.end
            };
        }
    }
    segs.last().map_or(0, |s| s.shown.end)
}

/// The column as written for a place in the shown text. A place inside a
/// replacement, such as a checkbox, goes after its syntax.
pub fn to_source(segs: &[Seg], shown: usize) -> usize {
    for seg in segs {
        if shown < seg.shown.end {
            return if seg.verbatim { seg.source.start + shown.saturating_sub(seg.shown.start) } else { seg.source.end };
        }
    }
    segs.last().map_or(0, |s| s.source.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A note's islands, with none from before an edit to keep.
    fn islands_of(text: &str) -> Vec<Island> {
        islands(text, Vec::new())
    }

    #[test]
    fn unchanged_islands_are_kept_where_they_moved() {
        let before = islands_of("| a |\n|---|\n| 1 |\n\n![[pic.png]]\n");
        let after = islands("new line\n| a |\n|---|\n| 1 |\n\n![[other.png]]\n", before.clone());
        assert_eq!(after[0].lines, 1..4, "the table moved down a line");
        assert_eq!((&after[0].blocks, &after[0].source), (&before[0].blocks, &before[0].source));
        assert_eq!(after[1].blocks, islands_of("![[other.png]]")[0].blocks, "the changed embed is parsed again");
    }

    #[test]
    fn islands_count_lines_as_the_editor_does() {
        // A lone carriage return breaks the line in the editor, so it must here too.
        let text = "intro\rmore\n| a |\r\n|---|\n| 1 |\n";
        assert_eq!(islands_of(text)[0].lines, 2..5);
        let content = iced::widget::text_editor::Content::<iced::Renderer>::with_text(text);
        assert_eq!(content.line(2).map(|l| l.text.into_owned()).as_deref(), Some("| a |"));
    }

    #[test]
    fn a_line_worked_out_on_its_own_matches_the_whole_note() {
        let note = ["---", "a: b", "---", "> [!note] Title", "> **body** [[x]]", "```", "# not a heading", "```", "## Head **b**"];
        let caret = Some(Caret::new((4, 3), (6, 1)));
        let all = lines(&note, caret);
        let mut carry = Carry::default();
        let mut carries = Vec::new();
        for (n, text) in note.iter().enumerate() {
            carries.push(carry.clone());
            next_line(text, n, &mut carry, None);
        }
        for (n, text) in note.iter().enumerate() {
            assert_eq!(next_line(text, n, &mut carries[n].clone(), caret), all[n], "line {n}");
        }
    }

    #[test]
    fn rules_need_three_of_one_mark() {
        for rule in ["---", "***", "___", "- - -", " *  * * * "] {
            assert!(is_rule(rule), "{rule:?}");
        }
        for text in ["--", "-*-", "", "   ", "-- x", "———"] {
            assert!(!is_rule(text), "{text:?}");
        }
    }

    #[test]
    fn links_to_notes_that_do_not_exist_are_missing() {
        set_known(["People/John.md", "Plan.md"].into_iter().map(str::to_owned), vec!["jo".into()]);
        assert!(!is_missing("john"));
        assert!(!is_missing("People/John|Johnny"));
        assert!(!is_missing("Plan#Goals"));
        assert!(!is_missing("jo"), "an alias");
        assert!(!is_missing("#Heading"), "the note itself");
        assert!(!is_missing("photo.png"), "an attachment");
        assert!(is_missing("Nobody"));
        assert!(is_missing("Other/Plan"));

        // The pane lays out again when a note appears, and only then.
        let generation = known_generation();
        set_known(["People/John.md", "Plan.md"].into_iter().map(str::to_owned), vec!["jo".into()]);
        assert_eq!(known_generation(), generation, "the same notes");
        set_known(["People/John.md", "Plan.md", "Nobody.md"].into_iter().map(str::to_owned), vec!["jo".into()]);
        assert_eq!(known_generation(), generation + 1);
        assert!(!is_missing("Nobody"));
    }

    /// What a one-line note shows with the cursor at `cursor`, if anywhere.
    fn show(text: &str, cursor: Option<usize>) -> String {
        let caret = cursor.map(|c| Caret::new((0, c), (0, c)));
        let lines = lines(&[text], caret);
        shown(&lines[0], text).0
    }

    /// What every line of a note shows with the cursor on `(line, column)`.
    fn show_all(text: &[&str], cursor: (usize, usize)) -> Vec<String> {
        let lines = lines(text, Some(Caret::new(cursor, cursor)));
        lines.iter().zip(text).map(|(l, t)| shown(l, t).0).collect()
    }

    fn covers(text: &str, line: &Line) {
        let mut at = 0;
        for piece in &line.pieces {
            assert_eq!(piece.range.start, at, "{text:?}: {:?}", line.pieces);
            assert!(!piece.range.is_empty(), "{text:?}");
            at = piece.range.end;
        }
        assert_eq!(at, text.len(), "{text:?}: {:?}", line.pieces);
    }

    #[test]
    fn heading_marks_show_only_on_the_cursor_line() {
        assert_eq!(show("## Plans", None), "Plans");
        assert_eq!(show("## Plans", Some(8)), "## Plans");
        let lines = lines(&["## Plans"], None);
        assert_eq!(lines[0].kind, Kind::Heading(2));
        assert_eq!(show("#tag", None), "#tag", "a tag is not a heading");
        assert_eq!(show("####### seven", None), "####### seven");
    }

    #[test]
    fn inline_syntax_shows_while_the_cursor_touches_it() {
        let text = "a **bold** and *it* and ~~gone~~ and ==lit== and `code`";
        assert_eq!(show(text, None), "a bold and it and gone and lit and code");
        // Inside the bold, and right after it: its stars show, nothing else's.
        assert_eq!(show(text, Some(5)), "a **bold** and it and gone and lit and code");
        assert_eq!(show(text, Some(10)), "a **bold** and it and gone and lit and code");
        assert_eq!(show(text, Some(11)), "a bold and it and gone and lit and code");
        assert_eq!(show(text, Some(text.len())), "a bold and it and gone and lit and `code`");
    }

    #[test]
    fn styles_nest() {
        let text = "**bold *both* bold** ***all***";
        let line = &lines(&[text], None)[0];
        let style_of = |word: &str| {
            let at = text.find(word).unwrap();
            line.pieces.iter().find(|p| p.range.contains(&at)).unwrap().style
        };
        assert!(style_of("bold").strong && !style_of("bold").emphasis);
        assert!(style_of("both").strong && style_of("both").emphasis);
        assert!(style_of("all").strong && style_of("all").emphasis);
        assert_eq!(show(text, None), "bold both bold all");
    }

    #[test]
    fn stray_marks_stay_text() {
        for text in ["snake_case_name", "2 * 3 * 4", "**open", "a ~ b", "= x =", "`open", "[[open", "[x](open"] {
            assert_eq!(show(text, None), text);
        }
    }

    #[test]
    fn links_show_their_text() {
        assert_eq!(show("see [[Note]] now", None), "see Note now");
        assert_eq!(show("see [[Note|the note]] now", None), "see the note now");
        assert_eq!(show("see [[Note#Plans]] now", None), "see Note > Plans now");
        assert_eq!(show("see [[#Plans]] now", None), "see Plans now");
        assert_eq!(show("see [site](https://x.org) now", None), "see site now");
        assert_eq!(show("see [**big** site](x.md)", None), "see big site");
        assert_eq!(show("see [[Note|the note]] now", Some(7)), "see [[Note|the note]] now");
        // A shown link is followed by clicking; one being edited is not.
        let text = "see [[Note|alias]]";
        let line = &lines(&[text], None)[0];
        let alias = line.pieces.iter().find(|p| &text[p.range.clone()] == "alias").unwrap();
        assert_eq!(alias.link, Some(4..text.len()));
        let line = &lines(&[text], Some(Caret::new((0, 6), (0, 6))))[0];
        assert!(line.pieces.iter().all(|p| p.link.is_none()));
    }

    #[test]
    fn tags_are_styled_not_hidden() {
        let text = "a #tag/sub and #123 and x#no";
        let line = &lines(&[text], None)[0];
        let tags: Vec<&str> = line.pieces.iter().filter(|p| p.style.tag).map(|p| &text[p.range.clone()]).collect();
        assert_eq!(tags, ["#tag/sub"]);
    }

    #[test]
    fn lists_and_tasks() {
        assert_eq!(show("- item", None), "• item");
        assert_eq!(show("- item", Some(1)), "- item");
        assert_eq!(show("- item", Some(3)), "• item", "the cursor in the text leaves the bullet");
        assert_eq!(show("  1. first", None), "  1. first");
        assert_eq!(show("- [ ] todo", None), format!("{CHECKBOX} todo"));
        assert_eq!(show("- [ ] todo", Some(4)), "- [ ] todo");
        assert_eq!(show("- [ ] todo", Some(6)), format!("{CHECKBOX} todo"));
        let done = &lines(&["- [x] done"], None)[0];
        assert!(done.done);
        assert!(done.pieces.iter().any(|p| p.show == Show::Checkbox(true)));
        assert_eq!(show("-not a list", None), "-not a list");
        assert_eq!(show("*not a list*", None), "not a list");
    }

    #[test]
    fn quotes_and_callouts() {
        let note = ["> [!warning]- Careful", "> body **here**", "> > nested", "after"];
        let away = show_all(&note, (3, 0));
        assert_eq!(away, ["Careful", "body here", "nested", "after"]);
        let lines = lines(&note, None);
        assert!(lines[0].title);
        assert_eq!(lines[0].callout.as_deref(), Some("warning"));
        assert_eq!(lines[1].callout.as_deref(), Some("warning"));
        assert_eq!(lines[2].quotes, 2);
        assert_eq!(lines[3].callout, None);
        assert_eq!(show_all(&note, (1, 3))[1], "> body here", "the `>` shows on the line, the stars only at the word");
        assert_eq!(show_all(&note, (1, 10))[1], "> body **here**");
        // No title of its own: the kind, as Obsidian shows it.
        assert_eq!(show("> [!tip]", None), "Tip");
        assert_eq!(show("> plain quote", None), "plain quote");
    }

    #[test]
    fn code_and_frontmatter_stay_as_written() {
        let note = ["---", "tags: [a]", "---", "```rust", "let **x** = 1;", "```", "**after**"];
        let away = show_all(&note, (0, 0));
        assert_eq!(&away[1..6], &note[1..6]);
        assert_eq!(away[6], "after");
        let kinds: Vec<Kind> = lines(&note, None).iter().map(|l| l.kind).collect();
        use Kind::*;
        assert_eq!(kinds, [Frontmatter, Frontmatter, Frontmatter, Fence, Code, Fence, Text]);
        // `---` further down is a rule.
        let lines = lines(&["text", "---"], None);
        assert_eq!(lines[1].kind, Kind::Rule);
        assert_eq!(shown(&lines[1], "---").0, "");
    }

    #[test]
    fn a_fence_closes_only_on_its_own_kind_and_length() {
        use Kind::*;
        let kinds = |note: &[&str]| lines(note, None).iter().map(|l| l.kind).collect::<Vec<_>>();
        // From the sample vault's Kanban page: a view block shown as code.
        let nested = ["````markdown", "```view", "type: kanban", "```", "````", "after"];
        assert_eq!(kinds(&nested), [Fence, Code, Code, Code, Fence, Text]);
        assert_eq!(kinds(&["~~~", "```", "code", "~~~", "x"]), [Fence, Code, Code, Fence, Text]);
        assert_eq!(kinds(&["```", "~~~", "```", "x"]), [Fence, Code, Fence, Text]);
        assert_eq!(kinds(&["```", "```rust", "```", "x"]), [Fence, Code, Fence, Text], "a fence with a language never closes");
        assert_eq!(kinds(&["```", "`````", "x"]), [Fence, Fence, Text], "a longer fence closes");
        assert!(islands_of("````markdown\n```view\nsource: x\n```\n````\n").is_empty(), "a view block shown as code");
        assert_eq!(islands_of("~~~\n```\n~~~\n```view\nsource: x\n```\n")[0].lines, 3..6);
    }

    #[test]
    fn a_selection_shows_the_syntax_of_every_line_it_covers() {
        let note = ["# One", "**two**", "# Three", "# Four"];
        let lines = lines(&note, Some(Caret::new((2, 3), (0, 2))));
        let shown: Vec<String> = lines.iter().zip(note).map(|(l, t)| shown(l, t).0).collect();
        assert_eq!(shown, ["# One", "**two**", "# Three", "Four"]);
    }

    #[test]
    fn pieces_cover_every_line() {
        let samples = [
            "",
            "plain",
            "## Head **bold** [[Link|alias]] #tag",
            "> [!note] Title with `code`",
            "  - [x] done ==lit== ~~gone~~ [a](b.md)",
            "- [ ]",
            "1) one",
            "![[pic.png|300]] and ![alt](pic.png)",
            "ünïcödé **ŵörds** _ítalic_ #tâg",
            "\\*escaped\\* *",
            "***",
        ];
        for text in samples {
            for cursor in [None, Some(0), Some(text.len() / 2), Some(text.len())] {
                let caret = cursor.filter(|&c| text.is_char_boundary(c)).map(|c| Caret::new((0, c), (0, c)));
                covers(text, &lines(&[text], caret)[0]);
            }
        }
    }

    #[test]
    fn blocks_the_preview_draws_are_islands() {
        let note = "---\ntags: [a]\n---\n# Title\n\n| a | b |\n|:--|--:|\n| 1 | 2 |\n\n![[Other note]]\n![alt|200](pic.png)\n\n```view\nsource: \"#a\"\n```\n\n```tasks\nstatus != done\n```\n\n```rust\n| not | a |\n|---|---|\n```\n> | quoted | table |\n> |---|---|\ntext ![[inline]] here\n";
        let found: Vec<(Range<usize>, &str)> = islands_of(note)
            .into_iter()
            .map(|i| {
                let kind = match &i.blocks[0] {
                    sage_markdown::Block::Properties(_) => "properties",
                    sage_markdown::Block::Table { .. } => "table",
                    sage_markdown::Block::Embed(_) => "embed",
                    sage_markdown::Block::View { .. } => "view",
                    sage_markdown::Block::TaskQuery { .. } => "tasks",
                    _ => "other",
                };
                (i.lines, kind)
            })
            .collect();
        assert_eq!(
            found,
            [
                (0..3, "properties"),
                (5..8, "table"),
                (9..10, "embed"),
                (10..11, "embed"),
                (12..15, "view"),
                (16..19, "tasks"),
            ]
        );
    }

    #[test]
    fn frontmatter_after_a_byte_order_mark_is_still_frontmatter() {
        assert_eq!(islands_of("\u{feff}---\ntitle: x\n---\nbody").len(), 1);
    }

    #[test]
    fn broken_or_open_blocks_are_not_islands() {
        assert!(islands_of("---\ntitle: x\nno end").is_empty(), "frontmatter never closed");
        assert!(islands_of("```view\nsource: x").is_empty(), "fence never closed");
        assert!(islands_of("| a | b |\nnot a delimiter\n").is_empty());
        assert!(islands_of("    ![[indented]]").is_empty());
    }

    #[test]
    fn the_cursor_inside_an_island_shows_it_as_written() {
        let island = &islands_of("intro\n| a |\n|---|\n| 1 |\nafter")[0];
        assert_eq!(island.lines, 1..4);
        let at = |line| Caret::new((line, 0), (line, 0));
        assert!(!touches_island(&island.lines, at(0)));
        assert!(touches_island(&island.lines, at(1)));
        assert!(touches_island(&island.lines, at(3)));
        assert!(!touches_island(&island.lines, at(4)));
        assert!(touches_island(&island.lines, Caret::new((0, 2), (4, 0))), "a selection across it");
    }

    #[test]
    fn columns_map_between_source_and_shown() {
        let text = "a **bold** b";
        let line = &lines(&[text], None)[0];
        let (shown, segs) = shown(line, text);
        assert_eq!(shown, "a bold b");
        assert_eq!(to_shown(&segs, 0), 0);
        assert_eq!(to_shown(&segs, 4), 2, "the b of bold");
        assert_eq!(to_shown(&segs, 3), 2, "inside the hidden stars");
        assert_eq!(to_shown(&segs, text.len()), shown.len());
        assert_eq!(to_source(&segs, 2), 4);
        assert_eq!(to_source(&segs, 6), 10, "after bold, past the closing stars");
        assert_eq!(to_source(&segs, shown.len()), text.len());

        let text = "- [ ] task";
        let line = &lines(&[text], None)[0];
        let (_, segs) = self::shown(line, text);
        assert_eq!(to_source(&segs, 0), 5, "a click on the box goes after it");
        assert_eq!(to_shown(&segs, 6), CHECKBOX.len() + 1);
    }
}
