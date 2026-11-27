//! The live preview pane: an editor that draws the note formatted, with the
//! Markdown syntax hidden away from the cursor (see `live.rs`).
//!
//! iced's `text_editor` cannot do this: its highlighter sets only colour and
//! font, every line has one height, and no character can be hidden. So this
//! widget draws each line as its own paragraph and maps columns between the
//! text as written and as shown. It edits the tab's `text_editor::Content`,
//! the same one the source pane edits, with the same actions, so autosave,
//! completion, find and merging work unchanged. What it does itself: clicks
//! and drags, moving up and down by what is shown, scrolling, and the caret.
//!
//! The app names each text with a revision (`next_revision`), so a layout can
//! tell the text is unchanged without reading it; then only the lines the
//! cursor left or reached are worked out again. Rendered blocks are laid out
//! where they sit in the document and moved by the scroll as they are drawn,
//! so scrolling lays nothing out again.

use crate::live::{self, Caret, Kind, Line, Seg, Show};
use crate::theme::{self, Tokens};
use iced::advanced::graphics::text::cosmic_text;
use iced::advanced::graphics::text::Paragraph;
use iced::advanced::layout::{self, Layout};
use iced::advanced::text::{self, Paragraph as _, Renderer as _, Span};
use iced::advanced::widget::{self, operation, tree, Tree, Widget};
use iced::advanced::{clipboard, mouse, renderer, Clipboard, InputMethod, Renderer as _, Shell};
use iced::alignment;
use iced::widget::text_editor::{Action, Binding, Content, Cursor, Edit, KeyPress, Motion, Position, Status};
use iced::{font, keyboard, Background, Border, Color, Element, Font, Length, Pixels, Point, Rectangle, Size, Theme, Vector};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

type Renderer = iced::Renderer;

const PAD_X: f32 = 36.0;
const PAD_Y: f32 = 28.0;
/// The text's widest, as in the preview.
const MAX_WIDTH: f32 = 688.0;
/// Room for each `>` bar, and around code.
const INDENT: f32 = 16.0;
const CODE_PAD: f32 = 12.0;

/// A number no text has had before. A tab takes a new one whenever its text
/// changes and passes it to the pane, which lays its lines out again only when
/// it differs. One counter for every tab, as one pane shows them all in turn.
pub fn next_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// What the pane asks of the app.
#[derive(Debug, Clone)]
pub enum Event {
    /// The same as the source pane's: typing, deleting, moving by character.
    Action(Action),
    /// Put the cursor here: a click, a drag, a move up or down.
    Place(Cursor),
    /// Follow a link, given as written, such as `[[Note|alias]]`.
    Follow(String),
    /// Ctrl+click on text: follow the link under the pointer, if any.
    CtrlClick(Position),
    /// Check or uncheck the task on this line.
    Toggle(usize),
}

pub struct LiveEditor<'a, Message> {
    content: &'a Content,
    /// Names `content`'s text; see `next_revision`.
    revision: u64,
    id: widget::Id,
    t: Tokens,
    /// The current find match as `(line, byte range)`.
    mark: Option<(usize, Range<usize>)>,
    on_event: Box<dyn Fn(Event) -> Message + 'a>,
    key_binding: Box<dyn Fn(KeyPress) -> Option<Binding<Message>> + 'a>,
    /// Blocks drawn as the preview draws them while the cursor is outside
    /// them, by the lines they span: tables, embeds, views, the properties.
    islands: Vec<(Range<usize>, Element<'a, Message>)>,
    editable: Vec<Range<usize>>,
    /// Drawn under the caret, over the text, such as the suggestions.
    popup: Option<Element<'a, Message>>,
}

impl<'a, Message> LiveEditor<'a, Message> {
    pub fn new(
        content: &'a Content,
        revision: u64,
        id: &'static str,
        t: Tokens,
        mark: Option<(usize, Range<usize>)>,
        on_event: impl Fn(Event) -> Message + 'a,
        key_binding: impl Fn(KeyPress) -> Option<Binding<Message>> + 'a,
    ) -> Self {
        Self {
            content,
            revision,
            id: widget::Id::new(id),
            t,
            mark,
            on_event: Box::new(on_event),
            key_binding: Box::new(key_binding),
            islands: Vec::new(),
            editable: Vec::new(),
            popup: None,
        }
    }

    /// Something to show just under the caret, or above it when there is no
    /// room below.
    pub fn popup(mut self, popup: Option<Element<'a, Message>>) -> Self {
        self.popup = popup;
        self
    }

    /// The lines of an island that is edited in place, such as the properties.
    /// It stays drawn while the cursor merely rests in it, and clicks in it
    /// belong to its own widgets. The cursor never turns it into text, as it
    /// starts on the first line when a note opens; source mode shows it raw.
    pub fn editable(mut self, lines: Vec<Range<usize>>) -> Self {
        self.editable = lines;
        self
    }

