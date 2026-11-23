//! The preview pane: the Markdown block tree rendered with the Sage & Paper
//! tokens. Links and tags inline, task checkboxes that edit the note,
//! callouts, the note's properties, embeds, and `view` and `tasks` blocks,
//! whose results come from the index. A link to a task document shows the
//! task's status before its title, and an embedded task shows as a card.

use crate::theme::{self, Tokens};
use crate::views::{self, Drag};
use crate::{file_stem, hline, is_attachment, plain_links, LinkTarget, Message, TaskRef};
use sage_core::TaskStatus;
use sage_markdown::{Align, Block, Embed, Inline, Item, Link, Property, Span, Task};
use sage_query::documents::{Document, TaskList};
use sage_query::Outcome;
use iced::widget::{button, column, container, image, rich_text, row, space, span, svg, text, Column, Row};
use iced::{font, Border, Color, Element, Fill, Font, Length};
use std::collections::HashMap;

/// How many notes deep embeds are shown: a note embedded in a note embedded
/// in the note. Deeper ones show as a line with their name.
pub const MAX_DEPTH: usize = 3;

/// What an embed has loaded so far.
#[derive(Debug, Clone)]
pub enum Embedded {
    Loading,
    Image { path: String, handle: image::Handle },
    Svg { path: String, handle: svg::Handle },
    /// The embedded note, or the section of it that was asked for, parsed.
    Note { path: String, blocks: Vec<Block> },
    /// Why there is nothing to show, as a short sentence.
    Missing(String),
}

/// Everything rendering needs besides the blocks themselves.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub t: Tokens,
    pub embeds: &'a HashMap<String, Embedded>,
    pub views: &'a HashMap<String, Result<Outcome, String>>,
    /// See `Tab::docs`.
    pub docs: &'a HashMap<String, Option<Document>>,
    pub task_lists: &'a HashMap<String, Result<TaskList, String>>,
    pub drag: Option<&'a Drag>,
    /// Whether each foldable callout of the note is open, by key, where the
    /// user opened or closed it.
    pub folds: Option<&'a HashMap<String, bool>>,
    /// False inside an embedded note: checkboxes and callouts there do not
    /// change, and its views are not run.
    pub interactive: bool,
    /// The note being drawn at `chain[0]`, then the notes embedded down to
    /// the one drawn now, at `chain[depth]`. Embeds resolve from there.
    pub chain: [&'a str; MAX_DEPTH + 1],
    pub depth: usize,
}

impl<'a> Ctx<'a> {
    /// For the note of `tab`, drawn in its own tab.
    pub fn new(t: Tokens, tab: &'a crate::Tab, drag: Option<&'a Drag>, folds: Option<&'a HashMap<String, bool>>) -> Self {
        Self {
            t,
            embeds: &tab.embeds,
            views: &tab.views,
            docs: &tab.docs,
            task_lists: &tab.task_lists,
            drag,
            folds,
            interactive: true,
            chain: [tab.path.as_str(); MAX_DEPTH + 1],
            depth: 0,
        }
    }

    /// The task document a link written in the note drawn now names.
    fn task(&self, target: &str) -> Option<&'a Document> {
        let key = sage_core::target_key(target, self.chain[self.depth]);
        self.docs.get(&key)?.as_ref().filter(|doc| doc.is_task())
    }
}

/// How an embed is found in `Ctx::embeds`: the note it is written in, then
/// its target as written, with the heading or block it asks for.
pub fn embed_key(src: &str, embed: &Embed) -> String {
    match (&embed.heading, &embed.block) {
        (Some(heading), _) => format!("{src}\n{}#{heading}", embed.target),
        (None, Some(block)) => format!("{src}\n{}#^{block}", embed.target),
        (None, None) => format!("{src}\n{}", embed.target),
    }
}

