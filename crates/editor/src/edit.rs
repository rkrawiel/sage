//! Editing commands on a note's text: formatting toggles, smart lists, moving
//! and duplicating lines, and auto-closed pairs.
//!
//! Pure string logic over the note's lines, so it is tested here without a UI.
//! Columns are byte offsets within a line, matching iced's editor cursor. Each
//! command returns a [`Change`]: the lines it replaces, their new text, and
//! where the cursor and selection go.

/// A place in the text: a line and a byte column in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// Lines `first..=last` become `text`; then the selection runs from `anchor`
/// to `head`, the cursor, and is empty when they are equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub first: usize,
    pub last: usize,
    pub text: String,
    pub anchor: Pos,
    pub head: Pos,
}

/// The inline formats Ctrl+B, Ctrl+I and Ctrl+` toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Code,
}

impl Format {
    fn marker(self) -> &'static str {
        match self {
            Format::Bold => "**",
            Format::Italic => "*",
            Format::Code => "`",
        }
    }
}

/// The lines `first..=last` as one string, and a way to turn a position in
/// them into a byte offset in it and back.
struct Block {
    first: usize,
    last: usize,
    text: String,
    starts: Vec<usize>,
}

impl Block {
    fn new(lines: &[String], first: usize, last: usize) -> Self {
        let mut text = String::new();
        let mut starts = Vec::new();
        for (i, line) in lines[first..=last].iter().enumerate() {
            if i > 0 {
                text.push('\n');
            }
            starts.push(text.len());
            text.push_str(line);
        }
        Self { first, last, text, starts }
    }

    fn offset(&self, pos: Pos) -> usize {
        self.starts[pos.line - self.first] + pos.col
    }

    /// The position of byte `offset` in `text`, which may have changed lines.
    fn pos_in(&self, text: &str, offset: usize) -> Pos {
        let before = &text[..offset];
        let line = before.matches('\n').count();
        let col = offset - before.rfind('\n').map_or(0, |i| i + 1);
        Pos::new(self.first + line, col)
    }

    fn change(&self, text: String, anchor: usize, head: usize) -> Change {
        let (anchor, head) = (self.pos_in(&text, anchor), self.pos_in(&text, head));
        Change { first: self.first, last: self.last, text, anchor, head }
    }
}

fn ordered(anchor: Pos, head: Pos) -> (Pos, Pos) {
    if anchor <= head { (anchor, head) } else { (head, anchor) }
}

/// How many `c` there are in a row at the end of `s`, and at its start.
fn run_back(s: &str, c: char) -> usize {
    s.chars().rev().take_while(|&x| x == c).count()
}
fn run_front(s: &str, c: char) -> usize {
    s.chars().take_while(|&x| x == c).count()
}

/// A run of `run` marker characters closes a span of this marker: `**` and
/// `***` for bold, `*` and `***` for italic, a single backtick for code.
fn fits(run: usize, marker: &str) -> bool {
    run == marker.len() || (marker.starts_with('*') && run == 3)
}

/// Bold, italic or inline code on the selection: added, or taken off when the
/// selection already has it, inside or just outside. Without a selection, the
/// word under the cursor; between an empty pair of markers, the pair goes.
pub fn toggle(lines: &[String], anchor: Pos, head: Pos, format: Format) -> Change {
    let marker = format.marker();
    let m = marker.len();
    let c = marker.chars().next().unwrap_or('*');
    let (start, end) = ordered(anchor, head);
    let block = Block::new(lines, start.line, end.line);
    let text = &block.text;
    let (mut s, mut e) = (block.offset(start), block.offset(end));

    if s == e {
        let (before, after) = (&text[..s], &text[s..]);
        // `**|**`: an empty pair, just inserted. Take it off again.
        if before.ends_with(marker) && after.starts_with(marker) && fits(run_back(before, c), marker) {
            let out = format!("{}{}", &text[..s - m], &text[s + m..]);
            return block.change(out, s - m, s - m);
        }
        let word = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '-';
        s = before.char_indices().rev().take_while(|(_, ch)| word(*ch)).last().map_or(s, |(i, _)| i);
        e += after.char_indices().take_while(|(_, ch)| word(*ch)).last().map_or(0, |(i, ch)| i + ch.len_utf8());
        if s == e {
            let out = format!("{}{marker}{marker}{}", &text[..s], &text[s..]);
            return block.change(out, s + m, s + m);
        }
        let cursor = block.offset(start);
        let (out, shift) = wrap_or_unwrap(text, s, e, marker, c);
        let at = (cursor as isize + shift) as usize;
        return block.change(out.0, at, at);
    }
    let (out, _) = wrap_or_unwrap(text, s, e, marker, c);
    block.change(out.0, out.1, out.2)
}

