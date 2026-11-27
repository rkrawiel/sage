//! The table widget of live preview: a Markdown pipe table drawn as a grid of
//! fields that edit the note in place, with buttons to add and remove rows
//! and columns, so a table never has to be seen as pipes. Each change writes
//! the table's lines back as one edit (`sage_editor::table`).

use crate::theme::{self, Tokens};
use crate::{Message, Tab};
use iced::widget::{button, column, container, row, space, text, text_input, Id};
use iced::{Element, Length};
use sage_editor::table::{self, Align, Table};
use std::sync::Arc;

/// A change to the table that starts on a line of the note.
#[derive(Debug, Clone)]
pub enum Edit {
    /// Row and column, and the text typed there.
    Cell(usize, usize, String),
    /// Enter pressed in a cell: on to the cell below, in a new row if it was the last.
    Enter(usize, usize),
    AddRow(usize),
    AddColumn(usize),
    RemoveRow(usize),
    RemoveColumn(usize),
}

/// The id of a cell's field, so a new cell can be given the focus.
pub fn cell_id(start: usize, row: usize, column: usize) -> Id {
    Id::from(format!("table-{start}-{row}-{column}"))
}

/// The text being typed in a cell: its table's first line, row, column and the text as
/// typed, which the file holds trimmed.
pub type Draft = (usize, usize, usize, String);

impl Tab {
    /// The table that starts on line `start`.
    pub(crate) fn table_at(&self, start: usize) -> Option<Table> {
        table::parse(&self.table_lines(start)?)
    }

    /// The lines of the table that starts on line `start`, exactly. Every
    /// frame asks for every table, so only its own lines are read, one at a
    /// time until one is past its end, rather than the whole note.
    fn table_lines(&self, start: usize) -> Option<Vec<String>> {
        let mut lines: Vec<String> = Vec::new();
        while let Some(line) = self.content.line(start + lines.len()) {
            lines.push(line.text.into_owned());
            if lines.len() >= 2 && table::extent(&lines).is_none_or(|count| count < lines.len()) {
                break;
            }
        }
        let count = table::extent(&lines)?;
        lines.truncate(count);
        Some(lines)
    }

    /// Changes the table that starts on line `start` as one edit that
    /// autosave then writes, keeping the cursor where it was.
    pub(crate) fn edit_table(&mut self, start: usize, typing: bool, change: impl FnOnce(&mut Table)) {
        let Some(rest) = self.table_lines(start) else { return };
        let count = rest.len();
        let Some(mut table) = table::parse(&rest) else { return };
        change(&mut table);
        let new = table.write();
        if new == rest {
            return;
        }
        let last = start + count - 1;
        let Some(end) = self.content.line(last).map(|l| l.text.len()) else { return };
        self.checkpoint(if typing { sage_editor::undo::Kind::Typing } else { sage_editor::undo::Kind::Other });
        let cursor = self.content.cursor().position;
        self.content.move_to(iced::widget::text_editor::Cursor {
            position: iced::widget::text_editor::Position { line: last, column: end },
            selection: Some(iced::widget::text_editor::Position { line: start, column: 0 }),
        });
        let text = self.with_ending(&new.join("\n"));
        self.content.perform(iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Paste(Arc::new(text))));
        let (line, column) = if cursor.line > last {
            ((cursor.line + new.len()).saturating_sub(count), cursor.column)
        } else if cursor.line >= start {
            (start, 0)
        } else {
            (cursor.line, cursor.column)
        };
        self.put_cursor(line, column);
        self.mark_edited();
    }
}

/// The table that starts on line `start`, as fields. `draft` is what is being
/// typed, which wins over the cell while it only differs by spaces at the ends.
pub fn editor<'a>(start: usize, table: &Table, draft: Option<&Draft>, t: Tokens) -> Element<'a, Message> {
    let columns = table.columns();
    let small = |label: &'a str, message: Message| {
        button(text(label).size(13))
            .padding([2, 8])
            .style(move |_, status| theme::nav_button(t, false, status))
            .on_press(message)
    };
    let gutter = Length::Fixed(26.0);
    let mut grid = column![].spacing(4);

    // Above each column, the button that removes it.
    if columns > 1 {
        let mut top = row![].spacing(4);
        for c in 0..columns {
            top = top.push(
                container(
                    button(text("×").size(12))
                        .padding([0, 6])
                        .style(move |_, status| theme::nav_button(t, false, status))
                        .on_press(Message::Table(start, Edit::RemoveColumn(c))),
                )
                .width(Length::FillPortion(1))
                .center_x(Length::Fill),
            );
        }
        grid = grid.push(row![top, space().width(gutter)].spacing(4));
    }

    for (r, cells) in table.rows.iter().enumerate() {
        let mut line = row![].spacing(4).align_y(iced::Center);
        for (c, cell) in cells.iter().enumerate() {
            let shown = match draft {
                Some((s, dr, dc, typed)) if (*s, *dr, *dc) == (start, r, c) && typed.trim() == cell => typed.as_str(),
                _ => cell.as_str(),
            };
            let align = match table.align.get(c) {
                Some(Align::Center) => iced::alignment::Horizontal::Center,
                Some(Align::Right) => iced::alignment::Horizontal::Right,
                _ => iced::alignment::Horizontal::Left,
            };
            let input = text_input("", shown)
                .id(cell_id(start, r, c))
                .on_input(move |s| Message::Table(start, Edit::Cell(r, c, s)))
                .on_submit(Message::Table(start, Edit::Enter(r, c)))
                .size(14)
                .padding([4, 8])
                .align_x(align)
                .font(if r == 0 { theme::SANS_BOLD } else { theme::SANS })
                .style(move |_, status| theme::chip_input(t, status));
            line = line.push(container(input).width(Length::FillPortion(1)).style(move |_| theme::chip(t, r == 0)));
        }
        let remove: Element<'a, Message> = if r == 0 {
            space().into()
        } else {
            button(text("×").size(13))
                .padding([0, 6])
                .style(move |_, status| theme::nav_button(t, false, status))
                .on_press(Message::Table(start, Edit::RemoveRow(r)))
                .into()
        };
        grid = grid.push(row![line, container(remove).width(gutter)].spacing(4).align_y(iced::Center));
    }

    let last = table.rows.len() - 1;
    grid.push(
        row![small("+ Row", Message::Table(start, Edit::AddRow(last))), small("+ Column", Message::Table(start, Edit::AddColumn(columns - 1)))]
            .spacing(6),
    )
    .into()
}