/// Every embed, view source and `tasks` block source in `blocks`, nested
/// ones included.
pub fn collect<'a>(blocks: &'a [Block], embeds: &mut Vec<&'a Embed>, views: &mut Vec<&'a str>, queries: &mut Vec<&'a str>) {
    for block in blocks {
        match block {
            Block::Embed(embed) => embeds.push(embed),
            Block::View { source, .. } => views.push(source),
            Block::TaskQuery { source, .. } => queries.push(source),
            Block::Quote(inner) | Block::Callout { blocks: inner, .. } => collect(inner, embeds, views, queries),
            Block::List { items, .. } => {
                for item in items {
                    collect(&item.blocks, embeds, views, queries);
                }
            }
            _ => {}
        }
    }
}

pub fn blocks<'a>(blocks: &'a [Block], ctx: Ctx<'a>) -> Element<'a, Message> {
    // An embedded note shows without its properties, as in Obsidian.
    let shown = blocks.iter().filter(|b| ctx.depth == 0 || !matches!(b, Block::Properties(_)));
    Column::with_children(shown.map(|b| block(b, ctx, ctx.t.text))).spacing(10).into()
}

/// One block; `color` is the body text colour, muted inside quotes and done tasks.
fn block<'a>(b: &'a Block, ctx: Ctx<'a>, color: Color) -> Element<'a, Message> {
    let t = ctx.t;
    match b {
        Block::Heading { level, text } => {
            let size = [30, 24, 20, 17, 16, 15][usize::from(*level).clamp(1, 6) - 1];
            container(inline(text, ctx, size, color, theme::SERIF))
                .padding(iced::Padding { top: 12.0, ..iced::Padding::ZERO })
                .into()
        }
        Block::Paragraph(text) => inline(text, ctx, 16, color, theme::SANS),
        Block::List { start, items } => list(*start, items, ctx, color),
        Block::Quote(inner) => row![
            container(space()).width(2).height(Fill).style(move |_| theme::accent_rule(t, true)),
            Column::with_children(inner.iter().map(|b| block(b, ctx, t.text_2))).spacing(8),
        ]
        .spacing(12)
        .height(iced::Shrink)
        .into(),
        Block::Callout { kind, title, fold, key, blocks } => callout(kind, title, *fold, key, blocks, ctx, color),
        Block::Properties(properties) => self::properties(properties, ctx),
        Block::Code { code, .. } => code_block(code, t),
        Block::View { source, .. } if ctx.interactive => views::view(source, ctx),
        Block::View { source, .. } => code_block(source, t),
        Block::TaskQuery { source, .. } if ctx.interactive => task_list(source, ctx),
        Block::TaskQuery { source, .. } => code_block(source, t),
        Block::Table { alignments, header, rows } => table(alignments, header, rows, ctx),
        Block::Embed(embed) => self::embed(embed, ctx),
        Block::Rule => container(hline(t)).padding([8, 0]).into(),
        Block::Html(html) => text(html).size(14).font(theme::MONO).color(t.text_2).into(),
    }
}

fn code_block(code: &str, t: Tokens) -> Element<'_, Message> {
    container(text(code).size(13).font(theme::MONO).line_height(1.5))
        .padding(14)
        .width(Fill)
        .style(move |_| theme::surface(t))
        .into()
}

/// Rich text for a run of spans. Links to notes and URLs are clickable;
/// links to attachments are shown but not followed.
pub fn inline<'a>(text: &'a Inline, ctx: Ctx<'a>, size: u32, color: Color, font: Font) -> Element<'a, Message> {
    rich_text(spans(&text.spans, ctx, font))
        .size(size)
        .line_height(1.55)
        .color(color)
        .font(font)
        .on_link_click(Message::FollowLink)
        .into()
}