    /// Blocks to draw rendered while the cursor is outside them, each with
    /// the lines it spans.
    pub fn islands(mut self, islands: Vec<(Range<usize>, Element<'a, Message>)>) -> Self {
        self.islands = islands;
        self
    }
}

/// Room above and below a rendered block.
const ISLAND_GAP: f32 = 8.0;

/// One line as laid out.
struct Laid {
    text: String,
    line: Line,
    /// What the line before passed on, to work this line out again alone.
    carry: live::Carry,
    width: f32,
    t: Tokens,
    paragraph: Paragraph,
    segs: Vec<Seg>,
    /// Height of the text alone.
    text_h: f32,
    /// Top of the line's band in the document, the space above its text,
    /// and the band's height.
    top: f32,
    pad: f32,
    height: f32,
    /// The text's offset from the left of the text column.
    x: f32,
    /// Inside a block drawn rendered: the line's text is not drawn.
    hidden: bool,
}

#[derive(Default)]
struct State {
    focused: bool,
    modifiers: keyboard::Modifiers,
    last_click: Option<mouse::Click>,
    /// Where a drag started, while the button is down.
    drag: Option<Position>,
    /// The column a run of up and down moves keeps to.
    goal_x: Option<f32>,
    scroll: f32,
    lines: Vec<Laid>,
    /// The left of the text column within the widget, its height, and the
    /// document's height.
    left: f32,
    viewport: f32,
    total: f32,
    caret: Option<Caret>,
    /// Which islands are drawn rendered, rather than as written.
    rendered: Vec<bool>,
    /// What the lines were last laid out for, besides the cursor; with the
    /// same, only the lines the cursor left or reached can come out different.
    laid_for: Option<LaidFor>,
}

/// The text's revision, the known notes' generation, the width, the theme and
/// the islands' lines and heights.
type LaidFor = (u64, u64, f32, Tokens, Vec<(Range<usize>, f32)>);

impl operation::Focusable for State {
    fn is_focused(&self) -> bool {
        self.focused
    }
    fn focus(&mut self) {
        self.focused = true;
    }
    fn unfocus(&mut self) {
        self.focused = false;
        self.drag = None;
    }
}

/// What lies under a point.
enum Hit {
    Text(Position),
    Checkbox(usize),
    Link(usize, Range<usize>),
}

fn metrics(kind: Kind) -> (Font, f32, f32) {
    match kind {
        Kind::Heading(level) => (theme::SERIF, [30.0, 24.0, 20.0, 17.0, 16.0, 15.0][usize::from(level).clamp(1, 6) - 1], 1.35),
        Kind::Code | Kind::Fence => (theme::MONO, 14.0, 1.5),
        Kind::Frontmatter => (theme::MONO, 13.0, 1.5),
        Kind::Text | Kind::Rule => (theme::SANS, 16.0, 1.55),
    }
}

fn callout_color(line: &Line, t: Tokens) -> Option<Color> {
    line.callout.as_deref().map(|kind| crate::preview::callout_color(kind, t))
}

/// Lays out one line's paragraph.
fn lay(text: String, line: Line, carry: live::Carry, width: f32, t: Tokens) -> Laid {
    let (base, size, height) = metrics(line.kind);
    let (shown, segs) = live::shown(&line, &text);
    let muted = line.done || (line.quotes > 0 && line.callout.is_none());
    let body = if muted { t.text_2 } else { t.text };
    let mut spans: Vec<Span<'_, (), Font>> = Vec::new();
    for (piece, seg) in line.pieces.iter().zip(&segs) {
        if seg.shown.is_empty() {
            continue;
        }
        let mut font = if line.title { theme::SANS_BOLD } else { base };
        let s = piece.style;
        if s.strong {
            font.weight = font::Weight::Semibold;
        }
        if s.emphasis {
            font.style = font::Style::Italic;
        }
        // Inline code: monospace a little smaller, as the preview draws it.
        let inline_code = s.code && matches!(line.kind, Kind::Text | Kind::Heading(_));
        if inline_code {
            font = theme::MONO;
        }
        let color = if s.syntax || s.tag || matches!(piece.show, Show::Replace(_)) && !line.title {
            t.text_2
        } else if s.link && s.missing {
            // A link to a note that does not exist yet recedes, underlined by its colour alone.
            t.text_2
        } else if s.link {
            t.accent
        } else if line.title {
            callout_color(&line, t).unwrap_or(t.accent)
        } else {
            body
        };
        let mut span = Span::new(&shown[seg.shown.clone()]).font(font).color(color);
        if inline_code {
            // At the body's line height, so the line measures what it shows.
            span = span.size(size * 0.85).line_height(text::LineHeight::Absolute(Pixels(size * height)));
        }
        spans.push(span);
    }
    if spans.is_empty() {
        spans.push(Span::new("").font(base));
    }
    let x = f32::from(line.quotes) * INDENT
        + if line.callout.is_some() { CODE_PAD - 4.0 } else { 0.0 }
        + if matches!(line.kind, Kind::Code | Kind::Fence) { CODE_PAD } else { 0.0 };
    let inner = (width - x - if matches!(line.kind, Kind::Code | Kind::Fence) { CODE_PAD } else { 0.0 }).max(20.0);
    let paragraph = Paragraph::with_spans(text::Text {
        content: spans.as_slice(),
        bounds: Size::new(inner, f32::INFINITY),
        size: Pixels(size),
        line_height: text::LineHeight::Relative(height),
        font: base,
        align_x: text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: text::Shaping::Advanced,
        wrapping: text::Wrapping::WordOrGlyph,
    });
    let text_h = paragraph.min_bounds().height.max(size * height);
    Laid { text, line, carry, width, t, paragraph, segs, text_h, top: 0.0, pad: 0.0, height: text_h, x, hidden: false }
}

/// The cursor and the selection's other end, ordered.
fn caret_of(content: &Content) -> Caret {
    let cursor = content.cursor();
    let at = (cursor.position.line, cursor.position.column);
    Caret::new(at, cursor.selection.map_or(at, |s| (s.line, s.column)))
}

fn caret_pos(content: &Content) -> (usize, usize) {
    let p = content.cursor().position;
    (p.line, p.column)
}

/// Where a byte of the shown text sits: its x, and its visual line's top and height.
fn caret_at(buffer: &cosmic_text::Buffer, index: usize) -> (f32, f32, f32) {
    let mut last = (0.0, 0.0, 0.0);
    for run in buffer.layout_runs() {
        let start = run.glyphs.first().map_or(0, |g| g.start);
        let end = run.glyphs.last().map_or(0, |g| g.end);
        last = (run.line_w, run.line_top, run.line_height);
        if index >= start && index <= end {
            return (x_in(&run, index), run.line_top, run.line_height);
        }
    }
    last
}

fn x_in(run: &cosmic_text::LayoutRun<'_>, index: usize) -> f32 {
    for g in run.glyphs {
        if g.end > index {
            if g.start >= index {
                return g.x;
            }
            // Inside a cluster: share its width out.
            let frac = (index - g.start) as f32 / (g.end - g.start).max(1) as f32;
            return g.x + g.w * frac;
        }
    }
    run.line_w
}

/// The boxes covering bytes `from..to` of the shown text, one per visual line.
fn boxes(buffer: &cosmic_text::Buffer, from: usize, to: usize) -> Vec<Rectangle> {
    let mut out = Vec::new();
    for run in buffer.layout_runs() {
        let start = run.glyphs.first().map_or(0, |g| g.start);
        let end = run.glyphs.last().map_or(0, |g| g.end);
        let (a, b) = (from.max(start), to.min(end));
        if a < b {
            let (x1, x2) = (x_in(&run, a), x_in(&run, b));
            out.push(Rectangle::new(Point::new(x1, run.line_top), Size::new(x2 - x1, run.line_height)));
        }
    }
    out
}

impl State {
    /// Keeps the button and key state that clicks and drags depend on. It sees
    /// every event before the rendered blocks do: one that captures the
    /// release, such as a checkbox or a board, would otherwise leave a drag
    /// running, and every later move of the pointer would select text. A
    /// window that loses focus misses releases, so it forgets both: Shift let
    /// go elsewhere would otherwise turn every click into a selection.
    fn track(&mut self, event: &iced::Event) {
        match event {
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => self.drag = None,
            iced::Event::Window(iced::window::Event::Unfocused) => {
                self.drag = None;
                self.modifiers = keyboard::Modifiers::default();
            }
            iced::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => self.modifiers = *modifiers,
            _ => {}
        }
    }

