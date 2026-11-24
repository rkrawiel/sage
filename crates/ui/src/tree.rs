//! The collapsible folder tree in the sidebar.
//!
//! Built from the flat `v_notes` rows on every view. Folders start collapsed,
//! and only what shows is built and sorted: a note in a closed folder costs a
//! lookup of its top folder, which keeps a 10,000-note vault cheap to draw.
//! Opening a note expands its ancestors. Notes whose content is still in OneDrive's cloud are marked. A
//! right click on a note or folder opens a menu to rename or delete it.
//!
//! Above it, a row of buttons: new note, new folder, the order of the tree,
//! and collapse or expand all folders.

use crate::theme::{self, Tokens};
use crate::{FileRow, Message};
use iced::widget::{button, column, container, mouse_area, row, space, svg, text, tooltip, Column};
use iced::{Element, Fill};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// How notes are ordered within a folder. Folders come first, by name.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Order {
    #[default]
    NameAscending,
    NameDescending,
    ModifiedNewest,
    ModifiedOldest,
}

impl Order {
    pub const ALL: [Order; 4] = [Order::NameAscending, Order::NameDescending, Order::ModifiedNewest, Order::ModifiedOldest];

    fn label(self) -> &'static str {
        match self {
            Order::NameAscending => "File name (A to Z)",
            Order::NameDescending => "File name (Z to A)",
            Order::ModifiedNewest => "Modified (newest first)",
            Order::ModifiedOldest => "Modified (oldest first)",
        }
    }
}

#[derive(Default)]
struct Node<'a> {
    /// Keyed by lowercase name for case-insensitive ordering.
    folders: BTreeMap<String, (&'a str, Node<'a>)>,
    files: Vec<&'a FileRow>,
}

const INDENT: f32 = 14.0;

/// What the tree shows besides the files: open folders and the current note.
struct Show<'s> {
    expanded: &'s HashSet<String>,
    order: Order,
    current: Option<&'s str>,
    t: Tokens,
}

/// The tree of `files`, with `folders` that hold no note shown as well.
pub fn view<'a>(
    files: &'a [FileRow],
    folders: &'a [String],
    expanded: &HashSet<String>,
    order: Order,
    current: Option<&str>,
    t: Tokens,
) -> Element<'a, Message> {
    let mut root = Node::default();
    for folder in folders {
        open_node(&mut root, folder, expanded);
    }
    for file in files {
        if let Some(node) = open_node(&mut root, &file.folder, expanded) {
            node.files.push(file);
        }
    }
    sort_files(&mut root, order);

    let mut rows = Vec::new();
    push_node(&mut rows, &root, "", 0, &Show { expanded, order, current, t });
    Column::with_children(rows).spacing(1).into()
}

/// The node for `folder` when it shows, that is when every folder down to it
/// is open, else `None`. The nodes on the way are made down to the first
/// closed folder, which shows as a row; nothing inside it is built.
fn open_node<'n, 'a>(root: &'n mut Node<'a>, folder: &'a str, expanded: &HashSet<String>) -> Option<&'n mut Node<'a>> {
    let mut node = root;
    if folder.is_empty() {
        return Some(node);
    }
    let mut end = 0;
    for part in folder.split('/') {
        end += part.len();
        node = &mut node.folders.entry(part.to_lowercase()).or_insert_with(|| (part, Node::default())).1;
        if !expanded.contains(&folder[..end]) {
            return None;
        }
        end += 1;
    }
    Some(node)
}