/// The spans as drawn. A link to a whole task document reads
/// `[In Progress] Fix voice reconnect bug`: the task's status, then its title,
/// or the alias the link was written with.
fn spans<'a>(spans: &'a [Span], ctx: Ctx<'a>, font: Font) -> Vec<iced::widget::text::Span<'a, LinkTarget, Font>> {
    let mut out = Vec::with_capacity(spans.len());
    for (i, s) in spans.iter().enumerate() {
        let task = match &s.link {
            Some(Link::Internal { target, heading: None, block: None }) => ctx.task(target).map(|doc| (target, doc)),
            _ => None,
        };
        let Some((target, doc)) = task else {
            out.push(styled(s, ctx.t, font));
            continue;
        };
        // A link of several spans gets the status once, before the first.
        if i == 0 || spans[i - 1].link != s.link {
            if let Some(status) = &doc.status {
                let label = doc.task_status().map_or(status.as_str(), |s| s.label());
                out.push(span(format!("[{label}] ")).font(font).color(ctx.t.text_2));
            }
        }
        // An unpiped wikilink's text is its target, which the title replaces.
        let mut drawn = styled(s, ctx.t, font);
        if s.text == *target {
            drawn.text = doc.title.clone().into();
        }
        out.push(drawn);
    }
    out
}

fn styled<'a>(s: &'a Span, t: Tokens, base: Font) -> iced::widget::text::Span<'a, LinkTarget, Font> {
    let mut font = base;
    if s.style.strong {
        font.weight = font::Weight::Semibold;
    }
    if s.style.emphasis {
        font.style = font::Style::Italic;
    }
    if s.style.code {
        font = theme::MONO;
    }
    let mut out = span(s.text.as_str()).font(font);
    if s.style.code {
        out = out
            .background(t.surface)
            .border(Border { radius: theme::RADIUS.into(), ..Border::default() })
            .padding([0, 3]);
    }
    if s.style.highlight {
        out = out.background(theme::highlight(t));
    }
    if s.style.strike {
        out = out.strikethrough(true);
    }
    if s.style.tag {
        out = out.color(t.text_2);
    }
    match &s.link {
        Some(Link::Internal { target, heading, block }) if !is_attachment(target) => out.color(t.accent).link(LinkTarget::Note {
            target: target.clone(),
            heading: heading.clone(),
            block: block.clone(),
        }),
        Some(Link::Internal { .. }) => out.color(t.text_2),
        Some(Link::External(url)) => out.color(t.accent).link(LinkTarget::Url(url.clone())),
        None => out,
    }
}

fn list<'a>(start: Option<u64>, items: &'a [Item], ctx: Ctx<'a>, color: Color) -> Element<'a, Message> {
    let t = ctx.t;
    Column::with_children(items.iter().enumerate().map(|(n, item)| {
        let marker: Element<'a, Message> = match (item.task, start) {
            (Some(task), _) => checkbox(task, ctx),
            (None, Some(first)) => text(format!("{}.", first + n as u64))
                .size(16)
                .line_height(1.55)
                .color(t.text_2)
                .into(),
            (None, None) => text("–").size(16).line_height(1.55).color(t.text_2).into(),
        };
        let color = if item.task.is_some_and(|task| task.done) { t.text_2 } else { color };
        row![
            container(marker).width(if start.is_some() { 26 } else { 16 }),
            Column::with_children(item.blocks.iter().map(|b| block(b, ctx, color))).spacing(6),
        ]
        .spacing(8)
        .into()
    }))
    .spacing(4)
    .into()
}

/// A task checkbox. Clicking it edits the note's text in the editor, so it
/// works with unsaved edits too; autosave then writes it.
fn checkbox<'a>(task: Task, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.t;
    let square = container(space()).width(13).height(13).style(move |_| theme::checkbox(t, task.done));
    let square = container(square).padding(iced::Padding { top: 6.0, ..iced::Padding::ZERO });
    if !ctx.interactive {
        return square.into();
    }
    button(square)
        .padding(0)
        .style(move |_, status| theme::link_button(t, status))
        .on_press(Message::PreviewTask(task.line))
        .into()
}