    /// Lays out every line. `revision` names the text: the same number, the
    /// same text. `islands` are the lines of blocks drawn rendered, with the
    /// height of their drawing.
    fn relayout(&mut self, content: &Content, revision: u64, width: f32, t: Tokens, islands: &[(Range<usize>, f32)]) {
        let caret = caret_of(content);
        // Scrolling, the autosave clock and index passes all ask for a layout.
        // With the same text, known notes, width, theme and islands, only the
        // lines the cursor left or reached can show differently.
        let laid_for = (revision, live::known_generation(), width, t, islands.to_vec());
        let moved = self.caret.filter(|old| *old != caret);
        if self.laid_for.as_ref() != Some(&laid_for) || self.lines.len() != content.line_count() || self.caret.is_none() {
            self.laid_for = Some(laid_for);
            self.lay_all(content, caret, width, t);
        } else if let Some(old) = moved {
            self.lay_caret(old, caret, width, t);
        } else {
            self.scroll = self.scroll.clamp(0.0, (self.total - self.viewport).max(0.0));
            return;
        }
        self.place(islands);
        if self.caret != Some(caret) {
            self.caret = Some(caret);
            self.reveal(caret_pos(content));
        }
        self.scroll = self.scroll.clamp(0.0, (self.total - self.viewport).max(0.0));
    }

    /// Works out every line's pieces again, keeping the paragraphs of the
    /// lines that come out the same, wherever they moved.
    fn lay_all(&mut self, content: &Content, caret: Caret, width: f32, t: Tokens) {
        let texts: Vec<String> = content.lines().map(|l| l.text.into_owned()).collect();
        let count = texts.len();
        // An edit leaves the lines before it and after it as they were, so
        // those are matched by place from both ends; only the lines between
        // are looked up by their text.
        let mut old: Vec<Option<Laid>> = std::mem::take(&mut self.lines).into_iter().map(Some).collect();
        let same = |laid: &Option<Laid>, text: &String| laid.as_ref().is_some_and(|l| l.text == *text);
        let head = old.iter().zip(&texts).take_while(|(laid, text)| same(laid, text)).count();
        let tail = old[head..].iter().rev().zip(texts[head..].iter().rev()).take_while(|(laid, text)| same(laid, text)).count();
        let mut between: HashMap<String, Vec<Laid>> = HashMap::new();
        let old_count = old.len();
        for laid in old[head..old_count - tail].iter_mut().filter_map(Option::take) {
            between.entry(laid.text.clone()).or_default().push(laid);
        }
        let fits = |laid: &Laid, line: &Line| laid.line == *line && laid.width == width && laid.t == t;
        let mut carry = live::Carry::default();
        for (n, text) in texts.into_iter().enumerate() {
            let entering = carry.clone();
            let line = live::next_line(&text, n, &mut carry, Some(caret));
            let kept = if n < head || n >= count - tail {
                let i = if n < head { n } else { old_count - (count - n) };
                old[i].take().filter(|laid| fits(laid, &line))
            } else {
                between.get_mut(&text).and_then(|v| {
                    let i = v.iter().position(|laid| fits(laid, &line))?;
                    Some(v.swap_remove(i))
                })
            };
            self.lines.push(match kept {
                Some(laid) => Laid { carry: entering, ..laid },
                None => lay(text, line, entering, width, t),
            });
        }
    }

    /// Works out again only the lines the cursor or selection left or
    /// reached: with the text unchanged, no other line shows differently.
    fn lay_caret(&mut self, old: Caret, caret: Caret, width: f32, t: Tokens) {
        let last = self.lines.len().saturating_sub(1);
        let left = old.from.0..=old.to.0.min(last);
        let reached = (caret.from.0..=caret.to.0.min(last)).filter(|n| !left.contains(n));
        for n in left.clone().chain(reached) {
            let laid = &mut self.lines[n];
            let line = live::next_line(&laid.text, n, &mut laid.carry.clone(), Some(caret));
            if line != laid.line {
                let text = std::mem::take(&mut laid.text);
                *laid = lay(text, line, laid.carry.clone(), width, t);
            }
        }
    }

    /// Stacks the lines: each one's room above, height and top, and the
    /// document's height.
    fn place(&mut self, islands: &[(Range<usize>, f32)]) {
        let mut top = PAD_Y;
        for (n, laid) in self.lines.iter_mut().enumerate() {
            // Headings keep some room above, except at the top.
            laid.pad = if matches!(laid.line.kind, Kind::Heading(_)) && n > 0 { 12.0 } else { 0.0 };
            laid.height = laid.pad + laid.text_h;
            // A rendered block takes the room of all its lines on its first.
            let island = islands.iter().find(|(lines, _)| lines.contains(&n));
            laid.hidden = island.is_some();
            if let Some((lines, height)) = island {
                laid.pad = 0.0;
                laid.height = if n == lines.start { height + 2.0 * ISLAND_GAP } else { 0.0 };
            }
            laid.top = top;
            top += laid.height;
        }
        self.total = top + PAD_Y;
    }

    /// How far the rendered blocks, laid out where they sit in the document,
    /// are moved up on screen: the scroll.
    fn offset(&self) -> Vector {
        Vector::new(0.0, self.scroll)
    }

    /// The pane's bounds as the rendered blocks see them, in the document.
    fn visible(&self, bounds: Rectangle) -> Rectangle {
        Rectangle { y: bounds.y + self.scroll, ..bounds }
    }

    /// Scrolls just enough to show the cursor.
    fn reveal(&mut self, at: (usize, usize)) {
        let Some((_, y, h)) = self.caret_point(Position { line: at.0, column: at.1 }) else { return };
        if y - 8.0 < self.scroll {
            self.scroll = y - 8.0;
        } else if y + h + 8.0 > self.scroll + self.viewport {
            self.scroll = y + h + 8.0 - self.viewport;
        }
    }