/// Wraps `s..e` in `marker`, or unwraps it if it is wrapped already. Returns
/// the text with the new selection, and how far the selection's start moved.
fn wrap_or_unwrap(text: &str, s: usize, e: usize, marker: &str, c: char) -> ((String, usize, usize), isize) {
    let m = marker.len();
    let inner = &text[s..e];
    // The markers are selected along with the text.
    if inner.len() >= 2 * m
        && inner.starts_with(marker)
        && inner.ends_with(marker)
        && fits(run_front(inner, c), marker)
        && fits(run_back(inner, c), marker)
    {
        let out = format!("{}{}{}", &text[..s], &inner[m..inner.len() - m], &text[e..]);
        return ((out, s, e - 2 * m), 0);
    }
    // The markers are just outside the selection.
    let (before, after) = (&text[..s], &text[e..]);
    if before.ends_with(marker) && after.starts_with(marker) && fits(run_back(before, c), marker) && fits(run_front(after, c), marker) {
        let out = format!("{}{inner}{}", &text[..s - m], &text[e + m..]);
        return ((out, s - m, e - m), -(m as isize));
    }
    let out = format!("{before}{marker}{inner}{marker}{after}");
    ((out, s + m, e + m), m as isize)
}

/// Ctrl+K: the selection becomes a Markdown link's text, with the cursor
/// where the address goes; a selected address becomes the address instead.
/// A selected link is taken apart again, back to its text.
pub fn link(lines: &[String], anchor: Pos, head: Pos) -> Change {
    let (start, end) = ordered(anchor, head);
    let block = Block::new(lines, start.line, end.line);
    let text = &block.text;
    let (s, e) = (block.offset(start), block.offset(end));
    let inner = &text[s..e];
    if let Some(label) = inner.strip_prefix('[').and_then(|r| r.strip_suffix(')')).and_then(|r| r.split_once("](")).map(|(l, _)| l) {
        let out = format!("{}{label}{}", &text[..s], &text[e..]);
        return block.change(out, s, s + label.len());
    }
    let url = inner.starts_with("http://") || inner.starts_with("https://") || inner.starts_with("mailto:");
    let out = if url {
        format!("{}[]({inner}){}", &text[..s], &text[e..])
    } else {
        format!("{}[{inner}](){}", &text[..s], &text[e..])
    };
    let at = if url { s + 1 } else { s + inner.len() + 3 };
    block.change(out, at, at)
}

/// What starts a line that Enter continues: indentation, quote markers, and a
/// bullet, number or task box. `body` is where the line's own text begins.
#[derive(Debug, PartialEq, Eq)]
struct Prefix<'a> {
    indent: &'a str,
    quote: &'a str,
    /// `- `, `* `, `+ `, `1. ` or `1) `, empty for a quote alone.
    bullet: &'a str,
    /// The next number for a numbered list.
    number: Option<u64>,
    task: bool,
    body: usize,
}