/// The colour that marks a callout, by its kind as Obsidian groups them.
pub(crate) fn callout_color(kind: &str, t: Tokens) -> Color {
    match kind {
        "tip" | "hint" | "important" | "success" | "check" | "done" => t.success,
        "question" | "help" | "faq" | "warning" | "caution" | "attention" => t.warning,
        "failure" | "fail" | "missing" | "danger" | "error" | "bug" => t.error,
        "example" | "quote" | "cite" => t.text_2,
        _ => t.accent,
    }
}

/// A callout: its title in the kind's colour over its blocks. A foldable one
/// opens and closes from its title, in the note's own tab, which remembers it.
fn callout<'a>(
    kind: &str,
    title: &'a Inline,
    fold: Option<bool>,
    key: &'a str,
    blocks: &'a [Block],
    ctx: Ctx<'a>,
    color: Color,
) -> Element<'a, Message> {
    let t = ctx.t;
    let accent = callout_color(kind, t);
    let remembered = ctx.folds.filter(|_| ctx.interactive).and_then(|folds| folds.get(key)).copied();
    let open = fold.is_none() || remembered.unwrap_or(fold == Some(true));
    let mut heading = row![inline(title, ctx, 15, accent, theme::SANS_BOLD)].spacing(8).align_y(iced::Center);
    if fold.is_some() {
        heading = heading.push(text(if open { "▾" } else { "▸" }).size(15).color(t.text_2));
    }
    let heading: Element<'a, Message> = if fold.is_some() && ctx.interactive {
        button(heading)
            .padding(0)
            .style(move |_, status| theme::link_button(t, status))
            .on_press(Message::ToggleCallout(key.to_owned(), !open))
            .into()
    } else {
        heading.into()
    };
    let mut body = Column::new().spacing(8).push(heading);
    if open {
        body = body.extend(blocks.iter().map(|b| block(b, ctx, color)));
    }
    container(
        row![
            container(space()).width(2).height(Fill).style(move |_| theme::fill(accent)),
            container(body).padding([10, 14]).width(Fill),
        ]
        .height(iced::Shrink),
    )
    .width(Fill)
    .style(move |_| theme::surface(t))
    .into()
}

/// The note's frontmatter: each key beside its values.
fn properties<'a>(properties: &'a [Property], ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.t;
    let rows = properties.iter().map(|p| {
        let mut spans = Vec::new();
        for (i, value) in p.values.iter().enumerate() {
            if i > 0 {
                spans.push(span(", "));
            }
            spans.extend(self::spans(&value.spans, ctx, theme::SANS));
        }
        row![
            // The key takes a third of a narrow pane, at most 140 px of a wide one.
            container(text(&p.key).size(13).color(t.text_2)).width(Length::FillPortion(1)).max_width(140),
            rich_text(spans)
                .size(14)
                .line_height(1.45)
                .color(t.text)
                .font(theme::SANS)
                .width(Length::FillPortion(2))
                .on_link_click(Message::FollowLink),
        ]
        .spacing(12)
        .into()
    });
    container(Column::with_children(rows).spacing(4)).padding([10, 14]).width(Fill).style(move |_| theme::panel(t)).into()
}