    /// The caret's x within the text column and its top and height in the
    /// document.
    fn caret_point(&self, at: Position) -> Option<(f32, f32, f32)> {
        let laid = self.lines.get(at.line)?;
        let shown = live::to_shown(&laid.segs, at.column);
        let (x, top, h) = caret_at(laid.paragraph.buffer(), shown);
        Some((laid.x + x, laid.top + laid.pad + top, h))
    }

    /// The line at a height in the document.
    fn line_at(&self, y: f32) -> usize {
        self.lines.partition_point(|l| l.top <= y).saturating_sub(1)
    }

    /// What lies under a point given in document coordinates, x from the
    /// left of the text column.
    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let first = self.lines.first()?;
        if y < first.top {
            return Some(Hit::Text(Position { line: 0, column: 0 }));
        }
        let last = self.lines.last()?;
        if y >= last.top + last.height {
            let n = self.lines.len() - 1;
            return Some(Hit::Text(Position { line: n, column: last.text.len() }));
        }
        let n = self.line_at(y);
        let laid = &self.lines[n];
        let buffer = laid.paragraph.buffer();
        let (px, py) = (x - laid.x, y - laid.top - laid.pad);
        // A checkbox or a link under the pointer itself, not just nearby.
        for (piece, seg) in laid.line.pieces.iter().zip(&laid.segs) {
            let inside = || boxes(buffer, seg.shown.start, seg.shown.end).iter().any(|b| b.contains(Point::new(px, py)));
            match (&piece.show, &piece.link) {
                (Show::Checkbox(_), _) if inside() => return Some(Hit::Checkbox(n)),
                (_, Some(link)) if inside() => return Some(Hit::Link(n, link.clone())),
                _ => {}
            }
        }
        Some(Hit::Text(self.position_in(n, x, py)))
    }

    /// Where a move by what is shown, rather than by the text as written,
    /// takes the cursor; `None` for moves the text does alike.
    fn moved(&mut self, at: Position, motion: Motion) -> Option<Position> {
        if !matches!(motion, Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown) {
            self.goal_x = None;
        }
        match motion {
            Motion::Up => self.vertical(at, true, false),
            Motion::Down => self.vertical(at, false, false),
            Motion::PageUp => self.vertical(at, true, true),
            Motion::PageDown => self.vertical(at, false, true),
            Motion::Home => Some(Position { line: at.line, column: 0 }),
            Motion::End => Some(Position { line: at.line, column: self.lines.get(at.line).map_or(0, |l| l.text.len()) }),
            _ => None,
        }
    }

    /// The position one visual line above (`up`) or below the cursor, or a
    /// page away, keeping to the goal column.
    fn vertical(&mut self, at: Position, up: bool, page: bool) -> Option<Position> {
        let laid = self.lines.get(at.line)?;
        let shown = live::to_shown(&laid.segs, at.column);
        let (x, y, h) = caret_at(laid.paragraph.buffer(), shown);
        let goal = *self.goal_x.get_or_insert(laid.x + x);
        if page {
            let distance = (self.viewport - 40.0).max(20.0);
            let doc_y = laid.top + laid.pad + y;
            let target = if up { doc_y - distance } else { doc_y + h + distance };
            let n = self.line_at(target.max(0.0));
            let laid = &self.lines[n];
            return Some(self.position_in(n, goal, target - laid.top - laid.pad));
        }
        // Another visual line of the same paragraph, else the nearest visual
        // line of the next line. By the paragraph's own visual lines, not by
        // heights, which a span with its own size can make disagree.
        let runs: Vec<(f32, f32)> = laid.paragraph.buffer().layout_runs().map(|r| (r.line_top, r.line_height)).collect();
        let k = runs.iter().position(|&(top, _)| (top - y).abs() < 0.5).unwrap_or(0);
        let middle = |(top, h): (f32, f32)| top + h / 2.0;
        let (n, py) = if up && k > 0 {
            (at.line, middle(runs[k - 1]))
        } else if !up && k + 1 < runs.len() {
            (at.line, middle(runs[k + 1]))
        } else if up {
            if at.line == 0 {
                return Some(Position { line: 0, column: 0 });
            }
            let above = &self.lines[at.line - 1];
            let last = above.paragraph.buffer().layout_runs().last().map(|r| (r.line_top, r.line_height));
            (at.line - 1, last.map_or(0.0, middle))
        } else {
            if at.line + 1 >= self.lines.len() {
                return Some(Position { line: at.line, column: laid.text.len() });
            }
            (at.line + 1, 1.0)
        };
        Some(self.position_in(n, goal, py))
    }

