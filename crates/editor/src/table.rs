//! Markdown tables as data, for the table widget of live preview: the lines
//! of a pipe table parsed into cells, edited by cell, row and column, and
//! written back as lines. Pure functions over `&str`, with no knowledge of
//! the editor.
//!
//! A cell's text is held unescaped: `\|` in the file is `|` here, and is
//! escaped again on writing. A table is written in its plainest form, one
//! space inside each pipe, so editing one cell does not move any other.

/// How a column's text lines up, from the colons of the delimiter row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The header row first, then the body rows; each as wide as the header.
    pub rows: Vec<Vec<String>>,
    pub align: Vec<Align>,
}

/// How many lines the table that starts at `lines[0]` spans: the header, the
/// delimiter row, and the rows after it that have a pipe. `None` if it does
/// not start a table.
pub fn extent(lines: &[String]) -> Option<usize> {
    let first = lines.first()?;
    if !first.starts_with('|') || !lines.get(1).is_some_and(|l| is_delimiter(l)) {
        return None;
    }
    Some(2 + lines[2..].iter().take_while(|l| !l.trim().is_empty() && l.contains('|')).count())
}

/// The table in `lines`, which must be exactly its lines.
pub fn parse(lines: &[String]) -> Option<Table> {
    let delimiter = lines.get(1).filter(|l| is_delimiter(l))?;
    let mut rows: Vec<Vec<String>> = std::iter::once(&lines[0]).chain(&lines[2..]).map(|l| split(l)).collect();
    // A row longer than the header widens the table rather than losing the
    // extra cells, since the table is written back from these rows.
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    rows.iter_mut().for_each(|r| r.resize(width, String::new()));
    let mut align: Vec<Align> = split_raw(delimiter).iter().map(|c| align_of(c)).collect();
    align.resize(width, Align::None);
    Some(Table { rows, align })
}

impl Table {
    pub fn columns(&self) -> usize {
        self.rows.first().map_or(0, Vec::len)
    }

    pub fn set(&mut self, row: usize, column: usize, text: &str) {
        if let Some(cell) = self.rows.get_mut(row).and_then(|r| r.get_mut(column)) {
            *cell = text.replace(['\r', '\n'], " ");
        }
    }

    /// An empty body row after row `row`, which may be the header.
    pub fn add_row(&mut self, row: usize) {
        let at = (row + 1).min(self.rows.len());
        self.rows.insert(at, vec![String::new(); self.columns()]);
    }

    /// Removes body row `row`; the header stays.
    pub fn remove_row(&mut self, row: usize) {
        if row >= 1 && row < self.rows.len() {
            self.rows.remove(row);
        }
    }

    /// An empty column after column `column`.
    pub fn add_column(&mut self, column: usize) {
        let at = (column + 1).min(self.columns());
        self.rows.iter_mut().for_each(|r| r.insert(at, String::new()));
        self.align.insert(at.min(self.align.len()), Align::None);
    }

    /// Removes column `column`; the last column stays.
    pub fn remove_column(&mut self, column: usize) {
        if self.columns() > 1 && column < self.columns() {
            self.rows.iter_mut().for_each(|r| {
                r.remove(column);
            });
            if column < self.align.len() {
                self.align.remove(column);
            }
        }
    }

    pub fn write(&self) -> Vec<String> {
        let line = |cells: Vec<String>| format!("|{}|", cells.join("|"));
        let cell = |c: &String| if c.is_empty() { "  ".to_owned() } else { format!(" {} ", c.replace('|', "\\|")) };
        let mark = |a: &Align| match a {
            Align::None => " --- ",
            Align::Left => " :-- ",
            Align::Center => " :-: ",
            Align::Right => " --: ",
        };
        let mut out = Vec::with_capacity(self.rows.len() + 1);
        for (i, cells) in self.rows.iter().enumerate() {
            out.push(line(cells.iter().map(cell).collect()));
            if i == 0 {
                let aligns = (0..self.columns()).map(|c| self.align.get(c).copied().unwrap_or(Align::None));
                out.push(line(aligns.map(|a| mark(&a).to_owned()).collect()));
            }
        }
        out
    }
}

fn is_delimiter(line: &str) -> bool {
    let cells = split_raw(line);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let dashes = c.trim_start_matches(':').trim_end_matches(':');
            !dashes.is_empty() && dashes.chars().all(|ch| ch == '-')
        })
}

fn align_of(cell: &str) -> Align {
    match (cell.starts_with(':'), cell.ends_with(':')) {
        (true, true) => Align::Center,
        (true, false) => Align::Left,
        (false, true) => Align::Right,
        _ => Align::None,
    }
}

/// The trimmed cells of a row, escapes still in them, without the outer pipes.
fn split_raw(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    // A trailing `\|` is an escaped pipe in the last cell, not a closing one.
    let line = if line.ends_with('|') && !line.ends_with("\\|") { &line[..line.len() - 1] } else { line };
    let mut cells = vec![String::new()];
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cells.last_mut().unwrap().push_str("\\|");
                chars.next();
            }
            '|' => cells.push(String::new()),
            c => cells.last_mut().unwrap().push(c),
        }
    }
    cells.into_iter().map(|c| c.trim().to_owned()).collect()
}

fn split(line: &str) -> Vec<String> {
    split_raw(line).into_iter().map(|c| c.replace("\\|", "|")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    #[test]
    fn a_table_round_trips_in_its_plainest_form() {
        let src = lines("| Name | Qty |\n|:--|--:|\n| a \\| b | 2 |\n| c |");
        assert_eq!(extent(&src), Some(4));
        let table = parse(&src).unwrap();
        assert_eq!(table.rows[1], ["a | b", "2"]);
        assert_eq!(table.rows[2], ["c", ""], "a short row is filled up");
        assert_eq!(table.write(), ["| Name | Qty |", "| :-- | --: |", "| a \\| b | 2 |", "| c |  |"]);
        assert_eq!(parse(&table.write()).unwrap(), table);
    }

    #[test]
    fn rows_and_columns_are_added_and_removed() {
        let mut table = parse(&lines("| a | b |\n| --- | --- |\n| 1 | 2 |")).unwrap();
        table.add_row(0);
        table.add_column(0);
        table.set(2, 1, "x");
        assert_eq!(table.write(), ["| a |  | b |", "| --- | --- | --- |", "|  |  |  |", "| 1 | x | 2 |"]);
        table.remove_row(0);
        table.remove_column(1);
        assert_eq!(table.write(), ["| a | b |", "| --- | --- |", "|  |  |", "| 1 | 2 |"]);
        table.remove_column(0);
        table.remove_column(0);
        assert_eq!(table.columns(), 1, "the last column stays");
    }

    #[test]
    fn a_row_wider_than_the_header_widens_the_table_and_keeps_its_cells() {
        let table = parse(&lines("| a | b |\n|---|---|\n| 1 | 2 | 3 |")).unwrap();
        assert_eq!(table.rows, [vec!["a", "b", ""], vec!["1", "2", "3"]]);
        assert_eq!(table.write(), ["| a | b |  |", "| --- | --- | --- |", "| 1 | 2 | 3 |"]);
    }

    #[test]
    fn extent_stops_at_the_first_line_without_a_pipe() {
        assert_eq!(extent(&lines("| a |\n| - |\n| 1 |\n\n| 2 |")), Some(3));
        assert_eq!(extent(&lines("| a |\nnot a delimiter")), None);
    }
}
