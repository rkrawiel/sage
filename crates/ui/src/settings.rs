//! The settings panel, shown in place of the note: the templates folder and
//! the Quick Add entries. Changes are saved to the vault's
//! `.sage/settings.json` shortly after the last one.

use crate::theme::{self, Tokens};
use crate::Message;
use sage_editor::settings::{QuickAdd, Settings};
use iced::widget::{button, checkbox, container, pick_list, row, scrollable, space, text, text_input, Column};
use iced::{Element, Fill};

/// A choice in a drop-down: a vault path, or `""` for the default its label names.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub value: String,
    label: String,
}

impl std::fmt::Display for Pick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// An edit to one field of a Quick Add entry.
#[derive(Debug, Clone)]
pub enum Field {
    Name(String),
    Template(Pick),
    Folder(String),
    FileName(String),
    Open(bool),
    Target(String),
    Heading(String),
    Format(String),
    Top(bool),
    Create(bool),
}

pub fn apply(entry: &mut QuickAdd, field: Field) {
    match (entry, field) {
        (QuickAdd::Template { name, .. } | QuickAdd::Capture { name, .. }, Field::Name(value)) => *name = value,
        (QuickAdd::Template { template, .. }, Field::Template(pick)) => *template = pick.value,
        (QuickAdd::Template { folder, .. }, Field::Folder(value)) => *folder = value,
        (QuickAdd::Template { file_name, .. }, Field::FileName(value)) => *file_name = value,
        (QuickAdd::Template { open, .. }, Field::Open(value)) => *open = value,
        (QuickAdd::Capture { target, .. }, Field::Target(value)) => *target = value,
        (QuickAdd::Capture { heading, .. }, Field::Heading(value)) => *heading = value,
        (QuickAdd::Capture { format, .. }, Field::Format(value)) => *format = value,
        (QuickAdd::Capture { top, .. }, Field::Top(value)) => *top = value,
        (QuickAdd::Capture { create, .. }, Field::Create(value)) => *create = value,
        _ => {}
    }
}

/// Light or dark, or as the system has it. Kept with the session on this
/// machine, like Ctrl+Shift+L, which switches between light and dark.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    /// The choice an override of the system's theme stands for.
    pub fn of(dark_override: Option<bool>) -> Self {
        match dark_override {
            None => ThemeChoice::System,
            Some(false) => ThemeChoice::Light,
            Some(true) => ThemeChoice::Dark,
        }
    }

    /// The override of the system's theme this choice stands for.
    pub fn dark(self) -> Option<bool> {
        match self {
            ThemeChoice::System => None,
            ThemeChoice::Light => Some(false),
            ThemeChoice::Dark => Some(true),
        }
    }
}

impl std::fmt::Display for ThemeChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ThemeChoice::System => "As the system has it",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        })
    }
}

/// One of Sage's own shortcuts in the list.
pub struct BuiltIn {
    /// `None` when the vault took the keys away.
    pub keys: Option<String>,
    pub what: &'static str,
    /// Rebound or taken away in this vault's settings.
    pub changed: bool,
}

/// What the panel shows besides the settings themselves.
pub struct Ctx<'a> {
    pub settings: &'a Settings,
    /// The folder Obsidian's Templates setting names.
    pub obsidian_folder: &'a str,
    /// Every folder in the vault.
    pub folders: Vec<String>,
    /// The notes in the templates folder in use, by vault path.
    pub templates: Vec<String>,
    /// Why the settings could not be read or saved.
    pub error: Option<&'a str>,
    pub theme: ThemeChoice,
    /// The Quick Add entry whose shortcut is being recorded.
    pub recording: Option<usize>,
    /// Sage's own shortcuts, as this vault has them.
    pub built_in: Vec<BuiltIn>,
    /// The built-in shortcut being rebound.
    pub recording_built_in: Option<usize>,
    /// Keys and clicks that always do the same: keys, and what they do.
    pub fixed: &'static [(&'static str, &'static str)],
    /// The folder of the vault open now.
    pub vault: &'a std::path::Path,
    pub t: Tokens,
}