    /// The position at `x` from the text column's left and `py` from the top
    /// of line `n`'s text.
    fn position_in(&self, n: usize, x: f32, py: f32) -> Position {
        let laid = &self.lines[n];
        let py = py.clamp(0.0, (laid.height - laid.pad - 1.0).max(0.0));
        let shown = laid.paragraph.buffer().hit((x - laid.x).max(0.0), py).map_or(0, |c| c.index);
        Position { line: n, column: live::to_source(&laid.segs, shown) }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for LiveEditor<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn children(&self) -> Vec<Tree> {
        self.islands.iter().map(|(_, element)| Tree::new(element)).chain(self.popup.as_ref().map(Tree::new)).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        let elements: Vec<&Element<'_, Message>> = self.islands.iter().map(|(_, e)| e).chain(self.popup.as_ref()).collect();
        tree.diff_children(&elements);
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let size = limits.width(Length::Fill).height(Length::Fill).max();
        let width = (size.width - 2.0 * PAD_X).clamp(80.0, MAX_WIDTH);
        // A block shows as written while the cursor or selection is in it.
        let caret = caret_of(self.content);
        let island_limits = layout::Limits::new(Size::ZERO, Size::new(width, f32::INFINITY));
        let mut nodes = Vec::with_capacity(self.islands.len());
        let mut heights = Vec::new();
        let mut rendered = Vec::with_capacity(self.islands.len());
        for ((lines, element), child) in self.islands.iter_mut().zip(&mut tree.children) {
            let editable = self.editable.contains(lines);
            let raw = live::touches_island(lines, caret) && !editable;
            rendered.push(!raw);
            if raw {
                nodes.push(layout::Node::new(Size::ZERO));
            } else {
                let node = element.as_widget_mut().layout(child, renderer, &island_limits);
                heights.push((lines.clone(), node.size().height));
                nodes.push(node);
            }
        }
        let state = tree.state.downcast_mut::<State>();
        state.left = PAD_X;
        state.viewport = size.height;
        state.rendered = rendered;
        state.relayout(self.content, self.revision, width, self.t, &heights);
        // Placed where their first line is in the document, not scrolled:
        // drawing and events move them by the scroll (`State::offset`).
        for ((lines, _), node) in self.islands.iter().zip(&mut nodes) {
            if let Some(laid) = state.lines.get(lines.start) {
                *node = node.clone().move_to(Point::new(state.left, laid.top + ISLAND_GAP));
            }
        }
        // The popup goes under the caret, or above it at the bottom of the pane.
        if let Some(popup) = self.popup.as_mut() {
            let child = &mut tree.children[self.islands.len()];
            let limits = layout::Limits::new(Size::ZERO, Size::new((size.width - 16.0).min(560.0), size.height));
            let node = popup.as_widget_mut().layout(child, renderer, &limits);
            let state = tree.state.downcast_ref::<State>();
            let (x, top, h) = state.caret_point(self.content.cursor().position).unwrap_or((0.0, 0.0, 0.0));
            let (x, top) = (state.left + x, top - state.scroll);
            let below = top + h + 4.0;
            let y = if below + node.size().height > size.height { (top - node.size().height - 4.0).max(0.0) } else { below };
            let x = x.min(size.width - node.size().width - 8.0).max(0.0);
            nodes.push(node.move_to(Point::new(x, y)));
        }
        layout::Node::with_children(size, nodes)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        tree.state.downcast_mut::<State>().track(event);
        // The popup is on top, so it takes its events first.
        if let Some(popup) = self.popup.as_mut() {
            let n = self.islands.len();
            if let Some(node) = layout.children().nth(n) {
                let inner = if cursor.is_over(bounds) { cursor } else { mouse::Cursor::Unavailable };
                popup.as_widget_mut().update(&mut tree.children[n], event, node, inner, renderer, clipboard, shell, &bounds);
                if shell.is_event_captured() {
                    return;
                }
            }
        }
        // Rendered blocks first: their links, checkboxes and boards. They are
        // scrolled within the pane, so the pointer counts only inside it, and
        // moves down the document by the scroll as they are laid out there.
        let (rendered, offset, visible) = {
            let state = tree.state.downcast_ref::<State>();
            (state.rendered.clone(), state.offset(), state.visible(bounds))
        };
        let inner = if cursor.is_over(bounds) { cursor + offset } else { mouse::Cursor::Unavailable };
        let had_input_method = shell.input_method().is_enabled();
        for ((((_, element), child), node), shown) in
            self.islands.iter_mut().zip(&mut tree.children).zip(layout.children()).zip(rendered)
        {
            if shown {
                element.as_widget_mut().update(child, event, node, inner, renderer, clipboard, shell, &visible);
            }
        }
        // A field in a block placed its input method in the document; on screen it is scrolled.
        if !had_input_method {
            if let InputMethod::Enabled { cursor, .. } = shell.input_method_mut() {
                *cursor = *cursor - offset;
            }
        }
        if shell.is_event_captured() {
            return;
        }
        let state = tree.state.downcast_mut::<State>();
        let on = &self.on_event;
        let current = self.content.cursor();
        let anchor = current.selection.unwrap_or(current.position);
        // From a point on screen to the document.
        let doc = |state: &State, p: Point| (p.x - bounds.x - state.left, p.y - bounds.y + state.scroll);

        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_over(bounds) else {
                    state.focused = false;
                    state.drag = None;
                    return;
                };
                state.focused = true;
                state.goal_x = None;
                let click = mouse::Click::new(p, mouse::Button::Left, state.last_click);
                state.last_click = Some(click);
                let (x, y) = doc(state, p);
                // Over an editable island, a click that its widgets did not take is theirs alone.
                let n = state.line_at(y);
                let over_editable = self
                    .islands
                    .iter()
                    .zip(&state.rendered)
                    .any(|((l, _), shown)| *shown && l.contains(&n) && self.editable.contains(l));
                if over_editable {
                    state.focused = false;
                    shell.capture_event();
                    return;
                }
                let Some(hit) = state.hit(x, y) else { return };
                let place = |position, selection| on(Event::Place(Cursor { position, selection }));
                match (click.kind(), hit) {
                    (mouse::click::Kind::Single, Hit::Text(p)) if state.modifiers.shift() => {
                        shell.publish(place(p, Some(anchor)));
                    }
                    // Ctrl+click follows a link whose syntax is showing, as the cursor is on it.
                    (mouse::click::Kind::Single, Hit::Text(p)) if state.modifiers.command() => {
                        shell.publish(place(p, None));
                        shell.publish(on(Event::CtrlClick(p)));
                    }
                    (mouse::click::Kind::Single, Hit::Checkbox(line)) => shell.publish(on(Event::Toggle(line))),
                    (mouse::click::Kind::Single, Hit::Link(line, range)) => {
                        let text = &state.lines[line].text;
                        shell.publish(on(Event::Follow(text[range].to_owned())));
                    }
                    (kind, hit) => {
                        let p = match hit {
                            Hit::Text(p) => p,
                            Hit::Checkbox(line) | Hit::Link(line, _) => Position { line, column: 0 },
                        };
                        shell.publish(place(p, None));
                        match kind {
                            mouse::click::Kind::Single => state.drag = Some(p),
                            mouse::click::Kind::Double => shell.publish(on(Event::Action(Action::SelectWord))),
                            mouse::click::Kind::Triple => shell.publish(on(Event::Action(Action::SelectLine))),
                        }
                    }
                }
                shell.capture_event();
            }
            // Releases, focus and modifiers were taken in by `State::track`.
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some(from) = state.drag {
                    let (x, y) = doc(state, *position);
                    let to = match state.hit(x, y) {
                        Some(Hit::Text(p)) => p,
                        Some(Hit::Checkbox(line) | Hit::Link(line, _)) => Position { line, column: 0 },
                        None => return,
                    };
                    // Back where the drag began, nothing is selected rather than an
                    // empty selection, which the next click would select from.
                    if to != current.position || current.selection.is_some_and(|s| s != from) {
                        let selection = (to != from).then_some(from);
                        shell.publish(on(Event::Place(Cursor { position: to, selection })));
                    }
                }
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * 60.0,
                    mouse::ScrollDelta::Pixels { y, .. } => *y,
                };
                state.scroll = (state.scroll - dy).clamp(0.0, (state.total - state.viewport).max(0.0));
                // Rendered blocks move with the scroll as they are drawn; only
                // the popup, placed by the caret on screen, needs a layout.
                if self.popup.is_some() {
                    shell.invalidate_layout();
                }
                shell.request_redraw();
                shell.capture_event();
            }
            iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modified_key, physical_key, modifiers, text, .. }) => {
                let status = if state.focused {
                    Status::Focused { is_hovered: cursor.is_over(bounds) }
                } else {
                    Status::Active
                };
                let press = KeyPress {
                    key: key.clone(),
                    modified_key: modified_key.clone(),
                    physical_key: *physical_key,
                    modifiers: *modifiers,
                    text: text.clone(),
                    status,
                };
                let Some(binding) = (self.key_binding)(press) else { return };
                if !matches!(binding, Binding::Unfocus) {
                    shell.capture_event();
                }
                self.apply(binding, state, clipboard, shell);
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let t = self.t;
        let pointer = if cursor.is_over(bounds) { cursor } else { mouse::Cursor::Unavailable };
        let quad = |renderer: &mut Renderer, rect: Rectangle, color: Color| {
            renderer.fill_quad(renderer::Quad { bounds: rect, ..renderer::Quad::default() }, Background::Color(color));
        };
        quad(renderer, bounds, t.bg);

        let cursor = self.content.cursor();
        let caret = state.caret;
        renderer.with_layer(bounds, |renderer| {
            let left = bounds.x + state.left;
            let width = state.lines.first().map_or(0.0, |l| l.width);
            let first = state.line_at(state.scroll);
            for (n, laid) in state.lines.iter().enumerate().skip(first) {
                let top = bounds.y + laid.top - state.scroll;
                if top > bounds.y + bounds.height {
                    break;
                }
                if laid.hidden {
                    continue;
                }
                let line = &laid.line;
                let band = Rectangle::new(Point::new(left, top + laid.pad), Size::new(width, laid.height - laid.pad));
                let origin = Point::new(left + laid.x, top + laid.pad);
                let buffer = laid.paragraph.buffer();
                let at = |r: Rectangle| Rectangle::new(Point::new(origin.x + r.x, origin.y + r.y), r.size());

                // Blocks: code on the surface, quotes and callouts with a bar.
                if matches!(line.kind, Kind::Code | Kind::Fence) {
                    quad(renderer, band, t.surface);
                }
                if let Some(color) = callout_color(line, t) {
                    quad(renderer, band, t.surface);
                    quad(renderer, Rectangle::new(band.position(), Size::new(2.0, band.height)), color);
                } else {
                    for q in 0..line.quotes {
                        let x = left + f32::from(q) * INDENT;
                        quad(renderer, Rectangle::new(Point::new(x, band.y), Size::new(2.0, band.height)), t.accent);
                    }
                }
                if line.kind == Kind::Rule && laid.segs.iter().all(|s| s.shown.is_empty()) {
                    let y = band.y + band.height / 2.0;
                    quad(renderer, Rectangle::new(Point::new(left, y), Size::new(width, 1.0)), t.border);
                }

                // The selection, and the find match.
                if let Some(c) = caret.filter(|c| c.from != c.to && (c.from.0..=c.to.0).contains(&n)) {
                    let from = if n == c.from.0 { c.from.1 } else { 0 };
                    let to = if n == c.to.0 { c.to.1 } else { laid.text.len() };
                    let (a, b) = (live::to_shown(&laid.segs, from), live::to_shown(&laid.segs, to));
                    let selection = theme::selection(t);
                    for r in boxes(buffer, a, b) {
                        quad(renderer, at(r), selection);
                    }
                    if n < c.to.0 {
                        // The line break, so selected blank lines show.
                        let (x, y, h) = caret_at(buffer, b);
                        quad(renderer, at(Rectangle::new(Point::new(x, y), Size::new(5.0, h))), selection);
                    }
                }
                if let Some((_, range)) = self.mark.as_ref().filter(|(line, _)| *line == n) {
                    let (a, b) = (live::to_shown(&laid.segs, range.start), live::to_shown(&laid.segs, range.end));
                    for r in boxes(buffer, a, b) {
                        quad(renderer, at(r), theme::find_match(t));
                    }
                }

                // Backgrounds of inline code and highlights.
                for (piece, seg) in line.pieces.iter().zip(&laid.segs) {
                    let fill = if piece.style.code && matches!(line.kind, Kind::Text | Kind::Heading(_)) {
                        Some(t.surface)
                    } else if piece.style.highlight {
                        Some(theme::highlight(t))
                    } else {
                        None
                    };
                    if let Some(fill) = fill {
                        for r in boxes(buffer, seg.shown.start, seg.shown.end) {
                            let r = at(r);
                            let r = Rectangle::new(Point::new(r.x - 2.0, r.y + 2.0), Size::new(r.width + 4.0, r.height - 4.0));
                            renderer.fill_quad(
                                renderer::Quad {
                                    bounds: r,
                                    border: Border { radius: theme::RADIUS.into(), ..Border::default() },
                                    ..renderer::Quad::default()
                                },
                                Background::Color(fill),
                            );
                        }
                    }
                }

                renderer.fill_paragraph(&laid.paragraph, origin, t.text, bounds);

                // Strikethrough, and task checkboxes over their placeholders.
                for (piece, seg) in line.pieces.iter().zip(&laid.segs) {
                    if piece.style.strike {
                        for r in boxes(buffer, seg.shown.start, seg.shown.end) {
                            let r = at(r);
                            quad(renderer, Rectangle::new(Point::new(r.x, r.y + r.height / 2.0), Size::new(r.width, 1.0)), t.text_2);
                        }
                    }
                    if let Show::Checkbox(done) = piece.show {
                        let (x, y, h) = caret_at(buffer, seg.shown.start);
                        let side = 13.0;
                        let r = Rectangle::new(Point::new(origin.x + x + 1.0, origin.y + y + (h - side) / 2.0), Size::new(side, side));
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: r,
                                border: Border { color: if done { t.accent } else { t.text_2 }, width: 1.0, radius: 2.0.into() },
                                ..renderer::Quad::default()
                            },
                            Background::Color(if done { t.accent } else { Color::TRANSPARENT }),
                        );
                    }
                }

                // The caret.
                if state.focused && n == cursor.position.line {
                    if let Some((x, y, h)) = state.caret_point(cursor.position) {
                        let r = Rectangle::new(Point::new(left + x, bounds.y + y - state.scroll), Size::new(1.5, h));
                        quad(renderer, r, t.text);
                    }
                }
            }

            // Rendered blocks, over the room their lines left, moved by the scroll.
            let visible = state.visible(bounds);
            renderer.with_translation(Vector::new(0.0, -state.scroll), |renderer| {
                for ((((_, element), child), node), shown) in
                    self.islands.iter().zip(&tree.children).zip(layout.children()).zip(&state.rendered)
                {
                    if *shown && node.bounds().intersects(&visible) {
                        element.as_widget().draw(child, renderer, theme, style, node, pointer + state.offset(), &visible);
                    }
                }
            });
        });
        // The popup, over everything.
        if let Some(popup) = &self.popup {
            let n = self.islands.len();
            if let (Some(node), Some(child)) = (layout.children().nth(n), tree.children.get(n)) {
                renderer.with_layer(bounds, |renderer| {
                    popup.as_widget().draw(child, renderer, theme, style, node, pointer, &bounds);
                });
            }
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let Some(p) = cursor.position_over(bounds) else { return mouse::Interaction::default() };
        if let Some(popup) = &self.popup {
            let n = self.islands.len();
            if let Some(node) = layout.children().nth(n).filter(|node| cursor.is_over(node.bounds())) {
                return popup.as_widget().mouse_interaction(&tree.children[n], node, cursor, &bounds, renderer);
            }
        }
        let (inner, visible) = (cursor + state.offset(), state.visible(bounds));
        for ((((_, element), child), node), shown) in
            self.islands.iter().zip(&tree.children).zip(layout.children()).zip(&state.rendered)
        {
            if *shown && inner.is_over(node.bounds()) {
                return element.as_widget().mouse_interaction(child, node, inner, &visible, renderer);
            }
        }
        match state.hit(p.x - bounds.x - state.left, p.y - bounds.y + state.scroll) {
            Some(Hit::Checkbox(_) | Hit::Link(..)) => mouse::Interaction::Pointer,
            _ => mouse::Interaction::Text,
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        // The menus and tooltips of the drawn blocks, such as a property's type
        // list, moved by the scroll as the blocks are.
        let (shown, offset) = {
            let state = tree.state.downcast_ref::<State>();
            (state.rendered.clone(), state.offset())
        };
        let overlays: Vec<_> = self
            .islands
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
            .zip(shown)
            .filter(|(_, shown)| *shown)
            .filter_map(|((((_, element), child), node), _)| {
                element.as_widget_mut().overlay(child, node, renderer, viewport, translation - offset)
            })
            .collect();
        (!overlays.is_empty()).then(|| iced::advanced::overlay::Group::with_children(overlays).overlay())
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        let state = tree.state.downcast_mut::<State>();
        operation.focusable(Some(&self.id), layout.bounds(), state);
        operation.traverse(&mut |operation| {
            for (((_, element), child), node) in self.islands.iter_mut().zip(&mut tree.children).zip(layout.children()) {
                element.as_widget_mut().operate(child, node, renderer, operation);
            }
        });
    }
}