fn prefix(line: &str) -> Option<Prefix<'_>> {
    let indent_end = line.len() - line.trim_start_matches([' ', '\t']).len();
    let indent = &line[..indent_end];
    let mut at = indent_end;
    // Quote markers: `>`, `> >`, `>> `…
    while line[at..].starts_with('>') {
        at += 1;
        if line[at..].starts_with(' ') {
            at += 1;
        }
    }
    let quote = &line[indent_end..at];
    let rest = &line[at..];
    let mut bullet = "";
    let mut number = None;
    if matches!(rest.chars().next(), Some('-' | '*' | '+')) {
        if rest[1..].starts_with(' ') {
            bullet = &line[at..at + 2];
        }
    } else {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && digits < 10 && rest[digits..].starts_with(['.', ')']) && rest[digits + 1..].starts_with(' ') {
            bullet = &line[at..at + digits + 2];
            number = rest[..digits].parse::<u64>().ok().map(|n| n + 1);
        }
    }
    at += bullet.len();
    let mut task = false;
    if !bullet.is_empty() {
        let r = &line[at..];
        if r.len() >= 4 && r.starts_with('[') && r.is_char_boundary(2) && r[2..].starts_with("] ") {
            task = true;
            at += 4;
        }
    }
    if quote.is_empty() && bullet.is_empty() {
        return None;
    }
    Some(Prefix { indent, quote, bullet, number, task, body: at })
}

/// Enter at `at`: a list item, task or quote carries on onto the new line.
/// Enter on an empty item ends the list: a nested item moves out a level
/// first, using `unit` as one level of indentation. `None` for a plain line,
/// or with the cursor inside the marker, where Enter breaks the line as usual.
pub fn enter(lines: &[String], at: Pos, unit: &str) -> Option<Change> {
    let line = lines.get(at.line)?;
    let p = prefix(line)?;
    if at.col < p.body {
        return None;
    }
    let one = |text: String, col: usize| Change {
        first: at.line,
        last: at.line,
        text,
        anchor: Pos::new(at.line, col),
        head: Pos::new(at.line, col),
    };
    if line[p.body..].trim().is_empty() {
        if !p.indent.is_empty() && !p.bullet.is_empty() {
            let indent = outdented(p.indent, unit);
            let text = format!("{indent}{}", &line[p.indent.len()..]);
            let col = text.len();
            return Some(one(text, col));
        }
        // An empty bullet or quote line: the marker goes, the line stays.
        let keep = if p.bullet.is_empty() {
            // `> > ` ends one level of quote at a time.
            let inner = p.quote.trim_end().strip_suffix('>').unwrap_or("").trim_end();
            if inner.is_empty() { String::new() } else { format!("{}{inner} ", p.indent) }
        } else {
            format!("{}{}", p.indent, p.quote)
        };
        let col = keep.len();
        return Some(one(keep, col));
    }
    let bullet = match p.number {
        Some(n) => format!("{n}{}", &p.bullet[p.bullet.len() - 2..]),
        None => p.bullet.to_owned(),
    };
    let task = if p.task { "[ ] " } else { "" };
    let next = format!("{}{}{bullet}{task}", p.indent, p.quote);
    let col = next.len();
    let text = format!("{}\n{next}{}", &line[..at.col], &line[at.col..]);
    Some(Change { first: at.line, last: at.line, text, anchor: Pos::new(at.line + 1, col), head: Pos::new(at.line + 1, col) })
}

fn outdented<'a>(indent: &'a str, unit: &str) -> &'a str {
    if let Some(rest) = indent.strip_suffix('\t') {
        return rest;
    }
    // With tabs as the unit, a level written in spaces is four of them.
    let width = if unit == "	" { 4 } else { unit.len().max(1) };
    let spaces = indent.len() - indent.trim_end_matches(' ').len();
    let cut = spaces.min(width);
    &indent[..indent.len() - cut]
}

