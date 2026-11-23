//! Walks pulldown-cmark's events into the block tree.
//!
//! The events are collected first, so a list item can look ahead for its
//! checkbox: pulldown-cmark puts the marker straight into a tight item but
//! inside the first paragraph of a loose one.

use crate::{Align, Block, Embed, Inline, Item, Link, Property, Span, Style, Task};
use sage_core::FmValue;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;
use std::ops::Range;

pub(crate) fn parse(src: &str) -> Vec<Block> {
    // pulldown-cmark keeps a byte-order mark as text, which would turn a
    // heading on the first line into a paragraph. It contains no newline, so
    // line numbers stay those of the file.
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    // Only a block at the very top is frontmatter, found as the indexer finds
    // it. pulldown-cmark's metadata option would also take a `---` pair further
    // down, which Obsidian draws as a rule and a heading. The rest is parsed on
    // its own, its offsets moved back so lines stay those of the file.
    let (yaml, body, _) = sage_core::split_frontmatter(src);
    let start = src.len() - body.len();
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_WIKILINKS;
    let mut walker = Walker {
        src,
        events: Parser::new_ext(body, options)
            .into_offset_iter()
            .map(|(event, range)| (event, range.start + start..range.end + start))
            .collect(),
        pos: 0,
        line_starts: std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i + 1)).collect(),
        callouts: HashMap::new(),
    };
    let properties = yaml.map(properties).unwrap_or_default();
    let mut blocks = if properties.is_empty() { Vec::new() } else { vec![Block::Properties(properties)] };
    blocks.extend(walker.blocks());
    blocks
}

struct Walker<'a> {
    src: &'a str,
    events: Vec<(Event<'a>, Range<usize>)>,
    pos: usize,
    line_starts: Vec<usize>,
    /// Callouts so far by kind and title, for their keys.
    callouts: HashMap<String, usize>,
}

/// What an inline run does with an image.
#[derive(Clone, Copy, PartialEq)]
enum Images {
    /// Paragraphs and list items: the image becomes an embed block of its own.
    Embed,
    /// Headings and table cells, which cannot be split: its alt text.
    Text,
}

enum Segment {
    Text(Inline),
    Embed(Embed),
}

impl<'a> Walker<'a> {
    fn line(&self, offset: usize) -> u32 {
        (self.line_starts.partition_point(|&start| start <= offset) - 1) as u32
    }

