//! The timeline under a project or a person: the dated, kind-tagged log lines
//! in the note itself and in any other note that links to it, newest first.
//!
//! Read from `tags` and `links` plus the line's text in `texts`, so it needs
//! no table of its own. Clicking an entry opens the line it came from. When
//! there are more tagged lines than are read, the most recently changed notes
//! and their last lines are read first. That text makes `load` and `since`
//! too slow for the UI thread: the app calls them on a blocking task, with a
//! connection of their own.

use crate::theme::{self, Tokens};
use crate::{file_stem, plain_links, Message};
use iced::widget::{button, column, container, row, scrollable, text, Column, Row};
use iced::{Element, Fill};
use rusqlite::Connection;
use sage_editor::log::{date_in_name, parse_entry, KINDS};

/// Entries beyond this are not read.
const LIMIT: usize = 200;

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: String,
    pub line: usize,
    pub date: String,
    pub kind: String,
    pub text: String,
}

/// The entries that belong to the note at `path`, newest first. Those with
/// no date of their own take the date their note's name starts with.
pub fn load(db: &Connection, path: &str) -> rusqlite::Result<Vec<Entry>> {
    let kinds = KINDS.map(|k| format!("'{k}'")).join(", ");
    let mut stmt = db.prepare_cached(&format!(
        "SELECT DISTINCT f.id, f.path, g.line FROM tags g
         JOIN files f ON f.id = g.file_id
         JOIN files d ON d.path = ?1
         WHERE g.tag IN ({kinds})
           AND (g.file_id = d.id
                OR EXISTS (SELECT 1 FROM links l WHERE l.src_file_id = g.file_id AND l.line = g.line AND l.dst_file_id = d.id))
         ORDER BY f.mtime_ns DESC, f.path DESC, g.line DESC
         LIMIT 2000"
    ))?;
    let found = stmt.query_map([path], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
    let mut out = read(db, found);
    out.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.path.cmp(&b.path)).then(a.line.cmp(&b.line)));
    out.truncate(LIMIT);
    Ok(out)
}

/// Every entry in the vault dated from or later, for the weekly review.
pub fn since(db: &Connection, from: &str) -> rusqlite::Result<Vec<Entry>> {
    let kinds = KINDS.map(|k| format!("'{k}'")).join(", ");
    let mut stmt = db.prepare_cached(&format!(
        "SELECT DISTINCT f.id, f.path, g.line FROM tags g JOIN files f ON f.id = g.file_id
         WHERE g.tag IN ({kinds})
         ORDER BY f.mtime_ns DESC, f.path DESC, g.line DESC
         LIMIT 5000"
    ))?;
    let found = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
    let mut out: Vec<Entry> = read(db, found).into_iter().filter(|e| e.date.as_str() >= from).collect();
    out.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.path.cmp(&b.path)).then(a.line.cmp(&b.line)));
    Ok(out)
}

/// The entries on the given lines, read from the indexed text of their notes.
/// `found` comes a note at a time, so only the last note's text is kept.
fn read(db: &Connection, found: Vec<(i64, String, i64)>) -> Vec<Entry> {
    let mut stmt = db.prepare_cached("SELECT text FROM texts WHERE file_id = ?1").ok();
    let mut current: Option<(i64, String)> = None;
    let mut out = Vec::new();
    for (id, file, line) in found {
        if current.as_ref().is_none_or(|(at, _)| *at != id) {
            let text: Option<String> = stmt.as_mut().and_then(|s| s.query_row([id], |r| r.get(0)).ok());
            current = Some((id, text.unwrap_or_default()));
        }
        let text = current.as_ref().map_or("", |(_, text)| text.as_str());
        let Some(entry) = text.lines().nth(line as usize).and_then(parse_entry) else { continue };
        let date = if entry.date.is_empty() { date_in_name(&file).unwrap_or_default() } else { entry.date };
        out.push(Entry { path: file, line: line as usize, date, kind: entry.kind, text: plain_links(&entry.text) });
    }
    out
}

/// The panel: kind chips, then the entries of that kind.
pub fn panel<'a>(rows: &'a [Entry], kind: Option<&'static str>, today: &str, t: Tokens) -> Element<'a, Message> {
    let last = rows.first().filter(|r| !r.date.is_empty()).map(|r| match sage_editor::log::days_between(&r.date, today) {
        Some(0) => "last today".to_owned(),
        Some(d) if d > 0 => format!("last {d}d ago"),
        _ => format!("last {}", r.date),
    });
    let chip = |label: &'static str, value: Option<&'static str>| {
        button(text(label).size(12).color(if kind == value { t.text } else { t.text_2 }))
            .padding([2, 8])
            .style(move |_, status| theme::nav_button(t, kind == value, status))
            .on_press(Message::TimelineKind(value))
    };
    let mut chips = Row::new().spacing(4).align_y(iced::Center).push(text("Timeline").size(13).color(t.text_2));
    if let Some(last) = last {
        chips = chips.push(text(format!("· {last}")).size(12).color(t.text_2));
    }
    chips = chips.push(chip("All", None));
    for k in KINDS {
        chips = chips.push(chip(k, Some(k)));
    }
    let mut list = Column::new().spacing(2);
    for r in rows.iter().filter(|r| kind.is_none_or(|k| r.kind == k)) {
        list = list.push(
            row![
                text(&r.date).size(12).color(t.text_2).font(theme::MONO).width(80),
                text(&r.kind).size(12).color(t.text_2).width(64),
                button(text(&r.text).size(14).color(t.text))
                    .width(Fill)
                    .padding([2, 6])
                    .style(move |_, status| theme::nav_button(t, false, status))
                    .on_press(Message::OpenAt(r.path.clone(), r.line)),
                text(file_stem(&r.path)).size(12).color(t.text_2).wrapping(text::Wrapping::None),
            ]
            .spacing(8)
            .align_y(iced::Center),
        );
    }
    container(column![chips, scrollable(list).height(iced::Length::Shrink)].spacing(4))
        .padding([8, 16])
        .max_height(200)
        .width(Fill)
        .style(move |_| theme::surface(t))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed(files: &[(&str, &str)]) -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        for (rel, text) in files {
            let path = vault.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let db_path = dir.path().join("index.db");
        let mut ix = sage_indexer::Indexer::open(vault.canonicalize().unwrap(), &db_path).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        drop(ix);
        let db = sage_db::open_reader(&db_path).unwrap();
        (dir, db)
    }

    #[test]
    fn a_timeline_holds_own_and_linking_entries_newest_first() {
        let (_dir, db) = indexed(&[
            ("Projects/Alpha.md", "# Alpha\n## Log\n- 2026-11-10 #meeting Kick-off\n- 2026-11-20 #decision Ship Friday\n- plain #x\n"),
            ("Daily/2026-11-25.md", "- #email from [[Alpha]] about budget\n- #email unrelated\n"),
            ("People/Anna.md", "# Anna\n"),
        ]);
        let rows = load(&db, "Projects/Alpha.md").unwrap();
        let shown: Vec<(&str, &str, &str)> = rows.iter().map(|r| (r.date.as_str(), r.kind.as_str(), r.text.as_str())).collect();
        assert_eq!(
            shown,
            [
                ("2026-11-25", "email", "from Alpha about budget"),
                ("2026-11-20", "decision", "Ship Friday"),
                ("2026-11-10", "meeting", "Kick-off"),
            ]
        );
        assert!(load(&db, "People/Anna.md").unwrap().is_empty());
    }
}