/// The buttons above the tree, and the order choices while they are open.
pub fn toolbar<'a>(order: Order, choosing: bool, any_expanded: bool, t: Tokens) -> Element<'a, Message> {
    let (fold_icon, fold_tip) = if any_expanded { (COLLAPSE, "Collapse all") } else { (EXPAND, "Expand all") };
    let buttons = row![
        icon_button(NEW_NOTE, "New note (Ctrl+N)", Message::TreeNewNote, false, t),
        icon_button(NEW_FOLDER, "New folder", Message::TreeNewFolder, false, t),
        space().width(Fill),
        icon_button(SORT, "Change sort order", Message::TreeOrderMenu, choosing, t),
        icon_button(fold_icon, fold_tip, Message::TreeFoldAll, false, t),
    ]
    .spacing(2)
    .align_y(iced::Center);
    if !choosing {
        return container(buttons).padding([0, 8]).into();
    }
    let choices = Order::ALL.into_iter().map(|choice| {
        let chosen = choice == order;
        button(
            row![text(if chosen { "✓" } else { "" }).size(13).width(14), text(choice.label()).size(13)]
                .spacing(4)
                .align_y(iced::Center),
        )
        .width(Fill)
        .padding([3, 8])
        .style(move |_, status| theme::nav_button(t, chosen, status))
        .on_press(Message::TreeOrder(choice))
        .into()
    });
    column![buttons, container(Column::with_children(choices).spacing(1)).padding([4, 0])].padding([0, 8]).into()
}

fn icon_button<'a>(icon: &'static [u8], tip: &'a str, message: Message, active: bool, t: Tokens) -> Element<'a, Message> {
    let glyph = svg(svg::Handle::from_memory(icon))
        .width(16)
        .height(16)
        .style(move |_, _| svg::Style { color: Some(t.text_2) });
    tooltip(
        button(glyph).padding(5).style(move |_, status| theme::nav_button(t, active, status)).on_press(message),
        container(text(tip).size(12)).padding([4, 8]).style(move |_| theme::panel(t)),
        tooltip::Position::Bottom,
    )
    .into()
}

// 16 px line icons, tinted with the secondary text colour.
const NEW_NOTE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M9.5 1.5H4a1 1 0 0 0-1 1v11a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V5z"/><path d="M9.5 1.5V5H13"/><path d="M8 7.5v5M5.5 10h5"/></svg>"#;
const NEW_FOLDER: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M1.5 4a1 1 0 0 1 1-1h3.5l1.5 1.5h6a1 1 0 0 1 1 1V12.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z"/><path d="M8 7v4.5M5.75 9.25h4.5"/></svg>"#;
const SORT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 2.5v11M2 11l2.5 2.5L7 11"/><path d="M11.5 13.5v-11M9 5l2.5-2.5L14 5"/></svg>"#;
const COLLAPSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 2.5 8 6l3.5-3.5"/><path d="M4.5 13.5 8 10l3.5 3.5"/></svg>"#;
const EXPAND: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 5.5 8 2l3.5 3.5"/><path d="M4.5 10.5 8 14l3.5-3.5"/></svg>"#;

fn push_node<'a>(rows: &mut Vec<Element<'a, Message>>, node: &Node<'a>, prefix: &str, depth: usize, show: &Show) {
    let t = show.t;
    let indent = INDENT * depth as f32;

    let folders: Vec<_> = if show.order == Order::NameDescending {
        node.folders.values().rev().collect()
    } else {
        node.folders.values().collect()
    };
    for (name, child) in folders {
        let path = if prefix.is_empty() { (*name).to_owned() } else { format!("{prefix}/{name}") };
        let open = show.expanded.contains(&path);
        let entry = button(
            row![
                text(if open { "▾" } else { "▸" }).size(16).color(t.text_2).width(12),
                text(*name).size(14).font(theme::SANS_BOLD).wrapping(text::Wrapping::None),
            ]
            .spacing(6)
            .align_y(iced::Center),
        )
        .width(Fill)
        .padding(iced::Padding { top: 4.0, bottom: 4.0, left: 8.0 + indent, right: 8.0 })
        .style(move |_, status| theme::nav_button(t, false, status))
        .on_press(Message::ToggleFolder(path.clone()));
        rows.push(mouse_area(entry).on_right_press(Message::TreeMenu(path.clone(), true)).into());

        if open {
            push_node(rows, child, &path, depth + 1, show);
        }
    }

    for file in &node.files {
        let is_current = show.current == Some(file.path.as_str());
        // A OneDrive placeholder recedes; opening it downloads it.
        let label = if file.cloud {
            text(format!("{} ☁", file.title)).color(t.text_2)
        } else {
            text(&file.title)
        };
        let entry = button(label.size(14).wrapping(text::Wrapping::None))
            .width(Fill)
            // Files line up with folder names, past the chevron.
            .padding(iced::Padding { top: 4.0, bottom: 4.0, left: 8.0 + indent + 18.0, right: 8.0 })
            .style(move |_, status| theme::nav_button(t, is_current, status))
            .on_press(Message::Open(file.path.clone()));
        rows.push(mouse_area(entry).on_right_press(Message::TreeMenu(file.path.clone(), false)).into());
    }
}