    fn peek(&self, ahead: usize) -> Option<&Event<'a>> {
        self.events.get(self.pos + ahead).map(|(event, _)| event)
    }

    /// Blocks up to the end of the enclosing element, whose `End` is consumed.
    fn blocks(&mut self) -> Vec<Block> {
        let mut out = Vec::new();
        while let Some((event, range)) = self.events.get(self.pos).cloned() {
            if is_inline(&event) && !matches!(event, Event::End(_)) {
                // A tight list item: inline content with no paragraph around it.
                let segments = self.inline(Images::Embed);
                push_segments(&mut out, segments);
                continue;
            }
            self.pos += 1;
            match event {
                Event::End(_) => break,
                Event::Rule => out.push(Block::Rule),
                Event::Start(tag) => self.block(tag, range, &mut out),
                _ => {}
            }
        }
        out
    }

    /// The block that `tag` starts; its events are consumed.
    fn block(&mut self, tag: Tag<'a>, range: Range<usize>, out: &mut Vec<Block>) {
        match tag {
            Tag::Paragraph => {
                let segments = self.inline(Images::Embed);
                self.pos += 1;
                push_segments(out, segments);
            }
            Tag::Heading { level, .. } => {
                let text = self.inline_text();
                self.pos += 1;
                out.push(Block::Heading { level: level as u8, text });
            }
            Tag::BlockQuote(_) => match callout_header(self.src, range.start) {
                Some(header) => {
                    let blocks = self.callout_body(header.end);
                    let Header { kind, title, fold, .. } = header;
                    let name = format!("{kind}|{title}");
                    let seen = self.callouts.entry(name.clone()).or_default();
                    let key = format!("{name}|{seen}");
                    *seen += 1;
                    out.push(Block::Callout { kind, title: callout_title(&title), fold, key, blocks });
                }
                None => {
                    let blocks = self.blocks();
                    out.push(Block::Quote(blocks));
                }
            },
            Tag::List(start) => {
                let items = self.list();
                out.push(Block::List { start, items });
            }
            Tag::CodeBlock(kind) => {
                let line = self.line(range.start);
                let mut code = self.text_until_end().replace("\r\n", "\n");
                if code.ends_with('\n') {
                    code.pop();
                }
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().map(str::to_owned),
                    CodeBlockKind::Indented => None,
                };
                out.push(match language {
                    Some(language) if language.eq_ignore_ascii_case("view") => Block::View { source: code, line },
                    Some(language) if language.eq_ignore_ascii_case("tasks") => Block::TaskQuery { source: code, line },
                    language => Block::Code { language, code },
                });
            }
            Tag::HtmlBlock => {
                let html = self.text_until_end();
                out.push(Block::Html(html.trim_end().to_owned()));
            }
            Tag::Table(alignments) => {
                let table = self.table(alignments);
                out.push(table);
            }
            // Anything else that holds blocks keeps its content.
            _ => {
                let blocks = self.blocks();
                out.extend(blocks);
            }
        }
    }

    /// A callout's blocks, without its header line, which ends at byte
    /// `header_end`. The header is the start of the first paragraph; whatever
    /// follows it on the next lines is the callout's first paragraph.
    fn callout_body(&mut self, header_end: usize) -> Vec<Block> {
        let mut out = Vec::new();
        if matches!(self.peek(0), Some(Event::Start(Tag::Paragraph))) {
            self.pos += 1;
            while let Some((event, range)) = self.events.get(self.pos) {
                if range.start >= header_end || matches!(event, Event::End(TagEnd::Paragraph)) {
                    break;
                }
                self.pos += 1;
            }
            let segments = self.inline(Images::Embed);
            self.pos += 1;
            push_segments(&mut out, segments);
        }
        out.extend(self.blocks());
        out
    }

    fn list(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while let Some((event, range)) = self.events.get(self.pos).cloned() {
            self.pos += 1;
            match event {
                Event::Start(Tag::Item) => items.push(self.item(range.start)),
                Event::End(_) => break,
                _ => {}
            }
        }
        items
    }

    /// A list item starting at byte `start`.
    fn item(&mut self, start: usize) -> Item {
        let marker = match (self.peek(0), self.peek(1)) {
            (Some(Event::TaskListMarker(_)), _) => Some(self.pos),
            (Some(Event::Start(Tag::Paragraph)), Some(Event::TaskListMarker(_))) => Some(self.pos + 1),
            _ => None,
        };
        let task = marker.and_then(|i| match &self.events[i] {
            (Event::TaskListMarker(done), range) => Some(Task { done: *done, line: self.line(range.start) }),
            _ => None,
        });
        let mut blocks = self.blocks();
        // pulldown-cmark knows `[ ]` and `[x]` only. Obsidian draws any other
        // state, such as `[/]`, `[-]` or `[>]`, as a ticked box, as live
        // preview does; the indexer reads them as tasks too.
        let task = task.or_else(|| {
            let state = other_task_state(&self.src[start..])?;
            let Some(Block::Paragraph(first)) = blocks.first_mut() else { return None };
            let span = first.spans.first_mut()?;
            let rest = span.text.strip_prefix(['[']).and_then(|r| r.strip_prefix(state)).and_then(|r| r.strip_prefix(']'))?;
            span.text = rest.trim_start().to_owned();
            if span.text.is_empty() {
                first.spans.remove(0);
                if first.spans.is_empty() {
                    blocks.remove(0);
                }
            }
            Some(Task { done: true, line: self.line(start) })
        });
        Item { task, blocks }
    }

    fn table(&mut self, alignments: Vec<Alignment>) -> Block {
        let alignments = alignments
            .into_iter()
            .map(|a| match a {
                Alignment::None => Align::None,
                Alignment::Left => Align::Left,
                Alignment::Center => Align::Center,
                Alignment::Right => Align::Right,
            })
            .collect();
        let (mut header, mut rows, mut row) = (Vec::new(), Vec::new(), Vec::new());
        while let Some((event, _)) = self.events.get(self.pos).cloned() {
            self.pos += 1;
            match event {
                Event::Start(Tag::TableCell) => {
                    row.push(self.inline_text());
                    self.pos += 1;
                }
                Event::End(TagEnd::TableHead) => header = std::mem::take(&mut row),
                Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
                Event::End(TagEnd::Table) => break,
                _ => {}
            }
        }
        Block::Table { alignments, header, rows }
    }

    /// An inline run that cannot hold embeds, as one piece of text.
    fn inline_text(&mut self) -> Inline {
        let spans = self
            .inline(Images::Text)
            .into_iter()
            .flat_map(|segment| match segment {
                Segment::Text(inline) => inline.spans,
                Segment::Embed(_) => Vec::new(),
            })
            .collect();
        trimmed(Inline { spans }).unwrap_or_default()
    }

    /// One run of inline content, up to the event that ends it, which is left
    /// for the caller: the end of a paragraph, heading or cell, or a block
    /// inside a tight list item.
    fn inline(&mut self, images: Images) -> Vec<Segment> {
        let mut segments = Vec::new();
        let mut spans = Vec::new();
        let mut styles: Vec<Style> = Vec::new();
        let mut link: Option<Link> = None;
        let mut depth = 0usize;
        while let Some((event, _)) = self.events.get(self.pos).cloned() {
            if depth == 0 && (matches!(event, Event::End(_)) || !is_inline(&event)) {
                break;
            }
            self.pos += 1;
            let style = styles.last().copied().unwrap_or_default();
            match event {
                Event::Start(tag @ (Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Superscript | Tag::Subscript)) => {
                    styles.push(match tag {
                        Tag::Emphasis => Style { emphasis: true, ..style },
                        Tag::Strong => Style { strong: true, ..style },
                        Tag::Strikethrough => Style { strike: true, ..style },
                        _ => style,
                    });
                    depth += 1;
                }
                Event::Start(Tag::Link { link_type, dest_url, .. }) => {
                    let (target, label) = link_of(link_type, &dest_url);
                    match label {
                        // An unpiped wikilink's text is its raw target; show it as Obsidian does.
                        Some(label) => {
                            self.text_until_end();
                            push_span(&mut spans, Span { text: label, style, link: target });
                        }
                        None => {
                            link = target;
                            depth += 1;
                        }
                    }
                }
                Event::Start(Tag::Image { link_type, dest_url, .. }) => {
                    let alt = self.text_until_end();
                    let embed = embed_of(link_type, &dest_url, &alt);
                    if images == Images::Embed && link.is_none() {
                        if !spans.is_empty() {
                            segments.push(Segment::Text(Inline { spans: highlights(std::mem::take(&mut spans)) }));
                        }
                        segments.push(Segment::Embed(embed));
                    } else {
                        let text = if embed.alt.is_empty() { embed.target } else { embed.alt };
                        push_span(&mut spans, Span { text, style, link: link.clone() });
                    }
                }
                Event::End(TagEnd::Link) => {
                    link = None;
                    depth -= 1;
                }
                Event::End(_) => {
                    styles.pop();
                    depth -= 1;
                }
                Event::Text(text) => push_text(&mut spans, &text, style, link.as_ref()),
                Event::Code(code) => {
                    let style = Style { code: true, ..style };
                    push_span(&mut spans, Span { text: code.into_string(), style, link: link.clone() });
                }
                Event::InlineHtml(html) => push_span(&mut spans, Span { text: html.into_string(), style, link: link.clone() }),
                Event::SoftBreak => push_span(&mut spans, Span { text: " ".into(), style, link: link.clone() }),
                Event::HardBreak => push_span(&mut spans, Span { text: "\n".into(), style, link: link.clone() }),
                // The checkbox is read by its list item.
                _ => {}
            }
        }
        if !spans.is_empty() {
            segments.push(Segment::Text(Inline { spans: highlights(spans) }));
        }
        segments
    }

    /// The plain text up to the end of the element being read, whose `End` is consumed.
    fn text_until_end(&mut self) -> String {
        let mut text = String::new();
        let mut depth = 0usize;
        while let Some((event, _)) = self.events.get(self.pos).cloned() {
            self.pos += 1;
            match event {
                Event::Start(_) => depth += 1,
                Event::End(_) if depth == 0 => break,
                Event::End(_) => depth -= 1,
                Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => text.push_str(&t),
                Event::SoftBreak | Event::HardBreak => text.push(' '),
                _ => {}
            }
        }
        text
    }
}

