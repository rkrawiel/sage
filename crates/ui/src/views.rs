//! `view` blocks in the preview: a table of notes, a task list, or a Kanban
//! board. The rows come from sage-query; this module only draws them and
//! turns clicks and drags into messages.

use crate::order;
use crate::preview::Ctx;
use crate::theme::{self, Tokens};
use crate::{plain_links, Message, TaskRef};
use sage_query::{Board, Outcome, Table, TaskRow, Tasks};
use iced::widget::{button, column, container, mouse_area, row, space, text, Column, Row};
use iced::{mouse, Border, Element, Fill, Length};

/// A Kanban card being dragged.
#[derive(Debug, Clone)]
pub struct Drag {
    /// The view block, by its source text.
    pub view: String,
    pub card: TaskRef,
    /// The card in the hand-placed order; see `order::card_key`.
    pub key: String,
    pub from: usize,
    /// The card's place in its column when the drag started.
    pub start: usize,
    /// The column under the pointer, while it is over this board.
    pub over: Option<usize>,
    /// The card under the pointer in that column, which the dragged card
    /// would go before; `None` for the end of the column.
    pub slot: Option<usize>,
    /// The pointer has been over another card or column, so releasing it
    /// back where it started is not a click.
    pub moved: bool,
}

pub fn view<'a>(source: &'a str, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.t;
    match ctx.views.get(source) {
        None => text("Running view…").size(13).color(t.text_2).into(),
        Some(Err(error)) => container(
            column![
                text("View").size(12).font(theme::SANS_BOLD).color(t.error),
                text(error).size(14).color(t.text),
            ]
            .spacing(4),
        )
        .padding([10, 14])
        .width(Fill)
        .style(move |_| theme::panel(t))
        .into(),
        Some(Ok(Outcome::Table(table))) => notes(table, t),
        Some(Ok(Outcome::Tasks(tasks))) => task_list(tasks, t),
        Some(Ok(Outcome::Board(board))) => kanban(source, board, ctx),
    }
}

/// Text columns get more room than the short date and name columns.
fn share(column: &str) -> Length {
    Length::FillPortion(if matches!(column, "task" | "title" | "path") { 3 } else { 1 })
}

fn header<'a>(columns: impl Iterator<Item = &'a str>, t: Tokens) -> Element<'a, Message> {
    Row::with_children(columns.map(|name| {
        let label = if name == "task" { "" } else { name };
        container(text(label).size(12).font(theme::SANS_BOLD).color(t.text_2)).width(share(name)).into()
    }))
    .spacing(16)
    .into()
}

fn truncated<'a>(shown: usize, t: Tokens) -> Element<'a, Message> {
    text(format!("Showing the first {shown}; raise `limit` to see more")).size(12).color(t.text_2).into()
}

fn notes(table: &Table, t: Tokens) -> Element<'_, Message> {
    let mut grid = Column::new().spacing(6).push(header(table.columns.iter().map(String::as_str), t)).push(crate::hline(t));
    for row in &table.rows {
        grid = grid.push(
            Row::with_children(table.columns.iter().zip(&row.cells).map(|(column, cell)| {
                let content: Element<'_, Message> = if column == "file" {
                    button(text(cell).size(15))
                        .padding(0)
                        .style(move |_, status| theme::link_button(t, status))
                        .on_press(Message::Open(row.path.clone()))
                        .into()
                } else {
                    text(cell).size(15).into()
                };
                container(content).width(share(column)).into()
            }))
            .spacing(16),
        );
    }
    if table.rows.is_empty() {
        grid = grid.push(text("No notes match").size(14).color(t.text_2));
    }
    if table.truncated {
        grid = grid.push(truncated(table.rows.len(), t));
    }
    grid.into()
}

/// What a row's checkbox or a card's move writes to: a line, or a task
/// document's `status` as the index read it.
fn task_ref(row: &TaskRow) -> TaskRef {
    if row.document {
        let status = row.status.clone().unwrap_or_default();
        TaskRef { path: row.path.clone(), line: 0, raw: status, document: true }
    } else {
        TaskRef { path: row.path.clone(), line: row.line, raw: row.raw.clone(), document: false }
    }
}

fn priority_name(priority: Option<u8>) -> &'static str {
    match priority {
        Some(5) => "highest",
        Some(4) => "high",
        Some(3) => "medium",
        Some(2) => "low",
        Some(1) => "lowest",
        _ => "",
    }
}