impl<Message> LiveEditor<'_, Message> {
    fn apply(&self, binding: Binding<Message>, state: &mut State, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>) {
        let on = &self.on_event;
        let action = |action| on(Event::Action(action));
        let current = self.content.cursor();
        let anchor = current.selection.unwrap_or(current.position);
        match binding {
            Binding::Unfocus => {
                state.focused = false;
                state.drag = None;
            }
            Binding::Copy => {
                if let Some(selection) = self.content.selection() {
                    clipboard.write(clipboard::Kind::Standard, selection);
                }
            }
            Binding::Cut => {
                if let Some(selection) = self.content.selection() {
                    clipboard.write(clipboard::Kind::Standard, selection);
                    shell.publish(action(Action::Edit(Edit::Delete)));
                }
            }
            Binding::Paste => {
                if let Some(text) = clipboard.read(clipboard::Kind::Standard) {
                    shell.publish(action(Action::Edit(Edit::Paste(Arc::new(text)))));
                }
            }
            Binding::Move(m) => match state.moved(current.position, m) {
                Some(to) => shell.publish(on(Event::Place(Cursor { position: to, selection: None }))),
                None => shell.publish(action(Action::Move(m))),
            },
            Binding::Select(m) => match state.moved(current.position, m) {
                Some(to) => shell.publish(on(Event::Place(Cursor { position: to, selection: Some(anchor) }))),
                None => shell.publish(action(Action::Select(m))),
            },
            Binding::SelectWord => shell.publish(action(Action::SelectWord)),
            Binding::SelectLine => shell.publish(action(Action::SelectLine)),
            Binding::SelectAll => shell.publish(action(Action::SelectAll)),
            Binding::Insert(c) => shell.publish(action(Action::Edit(Edit::Insert(c)))),
            Binding::Enter => shell.publish(action(Action::Edit(Edit::Enter))),
            Binding::Backspace => shell.publish(action(Action::Edit(Edit::Backspace))),
            Binding::Delete => shell.publish(action(Action::Edit(Edit::Delete))),
            Binding::Sequence(bindings) => {
                for binding in bindings {
                    self.apply(binding, state, clipboard, shell);
                }
            }
            Binding::Custom(message) => shell.publish(message),
        }
    }
}