fn is_inline(event: &Event) -> bool {
    match event {
        Event::Start(tag) => matches!(
            tag,
            Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Superscript | Tag::Subscript | Tag::Link { .. } | Tag::Image { .. }
        ),
        Event::End(tag) => matches!(
            tag,
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Superscript | TagEnd::Subscript | TagEnd::Link | TagEnd::Image
        ),
        Event::Html(_) | Event::Rule => false,
        _ => true,
    }
}

/// Paragraphs and embeds from one inline run; runs of only whitespace vanish.
fn push_segments(out: &mut Vec<Block>, segments: Vec<Segment>) {
    for segment in segments {
        match segment {
            Segment::Text(inline) => out.extend(trimmed(inline).map(Block::Paragraph)),
            Segment::Embed(embed) => out.push(Block::Embed(embed)),
        }
    }
}

/// Without the whitespace that an embed split off at its edges.
fn trimmed(mut inline: Inline) -> Option<Inline> {
    if let Some(first) = inline.spans.first_mut().filter(|s| !s.style.code) {
        first.text = first.text.trim_start().to_owned();
    }
    if let Some(last) = inline.spans.last_mut().filter(|s| !s.style.code) {
        last.text = last.text.trim_end().to_owned();
    }
    inline.spans.retain(|s| !s.text.is_empty());
    (!inline.spans.is_empty()).then_some(inline)
}