fn task_list(tasks: &Tasks, t: Tokens) -> Element<'_, Message> {
    let mut list = Column::new().spacing(4);
    if tasks.columns.len() > 1 {
        let names = tasks.columns.iter().map(String::as_str);
        list = list.push(row![space().width(28), header(names, t)].spacing(0)).push(crate::hline(t));
    }
    for task in &tasks.rows {
        let done = task.done;
        let cells = Row::with_children(tasks.columns.iter().map(|column| {
            let value = match column.as_str() {
                "task" => {
                    return container(
                        button(text(plain_links(&task.text)).size(15).color(if done { t.text_2 } else { t.text }))
                            .padding(0)
                            .style(move |_, status| theme::link_button(t, status))
                            .on_press(Message::OpenAt(task.path.clone(), task.line as usize)),
                    )
                    .width(share(column))
                    .into();
                }
                "file" => task.note.clone(),
                "path" => task.path.clone(),
                "due" => task.due.clone().unwrap_or_default(),
                "scheduled" => task.scheduled.clone().unwrap_or_default(),
                "start" => task.start.clone().unwrap_or_default(),
                "done" => task.done_on.clone().unwrap_or_default(),
                "priority" => priority_name(task.priority).to_owned(),
                "status" => task.status.clone().unwrap_or_default(),
                _ => String::new(),
            };
            container(text(value).size(13).color(t.text_2)).width(share(column)).into()
        }))
        .spacing(16)
        .align_y(iced::Center);
        list = list.push(
            row![
                button(container(space()).width(13).height(13).style(move |_| theme::checkbox(t, done)))
                    .padding([6, 6])
                    .style(move |_, status| theme::nav_button(t, false, status))
                    .on_press(Message::ViewTask(task_ref(task))),
                cells,
            ]
            .spacing(3)
            .align_y(iced::Center),
        );
    }
    if tasks.rows.is_empty() {
        list = list.push(text("No tasks match").size(14).color(t.text_2));
    }
    if tasks.truncated {
        list = list.push(truncated(tasks.rows.len(), t));
    }
    list.into()
}

/// The card text without the `#status/` tag, which the column already shows.
fn card_text(task: &TaskRow) -> String {
    plain_links(&task.text)
        .split_whitespace()
        .filter(|word| !order::is_status_tag(word))
        .collect::<Vec<_>>()
        .join(" ")
}

fn kanban<'a>(source: &'a str, board: &'a Board, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.t;
    let drag = ctx.drag.filter(|d| d.view == source);
    // Where a dropped card would land shows as a rule in the gap before a
    // card or at the end. The gaps are always there, so the cards never move
    // under the pointer while it looks for a place.
    let gap = move |on: bool| container(space()).height(2).width(Fill).style(move |_| if on { theme::fill(t.accent) } else { container::Style::default() });
    let columns = Row::with_children(board.columns.iter().enumerate().map(|(n, column)| {
        let target = |slot: Option<usize>| drag.is_some_and(|d| d.moved && d.over == Some(n) && d.slot == slot);
        let mut cards = Column::new().spacing(2).push(
            text(format!("{} · {}", column.name, column.cards.len())).size(12).font(theme::SANS_BOLD).color(t.text_2),
        );
        for (i, card) in column.cards.iter().enumerate() {
            let dragged = drag.is_some_and(|d| d.card.path == card.path && d.card.line == card.line);
            cards = cards.push(gap(target(Some(i)) && !dragged));
            let face = container(
                column![
                    text(card_text(card)).size(14).color(if dragged || card.done { t.text_2 } else { t.text }),
                    // A long note name wraps instead of running into the date.
                    row![
                        text(&card.note).size(11).color(t.text_2).width(Fill),
                        text(card.due.as_deref().unwrap_or_default()).size(11).color(t.text_2),
                    ]
                    .spacing(6),
                ]
                .spacing(4),
            )
            .padding([8, 10])
            .width(Fill)
            .style(move |_| theme::panel(t));
            cards = cards.push(
                mouse_area(face)
                    .interaction(mouse::Interaction::Grab)
                    .on_press(Message::Grab {
                        view: source.to_owned(),
                        card: task_ref(card),
                        key: order::row_key(card),
                        column: n,
                        index: i,
                    })
                    .on_enter(Message::DragCard(source.to_owned(), n, i))
                    .on_exit(Message::DragCardLeave(source.to_owned(), n, i)),
            );
        }
        cards = cards.push(gap(target(None)));
        let over = drag.is_some_and(|d| d.over == Some(n) && (d.moved || d.over != Some(d.from)));
        let lane = container(cards).padding(8).width(Fill).height(iced::Shrink).style(move |_| {
            container::Style {
                border: Border { color: if over { t.accent } else { t.border }, width: 1.0, radius: theme::RADIUS.into() },
                ..container::Style::default()
            }
        });
        mouse_area(lane)
            .on_enter(Message::DragOver(source.to_owned(), n))
            .on_exit(Message::DragLeave(source.to_owned(), n))
            .into()
    }))
    .spacing(10);
    let mut board_view = Column::new().spacing(6).push(columns);
    if board.truncated {
        let shown = board.columns.iter().map(|c| c.cards.len()).sum();
        board_view = board_view.push(truncated(shown, t));
    }
    board_view.into()
}