impl<'a, Message: 'a> From<LiveEditor<'a, Message>> for Element<'a, Message> {
    fn from(editor: LiveEditor<'a, Message>) -> Self {
        Element::new(editor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_do_not_select_after_a_missed_release() {
        let mut state = State { drag: Some(Position { line: 0, column: 0 }), ..State::default() };
        // The release lands on a rendered block, which captures it.
        state.track(&iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)));
        assert_eq!(state.drag, None, "moving the pointer must not go on selecting");

        // Shift held while the window lost focus, let go elsewhere.
        state.track(&iced::Event::Keyboard(keyboard::Event::ModifiersChanged(keyboard::Modifiers::SHIFT)));
        state.drag = Some(Position { line: 1, column: 2 });
        state.track(&iced::Event::Window(iced::window::Event::Unfocused));
        assert!(!state.modifiers.shift(), "a later click must not extend the selection");
        assert_eq!(state.drag, None);
    }

    /// The pane laid out over `text` with the cursor at `at`.
    fn laid(text: &str, at: (usize, usize)) -> State {
        {
            let mut fonts = iced::advanced::graphics::text::font_system().write().unwrap();
            for bytes in theme::FONT_FILES {
                fonts.load_font(std::borrow::Cow::Borrowed(bytes));
            }
        }
        let mut content = Content::with_text(text);
        content.move_to(Cursor { position: Position { line: at.0, column: at.1 }, selection: None });
        let mut state = State { viewport: 600.0, ..State::default() };
        state.relayout(&content, next_revision(), 600.0, Tokens::LIGHT, &[]);
        state
    }