/// `==text==` marks: the spans between a pair are highlighted and the marks
/// dropped. Marks pair up in order across the run, so emphasis or a link can
/// sit inside; a last mark without a partner stays text. Code keeps its marks.
fn highlights(spans: Vec<Span>) -> Vec<Span> {
    let marks: usize = spans.iter().filter(|s| !s.style.code).map(|s| s.text.matches("==").count()).sum();
    let mut left = marks - marks % 2;
    if left == 0 {
        return spans;
    }
    let mut out = Vec::new();
    let mut on = false;
    for span in spans {
        if span.style.code {
            push_span(&mut out, Span { style: Style { highlight: on, ..span.style }, ..span });
            continue;
        }
        let mut pieces = span.text.split("==").peekable();
        while let Some(piece) = pieces.next() {
            let style = Style { highlight: on, ..span.style };
            push_span(&mut out, Span { text: piece.to_owned(), style, link: span.link.clone() });
            if pieces.peek().is_some() {
                if left > 0 {
                    left -= 1;
                    on = !on;
                } else {
                    push_span(&mut out, Span { text: "==".into(), style, link: span.link.clone() });
                }
            }
        }
    }
    out
}

struct Header {
    kind: String,
    title: String,
    fold: Option<bool>,
    /// Where the header line ends, before its line break.
    end: usize,
}

/// The `[!kind]± Title` line that makes the quote starting at byte `start` a
/// callout, if it has one.
fn callout_header(src: &str, start: usize) -> Option<Header> {
    let end = start + src[start..].find(['\r', '\n']).unwrap_or(src.len() - start);
    let rest = src[start..end].trim_start().strip_prefix('>')?.trim_start().strip_prefix("[!")?;
    let close = rest.find(']')?;
    let kind = &rest[..close];
    if kind.is_empty() || !kind.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let after = &rest[close + 1..];
    let fold = match after.chars().next() {
        Some('+') => Some(true),
        Some('-') => Some(false),
        _ => None,
    };
    let title = after[usize::from(fold.is_some())..].trim();
    let kind = kind.to_lowercase();
    let title = if title.is_empty() {
        let mut chars = kind.chars();
        chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
    } else {
        title.to_owned()
    };
    Some(Header { kind, title, fold, end })
}