pub fn view(ctx: Ctx<'_>) -> Element<'_, Message> {
    let t = ctx.t;
    let section = |title: &'static str| text(title).size(12).font(theme::SANS_BOLD).color(t.text_2);
    let hint = |line: String| text(line).size(13).color(t.text_2);
    let mut page = Column::new().spacing(14).padding([28, 36]).max_width(760);

    page = page.push(
        row![
            text("Settings").size(24).font(theme::SERIF).color(t.text),
            space().width(Fill),
            action("Done", true, Message::ToggleSettings, t),
        ]
        .align_y(iced::Center),
    );
    if let Some(error) = ctx.error {
        page = page.push(text(error).size(13).color(t.error));
    }

    // Vault.
    page = page.push(section("VAULT"));
    page = page.push(
        row![
            text(ctx.vault.display().to_string()).size(14).width(Fill),
            action("Open vault folder…", false, Message::OpenVault, t),
        ]
        .spacing(12)
        .align_y(iced::Center),
    );
    page = page.push(hint(
        "Sage starts again on the folder you choose, and opens it next time. Unsaved notes are saved first.".into(),
    ));

    // Appearance.
    page = page.push(container(section("APPEARANCE")).padding(iced::Padding { top: 12.0, ..iced::Padding::ZERO }));
    page = page.push(labelled(
        "Theme",
        pick_list(ThemeChoice::ALL, Some(ctx.theme), Message::SetTheme)
            .text_size(14)
            .padding([5, 8])
            .width(Fill)
            .style(move |_, status| theme::drop_down(t, status))
            .menu_style(move |_| theme::menu(t)),
    ));
    page = page.push(hint("Ctrl+Shift+L switches between light and dark. This choice is kept on this computer.".into()));

    // Templates.
    page = page.push(container(section("TEMPLATES")).padding(iced::Padding { top: 12.0, ..iced::Padding::ZERO }));
    let default = Pick { value: String::new(), label: format!("Obsidian's setting: {}/", ctx.obsidian_folder) };
    let mut folders = vec![default.clone()];
    folders.extend(ctx.folders.iter().map(|f| Pick { value: f.clone(), label: format!("{f}/") }));
    let current = match &ctx.settings.templates_folder {
        Some(f) => Pick { value: f.clone(), label: format!("{f}/") },
        None => default,
    };
    page = page.push(labelled("Templates folder", choose(folders, current, Message::SettingsTemplatesFolder, t)));
    page = page.push(hint(
        "Insert template and Quick Add take their templates from here. Obsidian's own setting is left as it is.".into(),
    ));

    // Quick Add.
    page = page.push(container(section("QUICK ADD")).padding(iced::Padding { top: 12.0, ..iced::Padding::ZERO }));
    page = page.push(hint(
        "Run an entry with the + beside the vault name, Ctrl+Shift+N, or by its name in Ctrl+P. {{value}} is asked \
         for when it runs; {{date:YYYY-MM-DD}}, {{time}} and {{title}} work as in templates."
            .into(),
    ));
    let templates: Vec<Pick> = std::iter::once(Pick { value: String::new(), label: "No template, an empty note".into() })
        .chain(ctx.templates.iter().map(|path| Pick { value: path.clone(), label: crate::file_stem(path).to_owned() }))
        .collect();
    for (i, entry) in ctx.settings.quick_add.iter().enumerate() {
        page = page.push(entry_panel(i, entry, &templates, ctx.recording == Some(i), t));
    }
    page = page.push(
        row![
            action("Add a template entry", false, Message::QuickAddNew(false), t),
            action("Add a capture entry", false, Message::QuickAddNew(true), t),
        ]
        .spacing(8),
    );

    // Shortcuts.
    page = page.push(container(section("SHORTCUTS")).padding(iced::Padding { top: 12.0, ..iced::Padding::ZERO }));
    page = page.push(hint(
        "Click a shortcut to change it, then press the keys. A shortcut needs Ctrl, or is a function key.          Changes are kept in this vault's settings."
            .into(),
    ));
    let line = |keys: Element<'static, Message>, what: String| -> Element<'static, Message> {
        row![container(keys).width(240), text(what).size(14)].spacing(12).align_y(iced::Center).into()
    };
    for (i, built_in) in ctx.built_in.iter().enumerate() {
        page = page.push(line(rebinder(i, built_in, ctx.recording_built_in == Some(i), t), built_in.what.to_owned()));
    }
    for (i, entry) in ctx.settings.quick_add.iter().enumerate() {
        page = page.push(line(recorder(i, entry, ctx.recording == Some(i), t), format!("Quick add: {}", entry.name())));
    }
    for &(keys, what) in ctx.fixed {
        page = page.push(line(keycap(keys, t), what.to_owned()));
    }
    scrollable(container(page).width(Fill)).height(Fill).into()
}

/// Keys as they are pressed, set apart like a key on a keyboard.
fn keycap(keys: &'static str, t: Tokens) -> Element<'static, Message> {
    container(text(keys).size(12).font(theme::MONO)).padding([2, 6]).style(move |_| theme::surface(t)).into()
}

/// One of Sage's own shortcuts: click to record new keys; take them away,
/// or give back the usual ones.
fn rebinder(i: usize, built_in: &BuiltIn, recording: bool, t: Tokens) -> Element<'static, Message> {
    let label = match (recording, &built_in.keys) {
        (true, _) => "Press the keys, or Esc".to_owned(),
        (false, Some(keys)) => keys.clone(),
        (false, None) => "No shortcut".to_owned(),
    };
    let mut keys = row![button(text(label).size(13).font(theme::MONO))
        .padding([4, 10])
        .style(move |_, status| theme::nav_button(t, recording, status))
        .on_press(Message::RecordBuiltIn(i))]
    .spacing(6)
    .align_y(iced::Center);
    if !recording && built_in.changed {
        keys = keys.push(action("Reset", false, Message::ResetBuiltIn(i), t));
    } else if !recording && built_in.keys.is_some() {
        keys = keys.push(action("Clear", false, Message::ClearBuiltIn(i), t));
    }
    keys.into()
}

/// A Quick Add entry's shortcut: click to record new keys, or clear it.
fn recorder(i: usize, entry: &QuickAdd, recording: bool, t: Tokens) -> Element<'static, Message> {
    let label = match (recording, entry.shortcut()) {
        (true, _) => "Press the keys, or Esc".to_owned(),
        (false, Some(chord)) => chord.to_string(),
        (false, None) => "Set a shortcut".to_owned(),
    };
    let mut keys = row![button(text(label).size(13).font(theme::MONO))
        .padding([4, 10])
        .style(move |_, status| theme::nav_button(t, recording, status))
        .on_press(Message::RecordShortcut(i))]
    .spacing(6)
    .align_y(iced::Center);
    if entry.shortcut().is_some() && !recording {
        keys = keys.push(action("Clear", false, Message::ClearShortcut(i), t));
    }
    keys.into()
}

