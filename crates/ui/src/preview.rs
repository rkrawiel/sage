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
        // Lists, links and tags are pills that go somewhere when clicked.
        let pills = p.values.len() != 1 || p.values.iter().any(|v| v.spans.iter().any(|s| s.link.is_some() || s.style.tag));
        if pills && !p.values.is_empty() {
            let cells = p.values.iter().map(|v| {
                let label: String = v.spans.iter().map(|s| s.text.as_str()).collect();
                let action = v.spans.iter().find_map(|s| match (&s.link, s.style.tag) {
                    (Some(Link::Internal { target, heading, block }), _) if !is_attachment(target) => {
                        Some(Message::FollowLink(LinkTarget::Note { target: target.clone(), heading: heading.clone(), block: block.clone() }))
                    }
                    (Some(Link::External(url)), _) => Some(Message::FollowLink(LinkTarget::Url(url.clone()))),
                    (None, true) => Some(Message::SearchTag(s.text.trim_start_matches('#').to_owned())),
                    _ => None,
                });
                pill(t, label, action)
            });
            return row![
                container(text(&p.key).size(13).color(t.text_2)).width(Length::FillPortion(1)).max_width(140),
                container(Row::with_children(cells).spacing(6).wrap().vertical_spacing(6)).width(Length::FillPortion(2)),
            ]
            .spacing(12)
            .into();
        }
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

/// A small rounded label that runs `action` when clicked, if it has one.
fn pill<'a>(t: Tokens, label: String, action: Option<Message>) -> Element<'a, Message> {
    let b = button(text(label).size(14)).padding([2, 8]).style(move |_, status| theme::pill(t, status));
    match action {
        Some(message) => b.on_press(message),
        None => b,
    }
    .into()
}

/// The pill for one value of the property `key` (lower case): a wikilink goes
/// to its note, a tag to a search for it, and in `links` a bare name to a note.
fn value_pill<'a>(t: Tokens, key: &str, value: &str) -> Element<'a, Message> {
    let value = value.trim();
    if let Some(link) = sage_core::links_in_line(value).into_iter().find(|l| !l.markdown) {
        let label = link.alias.clone().filter(|a| !a.is_empty()).unwrap_or_else(|| if link.target.is_empty() { link.heading.clone().unwrap_or_default() } else { link.target.clone() });
        let target = LinkTarget::Note { target: link.target, heading: link.heading, block: link.block };
        return pill(t, label, Some(Message::FollowLink(target)));
    }
    match key {
        "tags" | "tag" => {
            let tag = value.trim_start_matches('#');
            pill(t, format!("#{tag}"), Some(Message::SearchTag(tag.to_owned())))
        }
        "links" => {
            let target = LinkTarget::Note { target: value.to_owned(), heading: None, block: None };
            pill(t, value.to_owned(), Some(Message::FollowLink(target)))
        }
        _ => pill(t, value.to_owned(), None),
    }
}