/// The note's properties as live preview edits them in place: one row per
/// top-level entry, a field for each value. Each change goes out as
/// `Message::Property` with the entry's index and its new value.
pub fn property_editor<'a>(entries: Vec<sage_editor::properties::Entry>, t: Tokens) -> Element<'a, Message> {
    use sage_editor::properties::Value;
    let field = move |value: &str, on_input: Box<dyn Fn(String) -> Message + 'a>| -> Element<'a, Message> {
        iced::widget::text_input("", value)
            .on_input(on_input)
            .size(14)
            .padding([3, 6])
            .style(move |_, status| theme::input(t, status))
            .into()
    };
    let small = move |label: &'static str, message: Message| -> Element<'a, Message> {
        button(text(label).size(13))
            .padding([2, 8])
            .style(move |_, status| theme::nav_button(t, false, status))
            .on_press(message)
            .into()
    };
    let rows = entries.into_iter().enumerate().map(|(i, entry)| {
        let editor: Element<'a, Message> = match entry.value {
            Value::Text { text, quote } => field(&text, Box::new(move |s| Message::Property(i, Value::Text { text: s, quote }))),
            Value::Bool(on) => button(text(if on { "✓ true" } else { "false" }).size(14))
                .padding([2, 8])
                .style(move |_, status| theme::nav_button(t, on, status))
                .on_press(Message::Property(i, Value::Bool(!on)))
                .into(),
            Value::List { items, block } => {
                let mut cells = Row::new().spacing(6).align_y(iced::Center);
                for (k, item) in items.iter().enumerate() {
                    let all = items.clone();
                    let mut rest = items.clone();
                    rest.remove(k);
                    // As wide as its text, so several fit on a line.
                    let width = item.chars().count() as f32 * 8.0 + 14.0;
                    let input = iced::widget::text_input("", item)
                        .on_input(move |s| {
                            let mut items = all.clone();
                            items[k] = s;
                            Message::Property(i, Value::List { items, block })
                        })
                        .size(14)
                        .padding([2, 6])
                        .width(Length::Fixed(width.max(28.0)))
                        .style(move |_, status| theme::chip_input(t, status));
                    let remove = button(text("×").size(13))
                        .padding([0, 6])
                        .style(move |_, status| theme::nav_button(t, false, status))
                        .on_press(Message::Property(i, Value::List { items: rest, block }));
                    cells = cells.push(
                        container(row![input, remove].align_y(iced::Center)).style(move |_| theme::chip(t)),
                    );
                }
                let mut more = items.clone();
                more.push(String::new());
                cells.push(small("+", Message::Property(i, Value::List { items: more, block }))).wrap().vertical_spacing(6).into()
            }
            Value::Raw(raw) => text(raw).size(13).color(t.text_2).into(),
        };
        row![
            container(text(entry.key).size(13).color(t.text_2)).width(Length::FillPortion(1)).max_width(140),
            container(editor).width(Length::FillPortion(2)),
        ]
        .spacing(12)
        .align_y(iced::Center)
        .into()
    });
    container(Column::with_children(rows).spacing(4)).padding([10, 14]).width(Fill).style(move |_| theme::panel(t)).into()
}

fn table<'a>(alignments: &'a [Align], header: &'a [Inline], rows: &'a [Vec<Inline>], ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.t;
    let cell = |content: Element<'a, Message>, i: usize| -> Element<'a, Message> {
        let align = match alignments.get(i) {
            Some(Align::Center) => iced::alignment::Horizontal::Center,
            Some(Align::Right) => iced::alignment::Horizontal::Right,
            _ => iced::alignment::Horizontal::Left,
        };
        container(content).width(Length::FillPortion(1)).align_x(align).into()
    };
    let mut grid = Column::new().spacing(6).push(
        Row::with_children(
            header.iter().enumerate().map(|(i, h)| cell(inline(h, ctx, 14, t.text_2, theme::SANS_BOLD), i)),
        )
        .spacing(16),
    );
    grid = grid.push(hline(t));
    for cells in rows {
        grid = grid.push(
            Row::with_children(cells.iter().enumerate().map(|(i, c)| cell(inline(c, ctx, 15, t.text, theme::SANS), i)))
                .spacing(16),
        );
    }
    grid.into()
}