    /// What a layout decided for each line, to compare two layouts by.
    fn decided(state: &State) -> Vec<(String, Line, live::Carry, f32, f32)> {
        state.lines.iter().map(|l| (l.text.clone(), l.line.clone(), l.carry.clone(), l.top, l.height)).collect()
    }

    #[test]
    fn moving_the_cursor_lays_out_as_a_fresh_layout_would() {
        let text = "# Title\n**bold** and [[Link|alias]]\n> [!note] Callout\n> body `code`\n\n- [ ] task\n```\n# code\n```\nlast";
        let mut state = laid(text, (0, 0));
        let mut content = Content::with_text(text);
        let revision = next_revision();
        let at = |line, column| Position { line, column };
        let moves = [
            Cursor { position: at(1, 3), selection: None },
            Cursor { position: at(3, 8), selection: None },
            Cursor { position: at(5, 2), selection: Some(at(0, 1)) },
            Cursor { position: at(7, 0), selection: None },
            Cursor { position: at(9, 2), selection: None },
            Cursor { position: at(0, 0), selection: None },
        ];
        for cursor in moves {
            content.move_to(cursor);
            state.relayout(&content, revision, 600.0, Tokens::LIGHT, &[]);
            let mut fresh = State { viewport: 600.0, ..State::default() };
            fresh.relayout(&content, next_revision(), 600.0, Tokens::LIGHT, &[]);
            assert_eq!(decided(&state), decided(&fresh), "{cursor:?}");
        }
    }

    #[test]
    fn a_new_revision_lays_out_the_new_text() {
        let mut state = laid("one\ntwo", (0, 0));
        let mut content = Content::with_text("one\n**three**\ntwo");
        content.move_to(Cursor { position: Position { line: 0, column: 0 }, selection: None });
        state.relayout(&content, next_revision(), 600.0, Tokens::LIGHT, &[]);
        let texts: Vec<&str> = state.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["one", "**three**", "two"]);
        let (shown, _) = live::shown(&state.lines[1].line, &state.lines[1].text);
        assert_eq!(shown, "three", "away from the cursor, the stars are hidden");
    }

    fn middle(state: &State, at: Position) -> (f32, f32) {
        let (x, y, h) = state.caret_point(at).unwrap();
        (x + 0.5, y + h / 2.0)
    }

    #[test]
    fn a_click_lands_on_the_column_as_written() {
        // Line 0 away from the cursor, so its stars and brackets are hidden.
        let text = "a **bold** and [[Note|alias]] end\n\nnext";
        let state = laid(text, (2, 0));
        let line = text.lines().next().unwrap();
        for word in ["bold", "and", "end"] {
            let column = line.find(word).unwrap();
            let (x, y) = middle(&state, Position { line: 0, column });
            match state.hit(x, y) {
                Some(Hit::Text(p)) => assert_eq!(p, Position { line: 0, column }, "{word}"),
                _ => panic!("{word}: not text"),
            }
        }
        let alias = line.find("alias").unwrap();
        let (x, y) = middle(&state, Position { line: 0, column: alias + 1 });
        assert!(matches!(state.hit(x, y), Some(Hit::Link(0, range)) if &line[range.clone()] == "[[Note|alias]]"));
    }

    #[test]
    fn a_click_on_a_checkbox_toggles_its_task() {
        let state = laid("- [ ] task\n\n", (2, 0));
        let (x, y) = middle(&state, Position { line: 0, column: 0 });
        assert!(matches!(state.hit(x + 4.0, y), Some(Hit::Checkbox(0))));
        let (x, y) = middle(&state, Position { line: 0, column: 8 });
        assert!(matches!(state.hit(x, y), Some(Hit::Text(_))));
    }

    #[test]
    fn up_and_down_step_one_line_at_a_time_past_headings() {
        let text = "first\n## Heading\nlast";
        let mut state = laid(text, (2, 2));
        let up = state.vertical(Position { line: 2, column: 2 }, true, false).unwrap();
        assert_eq!(up.line, 1, "onto the heading");
        // The heading shows its `##` now that the cursor is on it.
        let mut on_heading = laid(text, (1, up.column));
        let up = on_heading.vertical(up, true, false).unwrap();
        assert_eq!(up.line, 0, "past the room above the heading");
        let top = Position { line: 0, column: 3 };
        assert_eq!(on_heading.vertical(top, true, false), Some(Position { line: 0, column: 0 }));
        let down = state.vertical(Position { line: 2, column: 2 }, false, false).unwrap();
        assert_eq!(down, Position { line: 2, column: 4 }, "the last line goes to its end");
    }

    #[test]
    fn down_leaves_a_bullet_that_starts_with_code() {
        // From the sample vault's Kanban page, where Down stopped at the end
        // of the first bullet.
        let text = "\n- `columns` names the columns, at least two.\n- Each column's slug comes from its name.\n- last";
        for column in [0, 2, 12] {
            let mut state = laid(text, (1, column));
            let text_h = state.lines[1].text_h;
            let runs: Vec<f32> = state.lines[1].paragraph.buffer().layout_runs().map(|r| r.line_height).collect();
            let down = state.vertical(Position { line: 1, column }, false, false).unwrap();
            assert_eq!(down.line, 2, "from column {column}: text {text_h} runs {runs:?}");
            assert!(runs.iter().all(|h| (h - text_h).abs() < 0.5), "code keeps the line's height: {runs:?} vs {text_h}");
        }
    }
}