/// Tab and Shift+Tab. List lines and selections over several lines move in
/// or out by one `unit`; elsewhere Tab types `unit` over the selection.
/// `None` when Shift+Tab has nothing to take away.
pub fn indent(lines: &[String], anchor: Pos, head: Pos, out: bool, unit: &str) -> Option<Change> {
    let (start, end) = ordered(anchor, head);
    let whole = start.line != end.line || prefix(&lines[start.line]).is_some_and(|p| !p.bullet.is_empty());
    if !whole && !out {
        let block = Block::new(lines, start.line, end.line);
        let (s, e) = (block.offset(start), block.offset(end));
        let text = format!("{}{unit}{}", &block.text[..s], &block.text[e..]);
        return Some(block.change(text, s + unit.len(), s + unit.len()));
    }
    let mut text = Vec::new();
    let mut shifts = Vec::new();
    for line in &lines[start.line..=end.line] {
        let (new, shift) = if out {
            let lead = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
            let kept = outdented(lead, unit);
            let cut = lead.len() - kept.len();
            (format!("{kept}{}", &line[lead.len()..]), -(cut as isize))
        } else if line.trim().is_empty() {
            (line.clone(), 0)
        } else {
            (format!("{unit}{line}"), unit.len() as isize)
        };
        text.push(new);
        shifts.push(shift);
    }
    if shifts.iter().all(|s| *s == 0) {
        return None;
    }
    let moved = |p: Pos| {
        let shift = shifts[p.line - start.line];
        Pos::new(p.line, (p.col as isize + shift).max(0) as usize)
    };
    Some(Change { first: start.line, last: end.line, text: text.join("\n"), anchor: moved(anchor), head: moved(head) })
}

/// The lines the selection touches. A selection that ends at the very start
/// of a line does not take that line along.
fn span(anchor: Pos, head: Pos) -> (usize, usize) {
    let (start, end) = ordered(anchor, head);
    let last = if end.line > start.line && end.col == 0 { end.line - 1 } else { end.line };
    (start.line, last)
}

/// Alt+↑ and Alt+↓: the selected lines swap places with the line above or
/// below, keeping the selection on them. `None` at the top or bottom.
pub fn move_lines(lines: &[String], anchor: Pos, head: Pos, up: bool) -> Option<Change> {
    let (first, last) = span(anchor, head);
    let moved: Vec<&str> = lines[first..=last].iter().map(String::as_str).collect();
    let shifted = |p: Pos, by: isize| Pos::new((p.line as isize + by) as usize, p.col);
    if up {
        let above = lines.get(first.checked_sub(1)?)?;
        let text = [moved.join("\n"), above.clone()].join("\n");
        Some(Change { first: first - 1, last, text, anchor: shifted(anchor, -1), head: shifted(head, -1) })
    } else {
        let below = lines.get(last + 1)?;
        let text = [below.clone(), moved.join("\n")].join("\n");
        Some(Change { first, last: last + 1, text, anchor: shifted(anchor, 1), head: shifted(head, 1) })
    }
}

/// Ctrl+Shift+D: a copy of the selected lines goes below them, and the
/// selection moves onto the copy.
pub fn duplicate(lines: &[String], anchor: Pos, head: Pos) -> Change {
    let (first, last) = span(anchor, head);
    let block = lines[first..=last].join("\n");
    let n = last - first + 1;
    let down = |p: Pos| Pos::new(p.line + n, p.col);
    Change { first, last, text: format!("{block}\n{block}"), anchor: down(anchor), head: down(head) }
}

