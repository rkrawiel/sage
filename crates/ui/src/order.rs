//! The order of Kanban cards within their columns, set by dragging.
//!
//! The order is a preference of this app rather than part of the notes, so it
//! lives next to the index, never in the vault. Each card that was ever placed
//! by hand has a rank; a column shows its ranked cards first, by rank, then
//! the others in the board's order by due date and priority. A card keeps its
//! rank when it moves to another column or another board.

use sage_query::{Board, TaskRow};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Order {
    file: PathBuf,
    ranks: HashMap<String, f64>,
}

/// What identifies a card across moves: its note and its text, without the
/// `#status/` tag a move rewrites. The text of a [`TaskRow`] has no dates.
pub fn card_key(path: &str, text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().filter(|w| !is_status_tag(w)).collect();
    format!("{path}\n{}", words.join(" "))
}

/// Whether a word is a `#status/` tag, in any case. Compared without making
/// the word lowercase, as every word of every card is asked on every frame.
pub fn is_status_tag(word: &str) -> bool {
    word.get(..8).is_some_and(|start| start.eq_ignore_ascii_case("#status/"))
}

pub fn row_key(row: &TaskRow) -> String {
    card_key(&row.path, &row.text)
}

impl Order {
    /// The order kept in `file`; empty if it cannot be read.
    pub fn load(file: PathBuf) -> Self {
        let ranks = std::fs::read(&file).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        Self { file, ranks }
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string(&self.ranks) {
            crate::store::write_later(self.file.clone(), json);
        }
    }

    /// Sorts every column of `board`: ranked cards first, by rank, the
    /// others after them in the order they came.
    pub fn apply(&self, board: &mut Board) {
        for column in &mut board.columns {
            // Each card's rank is looked up once, not on every comparison.
            let mut ranked: Vec<(Option<f64>, TaskRow)> =
                std::mem::take(&mut column.cards).into_iter().map(|card| (self.ranks.get(&row_key(&card)).copied(), card)).collect();
            // A stable sort keeps the board's order among unranked cards.
            ranked.sort_by(|(a, _), (b, _)| match (a, b) {
                (Some(a), Some(b)) => a.total_cmp(b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });
            column.cards = ranked.into_iter().map(|(_, card)| card).collect();
        }
    }

    /// Places card `key` in a column showing the cards `shown`, in order,
    /// before the card `before`, or last. The cards shown get ranks too, so
    /// the column stays as it looks.
    pub fn place(&mut self, shown: &[String], key: &str, before: Option<&str>) {
        let mut keys: Vec<&str> = shown.iter().map(String::as_str).filter(|k| *k != key).collect();
        // Unranked cards come after the ranked ones; rank them in that order.
        let mut last = keys.iter().filter_map(|k| self.ranks.get(*k)).copied().fold(0.0, f64::max);
        for k in &keys {
            if !self.ranks.contains_key(*k) {
                last += 1.0;
                self.ranks.insert((*k).to_owned(), last);
            }
        }
        let at = before.and_then(|b| keys.iter().position(|k| *k == b)).unwrap_or(keys.len());
        let prev = at.checked_sub(1).map(|i| self.ranks[keys[i]]);
        let next = keys.get(at).map(|k| self.ranks[*k]);
        let rank = match (prev, next) {
            (None, None) => 0.0,
            (Some(p), None) => p + 1.0,
            (None, Some(n)) => n - 1.0,
            (Some(p), Some(n)) if n - p > 1e-9 => (p + n) / 2.0,
            // Halved too often: number the column afresh.
            _ => {
                keys.insert(at, key);
                for (i, k) in keys.iter().enumerate() {
                    self.ranks.insert((*k).to_owned(), i as f64);
                }
                return self.save();
            }
        };
        self.ranks.insert(key.to_owned(), rank);
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_query::BoardColumn;

    fn card(text: &str) -> TaskRow {
        TaskRow {
            path: "n.md".into(),
            note: "n".into(),
            line: 0,
            raw: String::new(),
            text: text.into(),
            done: false,
            status: None,
            due: None,
            scheduled: None,
            start: None,
            done_on: None,
            priority: None,
            document: false,
        }
    }

    #[test]
    fn status_tags_are_found_in_any_case() {
        for word in ["#status/doing", "#Status/Done", "#STATUS/x", "#status/"] {
            assert!(is_status_tag(word), "{word}");
        }
        for word in ["#status", "status/doing", "#statusx/doing", "#stätus/a", "", "#tag"] {
            assert!(!is_status_tag(word), "{word}");
        }
        assert_eq!(card_key("n.md", "Ship it #Status/Doing now"), "n.md\nShip it now");
    }

    fn texts(board: &Board) -> Vec<&str> {
        board.columns[0].cards.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn hand_order_comes_first_and_survives_moves() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("kanban.json");
        let mut board = Board {
            columns: vec![BoardColumn { name: "Todo".into(), slug: "todo".into(), cards: ["a", "b", "c", "d"].map(card).to_vec() }],
            truncated: false,
        };
        let key = |text: &str| card_key("n.md", text);
        let shown = |board: &Board| board.columns[0].cards.iter().map(row_key).collect::<Vec<_>>();

        let mut order = Order::load(file.clone());
        order.place(&shown(&board), &key("d"), Some(&key("b")));
        order.apply(&mut board);
        assert_eq!(texts(&board), ["a", "d", "b", "c"]);
        order.place(&shown(&board), &key("a"), None);
        order.apply(&mut board);
        assert_eq!(texts(&board), ["d", "b", "c", "a"]);

        // A card that has not been placed goes after the placed ones; a card
        // keeps its place when a move changes its status tag; and the order
        // is read back from the file.
        board.columns[0].cards.insert(0, card("new"));
        board.columns[0].cards[1].text = "d #status/doing".into();
        crate::store::flush();
        let mut order = Order::load(file);
        order.apply(&mut board);
        assert_eq!(texts(&board), ["d #status/doing", "b", "c", "a", "new"]);

        // Moving the last card to second place again and again halves the
        // same gap each time, until the column is numbered afresh.
        let mut expected: Vec<String> = texts(&board).iter().map(|s| s.to_string()).collect();
        for _ in 0..80 {
            let cards = &board.columns[0].cards;
            let (last, second) = (row_key(&cards[cards.len() - 1]), row_key(&cards[1]));
            order.place(&shown(&board), &last, Some(&second));
            order.apply(&mut board);
            let moved = expected.pop().unwrap();
            expected.insert(1, moved);
            assert_eq!(texts(&board), expected);
        }
    }
}