/// The note's properties as live preview edits them in place: one row per
/// top-level entry, a field for each value. A change goes out as
/// `Message::Property`, or `PropertyTyped` from a field, which also asks for
/// suggestions. `popup` is the suggestions, with the entry they are for; they
/// show under its row.
pub fn property_editor<'a>(
    entries: Vec<(sage_editor::properties::Entry, Option<sage_editor::properties::PropType>)>,
    popup: Option<(usize, Element<'a, Message>)>,
    menu: Option<usize>,
    hover: Option<(usize, usize)>,
    editing: Option<usize>,
    template: bool,
    new_key: Option<(sage_editor::properties::PropType, &str)>,
    t: Tokens,
) -> Element<'a, Message> {
    use sage_editor::properties::{PropType, Value};
    let mut popup = popup;
    let mut rows: Vec<Element<'a, Message>> = Vec::new();
    for (i, (entry, ty)) in entries.into_iter().enumerate() {
        let key_lc = entry.key.to_lowercase();
        let has_link = |text: &str| sage_core::links_in_line(text).iter().any(|l| !l.markdown);
        // Lists and links show as pills until an edit is asked for.
        let pillable = match &entry.value {
            Value::List { .. } => true,
            Value::Text { text, .. } => has_link(text),
            _ => false,
        };
        let editor: Element<'a, Message> = match entry.value {
            Value::List { items, .. } if editing != Some(i) => {
                let cells = items.iter().filter(|s| !s.trim().is_empty()).map(|s| value_pill(t, &key_lc, s));
                Row::with_children(cells).spacing(6).wrap().vertical_spacing(6).into()
            }
            Value::Text { text, .. } if editing != Some(i) && has_link(&text) => {
                let cells = sage_core::links_in_line(&text)
                    .into_iter()
                    .filter(|l| !l.markdown)
                    .map(|l| value_pill(t, &key_lc, &text[l.col as usize..(l.col + l.len) as usize]));
                Row::with_children(cells).spacing(6).wrap().vertical_spacing(6).into()
            }
            Value::Text { text, quote } => {
                let (hint, numeric) = match ty {
                    Some(PropType::Date) => ("YYYY-MM-DD", false),
                    Some(PropType::DateTime) => ("YYYY-MM-DDTHH:MM", false),
                    Some(PropType::Number) => ("", true),
                    _ => ("", false),
                };
                let field = iced::widget::text_input(hint, &text)
                    .id(iced::widget::Id::from(property_id(i, 0)))
                    .on_input(move |s| {
                        // A number takes digits only, so it always reads back as one.
                        let s = if numeric { s.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')).collect() } else { s };
                        Message::PropertyTyped(i, None, Value::Text { text: s, quote: if numeric || !hint.is_empty() { None } else { quote } })
                    })
                    .on_submit(Message::PropertySubmit(i, None))
                    .size(14)
                    .padding([3, 6])
                    .style(move |_, status| theme::input(t, status));
                if matches!(ty, Some(PropType::Date | PropType::DateTime)) {
                    // In a template this writes a placeholder, so each note made from it gets that day.
                    let tip = if template { "Today when a note is made from this template" } else { "Today" };
                    let today = button(iced::widget::text("Today").size(12))
                        .padding([2, 8])
                        .style(move |_, status| theme::nav_button(t, false, status))
                        .on_press(Message::PropertyToday(i));
                    let today = iced::widget::tooltip(today, iced::widget::text(tip).size(12), iced::widget::tooltip::Position::Bottom)
                        .gap(4)
                        .padding(4)
                        .style(move |_| theme::panel(t));
                    row![field, today].spacing(6).align_y(iced::Center).into()
                } else {
                    field.into()
                }
            }
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
                        .id(iced::widget::Id::from(property_id(i, k)))
                        .on_input(move |s| {
                            let mut items = all.clone();
                            items[k] = s;
                            Message::PropertyTyped(i, Some(k), Value::List { items, block })
                        })
                        .on_submit(Message::PropertySubmit(i, Some(k)))
                        .size(14)
                        .padding([2, 6])
                        .width(Length::Fixed(width.max(28.0)))
                        .style(move |_, status| theme::chip_input(t, status));
                    let remove = button(text("×").size(13))
                        .padding([0, 6])
                        .style(move |_, status| theme::nav_button(t, false, status))
                        .on_press(Message::Property(i, Value::List { items: rest, block }));
                    let hovered = hover == Some((i, k));
                    cells = cells.push(
                        iced::widget::mouse_area(
                            container(row![input, remove].align_y(iced::Center)).style(move |_| theme::chip(t, hovered)),
                        )
                        .on_enter(Message::ChipHover(Some((i, k))))
                        .on_exit(Message::ChipHover(None)),
                    );
                }
                let add = button(text("+").size(13))
                    .padding([2, 8])
                    .style(move |_, status| theme::nav_button(t, false, status))
                    .on_press(Message::PropertyAdd(i));
                cells.push(add).wrap().vertical_spacing(6).into()
            }
            Value::Raw(raw) => text(raw).size(13).color(t.text_2).into(),
        };
        // The key column of the row above, so the list lines up with the value.
        let key_width = |element: Element<'a, Message>| container(element).width(Length::FillPortion(1)).max_width(250);
        let kind: Element<'a, Message> = match ty {
            Some(current) => {
                let open = menu == Some(i);
                let button = button(text(type_icon(current)).size(12).width(Length::Fixed(22.0)).align_x(iced::Center))
                    .padding([2, 4])
                    .style(move |_, status| theme::nav_button(t, open, status))
                    .on_press(Message::PropertyTypeMenu(i));
                iced::widget::tooltip(button, text(current.label()).size(12), iced::widget::tooltip::Position::Bottom)
                    .gap(4)
                    .padding(4)
                    .style(move |_| theme::panel(t))
                    .into()
            }
            None => space().width(Length::Fixed(30.0)).into(),
        };
        let key = row![kind, text(entry.key).size(13).color(t.text_2)].spacing(8).align_y(iced::Center);
        let remove = button(text("×").size(13))
            .padding([0, 6])
            .style(move |_, status| theme::nav_button(t, false, status))
            .on_press(Message::PropertyRemove(i));
        // Editing is asked for, so a click on a pill goes to its note instead.
        let edit: Element<'a, Message> = if pillable {
            let on = editing == Some(i);
            let button = button(text(if on { "✓" } else { "✎" }).size(13))
                .padding([0, 6])
                .style(move |_, status| theme::nav_button(t, on, status))
                .on_press(Message::PropertyEdit(if on { None } else { Some(i) }));
            iced::widget::tooltip(button, text(if on { "Done" } else { "Edit" }).size(12), iced::widget::tooltip::Position::Bottom)
                .gap(4)
                .padding(4)
                .style(move |_| theme::panel(t))
                .into()
        } else {
            space().into()
        };
        rows.push(
            row![key_width(key.into()), container(editor).width(Length::FillPortion(2)), container(edit).width(Length::Fixed(26.0)), remove]
                .spacing(12)
                .align_y(iced::Center)
                .into(),
        );
        if let (true, Some(current)) = (menu == Some(i), ty) {
            let mut choices = Row::new().spacing(6);
            for choice in PropType::ALL {
                choices = choices.push(
                    button(text(format!("{} {}", type_icon(choice), choice.label())).size(12))
                        .padding([2, 8])
                        .style(move |_, status| theme::nav_button(t, choice == current, status))
                        .on_press(Message::PropertyType(i, choice)),
                );
            }
            rows.push(row![key_width(space().into()), container(choices.wrap().vertical_spacing(4)).width(Length::FillPortion(2))].spacing(12).into());
        }
        if popup.as_ref().is_some_and(|(at, _)| *at == i) {
            if let Some((_, list)) = popup.take() {
                rows.push(row![key_width(space().into()), container(list).width(Length::FillPortion(2))].spacing(12).into());
            }
        }
    }
    let add: Element<'a, Message> = match new_key {
        None => button(iced::widget::text("+ Add property").size(13))
            .padding([2, 8])
            .style(move |_, status| theme::nav_button(t, false, status))
            .on_press(Message::NewPropertyOpen)
            .into(),
        Some((current, name)) => {
            let field = iced::widget::text_input("Name", name)
                .id(iced::widget::Id::from(NEW_PROPERTY))
                .on_input(Message::NewPropertyTyped)
                .on_submit(Message::NewPropertySubmit)
                .size(13)
                .padding([3, 6])
                .style(move |_, status| theme::input(t, status));
            let mut kinds = Row::new().spacing(6);
            for choice in PropType::ALL {
                kinds = kinds.push(
                    button(text(format!("{} {}", type_icon(choice), choice.label())).size(12))
                        .padding([2, 8])
                        .style(move |_, status| theme::nav_button(t, choice == current, status))
                        .on_press(Message::NewPropertyType(choice)),
                );
            }
            row![
                container(field).width(Length::FillPortion(1)).max_width(250),
                container(kinds.wrap().vertical_spacing(4)).width(Length::FillPortion(2)),
            ]
            .spacing(12)
            .into()
        }
    };
    rows.push(add);
    container(Column::with_children(rows).spacing(4)).padding([10, 14]).width(Fill).style(move |_| theme::panel(t)).into()
}

/// The id of the field that names a new property.
pub const NEW_PROPERTY: &str = "property-new";

/// A short mark for a property type, shown in place of its name.
fn type_icon(ty: sage_editor::properties::PropType) -> &'static str {
    use sage_editor::properties::PropType;
    match ty {
        PropType::Text => "Aa",
        PropType::List => "≡",
        PropType::Number => "#",
        PropType::Checkbox => "✓",
        PropType::Date => "D",
        PropType::DateTime => "DT",
    }
}

/// The id of a list item's field, so a new item can be given the focus.
pub fn property_id(entry: usize, item: usize) -> String {
    format!("property-{entry}-{item}")
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