/// Typing `c`: brackets, backticks and `**` close themselves, a closing
/// character the cursor is already in front of is stepped over, and a
/// selection is wrapped. `None` where `c` is just typed.
pub fn type_char(lines: &[String], anchor: Pos, head: Pos, c: char) -> Option<Change> {
    let (start, end) = ordered(anchor, head);
    if start.line != end.line {
        return None;
    }
    let line = &lines[start.line];
    let (s, e) = (start.col, end.col);
    let (before, after) = (&line[..s], &line[e..]);
    let next = after.chars().next();
    let prev = before.chars().next_back();
    let close = match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '`' | '*' => Some(c),
        _ => None,
    };
    let one = |text: String, a: usize, h: usize| Change {
        first: start.line,
        last: start.line,
        text,
        anchor: Pos::new(start.line, a),
        head: Pos::new(start.line, h),
    };
    if s != e {
        let close = close?;
        let text = format!("{before}{c}{}{close}{after}", &line[s..e]);
        return Some(one(text, s + 1, e + 1));
    }
    // Step over the closing character already there.
    if matches!(c, ')' | ']' | '`' | '*') && next == Some(c) {
        return Some(one(line.clone(), s + 1, s + 1));
    }
    let free = next.is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | '}' | ',' | '.' | ';' | ':'));
    match c {
        '(' | '[' if free => Some(one(format!("{before}{c}{}{after}", close?), s + 1, s + 1)),
        // A second backtick in a row is a code fence being typed, not a pair.
        '`' if free && prev != Some('`') => Some(one(format!("{before}``{after}"), s + 1, s + 1)),
        // `*` alone may be a bullet or italics; a second one makes `**|**`.
        '*' if free && prev == Some('*') && run_back(before, '*') == 1 => {
            Some(one(format!("{before}***{after}"), s + 1, s + 1))
        }
        _ => None,
    }
}