fn entry_panel<'a>(i: usize, entry: &'a QuickAdd, templates: &[Pick], recording: bool, t: Tokens) -> Element<'a, Message> {
    let edit = move |field: fn(String) -> Field| move |value: String| Message::QuickAddEdit(i, field(value));
    let toggle = move |field: fn(bool) -> Field| move |value: bool| Message::QuickAddEdit(i, field(value));
    let kind = match entry {
        QuickAdd::Template { .. } => "Creates a note from a template",
        QuickAdd::Capture { .. } => "Adds text to a note",
    };
    let mut body = Column::new().spacing(8).push(
        row![
            input("Name", entry.name(), edit(Field::Name), t).width(240),
            text(kind).size(13).color(t.text_2),
            space().width(Fill),
            action("↑", false, Message::QuickAddMove(i, true), t),
            action("↓", false, Message::QuickAddMove(i, false), t),
            action("Remove", false, Message::QuickAddRemove(i), t),
        ]
        .spacing(12)
        .align_y(iced::Center),
    );
    match entry {
        QuickAdd::Template { template, folder, file_name, open, .. } => {
            let current = templates.iter().find(|p| p.value == *template).cloned().unwrap_or_else(|| Pick {
                value: template.clone(),
                label: format!("{template} (not in the templates folder)"),
            });
            body = body
                .push(labelled("Template", choose(templates.to_vec(), current, move |p| Message::QuickAddEdit(i, Field::Template(p)), t)))
                .push(labelled("Folder", input("the vault root", folder, edit(Field::Folder), t)))
                .push(labelled("File name", input("{{value}}", file_name, edit(Field::FileName), t)))
                .push(labelled("", check("Open the note once it is created", *open, toggle(Field::Open), t)));
        }
        QuickAdd::Capture { target, heading, format, top, create, .. } => {
            body = body
                .push(labelled("Note", input("Inbox, or Daily/{{date:YYYY-MM-DD}}", target, edit(Field::Target), t)))
                .push(labelled("Under heading", input("none: the whole note", heading, edit(Field::Heading), t)))
                .push(labelled("Text", input("- {{value}}", format, edit(Field::Format), t)))
                .push(labelled("", check("At the top, rather than at the end", *top, toggle(Field::Top), t)))
                .push(labelled("", check("Create the note when it does not exist", *create, toggle(Field::Create), t)));
        }
    }
    body = body.push(labelled("Shortcut", recorder(i, entry, recording, t)));
    container(body).padding([12, 14]).width(Fill).style(move |_| theme::panel(t)).into()
}

fn labelled<'a>(label: &'a str, field: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    row![text(label).size(14).width(150), field.into()].spacing(12).align_y(iced::Center).into()
}

fn input<'a>(
    placeholder: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> Message + 'a,
    t: Tokens,
) -> iced::widget::TextInput<'a, Message> {
    text_input(placeholder, value)
        .on_input(on_input)
        .size(14)
        .padding([5, 8])
        .style(move |_, status| theme::input(t, status))
}

fn choose<'a>(options: Vec<Pick>, current: Pick, on_pick: impl Fn(Pick) -> Message + 'a, t: Tokens) -> Element<'a, Message> {
    pick_list(options, Some(current), on_pick)
        .text_size(14)
        .padding([5, 8])
        .width(Fill)
        .style(move |_, status| theme::drop_down(t, status))
        .menu_style(move |_| theme::menu(t))
        .into()
}

fn check<'a>(label: &'a str, on: bool, on_toggle: impl Fn(bool) -> Message + 'a, t: Tokens) -> Element<'a, Message> {
    checkbox(on).label(label).size(14).text_size(14).on_toggle(on_toggle).style(move |_, status| theme::check(t, status)).into()
}

fn action<'a>(label: &'a str, current: bool, message: Message, t: Tokens) -> Element<'a, Message> {
    button(text(label).size(13))
        .padding([5, 12])
        .style(move |_, status| theme::nav_button(t, current, status))
        .on_press(message)
        .into()
}