/// The column a card belongs in, by the board's rule: done in the last column,
/// otherwise the column with its status, otherwise the first.
pub fn column_for(board: &Board, done: bool, status: Option<&str>) -> usize {
    let last = board.columns.len().saturating_sub(1);
    if done {
        return last;
    }
    status.and_then(|s| board.columns.iter().position(|c| sage_query::same_status(&c.slug, s))).unwrap_or(0)
}

/// The task rows of a view: list rows or board cards.
pub fn task_rows(outcome: &Outcome) -> Box<dyn Iterator<Item = &TaskRow> + '_> {
    match outcome {
        Outcome::Table(_) => Box::new(std::iter::empty()),
        Outcome::Tasks(tasks) => Box::new(tasks.rows.iter()),
        Outcome::Board(board) => Box::new(board.columns.iter().flat_map(|c| c.cards.iter())),
    }
}

/// Updates the rows for one task line after the app rewrote it, so a view
/// shows the change before the index has caught up.
pub fn patch(outcome: &mut Outcome, path: &str, line: u32, raw: &str) {
    let Some(task) = sage_core::parse(raw).tasks.into_iter().next() else { return };
    let update = |row: &mut TaskRow| {
        row.raw = raw.to_owned();
        row.text = sage_core::task_display(&task.text);
        row.done = matches!(task.state, 'x' | 'X');
        row.status = task.status.clone();
        row.due = task.due.clone();
        row.scheduled = task.scheduled.clone();
        row.start = task.start.clone();
        row.done_on = task.done.clone();
        row.priority = task.priority;
    };
    patch_rows(outcome, |r| !r.document && r.path == path && r.line == line, update);
}

/// Updates the rows for a task document after the app set its `status`.
pub fn patch_document(outcome: &mut Outcome, path: &str, status: Option<&str>) {
    let update = |row: &mut TaskRow| {
        row.status = status.map(str::to_owned);
        row.done = status == Some(sage_core::TaskStatus::Done.slug());
    };
    patch_rows(outcome, |r| r.document && r.path == path, update);
}

/// Updates the rows `matches` picks; a board card goes to its new column.
fn patch_rows(outcome: &mut Outcome, matches: impl Fn(&TaskRow) -> bool, update: impl Fn(&mut TaskRow)) {
    match outcome {
        Outcome::Table(_) => {}
        Outcome::Tasks(tasks) => {
            for row in tasks.rows.iter_mut().filter(|r| matches(r)) {
                update(row);
            }
        }
        Outcome::Board(board) => {
            let mut moved = None;
            for column in &mut board.columns {
                if let Some(i) = column.cards.iter().position(&matches) {
                    moved = Some(column.cards.remove(i));
                }
            }
            if let Some(mut card) = moved {
                update(&mut card);
                let n = column_for(board, card.done, card.status.as_deref());
                board.columns[n].cards.push(card);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_query::BoardColumn;

    fn card(path: &str, document: bool, status: Option<&str>) -> TaskRow {
        TaskRow {
            path: path.into(),
            note: path.into(),
            line: if document { 0 } else { 3 },
            raw: if document { String::new() } else { "- [ ] Line".into() },
            text: path.into(),
            done: false,
            status: status.map(str::to_owned),
            due: None,
            scheduled: None,
            start: None,
            done_on: None,
            priority: None,
            document,
        }
    }

    #[test]
    fn a_document_card_follows_its_status() {
        let column = |name: &str, cards: Vec<TaskRow>| BoardColumn { name: name.into(), slug: sage_core::status_slug(name), cards };
        let mut outcome = Outcome::Board(Board {
            columns: vec![
                column("Todo", vec![card("a.md", true, Some("todo")), card("a.md", false, None)]),
                column("In Progress", vec![]),
                column("Done", vec![]),
            ],
            truncated: false,
        });
        let names = |outcome: &Outcome| -> Vec<Vec<(String, bool)>> {
            let Outcome::Board(board) = outcome else { panic!() };
            board.columns.iter().map(|c| c.cards.iter().map(|c| (c.path.clone(), c.document)).collect()).collect()
        };
        let row = task_ref(&card("a.md", true, Some("todo")));
        assert_eq!((row.raw.as_str(), row.document), ("todo", true), "a move checks the status");

        patch_document(&mut outcome, "a.md", Some("in-progress"));
        assert_eq!(names(&outcome), [vec![("a.md".into(), false)], vec![("a.md".into(), true)], vec![]], "the line stays");
        patch_document(&mut outcome, "a.md", Some("done"));
        assert_eq!(names(&outcome)[2], [("a.md".into(), true)]);
        let Outcome::Board(board) = &outcome else { panic!() };
        assert!(board.columns[2].cards[0].done);
    }
}