/// A callout title with its inline formatting, links and tags. A title that
/// reads as more than one paragraph, like `1. Step`, is shown as written.
fn callout_title(title: &str) -> Inline {
    match parse(title).as_slice() {
        [Block::Paragraph(inline)] => inline.clone(),
        _ => plain(title.to_owned()),
    }
}

/// The frontmatter's keys with their values, in the order written.
fn properties(yaml: &str) -> Vec<Property> {
    let mut out: Vec<Property> = Vec::new();
    for entry in sage_core::frontmatter(yaml) {
        let tags = matches!(entry.key.to_lowercase().as_str(), "tags" | "tag");
        let values = match &entry.value {
            FmValue::Text(text) if tags => text
                .split([',', ' '])
                .filter(|t| !t.is_empty())
                .map(|t| {
                    let style = Style { tag: true, ..Style::default() };
                    Inline { spans: vec![Span { text: format!("#{}", t.trim_start_matches('#')), style, link: None }] }
                })
                .collect(),
            FmValue::Text(text) => vec![property_text(text)],
            FmValue::Num(n) => vec![plain(n.to_string())],
            FmValue::Bool(b) => vec![plain(b.to_string())],
            FmValue::Null => Vec::new(),
        };
        match out.last_mut() {
            Some(last) if last.key == entry.key && entry.ord > 0 => last.values.extend(values),
            _ => out.push(Property { key: entry.key, values }),
        }
    }
    out
}

fn plain(text: String) -> Inline {
    Inline { spans: vec![Span { text, style: Style::default(), link: None }] }
}

/// A property value, with its wikilinks as links showing what Obsidian shows.
fn property_text(text: &str) -> Inline {
    let mut spans = Vec::new();
    let mut at = 0;
    for link in sage_core::links_in_line(text).into_iter().filter(|l| !l.markdown) {
        let (col, end) = (link.col as usize, (link.col + link.len) as usize);
        push_span(&mut spans, Span { text: text[at..col].to_owned(), style: Style::default(), link: None });
        let label = link.alias.clone().unwrap_or_else(|| wiki_label(&link.target, link.heading.as_deref(), link.block.as_deref()));
        let target = Link::Internal { target: link.target, heading: link.heading, block: link.block };
        push_span(&mut spans, Span { text: label, style: Style::default(), link: Some(target) });
        at = end;
    }
    push_span(&mut spans, Span { text: text[at..].to_owned(), style: Style::default(), link: None });
    Inline { spans }
}

/// Appends a span, merging it into the last one when they look the same.
fn push_span(spans: &mut Vec<Span>, span: Span) {
    if span.text.is_empty() {
        return;
    }
    match spans.last_mut() {
        Some(last) if last.style == span.style && last.link == span.link => last.text.push_str(&span.text),
        _ => spans.push(span),
    }
}

/// Text, with its `#tags` as spans of their own. Link text has none.
fn push_text(spans: &mut Vec<Span>, text: &str, style: Style, link: Option<&Link>) {
    if link.is_some() {
        return push_span(spans, Span { text: text.to_owned(), style, link: link.cloned() });
    }
    let mut at = 0;
    for range in sage_core::tag_ranges(text) {
        push_span(spans, Span { text: text[at..range.start].to_owned(), style, link: None });
        push_span(spans, Span { text: text[range.clone()].to_owned(), style: Style { tag: true, ..style }, link: None });
        at = range.end;
    }
    push_span(spans, Span { text: text[at..].to_owned(), style, link: None });
}

/// The link for a destination, and for an unpiped wikilink the text to show.
fn link_of(link_type: LinkType, dest: &str) -> (Option<Link>, Option<String>) {
    match link_type {
        LinkType::WikiLink { has_pothole } => {
            let (target, heading, block) = split_target(dest, false);
            let label = (!has_pothole).then(|| wiki_label(&target, heading.as_deref(), block.as_deref()));
            (Some(Link::Internal { target, heading, block }), label)
        }
        LinkType::Email if !dest.starts_with("mailto:") => (Some(Link::External(format!("mailto:{dest}"))), None),
        _ if dest.is_empty() => (None, None),
        _ if has_scheme(dest) => (Some(Link::External(dest.to_owned())), None),
        _ => {
            let (target, heading, block) = split_target(dest, true);
            (Some(Link::Internal { target, heading, block }), None)
        }
    }
}