/// Backspace between an empty pair, `(|)`, `[|]`, `` `|` `` or `**|**`,
/// takes both sides away. `None` elsewhere.
pub fn backspace(lines: &[String], at: Pos) -> Option<Change> {
    let line = lines.get(at.line)?;
    let (before, after) = (&line[..at.col], &line[at.col..]);
    let width = ["**", "(", "[", "`"]
        .into_iter()
        .find(|open| {
            let close = match *open {
                "(" => ")",
                "[" => "]",
                other => other,
            };
            before.ends_with(open) && after.starts_with(close)
        })?
        .len();
    let text = format!("{}{}", &before[..before.len() - width], &after[width..]);
    let col = at.col - width;
    Some(Change { first: at.line, last: at.line, text, anchor: Pos::new(at.line, col), head: Pos::new(at.line, col) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text with `|` for the cursor, or `{` and `}` for a selection's anchor
    /// and head, turned into lines and positions.
    fn parse(marked: &str) -> (Vec<String>, Pos, Pos) {
        let mut lines = Vec::new();
        let (mut anchor, mut head) = (None, None);
        for (i, raw) in marked.split('\n').enumerate() {
            let mut line = String::new();
            for ch in raw.chars() {
                match ch {
                    '|' => {
                        anchor = Some(Pos::new(i, line.len()));
                        head = anchor;
                    }
                    '{' => anchor = Some(Pos::new(i, line.len())),
                    '}' => head = Some(Pos::new(i, line.len())),
                    _ => line.push(ch),
                }
            }
            lines.push(line);
        }
        (lines, anchor.unwrap(), head.unwrap())
    }

    /// The whole text after a change, marked the same way.
    fn show(lines: &[String], change: &Change) -> String {
        let mut all: Vec<String> = lines[..change.first].to_vec();
        all.extend(change.text.split('\n').map(str::to_owned));
        all.extend(lines[change.last + 1..].iter().cloned());
        let mark = |all: &mut Vec<String>, p: Pos, m: char| all[p.line].insert(p.col, m);
        if change.anchor == change.head {
            mark(&mut all, change.head, '|');
        } else {
            // The later mark first, so the earlier one's column stays right.
            let (a, h) = (change.anchor, change.head);
            if a > h {
                mark(&mut all, a, '{');
                mark(&mut all, h, '}');
            } else {
                mark(&mut all, h, '}');
                mark(&mut all, a, '{');
            }
        }
        all.join("\n")
    }

    fn fmt(marked: &str, format: Format) -> String {
        let (lines, a, h) = parse(marked);
        show(&lines, &toggle(&lines, a, h, format))
    }

    #[test]
    fn formatting_wraps_and_unwraps() {
        assert_eq!(fmt("say {hello} there", Format::Bold), "say **{hello}** there");
        assert_eq!(fmt("say **{hello}** there", Format::Bold), "say {hello} there");
        assert_eq!(fmt("say {**hello**} there", Format::Bold), "say {hello} there");
        assert_eq!(fmt("say {hello}", Format::Italic), "say *{hello}*");
        assert_eq!(fmt("say *{hello}*", Format::Italic), "say {hello}");
        // Italic on bold text adds a star rather than taking bold apart.
        assert_eq!(fmt("say **{hello}**", Format::Italic), "say ***{hello}***");
        assert_eq!(fmt("say ***{hello}***", Format::Bold), "say *{hello}*");
        assert_eq!(fmt("run {cargo}", Format::Code), "run `{cargo}`");
        assert_eq!(fmt("run `{cargo}`", Format::Code), "run {cargo}");
    }

    #[test]
    fn formatting_without_a_selection() {
        assert_eq!(fmt("say hel|lo there", Format::Bold), "say **hel|lo** there");
        assert_eq!(fmt("say **hel|lo** there", Format::Bold), "say hel|lo there");
        assert_eq!(fmt("say |", Format::Bold), "say **|**");
        assert_eq!(fmt("say **|**", Format::Bold), "say |");
        assert_eq!(fmt("say |", Format::Code), "say `|`");
        assert_eq!(fmt("zażółć gę|ślą", Format::Italic), "zażółć *gę|ślą*");
    }

    #[test]
    fn links() {
        let (lines, a, h) = parse("see {the docs} now");
        assert_eq!(show(&lines, &link(&lines, a, h)), "see [the docs](|) now");
        let (lines, a, h) = parse("see {https://x.dev} now");
        assert_eq!(show(&lines, &link(&lines, a, h)), "see [|](https://x.dev) now");
        let (lines, a, h) = parse("see | now");
        assert_eq!(show(&lines, &link(&lines, a, h)), "see [](|) now");
        let (lines, a, h) = parse("see {[the docs](https://x.dev)} now");
        assert_eq!(show(&lines, &link(&lines, a, h)), "see {the docs} now");
    }

    fn press_enter(marked: &str) -> Option<String> {
        let (lines, a, _) = parse(marked);
        enter(&lines, a, "\t").map(|c| show(&lines, &c))
    }

    #[test]
    fn enter_continues_lists_tasks_and_quotes() {
        assert_eq!(press_enter("- milk|").unwrap(), "- milk\n- |");
        assert_eq!(press_enter("\t* eggs|").unwrap(), "\t* eggs\n\t* |");
        assert_eq!(press_enter("- [x] done|").unwrap(), "- [x] done\n- [ ] |");
        assert_eq!(press_enter("9. nine|").unwrap(), "9. nine\n10. |");
        assert_eq!(press_enter("1) one|").unwrap(), "1) one\n2) |");
        assert_eq!(press_enter("> quoted|").unwrap(), "> quoted\n> |");
        assert_eq!(press_enter("> - in a quote|").unwrap(), "> - in a quote\n> - |");
        // The text after the cursor moves to the new item.
        assert_eq!(press_enter("- buy| milk").unwrap(), "- buy\n-  milk".replacen("-  milk", "- | milk", 1));
    }

    #[test]
    fn enter_on_an_empty_item_ends_the_list() {
        assert_eq!(press_enter("- milk\n- |").unwrap(), "- milk\n|");
        assert_eq!(press_enter("- [ ] |").unwrap(), "|");
        assert_eq!(press_enter("> |").unwrap(), "|");
        assert_eq!(press_enter("> > |").unwrap(), "> |");
        // A nested empty item moves out a level first.
        assert_eq!(press_enter("- a\n\t- |").unwrap(), "- a\n- |");
        assert_eq!(press_enter("    - |").unwrap(), "\t- |".replacen('\t', "", 1));
    }

    #[test]
    fn enter_elsewhere_is_plain() {
        assert_eq!(press_enter("just text|"), None);
        assert_eq!(press_enter("-|"), None);
        assert_eq!(press_enter("-| milk"), None);
        assert_eq!(press_enter("#tag|"), None);
        assert_eq!(press_enter("**bold**|"), None);
    }

    fn tab(marked: &str, out: bool) -> Option<String> {
        let (lines, a, h) = parse(marked);
        indent(&lines, a, h, out, "\t").map(|c| show(&lines, &c))
    }

    #[test]
    fn tab_indents_lists_and_selections() {
        assert_eq!(tab("- a\n- b|", false).unwrap(), "- a\n\t- b|");
        assert_eq!(tab("- a\n\t- b|", true).unwrap(), "- a\n- b|");
        assert_eq!(tab("{one\ntwo}", false).unwrap(), "\t{one\n\ttwo}");
        assert_eq!(tab("\t{one\n  two}", true).unwrap(), "{one\ntwo}");
        assert_eq!(tab("word|", false).unwrap(), "word\t|");
        assert_eq!(tab("word|", true), None);
    }

    fn move_by(marked: &str, up: bool) -> Option<String> {
        let (lines, a, h) = parse(marked);
        move_lines(&lines, a, h, up).map(|c| show(&lines, &c))
    }

    #[test]
    fn lines_move_and_duplicate() {
        assert_eq!(move_by("a\nb|\nc", true).unwrap(), "b|\na\nc");
        assert_eq!(move_by("a\nb|\nc", false).unwrap(), "a\nc\nb|");
        assert_eq!(move_by("a|\nb", true), None);
        assert_eq!(move_by("a\nb|", false), None);
        assert_eq!(move_by("x\n{a\nb}\nc", true).unwrap(), "{a\nb}\nx\nc");
        let (lines, a, h) = parse("a\nb|c\nd");
        assert_eq!(show(&lines, &duplicate(&lines, a, h)), "a\nbc\nb|c\nd");
    }

    fn typed(marked: &str, c: char) -> Option<String> {
        let (lines, a, h) = parse(marked);
        type_char(&lines, a, h, c).map(|ch| show(&lines, &ch))
    }

    #[test]
    fn pairs_close_themselves() {
        assert_eq!(typed("see |", '(').unwrap(), "see (|)");
        assert_eq!(typed("see |", '[').unwrap(), "see [|]");
        assert_eq!(typed("see [|]", '[').unwrap(), "see [[|]]");
        assert_eq!(typed("see [[x|]]", ']').unwrap(), "see [[x]|]");
        assert_eq!(typed("f(x|)", ')').unwrap(), "f(x)|");
        assert_eq!(typed("run |", '`').unwrap(), "run `|`");
        assert_eq!(typed("run `x|`", '`').unwrap(), "run `x`|");
        assert_eq!(typed("say *|", '*').unwrap(), "say **|**");
        assert_eq!(typed("say **x|**", '*').unwrap(), "say **x*|*");
        assert_eq!(typed("say {x}", '*').unwrap(), "say *{x}*");
        assert_eq!(typed("say {x}", '(').unwrap(), "say ({x})");
    }

    #[test]
    fn pairs_stay_out_of_the_way() {
        // Before a word, a bracket is just a bracket.
        assert_eq!(typed("|word", '('), None);
        // A bullet and a code fence.
        assert_eq!(typed("|", '*'), None);
        assert_eq!(typed("``|", '`'), None);
        assert_eq!(typed("a|", 'x'), None);
    }

    #[test]
    fn backspace_takes_empty_pairs() {
        let gone = |marked: &str| {
            let (lines, a, _) = parse(marked);
            backspace(&lines, a).map(|c| show(&lines, &c))
        };
        assert_eq!(gone("f(|)").unwrap(), "f|");
        assert_eq!(gone("[[|]]").unwrap(), "[|]");
        assert_eq!(gone("**|**").unwrap(), "|");
        assert_eq!(gone("`|`").unwrap(), "|");
        assert_eq!(gone("(x|)"), None);
    }
}