fn embed<'a>(embed: &'a Embed, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.t;
    let label = if embed.alt.is_empty() { &embed.target } else { &embed.alt };
    let note = |line: String| text(line).size(13).color(t.text_2).into();
    if ctx.depth >= MAX_DEPTH {
        return note(format!("↳ {label}"));
    }
    match ctx.embeds.get(&embed_key(ctx.chain[ctx.depth], embed)) {
        None | Some(Embedded::Loading) => note(format!("Loading {}…", embed.target)),
        Some(Embedded::Missing(why)) => note(format!("{}: {why}", embed.target)),
        Some(Embedded::Image { handle, .. }) => {
            let picture = image(handle.clone());
            match embed.width {
                Some(width) => picture.width(width as f32).into(),
                None => picture.into(),
            }
        }
        Some(Embedded::Svg { handle, .. }) => {
            let width = embed.width.map_or(Length::Shrink, |w| Length::Fixed(w as f32));
            svg(handle.clone()).width(width).into()
        }
        Some(Embedded::Note { path, blocks }) => {
            let inner = if ctx.chain[..=ctx.depth].contains(&path.as_str()) {
                note(format!("↳ {label}, which embeds this note"))
            } else {
                let mut chain = ctx.chain;
                chain[ctx.depth + 1] = path;
                self::blocks(blocks, Ctx { interactive: false, chain, depth: ctx.depth + 1, ..ctx })
            };
            // A whole task document shows its title and fields above its text.
            let task = (embed.heading.is_none() && embed.block.is_none()).then(|| ctx.task(&embed.target)).flatten();
            let title = task.map_or_else(|| file_stem(path).to_owned(), |doc| doc.title.clone());
            let mut card = column![button(text(title).size(task.map_or(13, |_| 16)).font(theme::SANS_BOLD))
                .padding(0)
                .style(move |_, status| theme::link_button(t, status))
                .on_press(Message::Open(path.clone()))]
            .spacing(8);
            if let Some(doc) = task {
                card = card.push(task_fields(doc, t));
            }
            container(card.push(inner))
            .padding([10, 14])
            .width(Fill)
            .style(move |_| theme::panel(t))
            .into()
        }
    }
}