/// The state of a task line that pulldown-cmark does not see as one: the
/// character in `- [/] text` after the list marker, other than a space, `x`
/// or `X`. Only on the item's first line, with a space or the end after `]`.
fn other_task_state(item: &str) -> Option<char> {
    let line = item.lines().next()?;
    let marker = line.find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '*' | '+' | '.' | ')')))?;
    let rest = line[marker..].strip_prefix([' ', '\t'])?.trim_start().strip_prefix('[')?;
    let mut chars = rest.chars();
    let state = chars.next().filter(|c| !matches!(c, ' ' | 'x' | 'X' | ']'))?;
    let after = chars.as_str().strip_prefix(']')?;
    (after.is_empty() || after.starts_with([' ', '\t'])).then_some(state)
}

/// Obsidian's text for an unpiped wikilink: `Note > Heading`, `Heading` for a
/// link into the same note, `Note > ^block`, or the target.
fn wiki_label(target: &str, heading: Option<&str>, block: Option<&str>) -> String {
    let section = heading
        .map(|h| h.split('#').map(str::trim).collect::<Vec<_>>().join(" > "))
        .or_else(|| block.map(|b| format!("^{b}")));
    match (target.is_empty(), section) {
        (true, Some(section)) => section,
        (false, Some(section)) => format!("{target} > {section}"),
        (_, None) => target.to_owned(),
    }
}

/// `Note#Heading`, `Note#^block` or `#Heading` as target, heading and block.
/// Markdown destinations are percent-decoded, wikilinks are taken as written.
fn split_target(dest: &str, decode: bool) -> (String, Option<String>, Option<String>) {
    // A wikilink in a table is written `[[Note\|alias]]`; the backslash only
    // escapes the pipe.
    let clean = |s: &str| {
        let s = if decode { sage_core::percent_decode(s) } else { s.to_owned() };
        let s = s.trim();
        if decode { s.to_owned() } else { s.trim_end_matches('\\').trim_end().to_owned() }
    };
    let Some((target, fragment)) = dest.split_once('#') else {
        return (clean(dest), None, None);
    };
    let fragment = clean(fragment);
    let (heading, block) = match fragment.strip_prefix('^') {
        Some(block) => (None, Some(block.to_owned())),
        None => (Some(fragment), None),
    };
    (clean(target), heading.filter(|h| !h.is_empty()), block.filter(|b| !b.is_empty()))
}

fn embed_of(link_type: LinkType, dest: &str, text: &str) -> Embed {
    let wiki = matches!(link_type, LinkType::WikiLink { .. });
    let (alt, width) = match link_type {
        LinkType::WikiLink { has_pothole: true } => alt_and_width(text, true),
        // Without an alias the text is the target again.
        LinkType::WikiLink { .. } => (String::new(), None),
        _ => alt_and_width(text, false),
    };
    let external = has_scheme(dest);
    let (target, heading, block) = if external { (dest.to_owned(), None, None) } else { split_target(dest, !wiki) };
    Embed { target, heading, block, width, alt, external }
}

/// Splits a size off an alias or alt text: `300`, `300x200`, `caption|300`.
/// A bare size counts only in a wikilink alias; Markdown alt text needs the `|`.
fn alt_and_width(text: &str, bare: bool) -> (String, Option<u32>) {
    let (rest, size) = match text.rsplit_once('|') {
        Some(split) => split,
        None if bare => ("", text),
        None => return (text.trim().to_owned(), None),
    };
    match width(size) {
        Some(width) => (rest.trim().to_owned(), Some(width)),
        None => (text.trim().to_owned(), None),
    }
}

fn width(size: &str) -> Option<u32> {
    let size = size.trim();
    let (width, height) = size.split_once('x').unwrap_or((size, "0"));
    height.parse::<u32>().ok()?;
    width.parse().ok()
}

/// `https:`, `mailto:` and other schemes.
fn has_scheme(dest: &str) -> bool {
    dest.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}