fn sort_files(node: &mut Node<'_>, order: Order) {
    // Keys made once per note, not once per comparison.
    let name = |f: &FileRow| f.title.to_lowercase();
    match order {
        Order::NameAscending => node.files.sort_by_cached_key(|f| name(f)),
        Order::NameDescending => node.files.sort_by_cached_key(|f| std::cmp::Reverse(name(f))),
        Order::ModifiedNewest => node.files.sort_by_cached_key(|f| (std::cmp::Reverse(f.modified), name(f))),
        Order::ModifiedOldest => node.files.sort_by_cached_key(|f| (f.modified, name(f))),
    }
    for (_, child) in node.folders.values_mut() {
        sort_files(child, order);
    }
}

/// The vault's folders on disk, hidden ones and what is in them left out, so
/// a folder with no note in it still shows in the tree.
pub fn disk_folders(vault: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![(vault.to_path_buf(), String::new())];
    while let Some((dir, rel)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            found.push(path.clone());
            pending.push((entry.path(), path));
        }
    }
    found.sort_by_key(|f| f.to_lowercase());
    found
}

/// Every ancestor folder of a vault-relative file path, outermost first.
pub fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').map(move |(i, _)| &path[..i])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str, modified: i64) -> FileRow {
        let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
        FileRow {
            path: path.into(),
            folder: folder.into(),
            title: name.trim_end_matches(".md").into(),
            cloud: false,
            modified,
        }
    }

    fn order_of(order: Order) -> Vec<&'static str> {
        let files = [row("b.md", 30), row("A.md", 10), row("c.md", 20)];
        let mut root = Node::default();
        for file in &files {
            open_node(&mut root, &file.folder, &HashSet::new()).unwrap().files.push(file);
        }
        sort_files(&mut root, order);
        root.files.iter().map(|f| match f.title.as_str() {
            "A" => "A",
            "b" => "b",
            _ => "c",
        })
        .collect()
    }

    #[test]
    fn notes_follow_the_chosen_order() {
        assert_eq!(order_of(Order::NameAscending), ["A", "b", "c"], "case does not matter");
        assert_eq!(order_of(Order::NameDescending), ["c", "b", "A"]);
        assert_eq!(order_of(Order::ModifiedNewest), ["b", "c", "A"]);
        assert_eq!(order_of(Order::ModifiedOldest), ["A", "c", "b"]);
    }

    #[test]
    fn empty_folders_on_disk_are_found_and_hidden_ones_are_not() {
        let dir = tempfile::tempdir().unwrap();
        for folder in ["Projects/Empty", "Daily", ".sage", ".trash/Old", "Projects/.hidden"] {
            std::fs::create_dir_all(dir.path().join(folder)).unwrap();
        }
        std::fs::write(dir.path().join("Note.md"), "").unwrap();
        assert_eq!(disk_folders(dir.path()), ["Daily", "Projects", "Projects/Empty"]);
    }

    #[test]
    fn empty_folders_get_a_place_in_the_tree() {
        let mut root = Node::default();
        let open = HashSet::from(["Projects".to_owned()]);
        assert!(open_node(&mut root, "Projects/Empty", &open).is_none(), "Empty itself is closed");
        let projects = &root.folders["projects"];
        assert_eq!(projects.0, "Projects");
        assert!(projects.1.folders.contains_key("empty"));
    }

    #[test]
    fn nothing_inside_a_closed_folder_is_built() {
        let mut root = Node::default();
        assert!(open_node(&mut root, "Projects/Deep/Deeper", &HashSet::new()).is_none());
        assert!(root.folders["projects"].1.folders.is_empty(), "the closed folder shows as a row only");
        let open = HashSet::from(["Projects".to_owned(), "Projects/Deep".to_owned(), "Projects/Deep/Deeper".to_owned()]);
        assert!(open_node(&mut root, "Projects/Deep/Deeper", &open).is_some());
    }
}