/// A task's status, due date and tags, a line each, for those it has.
fn task_fields<'a>(doc: &Document, t: Tokens) -> Element<'a, Message> {
    let status = doc.status.as_ref().map(|s| doc.task_status().map_or(s.clone(), |s| s.label().to_owned()));
    let tags = (!doc.tags.is_empty()).then(|| doc.tags.join(", "));
    let fields = [("Status", status), ("Due", doc.due.clone()), ("Tags", tags)];
    Column::with_children(fields.into_iter().filter_map(|(name, value)| {
        let spans: [iced::widget::text::Span<'a, LinkTarget, Font>; 2] = [span(format!("{name}: ")).color(t.text_2), span(value?)];
        Some(rich_text(spans).size(14).font(theme::SANS).into())
    }))
    .spacing(2)
    .into()
}

/// A `tasks` block: the tasks it matches with their due dates. A task
/// document shows its status and title, which opens it; a checkbox line shows
/// a checkbox that checks it in its note, its text, which opens the note on
/// that line, and the note's name.
fn task_list<'a>(source: &'a str, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.t;
    let list = match ctx.task_lists.get(source) {
        None => return text("Running tasks query…").size(13).color(t.text_2).into(),
        Some(Err(error)) => {
            return container(
                column![text("Tasks").size(12).font(theme::SANS_BOLD).color(t.error), text(error).size(14).color(t.text)]
                    .spacing(4),
            )
            .padding([10, 14])
            .width(Fill)
            .style(move |_| theme::panel(t))
            .into();
        }
        Some(Ok(list)) => list,
    };
    if list.rows.is_empty() {
        return text("No tasks match").size(13).color(t.text_2).into();
    }
    let mut rows = Column::new().spacing(4);
    for doc in &list.rows {
        let status = doc.status.as_ref().map(|s| doc.task_status().map_or(s.as_str(), |s| s.label()));
        let mut line = Row::new().spacing(8).align_y(iced::Center);
        match &doc.line {
            // A checkbox line: checked here, it is checked in its note.
            Some((number, raw)) => {
                let done = doc.task_status() == Some(TaskStatus::Done);
                let task = TaskRef { path: doc.path.clone(), line: *number, raw: raw.clone(), document: false };
                line = line.push(
                    button(container(space()).width(13).height(13).style(move |_| theme::checkbox(t, done)))
                        .padding([6, 6])
                        .style(move |_, status| theme::nav_button(t, false, status))
                        .on_press(Message::ViewTask(task)),
                );
                // Open and done show in the box; other statuses in words.
                if let Some(status) = status.filter(|_| !matches!(doc.task_status(), Some(TaskStatus::Todo | TaskStatus::Done))) {
                    line = line.push(text(format!("[{status}]")).size(15).color(t.text_2));
                }
                line = line.push(
                    button(text(plain_links(&doc.title)).size(15).color(if done { t.text_2 } else { t.text }))
                        .padding(0)
                        .style(move |_, status| theme::link_button(t, status))
                        .on_press(Message::OpenAt(doc.path.clone(), *number as usize)),
                );
                line = line.push(text(file_stem(&doc.path)).size(13).color(t.text_2));
            }
            None => {
                line = line.push(space().width(25));
                if let Some(status) = status {
                    line = line.push(text(format!("[{status}]")).size(15).color(t.text_2));
                }
                line = line.push(
                    button(text(&doc.title).size(15).color(t.accent))
                        .padding(0)
                        .style(move |_, status| theme::link_button(t, status))
                        .on_press(Message::Open(doc.path.clone())),
                );
            }
        }
        if let Some(due) = &doc.due {
            line = line.push(text(format!("due {due}")).size(13).color(t.text_2));
        }
        rows = rows.push(line);
    }
    if list.truncated {
        rows = rows.push(
            text(format!("Showing the first {}; raise `limit` to see more", list.rows.len())).size(12).color(t.text_2),
        );
    }
    rows.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(status: &str) -> Document {
        Document {
            path: "Tasks/Voice.md".into(),
            doc_type: "task".into(),
            id: "fix-voice-reconnect".into(),
            title: "Fix voice reconnect bug".into(),
            status: Some(status.into()),
            created: None,
            due: Some("2026-11-30".into()),
            tags: vec!["backend".into(), "voice".into()],
        }
    }

    /// What a paragraph reads as drawn, span by span.
    fn drawn(tab: &crate::Tab, src: &str) -> Vec<String> {
        let blocks = sage_markdown::parse(src);
        let Block::Paragraph(text) = &blocks[0] else { panic!("{blocks:?}") };
        let ctx = Ctx::new(Tokens::LIGHT, tab, None, None);
        spans(&text.spans, ctx, theme::SANS).iter().map(|s| s.text.to_string()).collect()
    }

    #[test]
    fn task_links_show_status_and_title_live() {
        let mut tab = crate::Tab::placeholder("Meetings/standup.md".into(), "standup".into());
        tab.docs.insert("fix-voice-reconnect".into(), Some(task("in-progress")));
        tab.docs.insert("plain".into(), None);
        let src = "See [[fix-voice-reconnect]], [[Fix-Voice-Reconnect|the bug]] and [[plain]].";
        assert_eq!(
            drawn(&tab, src),
            ["See ", "[In Progress] ", "Fix voice reconnect bug", ", ", "[In Progress] ", "the bug", " and ", "plain", "."]
        );
        // The index reports the task changed: the same tree draws the new status.
        tab.docs.insert("fix-voice-reconnect".into(), Some(task("done")));
        assert_eq!(drawn(&tab, "[[fix-voice-reconnect]]"), ["[Done] ", "Fix voice reconnect bug"]);
        // A link to a heading inside it, or another type of document, stays a plain link.
        assert_eq!(drawn(&tab, "[[fix-voice-reconnect#Acceptance]]"), ["fix-voice-reconnect > Acceptance"]);
        tab.docs.insert("fix-voice-reconnect".into(), Some(Document { doc_type: "project".into(), ..task("done") }));
        assert_eq!(drawn(&tab, "[[fix-voice-reconnect]]"), ["fix-voice-reconnect"]);
    }
}
