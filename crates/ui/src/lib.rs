//! The Iced application shell: folder tree, tabs, source pane with
//! autocomplete and find, preview pane with clickable links, the task list,
//! command palette and status bar, styled with the Sage & Paper tokens.
//!
//! Threading: this thread never writes to SQLite and never blocks on file IO.
//! The indexer runs on its own thread, started by a subscription. Reads and
//! saves run as tasks on the async runtime, blocking work on its blocking pool.

mod flow;
mod highlight;
mod live;
mod live_editor;
mod notify;
mod omni;
mod order;
mod preview;
mod settings;
mod suggest;
mod tables;
mod task_form;
mod tasks;
pub mod theme;
mod timeline;
mod tree;
mod store;
mod vault;
mod views;

use sage_editor::complete;
use sage_editor::edit::{self, Format, Pos};
use sage_editor::merge::{Choice, Chunk, Merge};
use sage_editor::rename::Rename;
use sage_editor::chord::{Chord, Key as ChordKey};
use sage_editor::settings::QuickAdd;
use sage_editor::template::Config as TemplateConfig;
use sage_editor::SaveError;
use sage_indexer::IndexEvent;
use sage_watcher::Backend;
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::keyboard::{self, key::Named, Key};
use iced::widget::text_editor::{Binding, Cursor, Edit, Position};
use iced::widget::{
    button, column, container, row, scrollable, space, stack, text, text_editor, text_input,
    Column, Row,
};
use sage_query::Outcome;
use iced::{Element, Fill, Length, Subscription, Task, Theme};
use preview::Embedded;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use suggest::Suggestion;
use tasks::TaskList;
use theme::Tokens;

const PALETTE_INPUT: &str = "palette-input";
const FIND_INPUT: &str = "find-input";
const EDITOR: &str = "source-editor";
const TABS: &str = "tab-bar";

/// The pointer and window as last seen, kept off the message queue so that
/// moving the mouse does not redraw the window. A context menu opens here.
static POINTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WINDOW_HEIGHT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The window's size when it opens.
const WINDOW_SIZE: (f32, f32) = (1280.0, 820.0);
/// The width of a divider between panes: the hairline and room to grab it.
const DIVIDER: f32 = 7.0;
/// The narrowest the folder tree and the workspace beside it can be dragged.
const MIN_SIDEBAR: f32 = 140.0;
const MIN_WORKSPACE: f32 = 360.0;
/// Quiet time after the last keystroke before a note is saved automatically.
const AUTOSAVE_AFTER: Duration = Duration::from_millis(1500);
/// A note that parses faster than this has its preview and outline parsed
/// again on every key; a longer one waits, as `parse_waits` says.
const PARSE_FAST: Duration = Duration::from_millis(8);
/// The pause in typing a long note's preview and outline wait for...
const PARSE_SETTLE: Duration = Duration::from_millis(250);
/// ...but never for longer than this after they were last parsed.
const PARSE_STALE: Duration = Duration::from_millis(750);
/// How long a notice stays in the status bar before index progress replaces it.
const NOTICE_FOR: Duration = Duration::from_secs(8);
/// How often open notes are checked for changes on a polled vault.
const TAB_POLL: Duration = Duration::from_secs(2);
/// How many notes Back and Forward and the recent notes remember.
const HISTORY: usize = 100;
const RECENT: usize = 20;
/// The widest the note is in focus mode.
const FOCUS_WIDTH: f32 = 820.0;
/// Every autosave changes the index: the active note's views run again for
/// that at most this often, however fast the changes come.
const PREVIEWS_EVERY: Duration = Duration::from_millis(300);
/// How many renames Undo last rename can still revert. Each keeps the text
/// of every note it rewrote, before and after.
const RENAMES: usize = 10;

/// A number nothing has had before. Names a tab, so that work done for it
/// off the UI thread finds it again even after its note was renamed, and a
/// request, so that an answer to an older one is known as stale.
fn unique() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn run(vault: PathBuf) -> anyhow::Result<()> {
    run_with(vault, None)
}

/// Opens `vault`, then `note` in it (a file inside the vault, as given on
/// the command line by "Open with Sage").
pub fn run_with(vault: PathBuf, note: Option<PathBuf>) -> anyhow::Result<()> {
    let vault = sage_watcher::canonical(&vault)?;
    let db_path = sage_db::index_path_for(&vault)?;
    // The window opens at the same size on screen whatever the zoom, which
    // scales everything in it, the window's own size included.
    let zoom = load_session(&db_path).zoom();
    // Vault-relative with `/` separators, as tabs name their notes.
    let note = note.and_then(|note| {
        let note = sage_watcher::canonical(&note).ok()?;
        let rel = note.strip_prefix(&vault).ok()?.to_str()?.replace('\\', "/");
        (!rel.is_empty()).then_some(rel)
    });

    let mut app = iced::application(
        move || {
            let (mut app, boot) = App::boot(vault.clone(), db_path.clone());
            match note.clone() {
                Some(note) => {
                    let open = app.open(note);
                    (app, Task::batch([boot, open]))
                }
                None => (app, boot),
            }
        },
        App::update,
        App::view,
    )
    .title(App::title)
    .theme(App::theme)
    .subscription(App::subscription)
    .default_font(theme::SANS)
    .scale_factor(App::scale_factor)
    .window(iced::window::Settings {
        size: (WINDOW_SIZE.0 / zoom, WINDOW_SIZE.1 / zoom).into(),
        icon: window_icon(),
        // Closing waits for unsaved notes to be written; see `close_requested`.
        exit_on_close_request: false,
        ..Default::default()
    });
    for bytes in theme::FONT_FILES {
        app = app.font(bytes);
    }
    app.run()?;
    Ok(())
}

/// The Sage crest for the title bar and taskbar, compiled into the binary.
fn window_icon() -> Option<iced::window::Icon> {
    let png = image::load_from_memory(include_bytes!("../../../assets/icons/sage.png")).ok()?.to_rgba8();
    let (width, height) = png.dimensions();
    iced::window::icon::from_rgba(png.into_raw(), width, height).ok()
}

#[derive(Debug, Clone)]
enum Message {
    Index(IndexEvent),
    /// The indexer is running; the UI keeps a handle to ask it for passes.
    IndexerReady(IndexerLink),
    WindowFocused,
    /// The window lost focus: a button let go elsewhere is never reported.
    WindowUnfocused,
    /// Time to stat the open notes again (polled vaults only).
    PollTabs,
    /// `(tab, path, stamp)` of every open note; `None` if it could not be read.
    Polled(Vec<(u64, String, Option<Stamp>)>),
    /// One conflict in the resolve panel decided.
    Choose(usize, Choice),
    /// Every conflict in the resolve panel decided the same way.
    ChooseAll(Choice),
    ApplyResolve,
    CancelResolve,
    /// Compare a OneDrive conflict copy with the active note.
    CompareCopy(String),
    /// Results of work done off the UI thread for a tab name it by `Tab::id`,
    /// never by path: the note may have been renamed meanwhile, and blank
    /// tabs all share the empty path.
    CopyLoaded { tab: u64, copy: String, result: Result<String, String> },
    SystemTheme(iced::theme::Mode),
    Open(String),
    /// Opens a note with the cursor on a line, from the task list.
    OpenAt(String, usize),
    FollowLink(LinkTarget),
    /// A note read for tab `tab`, from `path`.
    Loaded { tab: u64, path: String, result: Result<String, String> },
    /// A save of tab `tab` to `path` finished.
    Saved { tab: u64, path: String, seq: u64, result: Result<String, SaveError> },
    Created(Result<String, String>),
    StarterCreated(Result<sage_editor::starter::Written, String>),
    Trashed(String, Result<String, String>),
    Renamed(Result<Arc<Rename>, String>),
    /// The rename undone, with each restored note's path and text.
    RenameUndone(Result<(Arc<Rename>, Vec<(String, String)>), String>),
    /// A right click on a tree item: `(path, folder)`. Opens its menu.
    TreeMenu(String, bool),
    /// The buttons above the tree.
    TreeNewNote,
    TreeNewFolder,
    TreeOrderMenu,
    TreeOrder(tree::Order),
    TreeFoldAll,
    /// The vault's folders on disk, empty ones included.
    DiskFolders(Vec<String>),
    FolderCreated(Result<String, String>),
    /// The tree menu's choices, for the item it is open on.
    MenuRename,
    MenuDelete,
    /// Move the file or folder waiting for confirmation to `.trash`, or not.
    ConfirmDelete,
    CancelDelete,
    TaskFormTitle(String),
    TaskFormDue(String),
    TaskFormRemind(String),
    /// A system notification was shown, or why not.
    Notified(Result<(), String>),
    SettingsNotifications(bool),
    SettingsReminderTime(String),
    TaskFormSubmit,
    /// A task note was created, to be linked in `tab`: its path and id.
    TaskNoteCreated { tab: Option<u64>, result: Result<(String, String), String> },
    /// A divider pressed, the pointer moved while dragging it to `x`, and the
    /// window's new width.
    ResizeStart(Divider),
    ResizeMove(f32),
    WindowWidth(f32),
    /// A web or mail link, or a file to reveal, was handed to the desktop, or
    /// could not be.
    UrlOpened(Result<(), String>),
    /// A foldable callout in the active note's preview opened or closed: its
    /// key, and whether it is now open.
    ToggleCallout(String, bool),
    /// The vault's settings and Obsidian's Templates settings, read at start.
    SettingsLoaded(Result<sage_editor::settings::Settings, String>),
    ObsidianTemplates(TemplateConfig),
    SettingsSaved(Result<(), String>),
    ToggleSettings,
    SettingsTemplatesFolder(settings::Pick),
    SettingsNewNoteTemplate(settings::Pick),
    SettingsStartupNote(Option<String>),
    SettingsQuickAddTemplates(bool),
    QuickAddEdit(usize, settings::Field),
    /// A new Quick Add entry: a capture entry, or a template entry.
    QuickAddNew(bool),
    QuickAddRemove(usize),
    /// Record the next chord as Quick Add entry i's shortcut, or stop; clear it.
    RecordShortcut(usize),
    /// Record the next chord as built-in shortcut i's keys, or stop; take
    /// its keys away; give it its usual keys back.
    RecordBuiltIn(usize),
    ClearBuiltIn(usize),
    ResetBuiltIn(usize),
    /// Move Quick Add entry i up, or down when false.
    QuickAddMove(usize, bool),
    ClearShortcut(usize),
    /// A key press that may be a shortcut.
    Chord(Chord),
    /// The theme chosen in the settings.
    SetTheme(settings::ThemeChoice),
    /// A Quick Add note made: its path, and whether it was new; and whether to open it.
    QuickNoteDone { result: Result<(String, bool), String>, open: bool },
    /// Text captured into a note: its path and new text.
    Captured { path: String, result: Result<String, String> },
    /// A note written by a workflow: its path and new text, what to tell the
    /// user, and a note to open afterwards.
    Written { path: String, result: Result<String, String>, done: String, open: Option<String> },
    /// The lines of today's daily note that are not sorted yet.
    TriageLoaded(Result<Vec<flow::Line>, String>),
    /// Text read off the clipboard for an import.
    ImportText(Option<String>),
    /// The timeline's kind filter.
    TimelineKind(Option<&'static str>),
    /// The timeline of tab `tab`'s note at `path`, read as request `asked`;
    /// `None` when the index could not be read, and the timeline shown stays.
    TimelineLoaded { tab: u64, path: String, asked: u64, rows: Option<Vec<timeline::Entry>> },
    /// The Daily notes settings, read for Ctrl+D.
    DailyFound(sage_editor::obsidian::Daily),
    ToggleTask(usize),
    /// A task checkbox in the preview of the active note, by source line.
    PreviewTask(u32),
    /// A task checkbox in a view block.
    ViewTask(TaskRef),
    /// A task line rewritten on disk: the note's new text.
    TaskWritten { path: String, result: Result<String, String> },
    EmbedLoaded { tab: u64, key: String, embedded: Embedded },
    /// A Kanban card pressed: the start of a drag, or a click.
    Grab { view: String, card: TaskRef, key: String, column: usize, index: usize },
    DragOver(String, usize),
    DragLeave(String, usize),
    /// The pointer over a card while dragging: `(view, column, index)`.
    DragCard(String, usize, usize),
    DragCardLeave(String, usize, usize),
    MouseUp,
    TemplatesFound(Result<TemplateConfig, String>),
    TemplateLoaded { target: u64, config: TemplateConfig, result: Result<String, String> },
    SelectTab(usize),
    CloseTab(usize),
    TabMenu(usize),
    CloseOtherTabs(usize),
    CloseTabsRight(usize),
    /// Runs a command from a button.
    Run(Command),
    /// Moves a tab one place left (-1) or right (1).
    MoveTab(usize, isize),
    PinTab(usize),
    /// The toast's Undo button: moves a note back out of `.trash`.
    UndoTrash,
    TrashUndone(Result<String, String>),
    DismissToast,
    TaskFilter(tasks::Filter),
    ShortcutFilter(String),
    SettingsTrackedOnly(bool),
    /// Task notes closed because the checkbox linking them was checked.
    TaskNotesClosed(Vec<String>),
    /// Once a second while a toast shows, so it goes away by itself.
    ToastTick,
    CloseAllTabs,
    CloseMenu,
    ToggleFolder(String),
    Edit(text_editor::Action),
    /// A key the editors handle themselves: formatting, lists, lines, pairs.
    EditOp(EditOp),
    /// Ctrl+V in an editor: a picture on the clipboard becomes an attachment.
    PasteImage,
    /// A file dropped on the window.
    FileDropped(PathBuf),
    /// A picture saved into the vault for the note in tab `tab`: its vault
    /// path, or `None` when the clipboard held no picture.
    Attached { tab: u64, result: Result<Option<String>, String> },
    /// A heading in the outline clicked: its line.
    OutlineJump(usize),
    MenuReveal,
    MenuCopyLink,
    /// From the live preview pane.
    Live(live_editor::Event),
    /// An entry of the note's properties, by its index, set from the widget
    /// that live preview draws in place of the frontmatter.
    Property(usize, sage_editor::properties::Value),
    /// A field of the properties typed in: entry, list item, new value.
    PropertyTyped(usize, Option<usize>, sage_editor::properties::Value),
    /// Enter in a field: accepts the suggestion, else makes a link property's
    /// text a link.
    PropertySubmit(usize, Option<usize>),
    PropertyPick(usize),
    /// Property `entry` is given a type: its value is converted and the
    /// choice kept for that name.
    PropertyType(usize, sage_editor::properties::PropType),
    /// Opens or closes the list of types under property `entry`.
    PropertyTypeMenu(usize),
    /// Removes property `entry` altogether.
    PropertyRemove(usize),
    /// The pointer is over list item `(entry, item)`, or over none.
    ChipHover(Option<(usize, usize)>),
    /// Search everywhere for a tag, from a tag pill.
    SearchTag(String),
    /// The Today button of a date property: today's date, or in a template a placeholder for it.
    PropertyToday(usize),
    /// Starts (`Some`) or ends editing the pills of property entry `n`.
    PropertyEdit(Option<usize>),
    NewPropertyOpen,
    NewPropertyType(sage_editor::properties::PropType),
    /// Whether the new property's name field still has the focus.
    NewPropertyBlur(bool),
    NewPropertyTyped(String),
    NewPropertySubmit,
    /// Whether an empty list item still has the focus; once it has not, it goes.
    PropertyBlur(usize, usize, bool),
    /// A new, empty item at the end of a list.
    PropertyAdd(usize),
    /// A change to the table that starts on the given line, in live preview.
    Table(usize, tables::Edit),
    Tick,
    CompleteMove(i32),
    CompletePick(usize),
    CompleteAccept,
    CompleteDismiss,
    FindQuery(String),
    FindSubmit,
    FindClose,
    PaletteQuery(String),
    PaletteSubmit,
    PaletteClick(usize),
    Key(Shortcut),
    Modifiers(keyboard::Modifiers),
    /// Ask for another vault's folder; the folder chosen and checked, `None`
    /// if cancelled; switch to it once unsaved notes are written.
    OpenVault,
    VaultPicked(Result<Option<PathBuf>, String>),
    SwitchVault(PathBuf),
    /// The window's close button; then, once the saves it started are done,
    /// whether to close even with notes that could not be saved.
    CloseRequested,
    Quit { force: bool },
    /// A minute passed: views with `today` in them go stale at midnight.
    Clock,
    /// Time to see whether a long note's preview can be parsed again.
    Settle,
    /// Search everywhere's notes, read from the index.
    CorpusLoaded(Result<Arc<omni::Corpus>, String>),
}

/// What the editors do for a key themselves, instead of iced's default.
#[derive(Debug, Clone, Copy, PartialEq)]
enum EditOp {
    Format(Format),
    Link,
    /// Enter: lists, tasks and quotes carry on.
    Enter,
    /// Tab, or Shift+Tab when true.
    Indent(bool),
    /// Alt+↑ when true, Alt+↓ when false.
    MoveLines(bool),
    Duplicate,
    /// A bracket, backtick or star typed: pairs close themselves.
    Type(char),
    Backspace,
    Undo,
    Redo,
}

/// The editing key a press in an editor makes, if Sage handles it.
fn edit_op(press: &text_editor::KeyPress) -> Option<EditOp> {
    let m = press.modifiers;
    let (ctrl, alt, shift) = (m.command(), m.alt(), m.shift());
    let plain = !ctrl && !alt;
    match press.key.as_ref() {
        Key::Named(Named::Enter) if plain && !shift => Some(EditOp::Enter),
        Key::Named(Named::Tab) if plain => Some(EditOp::Indent(shift)),
        Key::Named(Named::Backspace) if plain && !shift => Some(EditOp::Backspace),
        Key::Named(Named::ArrowUp) if alt && !ctrl && !shift => Some(EditOp::MoveLines(true)),
        Key::Named(Named::ArrowDown) if alt && !ctrl && !shift => Some(EditOp::MoveLines(false)),
        _ if ctrl && !alt => match (press.key.to_latin(press.physical_key)?, shift) {
            ('b', false) => Some(EditOp::Format(Format::Bold)),
            ('i', false) => Some(EditOp::Format(Format::Italic)),
            ('`', false) => Some(EditOp::Format(Format::Code)),
            ('k', false) => Some(EditOp::Link),
            ('d', true) => Some(EditOp::Duplicate),
            ('z', false) => Some(EditOp::Undo),
            ('z', true) | ('y', false) => Some(EditOp::Redo),
            _ => None,
        },
        // AltGr arrives as Ctrl+Alt and types brackets on many keyboards;
        // those are typed as they are.
        _ if plain => {
            let mut chars = press.text.as_deref()?.chars();
            let c = chars.next()?;
            (chars.next().is_none() && matches!(c, '(' | '[' | '`' | '*' | ')' | ']')).then_some(EditOp::Type(c))
        }
        _ => None,
    }
}

/// Everything the command palette and the keyboard can trigger.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Command {
    Save,
    ForceSave,
    NewNote,
    DeleteNote,
    CycleView,
    /// Live preview, or back to the mode before it.
    ToggleLive,
    View(ViewMode),
    ToggleTheme,
    TogglePalette,
    SearchEverywhere,
    CloseActiveTab,
    /// A blank tab, from which a note is opened or created.
    NewTab,
    /// Finds a note and opens it in the current tab.
    OpenNote,
    Find,
    RenameNote,
    RenameFolder,
    MoveNote,
    UndoRename,
    InsertTemplate,
    ToggleTasks,
    NewInlineTask,
    /// Makes the line a task and offers the people to follow up with.
    FollowUp,
    /// The same, for something awaited from somebody: tagged `#waiting` with the day it began.
    WaitingFor,
    /// Adds a dated line to a project's log.
    LogToProject,
    /// Turns the log line under the cursor into a note of its own.
    PromoteLog,
    /// Files, makes a task of, or follows up the loose lines of today's daily note.
    Triage,
    /// Turns the text on the clipboard, an email or meeting notes, into an event.
    ImportEvent,
    /// This week's review note, made if it is missing.
    WeeklyReview,
    NewTaskNote,
    QuickAdd,
    ToggleSettings,
    OpenVault,
    /// The note before or after this one, in the order they were opened.
    Back,
    Forward,
    DailyNote,
    ReopenTab,
    ToggleOutline,
    ToggleFocus,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// The list of keyboard shortcuts and tricks.
    Shortcuts,
    /// Writes the starter vault's folders, templates and tutorial, keeping every file that exists.
    CreateStarterVault,
}

/// The commands the palette offers, by name.
const COMMANDS: [(&str, Command); 45] = [
    ("Keyboard shortcuts", Command::Shortcuts),
    ("Save note", Command::Save),
    ("Search everywhere", Command::SearchEverywhere),
    ("New note", Command::NewNote),
    ("Find in note", Command::Find),
    ("Rename or move note", Command::RenameNote),
    ("Rename or move this note's folder", Command::RenameFolder),
    ("Move note to folder", Command::MoveNote),
    ("Undo last rename", Command::UndoRename),
    ("Insert template", Command::InsertTemplate),
    ("Show tasks", Command::ToggleTasks),
    ("New task line", Command::NewInlineTask),
    ("Follow up with a person", Command::FollowUp),
    ("Waiting for a person", Command::WaitingFor),
    ("Log to project", Command::LogToProject),
    ("Promote log line to a note", Command::PromoteLog),
    ("Triage the daily note", Command::Triage),
    ("Import email or meeting notes from the clipboard", Command::ImportEvent),
    ("Weekly review", Command::WeeklyReview),
    ("New task note", Command::NewTaskNote),
    ("Quick add", Command::QuickAdd),
    ("Settings", Command::ToggleSettings),
    ("Open vault folder…", Command::OpenVault),
    ("Cycle view: source, split, preview, live preview", Command::CycleView),
    ("Toggle live preview", Command::ToggleLive),
    ("View: live preview", Command::View(ViewMode::Live)),
    ("View: source", Command::View(ViewMode::Source)),
    ("View: source and preview side by side", Command::View(ViewMode::Split)),
    ("View: preview", Command::View(ViewMode::Preview)),
    ("Toggle light and dark theme", Command::ToggleTheme),
    ("Close tab", Command::CloseActiveTab),
    ("New tab", Command::NewTab),
    ("Open note", Command::OpenNote),
    ("Delete note (move to .trash)", Command::DeleteNote),
    ("Overwrite file on disk with this tab", Command::ForceSave),
    ("Go back", Command::Back),
    ("Go forward", Command::Forward),
    ("Open today's daily note", Command::DailyNote),
    ("Reopen closed tab", Command::ReopenTab),
    ("Show outline", Command::ToggleOutline),
    ("Focus mode", Command::ToggleFocus),
    ("Zoom in", Command::ZoomIn),
    ("Zoom out", Command::ZoomOut),
    ("Reset zoom", Command::ZoomReset),
    ("Create starter vault: folders, templates and tutorial", Command::CreateStarterVault),
];

/// Sage's own shortcuts: what the keyboard runs, what the palette shows
/// beside each command, and what the shortcuts settings list.
const BUILT_IN: [(&str, Command); 28] = [
    ("F1", Command::Shortcuts),
    ("Ctrl+P", Command::TogglePalette),
    ("Ctrl+O", Command::OpenNote),
    ("Ctrl+Shift+A", Command::NewTab),
    ("Ctrl+Shift+F", Command::SearchEverywhere),
    ("Ctrl+S", Command::Save),
    ("Ctrl+N", Command::NewNote),
    ("Ctrl+Shift+N", Command::QuickAdd),
    ("Ctrl+F", Command::Find),
    ("Ctrl+T", Command::NewTaskNote),
    ("Ctrl+Shift+T", Command::NewInlineTask),
    ("Ctrl+Shift+J", Command::FollowUp),
    ("Ctrl+Shift+W", Command::WaitingFor),
    ("Ctrl+L", Command::LogToProject),
    ("Ctrl+Shift+I", Command::Triage),
    ("Ctrl+E", Command::CycleView),
    ("Ctrl+Shift+E", Command::ToggleLive),
    ("Ctrl+W", Command::CloseActiveTab),
    ("Ctrl+Shift+L", Command::ToggleTheme),
    ("F2", Command::RenameNote),
    ("Ctrl+M", Command::MoveNote),
    ("Ctrl+D", Command::DailyNote),
    ("Ctrl+Shift+R", Command::ReopenTab),
    ("Ctrl+Shift+O", Command::ToggleOutline),
    ("F11", Command::ToggleFocus),
    ("Ctrl+=", Command::ZoomIn),
    ("Ctrl+-", Command::ZoomOut),
    ("Ctrl+0", Command::ZoomReset),
];

/// The zoom levels Ctrl+= and Ctrl+- step through.
const ZOOM: [f32; 9] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.4, 1.6, 1.8, 2.0];

/// Shortcuts the text editor and text fields keep for themselves.
const EDITING: [(&str, &str); 12] = [
    ("Ctrl+Z", "undoes the last change to the note"),
    ("Ctrl+Y", "redoes what was undone"),
    ("Ctrl+Shift+Z", "redoes what was undone"),
    ("Ctrl+A", "selects all text"),
    ("Ctrl+C", "copies"),
    ("Ctrl+V", "pastes, a picture as an attachment"),
    ("Ctrl+X", "cuts"),
    ("Ctrl+B", "makes the selection bold, or plain again"),
    ("Ctrl+I", "makes the selection italic, or plain again"),
    ("Ctrl+`", "makes the selection inline code, or plain again"),
    ("Ctrl+K", "makes the selection a link"),
    ("Ctrl+Shift+D", "duplicates the line"),
];

/// Keys and clicks that always do the same, for the shortcuts settings.
const FIXED: [(&str, &str); 14] = [
    ("Delete", "Delete note (move to .trash), when no editor or text field has focus"),
    ("Esc", "Close the palette, new-task form, find bar, task list, settings or a question"),
    ("↑ ↓ Enter", "Choose in the palette and in suggestions"),
    ("Enter, Shift+Enter", "Next and previous match in find"),
    ("Middle click on a tab", "Close the tab"),
    ("Right click in the tree", "Rename, move or delete a note or folder"),
    ("Drag a divider", "Resize the tree, source and preview"),
    ("Enter", "Carry a list, task or quote on to the next line; on an empty item, end the list"),
    ("Tab, Shift+Tab", "Indent or outdent list items and selected lines"),
    ("Alt+↑, Alt+↓", "Move the line or selected lines up or down"),
    ("( [ ` **", "Close themselves; typing the closing one steps over it"),
    ("Alt+←, Alt+→", "Go back and forward between opened notes; also the mouse's side buttons"),
    ("Drop a picture on a note", "Copy it into the attachments folder and embed it"),
    ("Right click in the tree", "Also: reveal in the file manager, copy a link to the note"),
];

/// The keys shown beside a command in the palette, as the vault has them.
fn keys(command: Command, settings: &sage_editor::settings::Settings) -> String {
    match BUILT_IN.iter().position(|(_, c)| *c == command) {
        Some(i) => built_in_chord(i, settings).map(|c| c.to_string()).unwrap_or_default(),
        None if command == Command::DeleteNote => "Delete".into(),
        None if command == Command::Back => "Alt+←".into(),
        None if command == Command::Forward => "Alt+→".into(),
        None => String::new(),
    }
}

/// The keys that run built-in shortcut `i`: its usual ones, unless the
/// vault's settings rebound or removed them.
fn built_in_chord(i: usize, settings: &sage_editor::settings::Settings) -> Option<Chord> {
    let (default, command) = BUILT_IN[i];
    settings.shortcut_for(command_name(command), default)
}

/// What a command is called in the palette and the shortcuts settings.
fn command_name(command: Command) -> &'static str {
    match COMMANDS.iter().find(|(_, c)| *c == command) {
        Some((name, _)) => name,
        None => "Find a note or a command",
    }
}

#[derive(Debug, Clone, Copy)]
enum Shortcut {
    Command(Command),
    Up,
    Down,
    Escape,
    /// Moves between the new-task form's fields; nothing elsewhere.
    Tab,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
enum ViewMode {
    Source,
    #[default]
    Split,
    Preview,
    /// One pane, formatted as you type, as Obsidian's Live Preview.
    Live,
}

/// What is restored on the next start. Lives next to the index, never in the vault.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Session {
    tabs: Vec<String>,
    active: usize,
    view_mode: ViewMode,
    dark_override: Option<bool>,
    expanded: Vec<String>,
    /// Foldable callouts opened or closed by hand: note path, callout key, open.
    folds: HashMap<String, HashMap<String, bool>>,
    /// Where the panes were split, if the user moved the dividers.
    panes: Option<Panes>,
    tree_order: tree::Order,
    /// Notes opened lately, newest first, for the palette.
    recent: Vec<String>,
    /// Notes whose tabs are pinned.
    pinned: Vec<String>,
    /// The sidebar shows the note's outline instead of the tree.
    outline: bool,
    /// The interface's size, 1.0 as designed.
    zoom: Option<f32>,
    /// When reminders were last looked at, `YYYY-MM-DDTHH:MM:SS` local time:
    /// the next start tells the ones that went off since.
    reminders_checked: Option<String>,
}

/// The sizes of the panes, set by dragging the dividers between them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct Panes {
    /// The folder tree's width in logical pixels.
    sidebar: f32,
    /// The source pane's share of the space source and preview split.
    source: f32,
}

impl Default for Panes {
    fn default() -> Self {
        Self { sidebar: 260.0, source: 0.5 }
    }
}

/// A divider being dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Divider {
    /// Between the folder tree and the workspace.
    Sidebar,
    /// Between the source and preview panes.
    Split,
}

/// A file or folder to move to `.trash`, waiting for the user to confirm.
#[derive(Debug, Clone)]
struct Doomed {
    path: String,
    folder: bool,
}

/// A short message over the page. `undo` is where a trashed note came from and went to.
struct Toast {
    text: String,
    undo: Option<(String, String)>,
    /// A reminder's task, `(path, line)`, which Open goes to.
    open: Option<(String, Option<u32>)>,
    until: Instant,
}

const TOAST_FOR: Duration = Duration::from_secs(6);
/// A reminder's toast stays longer than a notice, to be read and opened.
const REMINDER_TOAST_FOR: Duration = Duration::from_secs(30);
/// How the session stores when reminders were last looked at.
const CHECKED_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

struct App {
    vault: PathBuf,
    db_path: PathBuf,
    /// Read-only. `None` until the indexer has created the database.
    reader: Option<Connection>,
    backend: Option<Backend>,
    system_dark: bool,
    /// `Some` once the user has toggled the theme by hand.
    dark_override: Option<bool>,
    files: Vec<FileRow>,
    /// Every folder on disk, so one without notes still shows in the tree.
    disk_folders: Vec<String>,
    /// The folders that hold notes, and those with the folders on disk added,
    /// sorted: made again when `files` or `disk_folders` change rather than
    /// on every frame. See `folder_lists`.
    note_folders: Vec<String>,
    tree_folders: Vec<String>,
    /// The folders on disk are being listed; list them again once that is
    /// done, as the vault changed meanwhile.
    scanning: bool,
    rescan: bool,
    expanded: HashSet<String>,
    tree_order: tree::Order,
    /// The tree's order choices are showing.
    tree_order_menu: bool,
    tabs: Vec<Tab>,
    active: usize,
    view_mode: ViewMode,
    /// The mode Ctrl+Shift+E goes back to from live preview.
    before_live: ViewMode,
    palette: Option<Palette>,
    completion: Option<Completion>,
    /// `(line, start)` of a completion the user dismissed, so it stays closed
    /// until they start a new one.
    dismissed: Option<(usize, usize)>,
    /// Esc already closed the completion popup; the same key press reaches the
    /// global shortcut listener afterwards and must not close the find bar too.
    escape_used: bool,
    /// Suggestions under a field of the properties widget.
    prop_complete: Option<PropCompletion>,
    /// The table cell being typed in, as typed, so spaces at its ends survive until the field is left.
    table_draft: Option<tables::Draft>,
    /// The name being typed for a new property.
    new_property: Option<(sage_editor::properties::PropType, String)>,
    /// The property whose list of types is open.
    type_menu: Option<usize>,
    chip_hover: Option<(usize, usize)>,
    /// The note and entry whose pills are being edited; the rest show as pills.
    editing_prop: Option<(String, usize)>,
    find: Option<Find>,
    /// The vault-wide task list, shown in place of the note while open.
    tasks: Option<TaskList>,
    /// Task documents checked this session, by path: the status each had
    /// before, which unchecking restores.
    reopen: HashMap<String, String>,
    /// Renames of this session, newest last, each undoable as one step.
    renames: Vec<Arc<Rename>>,
    /// Shift decides the direction of Enter in the find bar.
    modifiers: keyboard::Modifiers,
    /// A Kanban card being dragged in the preview.
    drag: Option<views::Drag>,
    /// Kanban cards placed by hand.
    order: order::Order,
    /// Foldable callouts opened or closed by hand: note path, callout key, open.
    folds: HashMap<String, HashMap<String, bool>>,
    panes: Panes,
    /// A divider being dragged.
    resizing: Option<Divider>,
    /// The window's width, which the dividers share out.
    window_width: f32,
    /// The tree item whose menu is open: its path, and whether it is a folder.
    /// A tab's menu sets this to the tab's note as well.
    menu: Option<(String, bool)>,
    /// Where the open menu was asked for, and the tab it belongs to, if any.
    menu_at: iced::Point,
    tab_menu: Option<usize>,
    /// The current tab and the tab count the tab bar was last scrolled for.
    tabs_shown: (usize, usize),
    /// Whether the start-up note has been dealt with.
    startup_done: bool,
    /// Whether the indexer has finished a pass, so an empty file list means an empty vault.
    indexed: bool,
    /// A deletion waiting for the user to confirm it.
    doomed: Option<Doomed>,
    /// The list of shortcuts, while it is open.
    help: bool,
    /// The settings panel's shortcut search.
    shortcut_filter: String,
    /// A message floating over the page for a few seconds, with Undo when it can.
    toast: Option<Toast>,
    /// Open tasks with reminders, read from the index after each change.
    reminders: Vec<sage_editor::reminders::Reminder>,
    /// When reminders were last looked at; kept in the session.
    reminders_checked: Option<chrono::NaiveDateTime>,
    /// Whether the first look since Sage started, which gathers the
    /// reminders missed while it was closed, has happened.
    reminders_caught_up: bool,
    /// Notes whose tabs stay at the left and ignore Close others and Close all.
    pinned: Vec<String>,
    /// The new-task form, while it is open.
    task_form: Option<task_form::TaskForm>,
    /// The Quick Add entry whose shortcut is being recorded: the next chord is it.
    recording: Option<usize>,
    /// The built-in shortcut being rebound: the next chord is its keys.
    recording_built_in: Option<usize>,
    /// The vault's Sage settings, from `.sage/settings.json`.
    settings: sage_editor::settings::Settings,
    /// Property types from Obsidian's `types.json`, read once at start.
    obsidian_types: std::collections::BTreeMap<String, String>,
    /// The settings panel is shown in place of the note.
    settings_open: bool,
    /// When the settings last changed, while that is not saved yet.
    settings_changed: Option<Instant>,
    /// Why the settings could not be read or saved. A file that could not
    /// be read is never overwritten.
    settings_error: Option<String>,
    /// Closing was asked for and stopped by notes that could not be saved;
    /// asking again closes anyway.
    closing: bool,
    /// The date views last ran on, `YYYY-MM-DD`.
    day: String,
    /// The timeline panel's kind filter; `None` shows every kind.
    timeline_kind: Option<&'static str>,
    /// The active note's preview or outline waits for typing to pause, or
    /// its views for `PREVIEWS_EVERY` to pass; a clock comes back for them.
    settling: bool,
    /// The left button went down in the source editor and has not come up.
    /// iced's editor keeps dragging until it sees the release itself, so one
    /// it missed (taken by an overlay, or let go outside the window) would
    /// turn every later move of the pointer into a selection.
    left_down: bool,
    /// Obsidian's Templates settings, which Sage's own may override.
    obsidian_templates: TemplateConfig,
    indexer: Option<IndexerLink>,
    /// The vault lies in a OneDrive folder: conflict copies are looked for.
    onedrive: bool,
    /// A stat of the open notes is under way (polled vaults).
    polling: bool,
    /// A rename waiting for the index to catch up with the disk, by sync id.
    pending_rename: Option<(u64, Move)>,
    /// A rename, or the undoing of one, under way off the UI thread. Notes it
    /// moves are not saved until it is done: a save would write the path the
    /// note is being moved away from. See `rename_holds`.
    renaming: Option<Move>,
    /// When views, task lists and documents last ran again for a change in
    /// the index. Bursts of changes run them at most every `PREVIEWS_EVERY`.
    previews_run: Instant,
    next_sync: u64,
    status: String,
    /// Index progress leaves `status` alone until then; see `notice`.
    status_until: Instant,
    /// One level of list indentation, as Obsidian is set to write it.
    indent_unit: String,
    /// Notes in the order they were opened, for Back and Forward, and where
    /// in that list the active note is.
    history: Vec<String>,
    history_at: usize,
    /// Notes opened lately, newest first.
    recent: Vec<String>,
    /// Tabs closed this session, last closed last.
    closed: Vec<String>,
    /// The sidebar shows the active note's headings instead of the tree.
    outline_open: bool,
    /// Only the note: no tree, tabs or status bar.
    focus: bool,
    /// The interface's size, 1.0 as designed; Ctrl+= and Ctrl+- change it.
    zoom: f32,
}

/// The UI's share of the indexer handle. The indexer stops once the
/// subscription and the app have both let go of it.
#[derive(Clone)]
struct IndexerLink(Arc<sage_indexer::IndexerHandle>);

impl std::fmt::Debug for IndexerLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IndexerLink")
    }
}

/// A file's modification time in nanoseconds and its size: what changes when
/// something else writes it.
type Stamp = (i64, u64);

/// A rename asked for: a note, or a folder with everything in it.
#[derive(Debug, Clone)]
struct Move {
    from: String,
    to: String,
    folder: bool,
}

/// A merge waiting for the user's choices, shown in place of the editor.
struct Resolve {
    source: ResolveSource,
    merge: Merge,
    /// One per conflict, `None` until chosen.
    choices: Vec<Option<Choice>>,
}

enum ResolveSource {
    /// The note changed on disk under unsaved edits. Its new text becomes the base.
    Disk { theirs: String },
    /// A OneDrive conflict copy, moved to .trash once merged in.
    Copy { path: String },
}

pub(crate) struct FileRow {
    pub path: String,
    pub folder: String,
    pub title: String,
    /// A OneDrive placeholder: its content is still in the cloud.
    pub cloud: bool,
    /// Unix seconds.
    pub modified: i64,
}

struct Tab {
    /// Never shared with another tab, and kept for the tab's life whatever
    /// note it shows; see [`unique`].
    id: u64,
    path: String,
    title: String,
    content: text_editor::Content,
    /// False until the first read finishes; the tab shows a placeholder.
    loaded: bool,
    /// The file's text as last read from or written to disk. Saves are refused
    /// when the disk no longer matches it, and step 5 uses it as the merge base.
    base: String,
    dirty: bool,
    /// The disk changed under unsaved edits. Autosave stops until resolved.
    conflict: bool,
    saving: bool,
    /// Incremented on every edit; tells a finished save whether newer edits exist.
    edit_seq: u64,
    /// Names the text in `content`, new whenever it changes, read from disk
    /// or edited; live preview lays out again only when it differs.
    revision: u64,
    last_edit: Instant,
    /// The parsed note, rebuilt on edit instead of on every frame.
    preview: Vec<sage_markdown::Block>,
    /// The blocks live preview draws rendered, rebuilt with `preview`.
    islands: Vec<live::Island>,
    /// Results of the note's view blocks, by block source. Cleared whenever
    /// the index changes.
    views: HashMap<String, Result<Outcome, String>>,
    /// The note's embeds, and those of the notes it embeds, by `preview::embed_key`.
    embeds: HashMap<String, Embedded>,
    /// What the documents its links and embeds name are, by link key
    /// (`sage_core::target_key`): a task shows its status beside the link
    /// and as a card where embedded. `None` for a plain note or a missing one.
    /// Cleared whenever the index changes, so they follow the task's file.
    docs: HashMap<String, Option<sage_query::documents::Document>>,
    /// Results of the note's `tasks` blocks, by block source, cleared whenever
    /// the index changes.
    task_lists: HashMap<String, Result<sage_query::documents::TaskList, String>>,
    /// The preview changed; views, embeds and documents have to catch up.
    /// Only the active tab catches up; the others wait until they are shown.
    stale: bool,
    /// The text changed since `preview`, `islands` and `outline` were each
    /// parsed from it; see `Tab::catch_up`.
    unparsed: bool,
    islands_stale: bool,
    outline_stale: bool,
    /// How long the preview's last parse took, and when it was.
    parse_cost: Duration,
    parsed_at: Instant,
    /// The index changed; backlinks and outgoing links have to be read again,
    /// once the tab is shown.
    links_stale: bool,
    backlinks: Vec<(String, String)>,
    /// Open `#followup` tasks elsewhere in the vault that link to this note.
    followups: Vec<tasks::TaskRow>,
    /// Dated log lines in this note, or linking to it.
    timeline: Vec<timeline::Entry>,
    outgoing: Vec<Outgoing>,
    /// Where the cursor goes once the first read finishes.
    jump: Option<Jump>,
    /// What the last stat of the file saw (polled vaults).
    disk_stamp: Option<Stamp>,
    /// The disk changed while a save was under way; look again once it is done.
    reread: bool,
    merge: Option<Resolve>,
    /// OneDrive conflict copies of this note.
    copies: Vec<String>,
    /// The note's headings for the outline: level, text and line.
    outline: Vec<(u8, String, usize)>,
    /// Words and characters in the note, for the status bar, and the
    /// `revision` they were counted for. Counted again once typing pauses
    /// (`catch_up`), not on every key.
    counts: (usize, usize),
    counted: u64,
    /// The index changed; views, task lists and documents run again once the
    /// tab is shown, at most every `PREVIEWS_EVERY`.
    index_stale: bool,
    /// The last timeline read asked for, by `unique` number; an older one
    /// that comes back later is dropped.
    timeline_asked: u64,
    /// Undo and redo for this note's edits.
    history: sage_editor::undo::History,
    /// This is the task list's tab, not a note.
    tasks: bool,
}

impl Tab {
    fn placeholder(path: String, title: String) -> Self {
        Self {
            id: unique(),
            path,
            title,
            content: text_editor::Content::new(),
            loaded: false,
            base: String::new(),
            dirty: false,
            conflict: false,
            saving: false,
            edit_seq: 0,
            revision: live_editor::next_revision(),
            last_edit: Instant::now(),
            preview: Vec::new(),
            islands: Vec::new(),
            views: HashMap::new(),
            embeds: HashMap::new(),
            docs: HashMap::new(),
            task_lists: HashMap::new(),
            stale: false,
            unparsed: false,
            islands_stale: false,
            outline_stale: false,
            parse_cost: Duration::ZERO,
            parsed_at: Instant::now(),
            links_stale: false,
            backlinks: Vec::new(),
            followups: Vec::new(),
            timeline: Vec::new(),
            outgoing: Vec::new(),
            jump: None,
            disk_stamp: None,
            reread: false,
            merge: None,
            copies: Vec::new(),
            outline: Vec::new(),
            counts: (0, 0),
            counted: 0,
            index_stale: false,
            timeline_asked: 0,
            history: sage_editor::undo::History::default(),
            tasks: false,
        }
    }

    /// A tab that shows no note yet, only the choice of what to open. Its
    /// path is empty.
    fn blank() -> Self {
        Self::placeholder(String::new(), "New tab".to_owned())
    }

    /// The tab the task list is shown in. Like a blank tab its path is empty.
    fn tasks() -> Self {
        Self { tasks: true, ..Self::placeholder(String::new(), "Tasks".to_owned()) }
    }

    fn is_blank(&self) -> bool {
        self.path.is_empty() && !self.tasks
    }

    fn snapshot(&self) -> sage_editor::undo::Snapshot {
        snapshot_of(&self.content)
    }

    /// Remembers the text as it is, just before an edit, for undo.
    fn checkpoint(&mut self, kind: sage_editor::undo::Kind) {
        let content = &self.content;
        self.history.record(|| snapshot_of(content), kind, Instant::now());
    }

    /// Undo, or redo when `forward`. False when there was nothing to do.
    fn step_history(&mut self, forward: bool) -> bool {
        let now = self.snapshot();
        let step = if forward { self.history.redo(now) } else { self.history.undo(now) };
        let Some(step) = step else { return false };
        self.content.perform(text_editor::Action::SelectAll);
        self.content.perform(text_editor::Action::Edit(Edit::Paste(Arc::new(step.text))));
        self.put_cursor(step.line, step.col);
        self.mark_edited();
        true
    }

    /// Takes a merge's result into the editor, keeping the cursor on the same
    /// text. `base` is the text on disk now; the tab stays dirty while the
    /// merged text differs from it, so autosave writes the merge.
    fn take_merge(&mut self, merge: &Merge, choices: &[Choice], base: String) {
        let cursor = self.content.cursor().position;
        let line = merge.ours_line(cursor.line, choices);
        let merged = merge.resolve(choices);
        let dirty = merged != base;
        self.set_text(merged);
        self.base = base;
        self.merge = None;
        if dirty {
            self.dirty = true;
            self.edit_seq += 1;
            // Saved on the next autosave tick rather than after a quiet period.
            self.last_edit = Instant::now().checked_sub(AUTOSAVE_AFTER).unwrap_or_else(Instant::now);
        }
        self.put_cursor(line, cursor.column);
    }

    fn set_text(&mut self, text: String) {
        // Text from disk or a merge: earlier steps no longer apply to it.
        self.history.clear();
        self.content = text_editor::Content::with_text(&text);
        self.revision = live_editor::next_revision();
        let started = Instant::now();
        self.preview = sage_markdown::parse(&text);
        (self.parse_cost, self.parsed_at) = (started.elapsed(), Instant::now());
        self.islands = live::islands(&text, std::mem::take(&mut self.islands));
        self.outline = outline(&text);
        self.counts = counts(&text);
        self.counted = self.revision;
        self.stale = true;
        self.unparsed = false;
        self.islands_stale = false;
        self.outline_stale = false;
        self.base = text;
        self.loaded = true;
        self.dirty = false;
        self.conflict = false;
    }

    fn mark_edited(&mut self) {
        self.dirty = true;
        self.edit_seq += 1;
        self.revision = live_editor::next_revision();
        self.last_edit = Instant::now();
        self.unparsed = true;
        self.islands_stale = true;
        self.outline_stale = true;
    }

    /// Parses the text again for what is on screen: the preview when a pane
    /// or live preview's blocks show it, the islands in live preview, the
    /// outline when its panel is open. Typing in the source pane of a long
    /// note would otherwise parse it three times on every key, about 30 ms
    /// at 200 KB.
    ///
    /// Islands say which lines live preview draws as blocks, so they follow
    /// the text at once. The preview and outline of a note that is slow to
    /// parse wait while typing goes on (`parse_waits`). Returns whether
    /// something is still waiting.
    ///
    /// The words and characters in the status bar are counted once typing
    /// pauses, in any note: counting is a pass over the whole text, copied
    /// out of the editor for it, which is not worth doing on every key. They
    /// need no clock of their own: an edit leaves the note unsaved, and the
    /// autosave tick comes by twice a second until it is saved.
    fn catch_up(&mut self, preview: bool, live: bool, outline_open: bool) -> bool {
        let islands = live && self.islands_stale;
        let waits = parse_waits(self.parse_cost, self.last_edit.elapsed(), self.parsed_at.elapsed());
        let preview = preview && self.unparsed;
        let outline_open = outline_open && self.outline_stale;
        let uncounted = self.loaded && self.counted != self.revision;
        let count = uncounted && self.last_edit.elapsed() >= PARSE_SETTLE;
        let parse = !waits && (preview || outline_open);
        let waiting = waits && (preview || outline_open);
        if !islands && !parse && !count {
            return waiting;
        }
        // One copy of the text for everything that needs it.
        let text = self.content.text();
        if islands {
            self.islands = live::islands(&text, std::mem::take(&mut self.islands));
            self.islands_stale = false;
        }
        if count {
            self.counts = counts(&text);
            self.counted = self.revision;
        }
        if !parse {
            return waiting;
        }
        if preview {
            let started = Instant::now();
            self.preview = sage_markdown::parse(&text);
            (self.parse_cost, self.parsed_at) = (started.elapsed(), Instant::now());
            self.unparsed = false;
            self.stale = true;
        }
        if outline_open {
            self.outline = outline(&text);
            self.outline_stale = false;
        }
        waiting
    }

    /// Replaces one line of the text in the editor, leaving the cursor where it
    /// was, as an edit that autosave then writes.
    fn replace_line(&mut self, line: usize, text: &str) {
        let Some(old) = self.content.line(line).map(|l| l.text.len()) else { return };
        self.checkpoint(sage_editor::undo::Kind::Other);
        let cursor = self.content.cursor().position;
        self.content.move_to(Cursor {
            position: Position { line, column: old },
            selection: Some(Position { line, column: 0 }),
        });
        self.content.perform(text_editor::Action::Edit(Edit::Paste(Arc::new(text.to_owned()))));
        self.put_cursor(cursor.line, cursor.column);
        self.mark_edited();
    }

    /// The frontmatter's own lines, between the `---` fences, with the line
    /// the first of them is on, or `None` when the note has none.
    fn frontmatter_lines(&self) -> Option<Vec<String>> {
        frontmatter_in(|i| self.content.line(i).map(|l| l.text.into_owned()))
    }

    /// Entry `index` of the frontmatter.
    fn property(&self, index: usize) -> Option<sage_editor::properties::Entry> {
        sage_editor::properties::parse(&self.frontmatter_lines()?).into_iter().nth(index)
    }

    /// Appends an empty property called `key` to the frontmatter, as a list
    /// for the names that hold lists. Gives its index, or `None` when the
    /// name is unusable or taken, or the note has no frontmatter.
    fn add_property(&mut self, key: &str) -> Option<usize> {
        use sage_editor::properties::{parse, write, Value};
        if key.is_empty() || key.contains([':', '#', '[', ']', '"', '\'']) {
            return None;
        }
        let inner = self.frontmatter_lines()?;
        let entries = parse(&inner);
        if entries.iter().any(|e| e.key.eq_ignore_ascii_case(key)) {
            return None;
        }
        let value = if matches!(key.to_lowercase().as_str(), "tags" | "aliases" | "cssclasses") || is_link_property(key) {
            Value::List { items: vec![String::new()], block: false }
        } else {
            Value::Text { text: String::new(), quote: None }
        };
        let fence = inner.len() + 1;
        let text = self.with_ending(&format!("{}\n", write(key, &value).join("\n")));
        self.checkpoint(sage_editor::undo::Kind::Other);
        let cursor = self.content.cursor().position;
        self.content.move_to(Cursor { position: Position { line: fence, column: 0 }, selection: None });
        self.content.perform(text_editor::Action::Edit(Edit::Paste(Arc::new(text))));
        let line = if cursor.line >= fence { cursor.line + 1 } else { cursor.line };
        self.put_cursor(line, cursor.column);
        self.mark_edited();
        Some(entries.len())
    }

    /// Removes entry `index` from the frontmatter, all its lines, as one edit.
    fn remove_property(&mut self, index: usize) {
        let Some(inner) = self.frontmatter_lines() else { return };
        let Some(entry) = sage_editor::properties::parse(&inner).into_iter().nth(index) else { return };
        // Within the note, past the opening fence; the closing fence always follows.
        let (first, after) = (entry.lines.start + 1, entry.lines.end + 1);
        self.checkpoint(sage_editor::undo::Kind::Other);
        let cursor = self.content.cursor().position;
        self.content.move_to(Cursor {
            position: Position { line: after, column: 0 },
            selection: Some(Position { line: first, column: 0 }),
        });
        self.content.perform(text_editor::Action::Edit(Edit::Delete));
        let gone = after - first;
        let line = if cursor.line >= after { cursor.line - gone } else if cursor.line >= first { first } else { cursor.line };
        self.put_cursor(line, if cursor.line >= after { cursor.column } else { 0 });
        self.mark_edited();
    }

    /// Sets entry `index` of the frontmatter from the properties widget as
    /// an edit that autosave then writes, leaving the cursor on its text.
    fn set_property(&mut self, index: usize, value: &sage_editor::properties::Value) {
        let Some(inner) = self.frontmatter_lines() else { return };
        let Some((range, new)) = sage_editor::properties::set(&inner, index, value) else { return };
        // Within the note, past the opening fence.
        let (first, last) = (range.start + 1, range.end);
        let Some(end) = self.content.line(last).map(|l| l.text.len()) else { return };
        let delta = new.len() as isize - range.len() as isize;
        self.checkpoint(sage_editor::undo::Kind::Typing);
        let cursor = self.content.cursor().position;
        self.content.move_to(Cursor {
            position: Position { line: last, column: end },
            selection: Some(Position { line: first, column: 0 }),
        });
        let text = self.with_ending(&new.join("\n"));
        self.content.perform(text_editor::Action::Edit(Edit::Paste(Arc::new(text))));
        let line = if cursor.line > last { (cursor.line as isize + delta).max(0) as usize } else { cursor.line };
        self.put_cursor(line, cursor.column);
        self.mark_edited();
    }

    /// The note's line ending, as the file on disk has it.
    fn ending(&self) -> &'static str {
        if self.base.contains("\r\n") { "\r\n" } else { "\n" }
    }

    /// `text` with the note's line endings. The editor keeps each line's own
    /// ending, so text pasted with `\n` into a CRLF note would mix the two.
    fn with_ending(&self, text: &str) -> String {
        with_ending(text, self.ending())
    }

    /// Pastes `text` at the cursor as an undo step of its own, in the note's
    /// line endings and never into the frontmatter: a cursor there goes to
    /// the first line after it.
    fn insert_at_cursor(&mut self, text: &str) {
        self.checkpoint(sage_editor::undo::Kind::Other);
        let fm_lines = sage_core::split_frontmatter(&self.content.text()).2 as usize;
        if self.content.cursor().position.line < fm_lines {
            place(&mut self.content, Cursor { position: Position { line: fm_lines, column: 0 }, selection: None });
        }
        let text = self.with_ending(text);
        self.content.perform(text_editor::Action::Edit(Edit::Paste(Arc::new(text))));
        self.mark_edited();
    }

    /// The selection as the editing commands see it: anchor and cursor.
    fn selection(&self) -> (Pos, Pos) {
        let cursor = self.content.cursor();
        let head = Pos::new(cursor.position.line, cursor.position.column);
        let anchor = cursor.selection.map_or(head, |p| Pos::new(p.line, p.column));
        (anchor, head)
    }

    /// Makes an editing command's change: its lines replaced, then its
    /// selection, as an edit that autosave then writes.
    fn apply(&mut self, change: edit::Change) {
        self.checkpoint(sage_editor::undo::Kind::Other);
        let end = self.content.line(change.last).map_or(0, |l| l.text.len());
        self.content.move_to(Cursor {
            position: Position { line: change.last, column: end },
            selection: Some(Position { line: change.first, column: 0 }),
        });
        self.content.perform(text_editor::Action::Edit(Edit::Paste(Arc::new(change.text))));
        let at = |p: Pos| Position { line: p.line, column: p.col };
        place(&mut self.content, Cursor {
            position: at(change.head),
            selection: (change.anchor != change.head).then(|| at(change.anchor)),
        });
        self.mark_edited();
    }

    fn wants_autosave(&self) -> bool {
        self.loaded && self.dirty && !self.conflict && !self.saving && self.last_edit.elapsed() >= AUTOSAVE_AFTER
    }

    /// Unsaved, being saved, or in conflict: text the app must not replace.
    fn busy(&self) -> bool {
        self.dirty || self.saving || self.conflict
    }

    /// Takes text the app itself just wrote to this note's file, keeping the
    /// cursor. A tab with edits of its own keeps them; its next save then meets
    /// the conflict guard.
    fn follow_disk(&mut self, text: String) {
        if self.loaded && !self.busy() {
            let cursor = self.content.cursor().position;
            self.set_text(text);
            self.put_cursor(cursor.line, cursor.column);
        }
    }

    /// Moves the cursor, clamped to the text; the editor scrolls to it.
    fn put_cursor(&mut self, line: usize, column: usize) {
        let line = line.min(self.content.line_count().saturating_sub(1));
        let text = self.content.line(line).map(|l| l.text.into_owned()).unwrap_or_default();
        let mut column = column.min(text.len());
        while !text.is_char_boundary(column) {
            column -= 1;
        }
        place(&mut self.content, Cursor { position: Position { line, column }, selection: None });
    }

    /// Puts the cursor on a line, heading or block, now or once loaded.
    fn jump_to(&mut self, jump: Jump) {
        if !self.loaded {
            self.jump = Some(jump);
            return;
        }
        let text = self.content.text();
        let line = match &jump {
            Jump::Line(line) => Some(*line),
            Jump::Heading(heading) => {
                // `[[Note#Part#Section]]` names the last heading; punctuation
                // and case do not matter, as in Obsidian.
                let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
                let wanted = norm(heading.rsplit('#').next().unwrap_or(heading));
                sage_core::parse(&text).headings.iter().find(|h| norm(&h.text) == wanted).map(|h| h.line as usize)
            }
            Jump::Block(id) => text.lines().position(|l| l.trim_end().ends_with(&format!("^{id}"))),
        };
        if let Some(line) = line {
            self.put_cursor(line, 0);
        }
    }
}

#[derive(Debug, Clone)]
enum Jump {
    Line(usize),
    Heading(String),
    Block(String),
}

/// A link in the preview.
#[derive(Debug, Clone)]
enum LinkTarget {
    /// A note, followed relative to the active note.
    Note { target: String, heading: Option<String>, block: Option<String> },
    /// A web or mail link, opened by the desktop.
    Url(String),
}

/// A task line as the index last saw it. Write-back checks `raw` first.
#[derive(Debug, Clone, PartialEq)]
struct TaskRef {
    path: String,
    line: u32,
    raw: String,
    /// A task document (`type: task`) rather than a line: `line` is 0, and
    /// `raw` is its `status` as the index read it, empty for none.
    document: bool,
}

/// One entry of the "Links to" list under the preview.
struct Outgoing {
    label: String,
    kind: OutKind,
}

enum OutKind {
    Note(String),
    /// Unresolved: following it creates the note, as in Obsidian.
    Missing(String),
    Attachment,
}

struct Completion {
    ctx: complete::Context,
    line: usize,
    items: Vec<Suggestion>,
    selected: usize,
}

/// Suggestions for a field of the properties widget, shown under its row.
struct PropCompletion {
    /// The note the field belongs to.
    tab: String,
    entry: usize,
    /// Which item of a list, or `None` for a text value.
    item: Option<usize>,
    ctx: complete::Context,
    items: Vec<Suggestion>,
    selected: usize,
}

/// The properties whose values are links to notes: typing in one suggests
/// notes as `[[` does, and what is picked or typed is written as `[[Note]]`.
fn is_link_property(key: &str) -> bool {
    key.eq_ignore_ascii_case("links")
}

/// Find in the active note. Closed when another tab becomes active.
#[derive(Default)]
struct Find {
    query: String,
    matches: Vec<(usize, Range<usize>)>,
    current: Option<usize>,
}

#[derive(Default)]
struct Palette {
    mode: PaletteMode,
    query: String,
    hits: Vec<Hit>,
    selected: usize,
    /// The project or people notes a flow picks from, as `(name, path)`,
    /// read once as it opens rather than on every key (`flow::open_flow`).
    kind_notes: Vec<(String, String)>,
}

#[derive(Default)]
enum PaletteMode {
    /// Notes by name and content, or commands after `>`.
    #[default]
    Search,
    /// The query is the new path for the note at this path.
    Rename(String),
    /// The query is the new path for the folder at this path.
    RenameFolder(String),
    /// The query filters the folders the note at this path can move to.
    MoveTo(String),
    Template(TemplateConfig),
    /// The Quick Add entries, filtered by the query.
    QuickAdd,
    /// The query is the value for the Quick Add entry with this index.
    QuickAddValue(usize),
    /// The query is the name of a note to create.
    NewNote,
    /// The query is the path of a folder to create.
    NewFolder,
    /// Fuzzy search over every note's metadata and text, read off the UI
    /// thread when it opened; `None` until it has been.
    Everywhere(Option<Arc<omni::Corpus>>),
    /// The query filters the projects to do something with.
    Project(flow::Then),
    /// The query is what happened, as `kind: text`, for the project at this path.
    LogEntry { project: String, source: Option<flow::Line> },
    /// The query filters the loose lines of the daily note.
    Triage(Vec<flow::Line>),
    /// What to make of a line.
    TriageAction(flow::Line),
    /// The query filters the people a line is for.
    Person(flow::Line, sage_editor::log::Triage),
}

struct Hit {
    label: String,
    detail: String,
    action: HitAction,
}

#[derive(Clone)]
enum HitAction {
    Open(String),
    /// Opens the note at this 0-based line.
    OpenAt(String, usize),
    Create(String),
    CreateFolder(String),
    Run(Command),
    Rename(String),
    RenameFolder(String),
    InsertTemplate(String),
    /// Runs the Quick Add entry with this index, asking for a value first if it needs one.
    QuickAdd(usize),
    /// Runs the Quick Add entry with this index with the query as its value.
    QuickAddRun(usize),
    /// A step of a workflow in `flow`.
    Flow(flow::Step),
    /// An explanation rather than a choice: Enter keeps the palette open.
    Nothing,
}

impl App {
    fn boot(vault: PathBuf, db_path: PathBuf) -> (Self, Task<Message>) {
        let session = load_session(&db_path);
        let zoom = session.zoom();
        let kanban_order = order::Order::load(db_path.with_file_name("kanban-order.json"));

        let mut app = Self {
            reader: sage_db::open_reader(&db_path).ok(),
            vault,
            db_path,
            backend: None,
            system_dark: false,
            dark_override: session.dark_override,
            files: Vec::new(),
            disk_folders: Vec::new(),
            note_folders: Vec::new(),
            tree_folders: Vec::new(),
            scanning: false,
            rescan: false,
            expanded: session.expanded.into_iter().collect(),
            tree_order: session.tree_order,
            tree_order_menu: false,
            tabs: Vec::new(),
            active: 0,
            view_mode: session.view_mode,
            before_live: ViewMode::Source,
            palette: None,
            completion: None,
            prop_complete: None,
            table_draft: None,
            new_property: None,
            type_menu: None,
            chip_hover: None,
            editing_prop: None,
            dismissed: None,
            escape_used: false,
            find: None,
            tasks: None,
            reopen: HashMap::new(),
            renames: Vec::new(),
            modifiers: keyboard::Modifiers::default(),
            drag: None,
            order: kanban_order,
            folds: session.folds,
            panes: session.panes.unwrap_or_default(),
            resizing: None,
            window_width: WINDOW_SIZE.0,
            menu: None,
            menu_at: iced::Point::ORIGIN,
            tab_menu: None,
            tabs_shown: (usize::MAX, 0),
            startup_done: false,
            indexed: false,
            doomed: None,
            help: false,
            shortcut_filter: String::new(),
            toast: None,
            reminders: Vec::new(),
            reminders_checked: session
                .reminders_checked
                .as_deref()
                .and_then(|t| chrono::NaiveDateTime::parse_from_str(t, CHECKED_FORMAT).ok()),
            reminders_caught_up: false,
            pinned: session.pinned.clone(),
            task_form: None,
            recording: None,
            recording_built_in: None,
            settings: sage_editor::settings::Settings::default(),
            obsidian_types: Default::default(),
            settings_open: false,
            settings_changed: None,
            settings_error: None,
            closing: false,
            day: sage_editor::today(),
            timeline_kind: None,
            settling: false,
            left_down: false,
            obsidian_templates: TemplateConfig::default(),
            indexer: None,
            onedrive: false,
            polling: false,
            pending_rename: None,
            renaming: None,
            previews_run: Instant::now(),
            next_sync: 0,
            status: "Opening index".into(),
            status_until: Instant::now(),
            indent_unit: String::new(),
            history: Vec::new(),
            history_at: 0,
            recent: session.recent,
            closed: Vec::new(),
            outline_open: session.outline,
            focus: false,
            zoom,
        };
        app.indent_unit = sage_editor::obsidian::indent_unit(&app.vault);
        app.obsidian_types = sage_editor::obsidian::property_types(&app.vault);
        // The interface's units are scaled by the zoom.
        app.window_width /= app.zoom;
        // Instant start: the tree is drawn from the cached index before the
        // indexer has looked at a single file.
        app.reload_files();

        // Restore tabs in their saved order. Each starts as a placeholder and
        // fills in when its read completes.
        let mut tasks =
            vec![iced::system::theme().map(Message::SystemTheme), load_settings(app.vault.clone()), app.scan_folders()];
        for path in session.tabs {
            if path.is_empty() {
                app.tabs.push(Tab::blank());
                continue;
            }
            let title = app.title_of(&path);
            let tab = Tab::placeholder(path, title);
            tasks.push(app.read(tab.id, &tab.path));
            app.tabs.push(tab);
        }
        app.active = session.active.min(app.tabs.len().saturating_sub(1));
        (app, Task::batch(tasks))
    }

    fn title(&self) -> String {
        let vault = self.vault.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        match self.tabs.get(self.active) {
            Some(tab) => format!("{} · {vault} · Sage", tab.title),
            None => format!("{vault} · Sage"),
        }
    }

    fn dark(&self) -> bool {
        self.dark_override.unwrap_or(self.system_dark)
    }

    fn tokens(&self) -> Tokens {
        Tokens::of(self.dark())
    }

    fn theme(&self) -> Theme {
        self.tokens().iced_theme(self.dark())
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            Subscription::run_with((self.vault.clone(), self.db_path.clone()), indexer_events),
            iced::system::theme_changes().map(Message::SystemTheme),
            iced::event::listen_with(shortcuts),
            iced::event::listen_with(mouse_up),
            iced::event::listen_with(window_focus),
            iced::event::listen_with(window_width),
            iced::event::listen_with(track_pointer),
            iced::event::listen_with(side_buttons),
            iced::event::listen_with(file_dropped),
            iced::window::close_requests().map(|_| Message::CloseRequested),
            iced::time::every(Duration::from_secs(60)).map(|_| Message::Clock),
        ];
        if self.toast.is_some() {
            subscriptions.push(iced::time::every(Duration::from_secs(1)).map(|_| Message::ToastTick));
        }
        if self.settling {
            subscriptions.push(iced::time::every(Duration::from_millis(100)).map(|_| Message::Settle));
        }
        // The pointer is followed only while a divider is dragged.
        if self.resizing.is_some() {
            subscriptions.push(iced::event::listen_with(pointer_x));
        }
        // The autosave clock only runs while something is waiting to be saved.
        if self.tabs.iter().any(|t| t.dirty && !t.conflict) || self.settings_changed.is_some() {
            subscriptions.push(iced::time::every(Duration::from_millis(500)).map(|_| Message::Tick));
        }
        // On a polled vault the indexer looks only every 45 s; open notes are
        // checked far more often, so an edit made elsewhere shows up quickly.
        if self.backend == Some(Backend::Poll) && self.tabs.iter().any(|t| t.loaded) {
            subscriptions.push(iced::time::every(TAB_POLL).map(|_| Message::PollTabs));
        }
        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        Task::batch([task, self.sync_previews(), self.show_current_tab()])
    }

    /// Scrolls the tab bar so the current tab is in view, when the current
    /// tab or the number of tabs changed. Tab widths are estimated from their
    /// titles, as the bar's content size is not known here.
    fn show_current_tab(&mut self) -> Task<Message> {
        let now = (self.active, self.tabs.len());
        if now == self.tabs_shown || self.tabs.is_empty() {
            return Task::none();
        }
        self.tabs_shown = now;
        let width = |tab: &Tab| tab.title.chars().count() as f32 * 7.5 + 70.0;
        let start: f32 = self.tabs.iter().take(self.active).map(width).sum();
        let own = self.tabs.get(self.active).map_or(0.0, width);
        let sidebar = if self.focus { 0.0 } else { self.sidebar_width() + DIVIDER };
        let room = (self.window_width - sidebar).max(100.0);
        let x = (start + own / 2.0 - room / 2.0).max(0.0);
        iced::widget::operation::scroll_to(TABS, iced::widget::operation::AbsoluteOffset { x: Some(x), y: None })
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Index(event) => return self.on_index(event),
            Message::IndexerReady(link) => self.indexer = Some(link),
            Message::WindowUnfocused => self.left_down = false,
            Message::WindowFocused => {
                // Coming back from Obsidian or Explorer: look at the disk now
                // instead of at the next scheduled pass.
                if self.backend == Some(Backend::Poll) {
                    if let Some(IndexerLink(handle)) = &self.indexer {
                        handle.rescan();
                    }
                    return self.poll_tabs();
                }
            }
            Message::PollTabs => return self.poll_tabs(),
            Message::Polled(stamps) => return self.on_polled(stamps),
            Message::Choose(n, choice) => {
                if let Some(slot) = self.active_resolve().and_then(|r| r.choices.get_mut(n)) {
                    *slot = Some(choice);
                }
            }
            Message::ChooseAll(choice) => {
                if let Some(resolve) = self.active_resolve() {
                    resolve.choices.iter_mut().for_each(|slot| *slot = Some(choice));
                }
            }
            Message::ApplyResolve => return self.apply_resolve(),
            Message::CancelResolve => {
                // Only a conflict copy can wait; a disk merge has to be decided.
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    if matches!(tab.merge, Some(Resolve { source: ResolveSource::Copy { .. }, .. })) {
                        tab.merge = None;
                    }
                }
            }
            Message::CompareCopy(copy) => {
                let Some(tab) = self.tabs.get(self.active).filter(|t| t.loaded && t.merge.is_none()) else {
                    return Task::none();
                };
                let (tab, abs) = (tab.id, self.vault.join(&copy));
                return Task::perform(
                    async move { tokio::fs::read_to_string(&abs).await.map_err(|e| e.to_string()) },
                    move |result| Message::CopyLoaded { tab, copy: copy.clone(), result },
                );
            }
            Message::CopyLoaded { tab, copy, result: Ok(text) } => return self.compare_copy(tab, copy, &text),
            Message::CopyLoaded { copy, result: Err(error), .. } => self.notice(format!("Could not read {copy}: {error}")),
            Message::SystemTheme(mode) => self.system_dark = mode == iced::theme::Mode::Dark,
            Message::Open(path) => {
                self.menu = None;
                return self.open(path);
            }
            Message::OpenAt(path, line) => return self.open_at(path, Some(Jump::Line(line))),
            Message::FollowLink(link) => return self.follow(link),
            Message::Loaded { tab, path, result } => return self.on_loaded(tab, path, result),
            Message::Saved { tab, path, seq, result } => return self.on_saved(tab, path, seq, result),
            Message::Created(Ok(path)) => return self.open(path),
            Message::Created(Err(error)) => self.notice(format!("Could not create the note: {error}")),
            Message::StarterCreated(Ok(written)) => {
                let kept = match written.kept {
                    0 => String::new(),
                    n => format!(", {n} already there were kept"),
                };
                self.notice(format!("Starter vault: {} files written{kept}", written.created));
                // The settings file on disk is now the truth, Quick Add shortcuts included.
                self.settings_changed = None;
                let open = self.open(sage_editor::starter::FIRST_NOTE.to_owned());
                return Task::batch([load_settings(self.vault.clone()), self.scan_folders(), open]);
            }
            Message::StarterCreated(Err(error)) => self.notice(format!("Could not create the starter vault: {error}")),
            Message::Trashed(path, Ok(target)) => {
                // A folder takes the notes in it along.
                let inside = |p: &str| p == path || p.strip_prefix(&path).is_some_and(|rest| rest.starts_with('/'));
                // A tab that got edits while the move was under way keeps them:
                // it stays open as in conflict, so autosave does not keep
                // failing on a file that is gone. Force save writes it back.
                let kept: Vec<String> = self.tabs.iter().filter(|t| t.busy() && inside(&t.path)).map(|t| t.path.clone()).collect();
                let gone = |t: &Tab| inside(&t.path) && !t.busy();
                self.active = active_after(self.active, &self.tabs, gone);
                self.tabs.retain(|t| !gone(t));
                for tab in self.tabs.iter_mut().filter(|t| inside(&t.path)) {
                    tab.conflict = true;
                }
                self.folds.retain(|p, _| !inside(p) || kept.contains(p));
                self.find = None;
                let warning = kept.first().map(|first| format!("; {first} has unsaved edits and stays open, Force save writes it back"));
                self.notice(format!("Moved {path} to {target}{}", warning.as_deref().unwrap_or("")));
                if let Some(toast) = &mut self.toast {
                    toast.text = format!("Moved {path} to .trash{}", warning.as_deref().unwrap_or(""));
                    toast.undo = Some((path.clone(), target.clone()));
                }
                self.persist();
            }
            Message::Trashed(path, Err(error)) => self.notice(format!("Could not move {path}: {error}")),
            Message::Renamed(Ok(rename)) => {
                self.on_renamed(rename);
                return self.rename_done();
            }
            Message::Renamed(Err(error)) => {
                self.notice(format!("Not renamed: {error}"));
                return self.rename_done();
            }
            Message::RenameUndone(Ok((rename, restored))) => {
                self.on_rename_undone(rename, restored);
                return self.rename_done();
            }
            Message::RenameUndone(Err(error)) => {
                self.notice(format!("Could not undo the rename: {error}"));
                return self.rename_done();
            }
            Message::TreeMenu(path, folder) => {
                self.tab_menu = None;
                self.menu_at = pointer();
                self.menu = Some((path, folder)).filter(|item| self.menu.as_ref() != Some(item));
            }
            Message::TreeNewNote => return self.run(Command::NewNote),
            Message::TreeNewFolder => {
                self.menu = None;
                self.completion = None;
                self.palette = Some(Palette { mode: PaletteMode::NewFolder, ..Palette::default() });
                self.search();
                return iced::widget::operation::focus(PALETTE_INPUT);
            }
            Message::TreeOrderMenu => self.tree_order_menu = !self.tree_order_menu,
            Message::TreeOrder(order) => {
                self.tree_order = order;
                self.tree_order_menu = false;
                self.persist();
            }
            Message::TreeFoldAll => {
                if self.tree_folders.iter().any(|f| self.expanded.contains(f)) {
                    self.expanded.clear();
                } else {
                    self.expanded.extend(self.tree_folders.iter().cloned());
                }
                self.persist();
            }
            Message::DiskFolders(folders) => {
                self.scanning = false;
                self.disk_folders = folders;
                self.refresh_folders();
                if std::mem::take(&mut self.rescan) {
                    return self.scan_folders();
                }
            }
            Message::FolderCreated(Ok(folder)) => {
                for ancestor in tree::ancestors(&format!("{folder}/")) {
                    self.expanded.insert(ancestor.to_owned());
                }
                self.persist();
                self.notice(format!("Created folder {folder}"));
                return self.scan_folders();
            }
            Message::FolderCreated(Err(error)) => self.notice(format!("Could not create the folder: {error}")),
            Message::MenuRename => match self.menu.take() {
                Some((path, true)) => return self.start_folder_rename(path),
                Some((path, false)) => return self.start_rename(path),
                None => {}
            },
            Message::MenuDelete => {
                if let Some((path, folder)) = self.menu.take() {
                    self.ask_delete(path, folder);
                }
            }
            Message::ConfirmDelete => {
                // Asked again: a note may have been edited since the question came up.
                if let Some(Doomed { path, folder }) = self.doomed.take() {
                    if let Some(busy) = self.busy_inside(&path, folder) {
                        self.notice(format!("Save {busy} before deleting it"));
                        return Task::none();
                    }
                    return self.trash_file(path);
                }
            }
            Message::CancelDelete => self.doomed = None,
            Message::ResizeStart(divider) => self.resizing = Some(divider),
            Message::ResizeMove(x) => self.resize(x),
            Message::WindowWidth(width) => self.window_width = width,
            Message::UrlOpened(Ok(())) => {}
            Message::UrlOpened(Err(error)) => self.notice(error),
            Message::ToggleCallout(key, open) => {
                if let Some(tab) = self.tabs.get(self.active) {
                    self.folds.entry(tab.path.clone()).or_default().insert(key, open);
                    self.persist();
                }
            }
            Message::SettingsLoaded(Ok(settings)) => {
                self.settings = settings;
                self.apply_tracking();
                // Only the first read, as Sage starts, opens the note.
                if !std::mem::replace(&mut self.startup_done, true) {
                    if let Some(path) = self.settings.startup_note.clone().filter(|p| self.files.iter().any(|f| f.path == *p)) {
                        return self.open(path);
                    }
                }
            }
            Message::SettingsLoaded(Err(error)) => {
                self.notice(format!("Sage's settings were not read, and will not be saved: {error}"));
                self.settings_error = Some(error);
            }
            Message::ObsidianTemplates(config) => self.obsidian_templates = config,
            Message::SettingsSaved(Ok(())) => {}
            Message::SettingsSaved(Err(error)) => self.notice(format!("Settings not saved: {error}")),
            Message::ToggleSettings => {
                self.settings_open = !self.settings_open;
                if self.settings_open {
                    self.palette = None;
                    self.tasks = None;
                    self.find = None;
                    self.completion = None;
                } else {
                    return self.save_settings();
                }
            }
            Message::SettingsTemplatesFolder(pick) => {
                self.settings.templates_folder = (!pick.value.is_empty()).then_some(pick.value);
                self.settings_edited();
            }
            Message::SettingsNewNoteTemplate(pick) => {
                self.settings.new_note_template = (!pick.value.is_empty()).then_some(pick.value);
                self.settings_edited();
            }
            Message::SettingsStartupNote(path) => {
                self.settings.startup_note = path;
                self.settings_edited();
            }
            Message::TaskNotesClosed(closed) => {
                if let Some(first) = closed.first() {
                    let more = if closed.len() > 1 { format!(" and {} more", closed.len() - 1) } else { String::new() };
                    self.status = format!("Closed task note {first}{more}");
                    self.load_tasks();
                }
            }
            Message::SettingsTrackedOnly(on) => {
                self.settings.tracked_tasks_only = on;
                self.apply_tracking();
                self.settings_edited();
            }
            Message::SettingsQuickAddTemplates(on) => {
                self.settings.quick_add_templates = Some(on);
                self.settings_edited();
            }
            Message::QuickAddEdit(i, field) => {
                if let Some(entry) = self.settings.quick_add.get_mut(i) {
                    settings::apply(entry, field);
                    self.settings_edited();
                }
            }
            Message::QuickAddNew(capture) => {
                let entry = if capture { QuickAdd::new_capture() } else { QuickAdd::new_template() };
                self.settings.quick_add.push(entry);
                self.settings_edited();
            }
            Message::RecordShortcut(i) => {
                self.recording = (self.recording != Some(i)).then_some(i);
            }
            Message::ClearShortcut(i) => {
                if let Some(entry) = self.settings.quick_add.get_mut(i) {
                    entry.set_shortcut(None);
                    self.settings_edited();
                }
            }
            Message::Chord(chord) => return self.on_chord(chord),
            Message::SetTheme(choice) => {
                self.dark_override = choice.dark();
                self.persist();
            }
            Message::RecordBuiltIn(i) => {
                self.recording = None;
                self.recording_built_in = (self.recording_built_in != Some(i)).then_some(i);
            }
            Message::ClearBuiltIn(i) | Message::ResetBuiltIn(i) => {
                self.recording_built_in = None;
                if let Some(&(_, command)) = BUILT_IN.get(i) {
                    let name = command_name(command).to_owned();
                    if matches!(message, Message::ClearBuiltIn(_)) {
                        self.settings.shortcuts.insert(name, String::new());
                    } else {
                        self.settings.shortcuts.remove(&name);
                    }
                    self.settings_edited();
                }
            }
            Message::QuickAddMove(i, up) => {
                self.recording = None;
                let j = if up { i.checked_sub(1) } else { Some(i + 1) };
                if let Some(j) = j.filter(|&j| j < self.settings.quick_add.len() && i < self.settings.quick_add.len()) {
                    self.settings.quick_add.swap(i, j);
                    self.settings_edited();
                }
            }
            Message::QuickAddRemove(i) => {
                self.recording = None;
                if i < self.settings.quick_add.len() {
                    self.settings.quick_add.remove(i);
                    self.settings_edited();
                }
            }
            Message::QuickNoteDone { result: Ok((path, created)), open } => {
                if !created {
                    self.notice(format!("{path} exists already, so nothing was created"));
                } else if !open {
                    self.notice(format!("Created {path}"));
                }
                if open {
                    return self.open(path);
                }
            }
            Message::QuickNoteDone { result: Err(error), .. } => self.notice(format!("Quick Add: {error}")),
            Message::Captured { path, result: Ok(text) } => {
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.path == path) {
                    tab.follow_disk(text);
                }
                self.refresh_find(false);
                self.notice(format!("Added to {path}"));
            }
            Message::Captured { result: Err(error), .. } => self.notice(format!("Quick Add: {error}")),
            Message::Written { path, result, done, open } => match result {
                Ok(text) => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.path == path) {
                        tab.follow_disk(text);
                    }
                    self.refresh_find(false);
                    self.notice(done);
                    if let Some(open) = open {
                        return self.open(open);
                    }
                }
                Err(error) => self.notice(error),
            },
            Message::TriageLoaded(result) => return self.triage_loaded(result),
            Message::ImportText(text) => return self.import_text(text),
            Message::TimelineKind(kind) => self.timeline_kind = kind,
            Message::TimelineLoaded { tab, path, asked, rows: Some(rows) } => {
                // Only the newest read for the note the tab still shows.
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab && t.path == path && t.timeline_asked == asked) {
                    tab.timeline = rows;
                }
            }
            Message::TimelineLoaded { rows: None, .. } => {}
            Message::DailyFound(daily) => return self.open_daily(daily),
            Message::ToggleTask(i) => {
                let Some(row) = self.tasks.as_ref().and_then(|list| list.rows.get(i)) else { return Task::none() };
                return self.write_task(tasks::task_ref(row), None);
            }
            Message::ViewTask(task) => return self.write_task(task, None),
            Message::PreviewTask(line) => {
                // The line is current when the preview was parsed from this very
                // text, or when live preview found it in the text itself. While
                // a long note's preview waits for typing to pause, a click on
                // it may name a line that has moved: it is left alone.
                let live = self.view_mode == ViewMode::Live;
                let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded && (live || !t.unparsed)) else {
                    return Task::none();
                };
                let Some(text) = tab.content.line(line as usize).map(|l| l.text.into_owned()) else {
                    return Task::none();
                };
                // A recurring task gets its next occurrence above it.
                if let Some(lines) = sage_editor::tasks::toggled(&text, &sage_editor::today()) {
                    let ending = tab.ending();
                    tab.replace_line(line as usize, &lines.join(ending));
                    self.refresh_find(false);
                }
            }
            Message::TaskWritten { path, result: Ok(text) } => self.on_task_written(path, text),
            Message::TaskWritten { path, result: Err(error) } => {
                self.notice(format!("Task in {path} not changed: {error}"));
            }
            Message::EmbedLoaded { tab, key, embedded } => {
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab) {
                    // Only if still wanted: the embed may have been edited away.
                    if let Some(slot) = tab.embeds.get_mut(&key) {
                        *slot = embedded;
                        // A note may embed more.
                        tab.stale = true;
                    }
                }
            }
            Message::Grab { view, card, key, column, index } => {
                // The pointer is over the card itself when the drag starts.
                self.drag = Some(views::Drag {
                    view,
                    card,
                    key,
                    from: column,
                    start: index,
                    over: Some(column),
                    slot: Some(index),
                    moved: false,
                });
            }
            Message::DragOver(view, column) => {
                if let Some(drag) = self.drag.as_mut().filter(|d| d.view == view) {
                    drag.moved |= column != drag.from;
                    if drag.over != Some(column) {
                        drag.slot = None;
                    }
                    drag.over = Some(column);
                }
            }
            Message::DragLeave(view, column) => {
                if let Some(drag) = self.drag.as_mut().filter(|d| d.view == view && d.over == Some(column)) {
                    drag.over = None;
                    drag.slot = None;
                }
            }
            Message::DragCard(view, column, index) => {
                if let Some(drag) = self.drag.as_mut().filter(|d| d.view == view) {
                    drag.moved |= (column, index) != (drag.from, drag.start);
                    drag.over = Some(column);
                    drag.slot = Some(index);
                }
            }
            Message::DragCardLeave(view, column, index) => {
                if let Some(drag) = self.drag.as_mut().filter(|d| d.view == view && d.over == Some(column)) {
                    if drag.slot == Some(index) {
                        drag.slot = None;
                    }
                }
            }
            Message::MouseUp => {
                self.left_down = false;
                // A divider let go: its place is kept for the next start.
                if self.resizing.take().is_some() {
                    self.persist();
                }
                let Some(drag) = self.drag.take() else { return self.prune_check() };
                return match drag.over {
                    Some(column) if column != drag.from => {
                        self.place_card(&drag, column);
                        self.move_card(&drag, column)
                    }
                    // Pressed and released on the card without going
                    // anywhere: a click, which opens the task's line.
                    Some(_) if !drag.moved => {
                        self.open_at(drag.card.path, Some(Jump::Line(drag.card.line as usize)))
                    }
                    // Dropped elsewhere in its own column: placed there.
                    Some(column) => {
                        self.place_card(&drag, column);
                        Task::none()
                    }
                    None => Task::none(),
                };
            }
            Message::TemplatesFound(Ok(config)) => return self.choose_template(config),
            Message::TemplatesFound(Err(error)) => self.notice(format!("Could not read the template settings: {error}")),
            Message::TemplateLoaded { target, config, result: Ok(template) } => {
                return self.insert_template(target, &template, &config);
            }
            Message::TemplateLoaded { result: Err(error), .. } => self.notice(format!("Could not read the template: {error}")),
            Message::SelectTab(i) => {
                let i = i.min(self.tabs.len().saturating_sub(1));
                if i != self.active {
                    self.find = None;
                }
                self.active = i;
                self.tasks = None;
                self.settings_open = false;
                self.completion = None;
                self.persist();
                if self.tabs.get(i).is_some_and(|t| t.tasks) {
                    self.tasks = Some(TaskList::default());
                    self.load_tasks();
                }
            }
            Message::CloseTab(i) => self.close_tab(i),
            Message::TabMenu(i) => {
                if let Some(tab) = self.tabs.get(i) {
                    self.menu = Some((tab.path.clone(), false));
                    self.tab_menu = Some(i);
                    self.menu_at = pointer();
                }
            }
            Message::CloseOtherTabs(i) => self.close_tabs(|j| j != i),
            Message::CloseTabsRight(i) => self.close_tabs(|j| j > i),
            Message::CloseAllTabs => self.close_tabs(|_| true),
            Message::MoveTab(i, by) => {
                self.menu = None;
                self.tab_menu = None;
                let j = i as isize + by;
                if i < self.tabs.len() && j >= 0 && (j as usize) < self.tabs.len() {
                    let current = self.tabs.get(self.active).map(|t| t.id);
                    self.tabs.swap(i, j as usize);
                    self.active = self.tabs.iter().position(|t| Some(t.id) == current).unwrap_or(0);
                    self.persist();
                }
            }
            Message::PinTab(i) => {
                self.menu = None;
                self.tab_menu = None;
                // Pins are kept by path: a blank tab or the task list, whose
                // path is empty, would pin every other one with it.
                if let Some(path) = self.tabs.get(i).map(|t| t.path.clone()).filter(|p| !p.is_empty()) {
                    if let Some(at) = self.pinned.iter().position(|p| *p == path) {
                        self.pinned.remove(at);
                    } else {
                        // Pinned tabs gather at the left.
                        let current = self.tabs.get(self.active).map(|t| t.id);
                        let tab = self.tabs.remove(i);
                        let to = self.tabs.iter().take_while(|t| self.pinned.contains(&t.path)).count();
                        self.tabs.insert(to, tab);
                        self.active = self.tabs.iter().position(|t| Some(t.id) == current).unwrap_or(0);
                        self.pinned.push(path);
                    }
                    self.persist();
                }
            }
            Message::ToastTick => {
                if self.toast.as_ref().is_some_and(|t| Instant::now() >= t.until) {
                    self.toast = None;
                }
            }
            Message::DismissToast => self.toast = None,
            Message::ShortcutFilter(filter) => self.shortcut_filter = filter,
            Message::TaskFilter(filter) => {
                if let Some(list) = &mut self.tasks {
                    list.filter = filter;
                }
            }
            Message::UndoTrash => {
                if let Some((from, target)) = self.toast.take().and_then(|t| t.undo) {
                    let vault = self.vault.clone();
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                let to = vault.join(&from);
                                if to.exists() {
                                    return Err(format!("{from} exists again"));
                                }
                                if let Some(parent) = to.parent() {
                                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                                }
                                std::fs::rename(vault.join(&target), &to).map_err(|e| e.to_string())?;
                                Ok(from)
                            })
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()))
                        },
                        Message::TrashUndone,
                    );
                }
            }
            Message::TrashUndone(Ok(path)) => {
                self.notice(format!("Restored {path}"));
                return self.open(path);
            }
            Message::TrashUndone(Err(error)) => self.notice(format!("Could not restore: {error}")),
            Message::CloseMenu => {
                self.menu = None;
                self.tab_menu = None;
            }
            Message::ToggleFolder(path) => {
                self.menu = None;
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
                self.persist();
            }
            Message::Edit(action) => {
                // Ctrl+click follows the link under the pointer, as in Obsidian.
                let ctrl_click = matches!(action, text_editor::Action::Click(_)) && self.modifiers.command();
                match action {
                    text_editor::Action::Click(_) => self.left_down = true,
                    text_editor::Action::Drag(_) if !self.left_down => return Task::none(),
                    _ => {}
                }
                let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) else {
                    return Task::none();
                };
                let is_edit = action.is_edit();
                // A key that was on its way as a merge came up: resolving the
                // merge replaces the text, and would drop the edit unseen.
                if is_edit && tab.merge.is_some() {
                    return Task::none();
                }
                self.prop_complete = None;
                if is_edit {
                    let typing = matches!(
                        action,
                        text_editor::Action::Edit(Edit::Insert(_) | Edit::Backspace | Edit::Delete)
                    );
                    tab.checkpoint(if typing { sage_editor::undo::Kind::Typing } else { sage_editor::undo::Kind::Other });
                } else {
                    tab.history.break_group();
                }
                tab.content.perform(action);
                if ctrl_click {
                    let at = tab.content.cursor().position;
                    let link = tab.content.line(at.line).and_then(|line| wikilink_at(&line.text, at.column).and_then(link_target));
                    if let Some(link) = link {
                        return self.follow(link);
                    }
                }
                if is_edit {
                    tab.mark_edited();
                    self.refresh_completion();
                    self.refresh_find(false);
                } else {
                    // Suggestions follow typing, as in Obsidian. Moving the
                    // cursor or clicking closes them.
                    self.completion = None;
                    self.dismissed = None;
                }
            }
            Message::EditOp(op) => return self.edit_op(op),
            Message::PasteImage => return self.paste_image(),
            Message::FileDropped(file) => return self.drop_file(file),
            Message::Attached { tab, result } => return self.on_attached(tab, result),
            Message::OutlineJump(line) => {
                if let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) {
                    tab.put_cursor(line, 0);
                    return iced::widget::operation::focus(EDITOR);
                }
            }
            Message::MenuReveal => {
                if let Some((path, _)) = self.menu.take() {
                    // xdg-open and wslview can take seconds to hand over.
                    let abs = self.vault.join(&path);
                    return Task::perform(
                        async move { tokio::task::spawn_blocking(move || reveal(&abs)).await.unwrap_or_else(|e| Err(e.to_string())) },
                        Message::UrlOpened,
                    );
                }
            }
            Message::MenuCopyLink => {
                if let Some((path, folder)) = self.menu.take() {
                    let link = if folder { path } else { format!("[[{}]]", link_name(&path, &self.files)) };
                    self.notice(format!("Copied {link}"));
                    return iced::clipboard::write(link);
                }
            }
            Message::Live(event) => match event {
                live_editor::Event::Action(action) => return self.handle(Message::Edit(action)),
                live_editor::Event::Place(cursor) => {
                    let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) else {
                        return Task::none();
                    };
                    place(&mut tab.content, cursor);
                    tab.history.break_group();
                    self.completion = None;
                    self.dismissed = None;
                    self.prop_complete = None;
                }
                live_editor::Event::CtrlClick(position) => {
                    let link = self
                        .tabs
                        .get(self.active)
                        .and_then(|tab| tab.content.line(position.line))
                        .and_then(|line| wikilink_at(&line.text, position.column).and_then(link_target));
                    if let Some(link) = link {
                        return self.follow(link);
                    }
                }
                live_editor::Event::Follow(source) => {
                    if let Some(link) = link_target(&source) {
                        return self.follow(link);
                    }
                }
                live_editor::Event::Toggle(line) => return self.handle(Message::PreviewTask(line as u32)),
            },
            Message::Property(index, value) => {
                self.prop_complete = None;
                if let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) {
                    tab.set_property(index, &value);
                }
            }
            Message::PropertyTyped(entry, item, value) => {
                if let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) {
                    tab.set_property(entry, &value);
                    self.refresh_prop_completion(entry, item, &value);
                }
            }
            Message::PropertySubmit(entry, item) => self.submit_property(entry, item),
            Message::PropertyBlur(entry, item, focused) => {
                if !focused && self.drop_empty_item(entry, item) {
                    self.prop_complete = None;
                    return self.prune_check();
                }
            }
            Message::NewPropertyOpen => {
                self.new_property = Some((sage_editor::properties::PropType::Text, String::new()));
                return iced::widget::operation::focus(iced::widget::Id::from(preview::NEW_PROPERTY));
            }
            Message::NewPropertyType(ty) => {
                if let Some((current, _)) = &mut self.new_property {
                    *current = ty;
                }
                return iced::widget::operation::focus(iced::widget::Id::from(preview::NEW_PROPERTY));
            }
            Message::NewPropertyBlur(focused) => {
                if !focused && self.new_property.as_ref().is_some_and(|(_, s)| s.trim().is_empty()) {
                    self.new_property = None;
                }
            }
            Message::NewPropertyTyped(s) => {
                if let Some((_, name)) = &mut self.new_property {
                    *name = s;
                }
            }
            Message::NewPropertySubmit => {
                let Some((ty, name)) = self.new_property.clone() else { return Task::none() };
                let key = name.trim().to_owned();
                // Left empty, it closes without adding anything.
                if key.is_empty() {
                    self.new_property = None;
                    return Task::none();
                }
                let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) else { return Task::none() };
                if let Some(entry) = tab.add_property(&key) {
                    self.new_property = None;
                    // The name's own kind (tags, links) stays a list; otherwise the chosen type applies.
                    if let Some(current) = tab.property(entry).filter(|e| !matches!(e.value, sage_editor::properties::Value::List { .. })) {
                        let value = sage_editor::properties::convert(&current.value, ty);
                        tab.set_property(entry, &value);
                    }
                    let path = tab.path.clone();
                    self.settings.property_types.insert(key.to_lowercase(), ty.id().to_owned());
                    self.settings_edited();
                    self.editing_prop = Some((path, entry));
                    return iced::widget::operation::focus(iced::widget::Id::from(preview::property_id(entry, 0)));
                }
            }
            Message::ChipHover(at) => self.chip_hover = at,
            Message::SearchTag(tag) => {
                // Close a search that is already open, which the command would toggle off.
                self.palette = None;
                let open = self.run(Command::SearchEverywhere);
                if let Some(palette) = &mut self.palette {
                    palette.query = format!("#{tag}");
                }
                self.search();
                return open;
            }
            Message::PropertyToday(entry) => {
                self.prop_complete = None;
                let template = self.tabs.get(self.active).is_some_and(|t| t.path.starts_with(&format!("{}/", self.template_config().folder)));
                let Some(current) = self.tabs.get(self.active).filter(|t| t.loaded).and_then(|t| t.property(entry)) else { return Task::none() };
                let chosen = self.chosen_type(&current.key);
                let date_time = sage_editor::properties::type_of(&current.value, chosen) == Some(sage_editor::properties::PropType::DateTime);
                let now = sage_editor::now();
                let text = match (template, date_time) {
                    (true, false) => "{{date:YYYY-MM-DD}}".to_owned(),
                    (true, true) => "{{date:YYYY-MM-DD}}T{{time:HH:mm}}".to_owned(),
                    (false, false) => now.format("%Y-%m-%d").to_string(),
                    (false, true) => now.format("%Y-%m-%dT%H:%M").to_string(),
                };
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.set_property(entry, &sage_editor::properties::Value::Text { text, quote: None });
                }
            }
            Message::PropertyEdit(None) => {
                self.editing_prop = None;
                self.prop_complete = None;
                return self.prune_check();
            }
            Message::PropertyEdit(Some(entry)) => {
                self.prop_complete = None;
                let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) else { return Task::none() };
                self.editing_prop = Some((tab.path.clone(), entry));
                let focus = match tab.property(entry).map(|e| e.value) {
                    Some(sage_editor::properties::Value::List { mut items, block }) => {
                        if items.is_empty() {
                            items.push(String::new());
                            tab.set_property(entry, &sage_editor::properties::Value::List { items: items.clone(), block });
                        }
                        items.len() - 1
                    }
                    _ => 0,
                };
                return iced::widget::operation::focus(iced::widget::Id::from(preview::property_id(entry, focus)));
            }
            Message::PropertyRemove(i) => {
                self.editing_prop = None;
                self.prop_complete = None;
                self.type_menu = None;
                if let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) {
                    tab.remove_property(i);
                }
            }
            Message::PropertyTypeMenu(i) => self.type_menu = if self.type_menu == Some(i) { None } else { Some(i) },
            Message::PropertyType(entry, ty) => {
                self.prop_complete = None;
                self.type_menu = None;
                let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) else { return Task::none() };
                let Some(current) = tab.property(entry) else { return Task::none() };
                let key = current.key.to_lowercase();
                let value = sage_editor::properties::convert(&current.value, ty);
                tab.set_property(entry, &value);
                self.settings.property_types.insert(key, ty.id().to_owned());
                self.settings_edited();
            }
            Message::PropertyPick(i) => self.accept_property(Some(i)),
            Message::PropertyAdd(entry) => {
                self.prop_complete = None;
                let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) else { return Task::none() };
                let Some(sage_editor::properties::Value::List { mut items, block }) = tab.property(entry).map(|e| e.value) else {
                    return Task::none();
                };
                items.push(String::new());
                let next = items.len() - 1;
                tab.set_property(entry, &sage_editor::properties::Value::List { items, block });
                return iced::widget::operation::focus(iced::widget::Id::from(preview::property_id(entry, next)));
            }
            Message::Table(start, edit) => {
                use tables::Edit;
                let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) else { return Task::none() };
                match edit {
                    Edit::Cell(r, c, s) => {
                        self.table_draft = Some((start, r, c, s.clone()));
                        tab.edit_table(start, true, |t| t.set(r, c, s.trim()));
                    }
                    Edit::Enter(r, c) => {
                        self.table_draft = None;
                        let below = r + 1;
                        if tab.table_at(start).is_some_and(|t| below >= t.rows.len()) {
                            tab.edit_table(start, false, |t| t.add_row(r));
                        }
                        return iced::widget::operation::focus(tables::cell_id(start, below, c));
                    }
                    Edit::AddRow(after) => {
                        self.table_draft = None;
                        tab.edit_table(start, false, |t| t.add_row(after));
                        return iced::widget::operation::focus(tables::cell_id(start, after + 1, 0));
                    }
                    Edit::AddColumn(after) => {
                        self.table_draft = None;
                        tab.edit_table(start, false, |t| t.add_column(after));
                        return iced::widget::operation::focus(tables::cell_id(start, 0, after + 1));
                    }
                    Edit::RemoveRow(r) => {
                        self.table_draft = None;
                        tab.edit_table(start, false, |t| t.remove_row(r));
                    }
                    Edit::RemoveColumn(c) => {
                        self.table_draft = None;
                        tab.edit_table(start, false, |t| t.remove_column(c));
                    }
                }
            }
            // Keys for the suggestions under a property field, which has the focus.
            Message::Key(Shortcut::Escape) if self.prop_complete.is_some() => self.prop_complete = None,
            Message::Key(Shortcut::Escape) if self.new_property.is_some() && self.palette.is_none() => self.new_property = None,
            Message::Key(Shortcut::Escape) if self.editing_prop.is_some() && self.palette.is_none() => {
                self.editing_prop = None;
                return self.prune_check();
            }
            Message::Key(Shortcut::Tab) if self.prop_complete.is_some() => self.accept_property(None),
            Message::Key(direction @ (Shortcut::Up | Shortcut::Down)) if self.prop_complete.is_some() => {
                if let Some(c) = &mut self.prop_complete {
                    let last = c.items.len().saturating_sub(1);
                    c.selected = if matches!(direction, Shortcut::Up) { c.selected.saturating_sub(1) } else { (c.selected + 1).min(last) };
                }
            }
            Message::Tick => {
                // A note a rename is moving waits for it quietly; `rename_done` saves it.
                let due: Vec<usize> =
                    (0..self.tabs.len()).filter(|&i| self.tabs[i].wants_autosave() && !self.rename_holds(&self.tabs[i].path)).collect();
                let mut saves: Vec<Task<Message>> = due.into_iter().map(|i| self.save(i, false)).collect();
                if self.settings_changed.is_some_and(|at| at.elapsed() >= AUTOSAVE_AFTER) {
                    saves.push(self.save_settings());
                }
                return Task::batch(saves);
            }
            Message::CompleteMove(delta) => {
                if let Some(c) = &mut self.completion {
                    let last = c.items.len().saturating_sub(1) as i32;
                    c.selected = (c.selected as i32 + delta).clamp(0, last) as usize;
                }
            }
            Message::CompletePick(i) => {
                if let Some(c) = &mut self.completion {
                    c.selected = i;
                }
                return self.accept_completion();
            }
            Message::CompleteAccept => return self.accept_completion(),
            Message::CompleteDismiss => {
                if let Some(c) = self.completion.take() {
                    self.dismissed = Some((c.line, c.ctx.start));
                    self.escape_used = true;
                }
            }
            Message::FindQuery(query) => {
                if let Some(find) = &mut self.find {
                    find.query = query;
                }
                self.refresh_find(true);
            }
            Message::FindSubmit => self.step_find(!self.modifiers.shift()),
            Message::FindClose => return self.close_find(),
            Message::PaletteQuery(query) => {
                if let Some(palette) = &mut self.palette {
                    palette.query = query;
                    palette.selected = 0;
                }
                self.search();
            }
            Message::PaletteClick(i) => {
                if let Some(palette) = &mut self.palette {
                    palette.selected = i;
                }
                return self.handle(Message::PaletteSubmit);
            }
            Message::PaletteSubmit => {
                let Some(palette) = self.palette.take() else { return Task::none() };
                let action = palette.hits.get(palette.selected).map(|hit| hit.action.clone());
                match (action, palette.mode) {
                    (Some(HitAction::Open(path)), _) => return self.open(path),
                    (Some(HitAction::OpenAt(path, line)), _) => return self.open_at(path, Some(Jump::Line(line))),
                    (Some(HitAction::Create(name)), mode) => return self.create(Some(&name), matches!(mode, PaletteMode::NewNote)),
                    (Some(HitAction::CreateFolder(folder)), _) => return self.create_folder(folder),
                    (Some(HitAction::Run(command)), _) => return self.run(command),
                    (Some(HitAction::Rename(to)), PaletteMode::Rename(from) | PaletteMode::MoveTo(from)) => {
                        return self.rename(Move { from, to, folder: false });
                    }
                    (Some(HitAction::RenameFolder(to)), PaletteMode::RenameFolder(from)) => {
                        return self.rename(Move { from, to, folder: true });
                    }
                    (Some(HitAction::InsertTemplate(path)), PaletteMode::Template(config)) => {
                        return self.read_template(path, config);
                    }
                    (Some(HitAction::QuickAdd(i)), _) => return self.choose_quick_add(i),
                    (Some(HitAction::QuickAddRun(i)), PaletteMode::QuickAddValue(_)) => {
                        return self.run_quick_add(i, palette.query.trim().to_owned());
                    }
                    (Some(HitAction::Flow(step)), mode) => return self.flow_step(step, mode, palette.query),
                    (Some(HitAction::Nothing), mode) => self.palette = Some(Palette { mode, ..palette }),
                    _ => {}
                }
            }
            Message::Key(Shortcut::Command(command)) | Message::Run(command) => return self.run(command),
            Message::Key(Shortcut::Escape) => {
                // One thing closes per press: a confirmation, the tree menu,
                // the palette, else the find bar, else the task list.
                if self.task_form.take().is_some()
                    || self.recording.take().is_some()
                    || self.recording_built_in.take().is_some()
                    || self.doomed.take().is_some()
                    || std::mem::take(&mut self.help)
                    || self.menu.take().is_some()
                {
                    return Task::none();
                }
                if !std::mem::take(&mut self.escape_used) {
                    if self.palette.take().is_some() {
                        // Typing goes back to the note the palette was opened over.
                        let editing = self.view_mode != ViewMode::Preview && !self.tabs.is_empty();
                        return if editing { iced::widget::operation::focus(EDITOR) } else { Task::none() };
                    }
                    if self.find.is_some() {
                        return self.close_find();
                    }
                    if std::mem::take(&mut self.focus) {
                        return iced::widget::operation::focus(EDITOR);
                    }
                    self.tasks = None;
                    if std::mem::take(&mut self.settings_open) {
                        return self.save_settings();
                    }
                }
            }
            Message::Modifiers(modifiers) => self.modifiers = modifiers,
            Message::OpenVault => return self.open_vault(),
            Message::VaultPicked(picked) => return self.vault_picked(picked),
            Message::SwitchVault(vault) => return self.switch_vault(vault),
            Message::CloseRequested => return self.close_requested(),
            Message::Quit { force } => return self.quit(force),
            Message::CorpusLoaded(result) => {
                // Only if search everywhere is still open and waiting for it.
                let waiting = matches!(&self.palette, Some(Palette { mode: PaletteMode::Everywhere(None), .. }));
                match result {
                    Ok(corpus) if waiting => {
                        if let Some(palette) = &mut self.palette {
                            palette.mode = PaletteMode::Everywhere(Some(corpus));
                        }
                        self.search();
                    }
                    Ok(_) => {}
                    Err(e) => {
                        if waiting {
                            self.palette = None;
                        }
                        self.notice(format!("Search everywhere could not read the index: {e}"));
                    }
                }
            }
            // `update` runs `sync_previews` after every message, which parses,
            // counts and runs views that were waiting.
            Message::Settle => {}
            Message::Clock => {
                // `due<today` meant yesterday until now: run views and the task
                // list again, as after a change in the index.
                let today = sage_editor::today();
                if today != self.day {
                    self.day = today;
                    self.load_tasks();
                    for tab in &mut self.tabs {
                        tab.views.clear();
                        tab.task_lists.clear();
                        tab.stale = true;
                    }
                }
                return self.check_reminders();
            }
            Message::Key(Shortcut::Tab) => return self.task_form_tab(),
            Message::TaskFormTitle(title) => {
                if let Some(form) = &mut self.task_form {
                    form.title = title;
                    form.error = None;
                }
            }
            Message::TaskFormDue(due) => {
                if let Some(form) = &mut self.task_form {
                    form.due = due;
                    form.error = None;
                }
            }
            Message::Notified(Ok(())) => {}
            Message::Notified(Err(error)) => {
                self.status = error;
                self.status_until = Instant::now() + NOTICE_FOR;
            }
            Message::SettingsNotifications(on) => {
                self.settings.notifications = Some(on);
                self.settings_edited();
            }
            Message::SettingsReminderTime(time) => {
                self.settings.reminder_time = Some(time).filter(|t| !t.trim().is_empty());
                self.settings_edited();
            }
            Message::TaskFormRemind(remind) => {
                if let Some(form) = &mut self.task_form {
                    form.remind = remind;
                    form.error = None;
                }
            }
            Message::TaskFormSubmit => return self.submit_task_form(),
            Message::TaskNoteCreated { tab, result } => return self.on_task_note_created(tab, result),
            Message::Key(direction) => {
                if let Some(palette) = &mut self.palette {
                    let last = palette.hits.len().saturating_sub(1);
                    palette.selected = match direction {
                        Shortcut::Up => palette.selected.saturating_sub(1),
                        _ => (palette.selected + 1).min(last),
                    };
                }
            }
        }
        Task::none()
    }

    /// Switches the view mode, focusing the editor if the new mode has one,
    /// since the switch replaces the widget that had focus.
    fn set_view(&mut self, mode: ViewMode) -> Task<Message> {
        if mode == ViewMode::Live && self.view_mode != ViewMode::Live {
            self.before_live = self.view_mode;
        }
        self.view_mode = mode;
        self.persist();
        if mode == ViewMode::Preview {
            Task::none()
        } else {
            iced::widget::operation::focus(EDITOR)
        }
    }

    fn run(&mut self, command: Command) -> Task<Message> {
        match command {
            Command::Save => return self.save(self.active, false),
            Command::ForceSave => return self.save(self.active, true),
            Command::NewNote => {
                self.palette = Some(Palette { mode: PaletteMode::NewNote, ..Palette::default() });
                return iced::widget::operation::focus(PALETTE_INPUT);
            }
            Command::DeleteNote => match self.tabs.get(self.active).filter(|t| t.loaded) {
                Some(tab) => self.ask_delete(tab.path.clone(), false),
                None => self.notice("Open the note to delete, or right-click it in the tree".into()),
            },
            Command::CycleView => {
                let next = match self.view_mode {
                    ViewMode::Source => ViewMode::Split,
                    ViewMode::Split => ViewMode::Preview,
                    ViewMode::Preview => ViewMode::Live,
                    ViewMode::Live => ViewMode::Source,
                };
                return self.set_view(next);
            }
            Command::ToggleLive => {
                let next = if self.view_mode == ViewMode::Live { self.before_live } else { ViewMode::Live };
                return self.set_view(next);
            }
            Command::View(mode) => return self.set_view(mode),
            Command::Shortcuts => self.help = !self.help,
            Command::CreateStarterVault => return self.create_starter(),
            Command::ToggleTheme => {
                self.dark_override = Some(!self.dark());
                self.persist();
            }
            Command::CloseActiveTab => self.close_tab(self.active),
            Command::NewTab => {
                self.palette = None;
                self.tasks = None;
                self.settings_open = false;
                match self.tabs.iter().position(Tab::is_blank) {
                    Some(i) => self.active = i,
                    None => {
                        self.tabs.push(Tab::blank());
                        self.active = self.tabs.len() - 1;
                    }
                }
                self.completion = None;
                self.find = None;
                self.persist();
            }
            Command::OpenNote => {
                if self.palette.is_none() {
                    return self.run(Command::TogglePalette);
                }
            }
            Command::Find => return self.open_find(),
            Command::RenameNote => {
                if let Some(tab) = self.tabs.get(self.active).filter(|t| t.loaded) {
                    return self.start_rename(tab.path.clone());
                }
            }
            Command::MoveNote => {
                if let Some(tab) = self.tabs.get(self.active).filter(|t| t.loaded) {
                    return self.start_move(tab.path.clone());
                }
            }
            Command::RenameFolder => {
                let folder = self.tabs.get(self.active).and_then(|t| t.path.rsplit_once('/')).map(|(f, _)| f.to_owned());
                match folder {
                    Some(folder) => return self.start_folder_rename(folder),
                    None => self.notice("Open a note in a folder, or right-click a folder in the tree".into()),
                }
            }
            Command::UndoRename => return self.undo_rename(),
            Command::InsertTemplate => return self.find_templates(),
            Command::QuickAdd => return self.start_quick_add(),
            Command::ToggleSettings => return self.handle(Message::ToggleSettings),
            Command::OpenVault => return self.open_vault(),
            Command::NewInlineTask => return self.open_task_form(false),
            Command::LogToProject => return self.start_log(),
            Command::PromoteLog => return self.promote_log(),
            Command::Triage => return self.start_triage(),
            Command::ImportEvent => return iced::clipboard::read().map(Message::ImportText),
            Command::WeeklyReview => return self.weekly_review(),
            Command::FollowUp | Command::WaitingFor => {
                if self.settings_open || self.tasks.is_some() || self.view_mode == ViewMode::Preview {
                    return Task::none();
                }
                // Not while a merge waits: resolving it replaces the text, and the edit with it.
                let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded && t.merge.is_none()) else { return Task::none() };
                let line = tab.content.cursor().position.line;
                let Some(old) = tab.content.line(line).map(|l| l.text.into_owned()) else { return Task::none() };
                let rest = old.trim_start();
                let indent = &old[..old.len() - rest.len()];
                let body = if rest.starts_with("- [") || rest.starts_with("* [") || rest.starts_with("+ [") {
                    rest.to_owned()
                } else if let Some(item) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* ")) {
                    format!("- [ ] {item}")
                } else {
                    format!("- [ ] {rest}")
                };
                let gap = if body.ends_with(' ') || body.ends_with("] ") { "" } else { " " };
                let mark = if command == Command::WaitingFor { format!("#{} ➕ {} ", sage_editor::log::WAITING_TAG, sage_editor::today()) } else { String::new() };
                let new = format!("{indent}{body}{gap}{mark}@");
                tab.replace_line(line, &new);
                tab.put_cursor(line, new.len());
                self.refresh_completion();
            }
            Command::NewTaskNote => return self.open_task_form(true),
            Command::ToggleTasks => {
                if self.tasks.take().is_some() {
                    // Toggling the list off closes the tab it was in.
                    if self.tabs.get(self.active).is_some_and(|t| t.tasks) {
                        self.close_tab(self.active);
                    }
                } else {
                    self.palette = None;
                    self.find = None;
                    self.completion = None;
                    self.settings_open = false;
                    match self.tabs.iter().position(|t| t.tasks) {
                        Some(i) => self.active = i,
                        None => {
                            self.tabs.push(Tab::tasks());
                            self.active = self.tabs.len() - 1;
                        }
                    }
                    self.tasks = Some(TaskList::default());
                    self.load_tasks();
                    self.persist();
                }
            }
            Command::SearchEverywhere => {
                if matches!(self.palette.take(), Some(Palette { mode: PaletteMode::Everywhere(_), .. })) {
                    return Task::none();
                }
                if self.reader.is_none() {
                    return Task::none();
                }
                // Every note's text is read and split into words: off the UI
                // thread, with the palette open at once so typing is not lost.
                self.completion = None;
                self.palette = Some(Palette { mode: PaletteMode::Everywhere(None), ..Palette::default() });
                self.search();
                let db_path = self.db_path.clone();
                let load = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let reader = sage_db::open_reader(&db_path).map_err(|e| format!("{e:#}"))?;
                            omni::Corpus::load(&reader).map(Arc::new).map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    Message::CorpusLoaded,
                );
                return Task::batch([iced::widget::operation::focus(PALETTE_INPUT), load]);
            }
            Command::Back => return self.go(-1),
            Command::Forward => return self.go(1),
            Command::DailyNote => return self.daily_note(),
            Command::ReopenTab => return self.reopen_tab(),
            Command::ToggleOutline => {
                self.outline_open = !self.outline_open;
                self.persist();
            }
            Command::ZoomIn | Command::ZoomOut | Command::ZoomReset => {
                let zoom = match command {
                    Command::ZoomIn => ZOOM.iter().copied().find(|z| *z > self.zoom + 0.01).unwrap_or(ZOOM[ZOOM.len() - 1]),
                    Command::ZoomOut => ZOOM.iter().rev().copied().find(|z| *z < self.zoom - 0.01).unwrap_or(ZOOM[0]),
                    _ => 1.0,
                };
                // The width is in the interface's units, which the zoom scales.
                self.window_width *= self.zoom / zoom;
                self.zoom = zoom;
                self.notice(format!("Zoom {}%", (zoom * 100.0).round()));
                self.persist();
            }
            Command::ToggleFocus => {
                self.focus = !self.focus;
                // The editor moves in the widget tree, which drops its focus.
                return iced::widget::operation::focus(EDITOR);
            }
            Command::TogglePalette => {
                if self.palette.take().is_none() {
                    self.completion = None;
                    self.palette = Some(Palette::default());
                    self.search();
                    return iced::widget::operation::focus(PALETTE_INPUT);
                }
            }
        }
        Task::none()
    }

    fn on_index(&mut self, event: IndexEvent) -> Task<Message> {
        match event {
            IndexEvent::Started { backend, known_files, onedrive } => {
                self.backend = Some(backend);
                self.onedrive = onedrive;
                if self.reader.is_none() {
                    self.reader = sage_db::open_reader(&self.db_path).ok();
                    self.reload_files();
                }
                self.load_reminders();
                self.status = format!("{known_files} files cached, checking for changes");
            }
            IndexEvent::Progress { done, total } => {
                if Instant::now() >= self.status_until {
                    self.status = format!("Indexing {done} of {total}");
                }
            }
            IndexEvent::Changed { paths } => {
                self.reload_files();
                self.load_tasks();
                self.load_reminders();
                let mut reads = Vec::new();
                for i in 0..self.tabs.len() {
                    self.tabs[i].links_stale = true;
                    // Re-read open notes that changed. on_loaded decides whether
                    // that is a no-op (our own save), a reload or a conflict.
                    if self.tabs[i].loaded && paths.contains(&self.tabs[i].path) {
                        reads.push(self.read(self.tabs[i].id, &self.tabs[i].path));
                    }
                    // Any change can alter what a view matches; `sync_previews`
                    // runs them again. An embed that was missing may exist now.
                    let tab = &mut self.tabs[i];
                    tab.index_stale = true;
                    tab.embeds.retain(|_, embed| match embed {
                        Embedded::Loading => true,
                        Embedded::Image { path, .. } | Embedded::Svg { path, .. } | Embedded::Note { path, .. } => {
                            !paths.contains(path)
                        }
                        Embedded::Missing(_) => false,
                    });
                    tab.stale = true;
                }
                reads.push(self.scan_folders());
                return Task::batch(reads);
            }
            IndexEvent::Idle { files, took_ms } => {
                // The first pass has caught up with changes made while Sage was
                // closed, so a task finished elsewhere no longer reminds.
                if !std::mem::replace(&mut self.indexed, true) {
                    self.load_reminders();
                    return self.check_reminders();
                }
                if !self.tabs.iter().any(|t| t.conflict) && Instant::now() >= self.status_until {
                    self.status = format!("{files} files indexed, last pass {took_ms} ms");
                }
            }
            IndexEvent::Synced(id) => {
                if let Some((_, request)) = self.pending_rename.take_if(|(wanted, _)| *wanted == id) {
                    return self.run_rename(request);
                }
            }
            IndexEvent::Error(error) => self.notice(format!("Index error: {error}")),
        }
        Task::none()
    }

    // ---- files --------------------------------------------------------------

    /// Reads the note at `path` for tab `tab`.
    fn read(&self, tab: u64, path: &str) -> Task<Message> {
        let abs = self.vault.join(path);
        let path = path.to_owned();
        Task::perform(
            async move {
                let bytes = tokio::fs::read(&abs).await.map_err(|e| e.to_string())?;
                // Never open a file we would corrupt on save.
                String::from_utf8(bytes).map_err(|_| "not valid UTF-8, refusing to edit it".to_owned())
            },
            move |result| Message::Loaded { tab, path: path.clone(), result },
        )
    }

    fn open(&mut self, path: String) -> Task<Message> {
        self.open_at(path, None)
    }

    fn open_at(&mut self, path: String, jump: Option<Jump>) -> Task<Message> {
        self.palette = None;
        self.tasks = None;
        // Unsaved settings are still written by the autosave tick.
        self.settings_open = false;
        self.visited(&path);
        for folder in tree::ancestors(&path) {
            self.expanded.insert(folder.to_owned());
        }
        if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
            if i != self.active {
                self.find = None;
            }
            self.active = i;
            if let Some(jump) = jump {
                self.tabs[i].jump_to(jump);
            }
            self.persist();
            return iced::widget::operation::focus(EDITOR);
        }
        let title = self.title_of(&path);
        let mut tab = Tab::placeholder(path, title);
        let task = self.read(tab.id, &tab.path);
        tab.jump = jump;
        // Navigating changes the current tab instead of adding one. A tab with
        // unsaved text, or a pinned one, is left as it is and the note opens
        // beside it, so nothing is lost.
        let replaces = self.tabs.get(self.active).is_some_and(|t| !t.busy() && !self.pinned.contains(&t.path));
        if replaces {
            self.tabs[self.active] = tab;
        } else {
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
        }
        self.completion = None;
        self.find = None;
        self.persist();
        task
    }

    /// Follows a link in the active note's preview. A link to a missing note
    /// creates it, as in Obsidian; a web or mail link opens on the desktop.
    fn follow(&mut self, link: LinkTarget) -> Task<Message> {
        let (target, heading, block) = match link {
            LinkTarget::Note { target, heading, block } => (target, heading, block),
            LinkTarget::Url(url) => return self.open_url(url),
        };
        let Some(src) = self.tabs.get(self.active).map(|t| t.path.clone()) else { return Task::none() };
        let jump = heading.map(Jump::Heading).or(block.map(Jump::Block));
        if target.is_empty() {
            return self.open_at(src, jump);
        }
        let path = sage_core::target_path(&target, &src);
        match self.resolve(&sage_core::link_key(&path)) {
            Some(found) if is_note_path(&found) => self.open_at(found, jump),
            Some(found) => {
                self.notice(format!("{found} is an attachment; Sage opens notes only"));
                Task::none()
            }
            None if is_attachment(&path) => {
                self.notice(format!("{path} does not exist"));
                Task::none()
            }
            None => self.create(Some(&path), true),
        }
    }

    /// Hands a web or mail link to the desktop; see [`open_link`]. Other
    /// schemes could start any program, so they are not opened.
    fn open_url(&mut self, url: String) -> Task<Message> {
        let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase()).unwrap_or_default();
        if !matches!(scheme.as_str(), "http" | "https" | "mailto") {
            self.notice(format!("Sage opens web and mail links only, not {scheme}: links"));
            return Task::none();
        }
        self.status = format!("Opening {url}");
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || open_link(&url)).await.unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::UrlOpened,
        )
    }

    /// The vault path a link key resolves to, by the indexer's rule: a file
    /// name, else for a bare key a note's frontmatter `id`.
    fn resolve(&self, key: &str) -> Option<String> {
        let reader = self.reader.as_ref()?;
        let name = key.rsplit('/').next().unwrap_or("");
        let mut stmt = reader.prepare_cached("SELECT path FROM files WHERE name_key = ?1").ok()?;
        let paths: Vec<String> = stmt.query_map([name], |r| r.get(0)).ok()?.flatten().collect();
        if let Some(path) = sage_core::resolve(key, paths.iter().map(String::as_str)) {
            return Some(path.to_owned());
        }
        if name.is_empty() || name != key {
            return None;
        }
        let mut stmt = reader
            .prepare_cached("SELECT path FROM files WHERE id_key = ?1 ORDER BY length(path), path LIMIT 1")
            .ok()?;
        stmt.query_row([key], |r| r.get(0)).ok()
    }

    /// The typed document a link key resolves to, if it is one.
    fn document(&self, key: &str) -> Option<sage_query::documents::Document> {
        let path = self.resolve(key)?;
        sage_query::documents::document(self.reader.as_ref()?, &path).ok().flatten()
    }

    fn on_loaded(&mut self, tab: u64, path: String, result: Result<String, String>) -> Task<Message> {
        let Some(i) = self.tabs.iter().position(|t| t.id == tab) else { return Task::none() };
        let tab = &mut self.tabs[i];
        // Read from where the note was before a rename moved it: no longer its
        // text. A tab still waiting for its first read reads the new place.
        if tab.path != path {
            let (loaded, id, current) = (tab.loaded, tab.id, tab.path.clone());
            return if loaded { Task::none() } else { self.read(id, &current) };
        }
        match result {
            Ok(text) if !tab.loaded => {
                tab.set_text(text);
                if let Some(jump) = tab.jump.take() {
                    tab.jump_to(jump);
                }
                let links = self.load_links(i);
                // A note that was just opened is ready to type into.
                if i == self.active && self.palette.is_none() {
                    return Task::batch([links, iced::widget::operation::focus(EDITOR)]);
                }
                return links;
            }
            // Our own save coming back, or a touch.
            Ok(text) if text == tab.base => {}
            // The save under way decides what the disk holds; look again after it.
            Ok(_) if tab.saving => tab.reread = true,
            // Changed elsewhere: reloaded, or merged with the edits made here.
            Ok(text) => return self.merge_disk(i, text),
            Err(error) if !tab.loaded => {
                let id = tab.id;
                self.active = active_after(self.active, &self.tabs, |t| t.id == id);
                self.tabs.remove(i);
                self.notice(format!("Cannot open {path}: {error}"));
                self.persist();
            }
            Err(error) => self.notice(format!("{path}: {error}")),
        }
        Task::none()
    }

    fn save(&mut self, i: usize, force: bool) -> Task<Message> {
        let backend = self.backend.unwrap_or(Backend::Poll);
        if self.tabs.get(i).is_some_and(|t| t.loaded && (t.dirty || force) && self.rename_holds(&t.path)) {
            self.notice(format!("{} is being moved; it is saved once that is done", self.tabs[i].path));
            return Task::none();
        }
        let Some(tab) = self.tabs.get_mut(i).filter(|t| t.loaded && !t.saving) else {
            return Task::none();
        };
        if !tab.dirty && !force {
            return Task::none();
        }
        tab.saving = true;
        let (abs, path, seq, id) = (self.vault.join(&tab.path), tab.path.clone(), tab.edit_seq, tab.id);
        let (text, base) = (tab.content.text(), tab.base.clone());
        Task::perform(
            async move {
                let written = text.clone();
                tokio::task::spawn_blocking(move || {
                    if force {
                        sage_editor::save(&abs, &written, backend).map_err(|e| SaveError::Io(format!("{e:#}")))
                    } else {
                        sage_editor::save_guarded(&abs, &base, &written, backend)
                    }
                })
                .await
                .unwrap_or_else(|e| Err(SaveError::Io(e.to_string())))
                .map(|()| text)
            },
            move |result| Message::Saved { tab: id, path: path.clone(), seq, result },
        )
    }

    fn on_saved(&mut self, id: u64, path: String, seq: u64, result: Result<String, SaveError>) -> Task<Message> {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return Task::none() };
        tab.saving = false;
        let mut closing = Task::none();
        match result {
            // Written where the note was before it moved. `rename_holds` keeps
            // saves from starting while a move is under way, so this should not
            // happen; if it does, the write is no save of the note where it is
            // now: the tab stays unsaved, and the user hears of the stray file.
            Ok(_) if tab.path != path => {
                tab.dirty = true;
                tab.last_edit = Instant::now();
                let now = tab.path.clone();
                self.notice(format!("{path} was written after the note moved to {now}; {now} is saved next, check {path}"));
            }
            Ok(text) => {
                let targets = sage_editor::tasks::checked_links(&tab.base, &text);
                if !targets.is_empty() {
                    let (db_path, vault, from) = (self.db_path.clone(), self.vault.clone(), path.clone());
                    let backend = self.backend.unwrap_or(Backend::Poll);
                    closing = Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || close_linked_notes(&db_path, &vault, &from, &targets, backend))
                                .await
                                .unwrap_or_default()
                        },
                        Message::TaskNotesClosed,
                    );
                }
                let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return closing };
                tab.base = text;
                tab.conflict = false;
                // "Overwrite file on disk" settles a merge that was waiting.
                if matches!(tab.merge, Some(Resolve { source: ResolveSource::Disk { .. }, .. })) {
                    tab.merge = None;
                }
                tab.dirty = tab.edit_seq != seq;
                self.status = format!("Saved {path}");
                if std::mem::take(&mut tab.reread) {
                    return Task::batch([closing, self.read(id, &path)]);
                }
            }
            // The file changed since the tab last read it: merge rather than refuse.
            Err(SaveError::Conflict) => {
                tab.reread = false;
                let current = tab.path.clone();
                return self.read(id, &current);
            }
            Err(SaveError::Io(error)) => {
                // Back off so autosave does not retry on every tick.
                tab.last_edit = Instant::now();
                self.notice(format!("Could not save {path}: {error}"));
            }
        }
        closing
    }

    /// The note changed on disk: takes the change into the tab. With no edits
    /// here that is a reload; edits on both sides merge line by line, and only
    /// sections both sides changed differently wait for the user.
    fn merge_disk(&mut self, i: usize, theirs: String) -> Task<Message> {
        let tab = &mut self.tabs[i];
        let merge = sage_editor::merge::merge3(&tab.base, &tab.content.text(), &theirs);
        let (path, had_edits, conflicts) = (tab.path.clone(), tab.dirty, merge.conflicts());
        if conflicts == 0 {
            tab.take_merge(&merge, &[], theirs);
            if had_edits {
                self.notice(format!("Merged the changes made to {path} elsewhere with yours"));
            } else {
                self.status = format!("Reloaded {path}, it changed on disk");
            }
        } else {
            // Autosave waits until the user has decided.
            tab.conflict = true;
            tab.merge = Some(Resolve { source: ResolveSource::Disk { theirs }, merge, choices: vec![None; conflicts] });
            let sections = if conflicts == 1 { "1 section" } else { &format!("{conflicts} sections") };
            self.notice(format!("{path} changed on disk where you were editing: {sections} to decide"));
        }
        if i == self.active {
            self.refresh_find(false);
        }
        Task::none()
    }

    fn active_resolve(&mut self) -> Option<&mut Resolve> {
        self.tabs.get_mut(self.active)?.merge.as_mut()
    }

    /// Takes the resolve panel's choices once every conflict has one.
    fn apply_resolve(&mut self) -> Task<Message> {
        let Some(tab) = self.tabs.get_mut(self.active) else { return Task::none() };
        let Some(resolve) = tab.merge.take() else { return Task::none() };
        let Some(choices) = resolve.choices.iter().copied().collect::<Option<Vec<Choice>>>() else {
            tab.merge = Some(resolve);
            return Task::none();
        };
        match resolve.source {
            ResolveSource::Disk { theirs } => {
                tab.take_merge(&resolve.merge, &choices, theirs);
                self.status = format!("Merged {}", tab.path);
                Task::none()
            }
            // The disk still holds this note's text, so it stays the base and
            // autosave writes the merge. The copy has nothing left to offer.
            ResolveSource::Copy { path: copy } => {
                let base = tab.base.clone();
                tab.take_merge(&resolve.merge, &choices, base);
                self.trash_file(copy)
            }
        }
    }

    /// Compares a OneDrive conflict copy with its note: lines both have are
    /// kept, every difference is the user's to choose.
    fn compare_copy(&mut self, tab: u64, copy: String, text: &str) -> Task<Message> {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab && t.loaded && t.merge.is_none()) else {
            return Task::none();
        };
        let merge = sage_editor::merge::diff2(&tab.content.text(), text);
        let conflicts = merge.conflicts();
        if conflicts == 0 {
            self.notice(format!("{copy} is the same as this note, so it goes to .trash"));
            return self.trash_file(copy);
        }
        tab.merge = Some(Resolve { source: ResolveSource::Copy { path: copy }, merge, choices: vec![None; conflicts] });
        Task::none()
    }

    /// Stats the open notes off the UI thread; `on_polled` reads the changed ones.
    fn poll_tabs(&mut self) -> Task<Message> {
        let files: Vec<(u64, String, PathBuf)> =
            self.tabs.iter().filter(|t| t.loaded).map(|t| (t.id, t.path.clone(), self.vault.join(&t.path))).collect();
        if self.polling || files.is_empty() {
            return Task::none();
        }
        self.polling = true;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    files.into_iter().map(|(id, path, abs)| (id, path, stamp(&abs))).collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default()
            },
            Message::Polled,
        )
    }

    fn on_polled(&mut self, stamps: Vec<(u64, String, Option<Stamp>)>) -> Task<Message> {
        self.polling = false;
        let mut changed = Vec::new();
        for (id, path, stamp) in stamps {
            // A note that is gone was moved or deleted; the indexer reports that.
            let (Some(tab), Some(stamp)) = (self.tabs.iter_mut().find(|t| t.id == id && t.path == path), stamp) else { continue };
            // The first stamp is only a reference point. Our own saves change it
            // too; their read finds the text equal to the base and stops there.
            if tab.disk_stamp.replace(stamp).is_some_and(|old| old != stamp) {
                changed.push((id, path));
            }
        }
        Task::batch(changed.iter().map(|(id, path)| self.read(*id, path)))
    }

    /// Creates a note in the vault root (or under the folders in `name`) and opens it.
    /// Writes the starter vault in the background; files that exist are kept.
    fn create_starter(&mut self) -> Task<Message> {
        self.palette = None;
        let (vault, backend) = (self.vault.clone(), self.backend.unwrap_or(Backend::Poll));
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || sage_editor::starter::create(&vault, backend))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::StarterCreated,
        )
    }

    fn create(&mut self, name: Option<&str>, default_template: bool) -> Task<Message> {
        self.palette = None;
        let vault = self.vault.clone();
        let name = name.map(sanitize_note_name);
        // Notes made from "New note" start from the default template; notes made by following a link stay empty.
        let template = if default_template { self.settings.new_note_template.clone() } else { None };
        let (config, now) = (self.template_config(), sage_editor::now());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let rel = match name {
                        Some(name) if !name.is_empty() => format!("{name}.md"),
                        _ => (0..)
                            .map(|n| if n == 0 { "Untitled.md".to_owned() } else { format!("Untitled {n}.md") })
                            .find(|candidate| !vault.join(candidate).exists())
                            .expect("an unused name exists"),
                    };
                    if let Some(template) = template.filter(|t| vault.join(t).is_file()) {
                        sage_editor::quick_add::create_note(&vault, &rel, &template, "", now, &config)?;
                        return Ok(rel);
                    }
                    let abs = vault.join(&rel);
                    if let Some(parent) = abs.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&abs)
                        .map_err(|e| e.to_string())?;
                    Ok(rel)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::Created,
        )
    }

    /// Asks before moving a note, or a folder with everything in it, into the
    /// vault's `.trash` folder, as Obsidian does. Refused while a note in it
    /// has unsaved edits.
    fn ask_delete(&mut self, path: String, folder: bool) {
        if let Some(busy) = self.busy_inside(&path, folder) {
            self.notice(format!("Save {busy} before deleting it"));
            return;
        }
        self.palette = None;
        self.doomed = Some(Doomed { path, folder });
    }

    /// The path of a note at `path`, or in it for a folder, whose tab has
    /// text that is not on disk yet.
    fn busy_inside(&self, path: &str, folder: bool) -> Option<String> {
        let inside = |p: &str| p == path || (folder && p.strip_prefix(path).is_some_and(|rest| rest.starts_with('/')));
        self.tabs.iter().find(|t| t.busy() && inside(&t.path)).map(|t| t.path.clone())
    }

    /// Moves a file or folder into the vault's `.trash` folder under a free
    /// name. Recoverable by moving it back; the indexer ignores hidden folders.
    fn trash_file(&self, path: String) -> Task<Message> {
        let vault = self.vault.clone();
        let rel = path.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let trash = vault.join(".trash");
                    std::fs::create_dir_all(&trash).map_err(|e| e.to_string())?;
                    let name = rel.rsplit('/').next().unwrap_or(&rel);
                    let folder = vault.join(&rel).is_dir();
                    let numbered = |n: usize| match name.rsplit_once('.') {
                        Some((stem, ext)) if !stem.is_empty() && !folder => format!("{stem} {n}.{ext}"),
                        _ => format!("{name} {n}"),
                    };
                    let target = (0..)
                        .map(|n| if n == 0 { name.to_owned() } else { numbered(n) })
                        .map(|candidate| trash.join(candidate))
                        .find(|candidate| !candidate.exists())
                        .expect("an unused name exists");
                    std::fs::rename(vault.join(&rel), &target).map_err(|e| e.to_string())?;
                    // As a vault path, with `/` on every platform.
                    Ok(target.strip_prefix(&vault).unwrap_or(&target).to_string_lossy().replace('\\', "/"))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Trashed(path.clone(), result),
        )
    }

    /// Notes a visit to `path` for Back and Forward and the recent notes.
    /// Going back or forward moves along the history instead of adding to it.
    fn visited(&mut self, path: &str) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_owned());
        self.recent.truncate(RECENT);
        if self.history.get(self.history_at).is_some_and(|p| p == path) {
            return;
        }
        if !self.history.is_empty() {
            self.history.truncate(self.history_at + 1);
        }
        self.history.push(path.to_owned());
        if self.history.len() > HISTORY {
            self.history.remove(0);
        }
        self.history_at = self.history.len() - 1;
    }

    /// Back (`-1`) or Forward (`1`) through the notes opened, skipping any
    /// that are gone.
    fn go(&mut self, step: isize) -> Task<Message> {
        let mut at = self.history_at as isize;
        loop {
            at += step;
            let Some(path) = usize::try_from(at).ok().and_then(|i| self.history.get(i)).cloned() else {
                self.notice(if step < 0 { "Nothing to go back to".into() } else { "Nothing to go forward to".into() });
                return Task::none();
            };
            if self.files.iter().any(|f| f.path == path) {
                self.history_at = at as usize;
                return self.open(path);
            }
        }
    }

    /// Applies the "only marked checkboxes are tasks" setting: queries and the
    /// task list run again, as after a change in the index.
    fn apply_tracking(&mut self) {
        sage_db::set_tracked_only(self.settings.tracked_tasks_only);
        self.load_tasks();
        for tab in &mut self.tabs {
            tab.views.clear();
            tab.task_lists.clear();
            tab.stale = true;
        }
    }

    fn reopen_tab(&mut self) -> Task<Message> {
        while let Some(path) = self.closed.pop() {
            if self.files.iter().any(|f| f.path == path) && !self.tabs.iter().any(|t| t.path == path) {
                return self.open(path);
            }
        }
        self.notice("No closed tab to reopen".into());
        Task::none()
    }

    /// Opens today's daily note, creating it first, from the template if the
    /// Daily notes plugin names one, as Obsidian does.
    fn daily_note(&mut self) -> Task<Message> {
        // Obsidian's settings are read off the UI thread, as they are now.
        let vault = self.vault.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || sage_editor::obsidian::Daily::load(&vault))
                    .await
                    .map_err(|e| e.to_string())
            },
            |result| match result {
                Ok(daily) => Message::DailyFound(daily),
                Err(error) => Message::QuickNoteDone { result: Err(error), open: true },
            },
        )
    }

    /// Opens today's note as the Daily notes settings `daily` name it.
    fn open_daily(&mut self, daily: sage_editor::obsidian::Daily) -> Task<Message> {
        let now = sage_editor::now();
        let rel = daily.path(now);
        if self.files.iter().any(|f| f.path == rel) {
            return self.open(rel);
        }
        let (vault, config) = (self.vault.clone(), self.template_config());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    sage_editor::quick_add::create_note(&vault, &rel, &daily.template, "", now, &config)
                        .map(|created| (rel, created))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            |result| Message::QuickNoteDone { result, open: true },
        )
    }

    /// Runs an editing command on the active note. Keys with nothing special
    /// to do there do what they always do.
    fn edit_op(&mut self, op: EditOp) -> Task<Message> {
        let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded && t.merge.is_none()) else {
            return Task::none();
        };
        if matches!(op, EditOp::Undo | EditOp::Redo) {
            let forward = op == EditOp::Redo;
            if !tab.step_history(forward) {
                self.notice(if forward { "Nothing to redo".into() } else { "Nothing to undo".into() });
            }
            self.completion = None;
            self.refresh_find(false);
            return Task::none();
        }
        let (anchor, head) = tab.selection();
        let unit = self.indent_unit.as_str();
        let content = &tab.content;
        let line = |i: usize| content.line(i).map(|l| l.text.into_owned()).unwrap_or_default();
        let change = windowed(content.line_count(), line, anchor, head, |lines, anchor, head| match op {
            EditOp::Format(format) => Some(edit::toggle(lines, anchor, head, format)),
            EditOp::Link => Some(edit::link(lines, anchor, head)),
            EditOp::Enter => edit::enter(lines, head, unit).filter(|_| anchor == head),
            EditOp::Indent(out) => edit::indent(lines, anchor, head, out, unit),
            EditOp::MoveLines(up) => edit::move_lines(lines, anchor, head, up),
            EditOp::Duplicate => Some(edit::duplicate(lines, anchor, head)),
            EditOp::Type(c) => edit::type_char(lines, anchor, head, c),
            EditOp::Backspace => edit::backspace(lines, head).filter(|_| anchor == head),
            EditOp::Undo | EditOp::Redo => None,
        });
        let Some(change) = change else {
            let fallback = match op {
                EditOp::Enter => Edit::Enter,
                EditOp::Type(c) => Edit::Insert(c),
                EditOp::Backspace => Edit::Backspace,
                _ => return Task::none(),
            };
            return self.handle(Message::Edit(text_editor::Action::Edit(fallback)));
        };
        tab.apply(change);
        self.refresh_completion();
        self.refresh_find(false);
        Task::none()
    }

    /// Saves the clipboard's picture, if it holds one and no text, next to
    /// the active note and embeds it there.
    fn paste_image(&mut self) -> Task<Message> {
        let Some(tab) = self.tabs.get(self.active).filter(|t| t.loaded) else { return Task::none() };
        let (vault, note, tab) = (self.vault.clone(), tab.path.clone(), tab.id);
        let name = sage_editor::obsidian::pasted_name(sage_editor::now());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let Some(png) = clipboard_picture()? else { return Ok(None) };
                    let folder = sage_editor::obsidian::attachment_folder(&vault, &note);
                    sage_editor::obsidian::save_attachment(&vault, &folder, &name, &png).map(Some)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Attached { tab, result },
        )
    }

    /// A file dropped on the window: a picture is copied into the active
    /// note's attachments folder and embedded where the cursor is.
    fn drop_file(&mut self, file: PathBuf) -> Task<Message> {
        let Some(tab) = self.tabs.get(self.active).filter(|t| t.loaded && t.merge.is_none()) else {
            return Task::none();
        };
        if !matches!(self.view_mode, ViewMode::Source | ViewMode::Split | ViewMode::Live) || self.settings_open || self.tasks.is_some() {
            return Task::none();
        }
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !is_picture(&name) {
            self.notice(format!("Only pictures can be dropped into a note, not {name}"));
            return Task::none();
        }
        let (vault, note, tab) = (self.vault.clone(), tab.path.clone(), tab.id);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let bytes = std::fs::read(&file).map_err(|e| format!("{name}: {e}"))?;
                    let folder = sage_editor::obsidian::attachment_folder(&vault, &note);
                    sage_editor::obsidian::save_attachment(&vault, &folder, &name, &bytes).map(Some)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Attached { tab, result },
        )
    }

    /// Embeds a picture just saved into the vault at the cursor of the note
    /// it was saved for.
    fn on_attached(&mut self, tab: u64, result: Result<Option<String>, String>) -> Task<Message> {
        let path = match result {
            Ok(Some(path)) => path,
            Ok(None) => return Task::none(),
            Err(error) => {
                self.notice(format!("The picture was not saved: {error}"));
                return Task::none();
            }
        };
        let Some(i) = self.tabs.iter().position(|t| t.id == tab && t.loaded) else {
            self.notice(format!("Saved {path}"));
            return Task::none();
        };
        // Resolving the merge replaces the text, and the link with it.
        if self.tabs[i].merge.is_some() {
            self.notice(format!("Saved {path}; not linked while {} has changes to resolve", self.tabs[i].path));
            return Task::none();
        }
        // The name alone, as Obsidian writes it, unless another file has it.
        let name = path.rsplit('/').next().unwrap_or(&path);
        let shared = self.files.iter().any(|f| f.path != path && f.path.rsplit('/').next() == Some(name));
        let target = if shared { path.as_str() } else { name };
        self.tabs[i].insert_at_cursor(&format!("![[{target}]]"));
        self.notice(format!("Saved {path}"));
        Task::none()
    }

    /// Closes every tab whose index `selected` picks, except the ones with
    /// unsaved changes, which stay open and are named.
    fn close_tabs(&mut self, selected: impl Fn(usize) -> bool) {
        self.menu = None;
        self.tab_menu = None;
        let (gone, kept): (Vec<usize>, Vec<usize>) = (0..self.tabs.len())
            .filter(|&i| selected(i) && !self.pinned.contains(&self.tabs[i].path))
            .partition(|&i| !self.tabs[i].busy());
        if !kept.is_empty() {
            self.notice(format!("{} tab(s) with unsaved changes stay open", kept.len()));
        }
        // By id: blank tabs all share the empty path.
        let ids: Vec<u64> = gone.iter().map(|&i| self.tabs[i].id).collect();
        let current = self.tabs.get(self.active).map(|t| t.id);
        self.active = active_after(self.active, &self.tabs, |t| ids.contains(&t.id));
        let mut closed = Vec::new();
        self.tabs.retain(|t| {
            let drop = ids.contains(&t.id);
            if drop && !t.path.is_empty() {
                closed.push(t.path.clone());
            }
            !drop
        });
        self.closed.extend(closed);
        if !self.tabs.iter().any(|t| t.tasks) {
            self.tasks = None;
        }
        // The current note stays current when it is not one that closed.
        if let Some(i) = current.and_then(|c| self.tabs.iter().position(|t| t.id == c)) {
            self.active = i;
        }
        self.completion = None;
        self.find = None;
        self.persist();
    }

    fn close_tab(&mut self, i: usize) {
        self.menu = None;
        self.tab_menu = None;
        let Some(tab) = self.tabs.get(i) else { return };
        if tab.dirty {
            // Closing would drop unsaved text; autosave will finish shortly.
            self.notice(format!("{} has unsaved changes. Save first (Ctrl+S)", tab.path));
            return;
        }
        let id = tab.id;
        self.active = active_after(self.active, &self.tabs, |t| t.id == id);
        let closed = self.tabs.remove(i);
        if !closed.path.is_empty() {
            self.closed.push(closed.path);
        }
        if closed.tasks {
            self.tasks = None;
        }
        self.completion = None;
        self.find = None;
        self.persist();
    }

    /// The window's close button. Unsaved notes and settings are written
    /// first, as before a vault switch, so text typed since the last autosave
    /// is kept. A note that cannot be saved (in conflict, or its save failed)
    /// stops the close once; asking again closes without it.
    fn close_requested(&mut self) -> Task<Message> {
        let force = std::mem::replace(&mut self.closing, true);
        let mut saves: Vec<Task<Message>> = (0..self.tabs.len()).map(|i| self.save(i, false)).collect();
        saves.push(self.save_settings());
        Task::batch(saves).chain(Task::done(Message::Quit { force }))
    }

    fn quit(&mut self, force: bool) -> Task<Message> {
        // An autosave that was already under way: wait for it rather than cut
        // it off, since a note on OneDrive is rewritten in place. A second
        // close does not wait for a save that hangs, on a lost network drive.
        // A rename under way holds saves back, and starts them when it is done.
        if !force && (self.renaming.is_some() || self.tabs.iter().any(|t| t.saving)) {
            return Task::perform(
                async { tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(100))).await },
                move |_| Message::Quit { force },
            );
        }
        let unsaved: Vec<&str> = self.tabs.iter().filter(|t| t.busy()).map(|t| t.title.as_str()).collect();
        if !force {
            if let [first, rest @ ..] = unsaved.as_slice() {
                let more = if rest.is_empty() { String::new() } else { format!(" and {} more", rest.len()) };
                self.notice(format!("Not closed: {first}{more} could not be saved. Close again to discard"));
                return Task::none();
            }
        }
        self.persist_now();
        iced::exit()
    }

    /// Moves the divider being dragged to `x`, within limits that keep every
    /// pane usable.
    fn resize(&mut self, x: f32) {
        match self.resizing {
            Some(Divider::Sidebar) => self.panes.sidebar = x.clamp(MIN_SIDEBAR, self.max_sidebar()),
            Some(Divider::Split) => {
                let start = self.sidebar_width() + DIVIDER;
                let room = self.window_width - start - DIVIDER;
                if room > 0.0 {
                    self.panes.source = ((x - start - DIVIDER / 2.0) / room).clamp(0.1, 0.9);
                }
            }
            None => {}
        }
    }

    /// The widest the folder tree may be: the workspace keeps room to work.
    fn max_sidebar(&self) -> f32 {
        (self.window_width - MIN_WORKSPACE).max(MIN_SIDEBAR)
    }

    /// The folder tree's width now, which a narrowed window may cut down.
    fn sidebar_width(&self) -> f32 {
        self.panes.sidebar.min(self.max_sidebar())
    }

    fn scale_factor(&self) -> f32 {
        self.zoom
    }

    fn persist(&self) {
        if let Some(json) = self.session_json() {
            store::write_later(session_path(&self.db_path), json);
        }
    }

    /// Writes the session and waits for it and the Kanban order, for when
    /// Sage is about to exit.
    fn persist_now(&self) {
        self.persist();
        store::flush();
    }

    fn session_json(&self) -> Option<String> {
        let session = Session {
            tabs: self.tabs.iter().map(|t| t.path.clone()).collect(),
            active: self.active,
            view_mode: self.view_mode,
            dark_override: self.dark_override,
            expanded: self.expanded.iter().cloned().collect(),
            folds: self.folds.clone(),
            panes: Some(self.panes),
            tree_order: self.tree_order,
            recent: self.recent.clone(),
            pinned: self.pinned.clone(),
            outline: self.outline_open,
            zoom: Some(self.zoom),
            reminders_checked: self.reminders_checked.map(|t| t.format(CHECKED_FORMAT).to_string()),
        };
        serde_json::to_string_pretty(&session).ok()
    }

    fn title_of(&self, path: &str) -> String {
        file_stem(path).to_owned()
    }

    fn reload_files(&mut self) {
        let Some(reader) = &self.reader else { return };
        let rows = reader
            .prepare_cached("SELECT path, folder, cloud, modified FROM v_notes ORDER BY path COLLATE NOCASE")
            .and_then(|mut stmt| {
                // Navigation shows file names, as Obsidian does: links resolve
                // by file name, so that is the name users need to see.
                stmt.query_map([], |r| {
                    let path: String = r.get(0)?;
                    Ok(FileRow {
                        title: file_stem(&path).to_owned(),
                        folder: r.get(1)?,
                        cloud: r.get(2)?,
                        modified: r.get(3)?,
                        path,
                    })
                })?
                    .collect::<Result<Vec<_>, _>>()
            });
        if let Ok(rows) = rows {
            let aliases: Vec<String> = reader
                .prepare_cached("SELECT alias_lc FROM aliases")
                .and_then(|mut stmt| stmt.query_map([], |r| r.get(0))?.collect())
                .unwrap_or_default();
            live::set_known(rows.iter().map(|f| f.path.clone()), aliases);
            self.files = rows;
            self.refresh_folders();
        }
    }

    /// Makes the folder lists again, after `files` or `disk_folders` changed.
    fn refresh_folders(&mut self) {
        (self.note_folders, self.tree_folders) = folder_lists(&self.files, &self.disk_folders);
    }

    /// Brings changed previews up to date: runs view blocks that have no result
    /// yet, and starts loading new embeds, also those in embedded notes. Runs
    /// after every message.
    /// Brings the active tab's views, task lists, documents, embeds and links
    /// up to date with its text and the index. Other tabs stay stale until
    /// they are shown: after every autosave the index changes, and running
    /// every view of every open note again would be work nobody sees.
    fn sync_previews(&mut self) -> Task<Message> {
        let mut loads = Vec::new();
        let (preview, live, outline_open) = (self.view_mode != ViewMode::Source, self.view_mode == ViewMode::Live, self.outline_open);
        self.settling = self.tabs.get_mut(self.active).is_some_and(|tab| tab.catch_up(preview, live, outline_open));
        if self.tabs.get(self.active).is_some_and(|t| t.links_stale) {
            self.tabs[self.active].links_stale = false;
            loads.push(self.load_links(self.active));
        }
        // The index changed: views, task lists and the documents links name
        // run again, though not on every change of a burst. Until then the
        // old results show; the Settle tick comes back for the rest.
        if let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.index_stale) {
            if self.previews_run.elapsed() >= PREVIEWS_EVERY {
                self.previews_run = Instant::now();
                tab.index_stale = false;
                tab.views.clear();
                tab.task_lists.clear();
                tab.docs.clear();
                tab.stale = true;
            } else {
                self.settling = true;
            }
        }
        let i = self.active;
        if self.tabs.get(i).is_some_and(|t| t.stale) {
            self.tabs[i].stale = false;
            let (mut ignored, mut sources, mut queries) = (Vec::new(), Vec::new(), Vec::new());
            preview::collect(&self.tabs[i].preview, &mut ignored, &mut sources, &mut queries);
            let sources: Vec<String> = sources.into_iter().map(str::to_owned).collect();
            let queries: Vec<String> = queries.into_iter().map(str::to_owned).collect();
            let mut embeds = Vec::new();
            let mut chain = vec![self.tabs[i].path.clone()];
            embeds_in(&self.tabs[i].preview, &mut chain, &self.tabs[i].embeds, &mut embeds);

            for source in &sources {
                if !self.tabs[i].views.contains_key(source) {
                    let mut outcome = self.run_view(source);
                    if let Ok(Outcome::Board(board)) = &mut outcome {
                        self.order.apply(board);
                    }
                    self.tabs[i].views.insert(source.clone(), outcome);
                }
            }
            self.tabs[i].views.retain(|source, _| sources.contains(source));

            for source in &queries {
                if !self.tabs[i].task_lists.contains_key(source) {
                    let list = self.run_tasks(source);
                    self.tabs[i].task_lists.insert(source.clone(), list);
                }
            }
            self.tabs[i].task_lists.retain(|source, _| queries.contains(source));

            // The documents that links and embeds name, also in embedded notes.
            let tab = &self.tabs[i];
            let mut keys: Vec<String> =
                sage_markdown::references(&tab.preview).iter().map(|t| sage_core::target_key(t, &tab.path)).collect();
            for (src, embed) in &embeds {
                if !embed.external && embed.heading.is_none() && embed.block.is_none() {
                    keys.push(sage_core::target_key(&embed.target, src));
                }
                if let Some(Embedded::Note { path, blocks }) = tab.embeds.get(&preview::embed_key(src, embed)) {
                    keys.extend(sage_markdown::references(blocks).iter().map(|t| sage_core::target_key(t, path)));
                }
            }
            for key in &keys {
                if !self.tabs[i].docs.contains_key(key) {
                    let doc = self.document(key);
                    self.tabs[i].docs.insert(key.clone(), doc);
                }
            }
            self.tabs[i].docs.retain(|key, _| keys.contains(key));

            let tab_id = self.tabs[i].id;
            let mut wanted = HashSet::new();
            for (src, embed) in &embeds {
                let key = preview::embed_key(src, embed);
                if !self.tabs[i].embeds.contains_key(&key) {
                    self.tabs[i].embeds.insert(key.clone(), Embedded::Loading);
                    loads.push(self.load_embed(tab_id, src, key.clone(), embed));
                }
                wanted.insert(key);
            }
            self.tabs[i].embeds.retain(|key, _| wanted.contains(key));
        }
        Task::batch(loads)
    }

    /// Runs one view block against the index. Short read-only SQL, which the UI
    /// thread may do.
    fn run_view(&self, source: &str) -> Result<Outcome, String> {
        let spec = sage_query::parse_spec(source)?;
        let reader = self.reader.as_ref().ok_or("the index is not open yet")?;
        sage_query::run(reader, &spec, sage_editor::now().date()).map_err(|e| e.to_string())
    }

    /// Runs one `tasks` block against the index, as `run_view` does a view.
    fn run_tasks(&self, source: &str) -> Result<sage_query::documents::TaskList, String> {
        let query = sage_query::documents::parse_tasks(source)?;
        let reader = self.reader.as_ref().ok_or("the index is not open yet")?;
        sage_query::documents::run_tasks(reader, &query, sage_editor::now().date()).map_err(|e| e.to_string())
    }

    /// Loads an embed written in the note at `src` off the UI thread: a
    /// picture's bytes, or a note's text, cut down to the section asked for
    /// and parsed.
    fn load_embed(&self, tab: u64, src: &str, key: String, embed: &sage_markdown::Embed) -> Task<Message> {
        let found = if embed.external { None } else { self.resolve(&sage_core::target_key(&embed.target, src)) };
        let done = move |embedded| Message::EmbedLoaded { tab, key: key.clone(), embedded };
        let Some(path) = found else {
            let why = if embed.external { "pictures from the web are not loaded" } else { "not in the vault" };
            return Task::done(done(Embedded::Missing(why.into())));
        };
        let abs = self.vault.join(&path);
        if is_note_path(&path) {
            let (heading, block) = (embed.heading.clone(), embed.block.clone());
            Task::perform(
                async move {
                    let text = tokio::fs::read_to_string(&abs).await.map_err(|e| e.to_string())?;
                    let part = match (heading, block) {
                        (Some(h), _) => sage_markdown::section(&text, &h).ok_or(format!("no heading \u{201c}{h}\u{201d}"))?,
                        (None, Some(b)) => sage_markdown::block(&text, &b).ok_or(format!("no block ^{b}"))?,
                        (None, None) => text,
                    };
                    Ok(sage_markdown::parse(&part))
                },
                move |result: Result<Vec<sage_markdown::Block>, String>| {
                    done(match result {
                        Ok(blocks) => Embedded::Note { path: path.clone(), blocks },
                        Err(why) => Embedded::Missing(why),
                    })
                },
            )
        } else if is_picture(&path) {
            // Decoded here rather than by iced as it draws, which would stall
            // a frame on a large photo. What does not decode is left to iced.
            let read = async move {
                tokio::task::spawn_blocking(move || {
                    let bytes = std::fs::read(&abs).map_err(|e| e.to_string())?;
                    Ok(match image::load_from_memory(&bytes) {
                        Ok(decoded) => {
                            let rgba = decoded.to_rgba8();
                            iced::widget::image::Handle::from_rgba(rgba.width(), rgba.height(), rgba.into_raw())
                        }
                        Err(_) => iced::widget::image::Handle::from_bytes(bytes),
                    })
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            };
            Task::perform(read, move |result| {
                done(match result {
                    Ok(handle) => Embedded::Image { path: path.clone(), handle },
                    Err(why) => Embedded::Missing(why),
                })
            })
        } else if path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("svg")) {
            Task::perform(async move { tokio::fs::read(&abs).await.map_err(|e| e.to_string()) }, move |result| {
                done(match result {
                    Ok(bytes) => Embedded::Svg { path: path.clone(), handle: iced::widget::svg::Handle::from_memory(bytes) },
                    Err(why) => Embedded::Missing(why),
                })
            })
        } else {
            let ext = path.rsplit_once('.').map_or("", |(_, ext)| ext).to_lowercase();
            Task::done(done(Embedded::Missing(format!("Sage cannot show .{ext} files yet"))))
        }
    }

    /// Backlinks, outgoing links and OneDrive conflict copies of a tab, as of
    /// the last index pass. The timeline reads the text of every note that
    /// links to this one, so it is read off the UI thread, by the task returned.
    fn load_links(&mut self, i: usize) -> Task<Message> {
        if self.onedrive {
            if let Some(path) = self.tabs.get(i).map(|t| t.path.clone()) {
                self.tabs[i].copies = conflict_copies(&self.files, &path);
            }
        }
        let (Some(reader), Some(tab)) = (&self.reader, self.tabs.get_mut(i)) else { return Task::none() };
        let rows = reader
            .prepare_cached(
                "SELECT DISTINCT b.src_path, b.src_title
                 FROM v_backlinks b JOIN files f ON f.id = b.file_id
                 WHERE f.path = ?1 AND b.src_file_id <> b.file_id
                 ORDER BY b.src_title COLLATE NOCASE",
            )
            .and_then(|mut stmt| {
                stmt.query_map([&tab.path], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()
            });
        if let Ok(rows) = rows {
            tab.backlinks = rows;
        }
        if let Ok(rows) = tasks::followups(reader, &tab.path) {
            tab.followups = rows;
        }
        tab.timeline_asked = unique();
        let (id, path, asked, db_path) = (tab.id, tab.path.clone(), tab.timeline_asked, self.db_path.clone());
        let timeline = Task::perform(
            async move {
                let note = path.clone();
                let rows = tokio::task::spawn_blocking(move || {
                    // A connection of its own: the UI's reader stays on the UI thread.
                    let db = sage_db::open_reader(&db_path).ok()?;
                    timeline::load(&db, &note).ok()
                })
                .await
                .ok()
                .flatten();
                (path, rows)
            },
            move |(path, rows)| Message::TimelineLoaded { tab: id, path, asked, rows },
        );

        let rows = reader
            .prepare_cached(
                "SELECT l.target, d.path FROM links l
                 JOIN files s ON s.id = l.src_file_id
                 LEFT JOIN files d ON d.id = l.dst_file_id
                 WHERE s.path = ?1 AND l.target <> '' AND (d.id IS NULL OR d.id <> s.id)
                 ORDER BY l.line, l.col",
            )
            .and_then(|mut stmt| {
                stmt.query_map([&tab.path], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?
                    .collect::<Result<Vec<_>, _>>()
            });
        if let Ok(rows) = rows {
            let mut seen = HashSet::new();
            tab.outgoing = rows
                .into_iter()
                .filter(|(target, path)| seen.insert(path.clone().unwrap_or_else(|| sage_core::link_key(target))))
                .map(|(target, path)| match path {
                    Some(path) if is_note_path(&path) => Outgoing { label: file_stem(&path).to_owned(), kind: OutKind::Note(path) },
                    Some(path) => Outgoing { label: path.rsplit('/').next().unwrap_or(&path).to_owned(), kind: OutKind::Attachment },
                    None if is_attachment(&target) => Outgoing { label: target, kind: OutKind::Attachment },
                    None => Outgoing { label: target.clone(), kind: OutKind::Missing(target) },
                })
                .collect();
        }
        timeline
    }

    // ---- rename -------------------------------------------------------------

    /// Opens the palette on a note's path, with the name selected so
    /// typing replaces it and keeps the folder.
    fn start_rename(&mut self, from: String) -> Task<Message> {
        let stem = from.strip_suffix(".md").unwrap_or(&from).to_owned();
        let name_start = stem.rfind('/').map_or(0, |i| i + 1);
        let (start, end) = (stem[..name_start].chars().count(), stem.chars().count());
        self.completion = None;
        self.palette = Some(Palette { mode: PaletteMode::Rename(from), query: stem, ..Palette::default() });
        self.search();
        iced::widget::operation::focus(PALETTE_INPUT)
            .chain(iced::widget::operation::select_range(PALETTE_INPUT, start, end))
    }

    /// Opens the palette on the folders the note at `from` can move to.
    fn start_move(&mut self, from: String) -> Task<Message> {
        self.completion = None;
        self.palette = Some(Palette { mode: PaletteMode::MoveTo(from), ..Palette::default() });
        self.search();
        iced::widget::operation::focus(PALETTE_INPUT)
    }

    /// Opens the palette on a folder's path, with its name selected.
    fn start_folder_rename(&mut self, folder: String) -> Task<Message> {
        let name_start = folder.rfind('/').map_or(0, |i| i + 1);
        let (start, end) = (folder[..name_start].chars().count(), folder.chars().count());
        self.completion = None;
        self.palette = Some(Palette { mode: PaletteMode::RenameFolder(folder.clone()), query: folder, ..Palette::default() });
        self.search();
        iced::widget::operation::focus(PALETTE_INPUT)
            .chain(iced::widget::operation::select_range(PALETTE_INPUT, start, end))
    }

    fn rename(&mut self, request: Move) -> Task<Message> {
        if self.renaming.is_some() {
            self.notice("A rename is under way; try again once it is done".into());
            return Task::none();
        }
        if let Some(tab) = self.tabs.iter().find(|t| t.busy() && moves(&request, &t.path)) {
            self.notice(format!("Save {} before renaming it", tab.path));
            return Task::none();
        }
        // The notes to rewrite come from the index, which on a polled vault can
        // trail the disk by a whole interval: let it catch up first.
        if let (Some(Backend::Poll), Some(IndexerLink(handle))) = (self.backend, &self.indexer) {
            self.next_sync += 1;
            handle.sync(self.next_sync);
            self.pending_rename = Some((self.next_sync, request));
            self.status = "Checking the vault for links before renaming…".into();
            return Task::none();
        }
        self.run_rename(request)
    }

    fn run_rename(&mut self, request: Move) -> Task<Message> {
        // Notes with unsaved text are never rewritten underneath the editor.
        let busy: HashSet<String> = self.tabs.iter().filter(|t| t.busy()).map(|t| t.path.clone()).collect();
        if let Some(path) = busy.iter().find(|path| moves(&request, path)) {
            self.notice(format!("Save {path} before renaming it"));
            return Task::none();
        }
        if self.renaming.is_some() {
            self.notice("A rename is under way; try again once it is done".into());
            return Task::none();
        }
        let (vault, db_path) = (self.vault.clone(), self.db_path.clone());
        let backend = self.backend.unwrap_or(Backend::Poll);
        self.status = format!("Renaming {}…", request.from);
        // From here until `Renamed` comes back, the notes it moves are not
        // saved: typing goes on, and autosave would write the old path.
        self.renaming = Some(request.clone());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    // A connection of its own: the UI's reader stays on the UI thread.
                    let db = sage_db::open_reader(&db_path).map_err(|e| format!("{e:#}"))?;
                    let Move { from, to, folder } = request;
                    let rename = if folder { sage_editor::rename::rename_folder } else { sage_editor::rename::rename };
                    rename(&vault, &db, &from, &to, &busy, backend).map(Arc::new)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::Renamed,
        )
    }

    fn on_renamed(&mut self, rename: Arc<Rename>) {
        for tab in &mut self.tabs {
            if let Some(to) = rename.moved(&tab.path) {
                tab.title = file_stem(&to).to_owned();
                tab.path = to;
            }
            if let Some(rw) = rename.rewrites.iter().find(|rw| rw.path == tab.path) {
                tab.follow_disk(rw.after.clone());
            }
        }
        self.refresh_find(false);
        self.follow_folders(&rename.from, &rename.to, rename.folder);
        // Folded callouts stay with their note.
        self.folds = std::mem::take(&mut self.folds).into_iter().map(|(p, f)| (rename.moved(&p).unwrap_or(p), f)).collect();
        self.follow_paths(|p| rename.moved(p));
        let what = if rename.folder { "Moved" } else { "Renamed" };
        self.notice(match (rename.links, rename.rewrites.len()) {
            (0, _) => format!("{what} {} to {}", rename.from, rename.to),
            (links, notes) => format!(
                "{what} {} to {} and rewrote {links} {} in {notes} {}. Ctrl+P > Undo last rename reverts all of it",
                rename.from,
                rename.to,
                if links == 1 { "link" } else { "links" },
                if notes == 1 { "note" } else { "notes" },
            ),
        });
        self.renames.push(rename);
        if self.renames.len() > RENAMES {
            self.renames.remove(0);
        }
        self.reload_files();
        self.persist();
    }

    /// Whether the note at `path` must not be saved now: a rename under way
    /// moves it, and a save would write where it is being moved away from.
    fn rename_holds(&self, path: &str) -> bool {
        self.renaming.as_ref().is_some_and(|request| moves(request, path))
    }

    /// A rename, or its undoing, came back, done or not: notes it held back
    /// are saved now, under whatever path they have after it.
    fn rename_done(&mut self) -> Task<Message> {
        self.renaming = None;
        let held: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.tabs[i].dirty && !self.tabs[i].conflict).collect();
        Task::batch(held.into_iter().map(|i| self.save(i, false)))
    }

    /// Keeps the tree as it was across a rename: a moved folder stays open,
    /// and the place a note went is shown.
    fn follow_folders(&mut self, from: &str, to: &str, folder: bool) {
        if folder {
            let prefix = format!("{from}/");
            self.expanded = std::mem::take(&mut self.expanded)
                .into_iter()
                .map(|f| {
                    if f == from {
                        to.to_owned()
                    } else if let Some(rest) = f.strip_prefix(&prefix) {
                        format!("{to}/{rest}")
                    } else {
                        f
                    }
                })
                .collect();
        }
        for folder in tree::ancestors(to) {
            self.expanded.insert(folder.to_owned());
        }
    }

    fn undo_rename(&mut self) -> Task<Message> {
        let Some(rename) = self.renames.last().cloned() else {
            self.notice("No rename to undo".into());
            return Task::none();
        };
        let touched = |path: &str| rename.unmoved(path).is_some() || rename.rewrites.iter().any(|rw| rw.path == path);
        if let Some(tab) = self.tabs.iter().find(|t| t.busy() && touched(&t.path)) {
            self.notice(format!("Save {} before undoing the rename", tab.path));
            return Task::none();
        }
        if self.renaming.is_some() {
            self.notice("A rename is under way; try again once it is done".into());
            return Task::none();
        }
        let vault = self.vault.clone();
        let backend = self.backend.unwrap_or(Backend::Poll);
        // Moving back holds saves back as a rename does.
        self.renaming = Some(Move { from: rename.to.clone(), to: rename.from.clone(), folder: rename.folder });
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    sage_editor::rename::undo(&vault, &rename, backend).map(|restored| (rename, restored))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::RenameUndone,
        )
    }

    fn on_rename_undone(&mut self, rename: Arc<Rename>, restored: Vec<(String, String)>) {
        self.renames.retain(|r| !Arc::ptr_eq(r, &rename));
        for tab in &mut self.tabs {
            if let Some(from) = rename.unmoved(&tab.path) {
                tab.title = file_stem(&from).to_owned();
                tab.path = from;
            }
            if let Some((_, text)) = restored.iter().find(|(path, _)| *path == tab.path) {
                tab.follow_disk(text.clone());
            }
        }
        self.refresh_find(false);
        self.follow_folders(&rename.to, &rename.from, rename.folder);
        self.folds = std::mem::take(&mut self.folds).into_iter().map(|(p, f)| (rename.unmoved(&p).unwrap_or(p), f)).collect();
        self.follow_paths(|p| rename.unmoved(p));
        self.notice(format!("Undid the rename: {} is back at {}", file_stem(&rename.from), rename.from));
        self.reload_files();
        self.persist();
    }

    /// Back and Forward, the notes opened lately and the closed tabs follow
    /// a note to where a rename moved it, so none of them loses it.
    fn follow_paths(&mut self, moved: impl Fn(&str) -> Option<String>) {
        for path in self.history.iter_mut().chain(&mut self.recent).chain(&mut self.closed) {
            if let Some(to) = moved(path) {
                *path = to;
            }
        }
    }

    /// A status message the user needs to read, kept up for `NOTICE_FOR`
    /// instead of giving way to the next index pass.
    fn notice(&mut self, message: String) {
        self.status = message.clone();
        self.status_until = Instant::now() + NOTICE_FOR;
        self.toast = Some(Toast { text: message, undo: None, open: None, until: Instant::now() + TOAST_FOR });
    }

    // ---- tasks --------------------------------------------------------------

    /// Reads the open tasks with reminders; short read-only SQL.
    fn load_reminders(&mut self) {
        let Some(reader) = &self.reader else { return };
        match sage_editor::reminders::load(reader) {
            Ok(list) => self.reminders = list,
            Err(error) => self.notice(format!("Reminders were not read: {error}")),
        }
    }

    /// Tells the reminders that went off since the last look: in Sage's
    /// toast, and as a system notification unless the settings turn those
    /// off. The first look after start gathers the ones missed while Sage was
    /// closed into one. Nothing is told before the first index pass, nor on
    /// the very first run, which only starts the clock.
    fn check_reminders(&mut self) -> Task<Message> {
        use sage_editor::reminders;
        if !self.indexed {
            return Task::none();
        }
        let now = sage_editor::now();
        let missed = !std::mem::replace(&mut self.reminders_caught_up, true);
        let after = match self.reminders_checked {
            // A clock set back keeps the later mark, so nothing is told twice.
            Some(after) if after >= now => return Task::none(),
            Some(after) => after,
            None => {
                self.reminders_checked = Some(now);
                self.persist();
                return Task::none();
            }
        };
        self.reminders_checked = Some(now);
        let default = self.settings.reminder_time.as_deref().and_then(reminders::parse_time);
        let default = default.unwrap_or_else(|| reminders::parse_time(reminders::DEFAULT_TIME).expect("a valid time"));
        let hits = reminders::between(&self.reminders, after, now, default);
        let Some(first) = hits.first() else { return Task::none() };
        let (title, body) = match hits.len() {
            1 if missed => ("Missed reminder".to_owned(), first.text.clone()),
            1 => ("Reminder".to_owned(), first.text.clone()),
            n => {
                let title = if missed { format!("{n} reminders while Sage was closed") } else { format!("{n} reminders") };
                let mut body: Vec<&str> = hits.iter().take(3).map(|r| r.text.as_str()).collect();
                let more = (n > 3).then(|| format!("and {} more", n - 3));
                body.extend(more.as_deref());
                (title, body.join("; "))
            }
        };
        let open = (hits.len() == 1).then(|| (first.path.clone(), first.line));
        self.status = format!("{title}: {body}");
        self.status_until = Instant::now() + NOTICE_FOR;
        self.toast = Some(Toast { text: format!("{title}: {body}"), undo: None, open, until: Instant::now() + REMINDER_TOAST_FOR });
        // Marked as told before Sage might close, so the next start does not tell it again.
        self.persist();
        if self.settings.notifications == Some(false) {
            return Task::none();
        }
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || notify::show(&title, &body)).await.unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::Notified,
        )
    }

    fn load_tasks(&mut self) {
        let (Some(list), Some(reader)) = (&mut self.tasks, &self.reader) else { return };
        list.today = sage_editor::today();
        if let Ok(rows) = tasks::load(reader, &list.today) {
            list.rows = rows;
        }
    }

    /// Rewrites a task line on disk: toggles it, or with `column` moves it to
    /// a Kanban column as `(slug, done)`. For a task document, sets its
    /// `status` instead; see `tasks::document_status`. A note with unsaved
    /// edits is left alone.
    fn write_task(&mut self, task: TaskRef, column: Option<(Option<String>, bool)>) -> Task<Message> {
        if self.tabs.iter().any(|t| t.path == task.path && t.busy()) {
            self.notice(format!("Save {} first, it has unsaved changes", task.path));
            return Task::none();
        }
        let (abs, db_path) = (self.vault.join(&task.path), self.db_path.clone());
        let vault = self.vault.clone();
        let backend = self.backend.unwrap_or(Backend::Poll);
        let TaskRef { path, line, raw, document } = task;
        // A task document gets a status instead of a line edit.
        let status = document
            .then(|| tasks::document_status(&mut self.reopen, &path, Some(raw.as_str()).filter(|s| !s.is_empty()), column.as_ref()));
        let rel = path.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let today = sage_editor::today();
                    // The text the index read the line from, to merge with
                    // in case the note changed since.
                    let base: Option<String> = sage_db::open_reader(&db_path).ok().and_then(|db| {
                        db.query_row(
                            "SELECT t.text FROM texts t JOIN files f ON f.id = t.file_id WHERE f.path = ?1",
                            [&rel],
                            |r| r.get(0),
                        )
                        .ok()
                    });
                    let base = base.as_deref();
                    if let Some(status) = status {
                        let expected = Some(raw.as_str()).filter(|s| !s.is_empty());
                        return sage_editor::tasks::set_status(&abs, expected, &status, base, backend);
                    }
                    let result = match &column {
                        None => sage_editor::tasks::toggle(&abs, line, &raw, base, &today, backend),
                        Some((slug, done)) => sage_editor::tasks::set_column(
                            &abs,
                            line,
                            &raw,
                            base,
                            slug.as_deref(),
                            *done,
                            &today,
                            backend,
                        ),
                    };
                    // Checking a checkbox that links a task note closes the note.
                    if result.is_ok() {
                        let after = match &column {
                            None => sage_editor::tasks::toggled(&raw, &today),
                            Some((slug, done)) => sage_editor::tasks::moved(&raw, slug.as_deref(), *done, &today),
                        };
                        if let Some(after) = after.as_ref().and_then(|lines| lines.last()) {
                            let targets = sage_editor::tasks::checked_links(&raw, after);
                            close_linked_notes(&db_path, &vault, &rel, &targets, backend);
                        }
                    }
                    result
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::TaskWritten { path: path.clone(), result },
        )
    }

    /// Drops a dragged card on another column of its board.
    fn move_card(&mut self, drag: &views::Drag, column: usize) -> Task<Message> {
        let target = self.tabs.get(self.active).and_then(|tab| match tab.views.get(&drag.view) {
            Some(Ok(Outcome::Board(board))) => {
                board.columns.get(column).map(|c| (c.slug.clone(), column + 1 == board.columns.len()))
            }
            _ => None,
        });
        let Some((slug, last)) = target else { return Task::none() };
        // The last column is the checkbox: checked, and no status tag.
        let column = if last { (None, true) } else { (Some(slug), false) };
        self.write_task(drag.card.clone(), Some(column))
    }

    /// Puts a dropped card where it was dropped in `column`: before the card
    /// under the pointer, or last.
    fn place_card(&mut self, drag: &views::Drag, column: usize) {
        let shown: Vec<String> = match self.tabs.get(self.active).and_then(|tab| tab.views.get(&drag.view)) {
            Some(Ok(Outcome::Board(board))) => match board.columns.get(column) {
                Some(c) => c.cards.iter().map(order::row_key).collect(),
                None => return,
            },
            _ => return,
        };
        let before = drag.slot.and_then(|i| shown.get(i)).cloned();
        // Dropped on itself.
        if before.as_ref() == Some(&drag.key) {
            return;
        }
        self.order.place(&shown, &drag.key, before.as_deref());
        self.apply_order();
    }

    /// Sorts the cards of every board on show by the hand-placed order.
    fn apply_order(&mut self) {
        for tab in &mut self.tabs {
            for outcome in tab.views.values_mut().flatten() {
                if let Outcome::Board(board) = outcome {
                    self.order.apply(board);
                }
            }
        }
    }

    fn on_task_written(&mut self, path: String, text: String) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.path == path) {
            tab.follow_disk(text.clone());
        }
        self.refresh_find(false);
        // Show the new state now. The index catches up a moment later, or a
        // whole polling interval later on 9p.
        let changed = |line: u32, raw: &str| sage_core::line_at(&text, line).filter(|l| *l != raw).map(str::to_owned);
        // A task document's rows show its status.
        let status = sage_editor::tasks::status_of(&text);
        if let Some(list) = &mut self.tasks {
            for row in list.rows.iter_mut().filter(|r| r.path == path && !r.document) {
                if let Some(line) = changed(row.line, &row.raw) {
                    row.done = sage_core::parse(&line).tasks.first().is_some_and(|t| matches!(t.state, 'x' | 'X'));
                    row.raw = line;
                }
            }
            tasks::patch_document(list, &path, status.as_deref());
        }
        for tab in &mut self.tabs {
            for outcome in tab.views.values_mut().flatten() {
                let stale: Vec<(u32, String)> = views::task_rows(outcome)
                    .filter(|row| row.path == path && !row.document)
                    .filter_map(|row| changed(row.line, &row.raw).map(|line| (row.line, line)))
                    .collect();
                for (line, raw) in stale {
                    views::patch(outcome, &path, line, &raw);
                }
                views::patch_document(outcome, &path, status.as_deref());
            }
        }
        // A card that changed column goes to its place there.
        self.apply_order();
    }

    // ---- templates ----------------------------------------------------------

    fn find_templates(&mut self) -> Task<Message> {
        if !self.tabs.get(self.active).is_some_and(|t| t.loaded) {
            self.notice("Open a note to insert a template into".into());
            return Task::none();
        }
        let (vault, folder) = (self.vault.clone(), self.settings.templates_folder.clone());
        Task::perform(
            async move {
                // Obsidian's settings as they are now, with Sage's folder if set.
                tokio::task::spawn_blocking(move || {
                    let config = TemplateConfig::load(&vault);
                    TemplateConfig { folder: folder.unwrap_or(config.folder.clone()), ..config }
                })
                .await
                .map_err(|e| e.to_string())
            },
            Message::TemplatesFound,
        )
    }

    fn choose_template(&mut self, config: TemplateConfig) -> Task<Message> {
        if template_hits(&config, "", &self.files).is_empty() {
            self.notice(format!(
                "No templates in {}/. The folder is set in Obsidian's Templates settings",
                config.folder
            ));
            return Task::none();
        }
        self.completion = None;
        self.palette = Some(Palette { mode: PaletteMode::Template(config), ..Palette::default() });
        self.search();
        iced::widget::operation::focus(PALETTE_INPUT)
    }

    fn read_template(&mut self, template: String, config: TemplateConfig) -> Task<Message> {
        let Some(tab) = self.tabs.get(self.active).filter(|t| t.loaded) else { return Task::none() };
        let target = tab.id;
        let abs = self.vault.join(&template);
        Task::perform(
            async move { tokio::fs::read_to_string(&abs).await.map_err(|e| e.to_string()) },
            move |result| Message::TemplateLoaded { target, config: config.clone(), result },
        )
    }

    /// Inserts a template at the cursor of the note it was chosen for. Its
    /// properties go into the note's frontmatter instead, merged with those
    /// already there.
    fn insert_template(&mut self, target: u64, template: &str, config: &TemplateConfig) -> Task<Message> {
        let Some(i) = self.tabs.iter().position(|t| t.id == target && t.loaded) else { return Task::none() };
        if self.tabs[i].merge.is_some() {
            self.notice(format!("Resolve the changes to {} before inserting a template", self.tabs[i].path));
            return Task::none();
        }
        let tab = &mut self.tabs[i];
        let text = sage_editor::template::expand(template, &tab.title, sage_editor::now(), config);
        let (properties, body, _) = sage_core::split_frontmatter(&text);
        tab.insert_at_cursor(body);
        if let Some(properties) = properties {
            let cursor = tab.content.cursor().position;
            let note = tab.content.text();
            let (current, _, old_lines) = sage_core::split_frontmatter(&note);
            let merged = sage_editor::template::merge_properties(current, properties);
            let block = tab.with_ending(&format!("---\n{merged}---\n"));
            // The old block, after a byte-order mark, is replaced as one edit.
            let bom = if note.starts_with('\u{feff}') { '\u{feff}'.len_utf8() } else { 0 };
            let (position, selection) = match old_lines {
                0 => (Position { line: 0, column: bom }, None),
                n => (Position { line: n as usize, column: 0 }, Some(Position { line: 0, column: bom })),
            };
            place(&mut tab.content, Cursor { position, selection });
            tab.content.perform(text_editor::Action::Edit(Edit::Paste(Arc::new(block.clone()))));
            let new_lines = block.matches('\n').count();
            tab.put_cursor((cursor.line + new_lines).saturating_sub(old_lines as usize), cursor.column);
            tab.mark_edited();
        }
        if i == self.active {
            self.refresh_find(false);
            return iced::widget::operation::focus(EDITOR);
        }
        Task::none()
    }

    // ---- settings and Quick Add ----------------------------------------------

    /// The Templates settings in use: Obsidian's, with Sage's folder if set.
    fn template_config(&self) -> TemplateConfig {
        let mut config = self.obsidian_templates.clone();
        if let Some(folder) = &self.settings.templates_folder {
            config.folder = folder.clone();
        }
        config
    }

    /// The type chosen for property `key`: Sage's own choice, else Obsidian's.
    fn chosen_type(&self, key: &str) -> Option<sage_editor::properties::PropType> {
        let key = key.to_lowercase();
        let id = self.settings.property_types.get(&key).or_else(|| self.obsidian_types.get(&key))?;
        sage_editor::properties::PropType::from_id(id)
    }

    /// Notes the settings changed; autosave writes them shortly.
    fn settings_edited(&mut self) {
        self.settings_changed = Some(Instant::now());
    }

    /// Writes the settings to the vault, unless they are unchanged or could
    /// not be read in the first place.
    fn save_settings(&mut self) -> Task<Message> {
        if self.settings_changed.take().is_none() || self.settings_error.is_some() {
            return Task::none();
        }
        let (vault, settings) = (self.vault.clone(), self.settings.clone());
        let backend = self.backend.unwrap_or(Backend::Poll);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || sage_editor::settings::store(&vault, &settings, backend))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::SettingsSaved,
        )
    }

    /// A chord pressed: recorded as a Quick Add shortcut while one is being
    /// set, else the built-in command or the Quick Add entry it runs.
    fn on_chord(&mut self, chord: Chord) -> Task<Message> {
        // Recording ends when the settings are left.
        if !self.settings_open {
            self.recording = None;
            self.recording_built_in = None;
        }
        if let Some(i) = self.recording {
            self.record_shortcut(i, chord);
            return Task::none();
        }
        if let Some(i) = self.recording_built_in {
            self.rebind(i, chord);
            return Task::none();
        }
        let find = |chord: Chord| (0..BUILT_IN.len()).find(|&i| built_in_chord(i, &self.settings) == Some(chord));
        // Ctrl++ and Ctrl+Shift+=, on the same key as Ctrl+=, zoom in too.
        let plus = chord.ctrl
            && !chord.alt
            && (chord.key == ChordKey::Char('+') || (chord.shift && chord.key == ChordKey::Char('=')));
        let found = find(chord).or_else(|| find(Chord { shift: false, key: ChordKey::Char('='), ..chord }).filter(|_| plus));
        if let Some(i) = found {
            return self.run(BUILT_IN[i].1);
        }
        match self.settings.quick_add.iter().position(|entry| entry.shortcut() == Some(chord)) {
            Some(i) => self.choose_quick_add(i),
            None => Task::none(),
        }
    }

    /// Why `chord` cannot be given to another shortcut, if it cannot: it
    /// could type text, or a shortcut other than the one being set has it.
    fn taken(&self, chord: Chord, setting_built_in: Option<usize>, setting_quick_add: Option<usize>) -> Option<String> {
        if let Some(why) = chord.unusable() {
            return Some(format!("{chord}: {why}"));
        }
        let built_in =
            (0..BUILT_IN.len()).find(|&i| Some(i) != setting_built_in && built_in_chord(i, &self.settings) == Some(chord));
        if let Some(i) = built_in {
            return Some(format!("{chord} is {}", command_name(BUILT_IN[i].1)));
        }
        if let Some((_, what)) = EDITING.iter().find(|(keys, _)| Chord::parse(keys) == Some(chord)) {
            return Some(format!("{chord} {what} in the editor"));
        }
        let quick = self.settings.quick_add.iter().enumerate().find(|(j, e)| Some(*j) != setting_quick_add && e.shortcut() == Some(chord));
        quick.map(|(_, entry)| format!("{chord} runs Quick add: {}", entry.name()))
    }

    /// Makes `chord` run built-in shortcut `i` in this vault, unless it is
    /// taken; then it says why and keeps listening.
    fn rebind(&mut self, i: usize, chord: Chord) {
        if let Some(why) = self.taken(chord, Some(i), None) {
            self.notice(format!("{why}. Press other keys, or Esc"));
            return;
        }
        self.recording_built_in = None;
        let (default, command) = BUILT_IN[i];
        let name = command_name(command);
        if Chord::parse(default) == Some(chord) {
            self.settings.shortcuts.remove(name);
        } else {
            self.settings.shortcuts.insert(name.to_owned(), chord.to_string());
        }
        self.settings_edited();
        self.notice(format!("{chord} now runs {name}"));
    }

    /// Makes `chord` Quick Add entry `i`'s shortcut, unless it could type text
    /// or another shortcut has it; then it says why and keeps listening.
    fn record_shortcut(&mut self, i: usize, chord: Chord) {
        let refused = self.taken(chord, None, Some(i));
        if let Some(why) = refused {
            self.notice(format!("{why}. Press other keys, or Esc"));
            return;
        }
        self.recording = None;
        if let Some(entry) = self.settings.quick_add.get_mut(i) {
            entry.set_shortcut(Some(chord));
            let name = entry.name().to_owned();
            self.settings_edited();
            self.notice(format!("{chord} now runs Quick add: {name}"));
        }
    }

    /// Opens the palette on the Quick Add entries, or the settings to make one.
    fn start_quick_add(&mut self) -> Task<Message> {
        if self.quick_entries().is_empty() {
            self.notice("No Quick Add entries or templates yet: add one here".into());
            self.settings_open = true;
            self.tasks = None;
            return Task::none();
        }
        self.completion = None;
        self.palette = Some(Palette { mode: PaletteMode::QuickAdd, ..Palette::default() });
        self.search();
        iced::widget::operation::focus(PALETTE_INPUT)
    }

    /// The Quick Add entries: the vault's own, then one for every note in the
    /// templates folder, so each template can start a note without any setup.
    fn quick_entries(&self) -> Vec<QuickAdd> {
        let prefix = format!("{}/", self.template_config().folder);
        let own = self.settings.quick_add.iter().cloned();
        let templates = self.files.iter().filter(|f| self.settings.quick_add_templates.unwrap_or(true) && f.path.starts_with(&prefix) && f.path.ends_with(".md")).map(|f| QuickAdd::Template {
            name: f.title.clone(),
            template: f.path.clone(),
            folder: String::new(),
            file_name: "{{value}}".into(),
            open: true,
            shortcut: String::new(),
        });
        own.chain(templates).collect()
    }

    /// Runs Quick Add entry `i`, asking for its value first if it needs one.
    fn choose_quick_add(&mut self, i: usize) -> Task<Message> {
        let Some(entry) = self.quick_entries().into_iter().nth(i) else { return Task::none() };
        if !needs_value(&entry) {
            return self.run_quick_add(i, String::new());
        }
        self.completion = None;
        self.palette = Some(Palette { mode: PaletteMode::QuickAddValue(i), ..Palette::default() });
        self.search();
        iced::widget::operation::focus(PALETTE_INPUT)
    }

    /// Where Quick Add entry `i` would write with `value`, as a vault path.
    fn quick_add_path(&self, i: usize, value: &str) -> Option<String> {
        let (config, now) = (self.template_config(), sage_editor::now());
        let fill = |pattern: &str| sage_editor::quick_add::fill(pattern, value, now, &config);
        let rel = match self.quick_entries().get(i)? {
            QuickAdd::Template { folder, file_name, .. } => {
                let name = if file_name.trim().is_empty() { "{{value}}" } else { file_name.as_str() };
                sanitize_note_name(&format!("{}/{}", fill(folder), fill(name)))
            }
            QuickAdd::Capture { target, .. } => sanitize_note_name(&fill(target)),
        };
        (!rel.is_empty()).then(|| format!("{rel}.md"))
    }

    fn run_quick_add(&mut self, i: usize, value: String) -> Task<Message> {
        self.palette = None;
        let Some(entry) = self.quick_entries().into_iter().nth(i) else { return Task::none() };
        let Some(rel) = self.quick_add_path(i, &value) else {
            self.notice(format!("{} would make a note without a name", entry.name()));
            return Task::none();
        };
        let (vault, config, now) = (self.vault.clone(), self.template_config(), sage_editor::now());
        match entry {
            QuickAdd::Template { template, open, .. } => Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        sage_editor::quick_add::create_note(&vault, &rel, &template, &value, now, &config)
                            .map(|created| (rel, created))
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                },
                move |result| Message::QuickNoteDone { result, open },
            ),
            QuickAdd::Capture { heading, format, top, create, .. } => {
                // Like other writes, never underneath unsaved edits.
                if self.tabs.iter().any(|t| t.path == rel && t.busy()) {
                    self.notice(format!("Save {rel} first, it has unsaved changes"));
                    return Task::none();
                }
                let format = if format.trim().is_empty() { "{{value}}".to_owned() } else { format };
                let text = sage_editor::quick_add::fill(&format, &value, now, &config);
                let backend = self.backend.unwrap_or(Backend::Poll);
                let path = rel.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            sage_editor::quick_add::capture_into(&vault, &rel, &text, &heading, top, create, backend)
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    move |result| Message::Captured { path: path.clone(), result },
                )
            }
        }
    }

    /// The palette while asking for Quick Add entry `i`'s value: what Enter
    /// will create or add to.
    fn value_hits(&self, i: usize, value: &str) -> Vec<Hit> {
        let entries = self.quick_entries();
        let Some(entry) = entries.get(i) else { return Vec::new() };
        let ask = match entry {
            QuickAdd::Template { .. } => "Type the new note's name",
            QuickAdd::Capture { .. } => "Type what to add",
        };
        let hit = match (entry, self.quick_add_path(i, value)) {
            (_, None) => Hit { label: ask.into(), detail: entry.name().into(), action: HitAction::Nothing },
            _ if value.is_empty() => Hit { label: ask.into(), detail: entry.name().into(), action: HitAction::Nothing },
            (QuickAdd::Template { .. }, Some(path)) if self.files.iter().any(|f| f.path == path) => Hit {
                label: format!("{path} exists already"),
                detail: entry.name().into(),
                action: HitAction::Nothing,
            },
            (QuickAdd::Template { .. }, Some(path)) => {
                Hit { label: format!("Create {path}"), detail: entry.name().into(), action: HitAction::QuickAddRun(i) }
            }
            (QuickAdd::Capture { .. }, Some(path)) => {
                Hit { label: format!("Add to {path}"), detail: entry.name().into(), action: HitAction::QuickAddRun(i) }
            }
        };
        vec![hit]
    }

    /// Lists the folders on disk off the UI thread. Every change in the index
    /// asks for it: while one listing is under way, another follows it rather
    /// than running beside it, where the older could come back last.
    fn scan_folders(&mut self) -> Task<Message> {
        if std::mem::replace(&mut self.scanning, true) {
            self.rescan = true;
            return Task::none();
        }
        let vault = self.vault.clone();
        Task::perform(
            async move { tokio::task::spawn_blocking(move || tree::disk_folders(&vault)).await.unwrap_or_default() },
            Message::DiskFolders,
        )
    }

    /// Creates `folder` and the folders above it.
    fn create_folder(&mut self, folder: String) -> Task<Message> {
        self.palette = None;
        let abs = self.vault.join(&folder);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || std::fs::create_dir_all(abs).map(|()| folder).map_err(|e| e.to_string()))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::FolderCreated,
        )
    }

    // ---- find ---------------------------------------------------------------

    fn open_find(&mut self) -> Task<Message> {
        let Some(tab) = self.tabs.get(self.active).filter(|t| t.loaded) else { return Task::none() };
        // A one-line selection is what the user wants to find.
        let selected = tab.content.selection().filter(|s| !s.is_empty() && !s.contains('\n'));
        if self.view_mode == ViewMode::Preview {
            self.view_mode = ViewMode::Split;
            self.persist();
        }
        self.tasks = None;
        self.palette = None;
        self.completion = None;
        let find = self.find.get_or_insert_with(Find::default);
        if let Some(selected) = selected {
            find.query = selected;
        }
        self.refresh_find(true);
        iced::widget::operation::focus(FIND_INPUT).chain(iced::widget::operation::select_all(FIND_INPUT))
    }

    /// Recomputes the matches in the active note. With `seek`, the editor
    /// selects the first match at or after the cursor; without, only the
    /// counter follows, so typing in the note is never interrupted.
    fn refresh_find(&mut self, seek: bool) {
        let (Some(find), Some(tab)) = (&mut self.find, self.tabs.get_mut(self.active)) else { return };
        let lines: Vec<String> = tab.content.lines().map(|l| l.text.into_owned()).collect();
        find.matches = sage_editor::find::find(lines.iter().map(String::as_str), &find.query);
        let cursor = tab.content.cursor();
        let anchor = match cursor.selection {
            Some(other) => (other.line, other.column).min((cursor.position.line, cursor.position.column)),
            None => (cursor.position.line, cursor.position.column),
        };
        find.current = find
            .matches
            .iter()
            .position(|(line, range)| (*line, range.start) >= anchor)
            .or((!find.matches.is_empty()).then_some(0));
        if seek {
            if let Some(i) = find.current {
                select_match(tab, &find.matches[i]);
            }
        }
    }

    fn step_find(&mut self, forward: bool) {
        let (Some(find), Some(tab)) = (&mut self.find, self.tabs.get_mut(self.active)) else { return };
        let n = find.matches.len();
        if n == 0 {
            return;
        }
        let i = match find.current {
            Some(i) if forward => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
            None => 0,
        };
        find.current = Some(i);
        select_match(tab, &find.matches[i]);
    }

    fn close_find(&mut self) -> Task<Message> {
        if self.find.take().is_none() {
            return Task::none();
        }
        // The current match is selected, and shows once the editor has focus.
        iced::widget::operation::focus(EDITOR)
    }

    /// The current match, drawn by the highlighter while the find bar is open.
    fn find_mark(&self) -> Option<(usize, Range<usize>)> {
        let find = self.find.as_ref()?;
        find.matches.get(find.current?).cloned()
    }

    // ---- autocomplete ---------------------------------------------------------

    /// Re-evaluates the completion popup after any editor action.
    fn refresh_completion(&mut self) {
        let (Some(tab), Some(reader)) = (self.tabs.get(self.active), &self.reader) else {
            self.completion = None;
            return;
        };
        let cursor = tab.content.cursor();
        let line_no = cursor.position.line;
        let ctx = if cursor.selection.is_some() {
            None
        } else {
            tab.content
                .line(line_no)
                .and_then(|line| complete::context(&line.text, cursor.position.column))
        };
        let Some(ctx) = ctx else {
            self.completion = None;
            self.dismissed = None;
            return;
        };
        if self.dismissed == Some((line_no, ctx.start)) {
            return;
        }
        self.dismissed = None;

        let items = suggest::for_context(reader, &ctx);
        if items.is_empty() {
            self.completion = None;
            return;
        }
        // Keep the highlighted row while the user keeps typing, when possible.
        let selected = self
            .completion
            .as_ref()
            .filter(|old| old.line == line_no && old.ctx.start == ctx.start)
            .map_or(0, |old| old.selected.min(items.len() - 1));
        self.completion = Some(Completion { ctx, line: line_no, items, selected });
    }

    /// Suggests notes for what is typed in a field of the properties widget,
    /// as `[[` does in the text. In a link property the whole text is the name.
    fn refresh_prop_completion(&mut self, entry: usize, item: Option<usize>, value: &sage_editor::properties::Value) {
        use sage_editor::properties::Value;
        self.prop_complete = None;
        let (Some(tab), Some(reader)) = (self.tabs.get(self.active), &self.reader) else { return };
        let Some(key) = tab.property(entry).map(|e| e.key) else { return };
        let text = match (value, item) {
            (Value::Text { text, .. }, _) => text.clone(),
            (Value::List { items, .. }, Some(k)) => items.get(k).cloned().unwrap_or_default(),
            _ => return,
        };
        let link_property = is_link_property(&key);
        let ctx = if link_property {
            let linked = text.len() >= 4 && text.starts_with("[[") && text.ends_with("]]");
            (!linked).then(|| complete::Context {
                kind: complete::Kind::Link,
                query: text.trim_start_matches('[').trim_end_matches(']').to_owned(),
                start: 0,
                closed: false,
                tagged: false,
            })
        } else {
            complete::context(&text, text.len()).filter(|c| c.kind == complete::Kind::Link)
        };
        let Some(ctx) = ctx else { return };
        let mut items = suggest::for_context(reader, &ctx);
        // A name that matches no note links to one that following the link creates.
        let query = ctx.query.trim();
        if link_property && !query.is_empty() && !items.iter().any(|i| i.value.eq_ignore_ascii_case(query)) {
            items.push(Suggestion { label: format!("Create “{query}”"), detail: "new note".into(), value: query.to_owned(), date: false });
        }
        if items.is_empty() {
            return;
        }
        self.prop_complete = Some(PropCompletion { tab: tab.path.clone(), entry, item, ctx, items, selected: 0 });
    }

    /// Writes the picked suggestion (the highlighted one for `None`) into its field.
    fn accept_property(&mut self, pick: Option<usize>) {
        use sage_editor::properties::Value;
        let Some(c) = self.prop_complete.take() else { return };
        let Some(suggestion) = c.items.get(pick.unwrap_or(c.selected)) else { return };
        let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded && t.path == c.tab) else { return };
        let Some(entry) = tab.property(c.entry) else { return };
        let write = |text: &str| {
            if is_link_property(&entry.key) {
                format!("[[{}]]", suggestion.value)
            } else {
                let start = c.ctx.start.min(text.len());
                format!("{}{}", &text[..start], complete::insertion(&c.ctx, &suggestion.value))
            }
        };
        let value = match (entry.value, c.item) {
            (Value::Text { text, quote }, _) => Value::Text { text: write(&text), quote },
            (Value::List { mut items, block }, Some(k)) if k < items.len() => {
                items[k] = write(&items[k]);
                Value::List { items, block }
            }
            _ => return,
        };
        tab.set_property(c.entry, &value);
    }

    /// Removes list item `item` of entry `entry` when it is empty. Whether it did.
    fn drop_empty_item(&mut self, entry: usize, item: usize) -> bool {
        use sage_editor::properties::Value;
        let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) else { return false };
        let Some(Value::List { mut items, block }) = tab.property(entry).map(|e| e.value) else { return false };
        if !items.get(item).is_some_and(|i| i.trim().is_empty()) {
            return false;
        }
        items.remove(item);
        tab.set_property(entry, &Value::List { items, block });
        true
    }

    /// Asks whether the first empty list item of the properties still has the
    /// focus, so that one left empty is removed, as in Obsidian.
    fn prune_check(&self) -> Task<Message> {
        let new = if self.new_property.as_ref().is_some_and(|(_, s)| s.trim().is_empty()) {
            iced::widget::operation::is_focused(iced::widget::Id::from(preview::NEW_PROPERTY)).map(Message::NewPropertyBlur)
        } else {
            Task::none()
        };
        Task::batch([new, self.prune_items()])
    }

    fn prune_items(&self) -> Task<Message> {
        use sage_editor::properties::Value;
        let Some(tab) = self.tabs.get(self.active).filter(|t| t.loaded) else { return Task::none() };
        let Some(lines) = tab.frontmatter_lines() else { return Task::none() };
        for (entry, e) in sage_editor::properties::parse(&lines).into_iter().enumerate() {
            if let Value::List { items, .. } = e.value {
                if let Some(item) = items.iter().position(|i| i.trim().is_empty()) {
                    return iced::widget::operation::is_focused(iced::widget::Id::from(preview::property_id(entry, item)))
                        .map(move |focused| Message::PropertyBlur(entry, item, focused));
                }
            }
        }
        Task::none()
    }

    /// Enter in a property field: takes the suggestion shown, else turns the
    /// text of a link property into a link.
    fn submit_property(&mut self, entry: usize, item: Option<usize>) {
        use sage_editor::properties::Value;
        if self.prop_complete.as_ref().is_some_and(|c| c.entry == entry && c.item == item) {
            return self.accept_property(None);
        }
        if self.drop_empty_item(entry, item.unwrap_or(usize::MAX)) {
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active).filter(|t| t.loaded) else { return };
        let Some(current) = tab.property(entry).filter(|e| is_link_property(&e.key)) else { return };
        let link = |text: &str| {
            let name = text.trim();
            let linked = name.len() >= 4 && name.starts_with("[[") && name.ends_with("]]");
            (!name.is_empty() && !linked).then(|| format!("[[{name}]]"))
        };
        let value = match (current.value, item) {
            (Value::Text { text, quote }, _) => link(&text).map(|text| Value::Text { text, quote }),
            (Value::List { mut items, block }, Some(k)) => {
                let linked = items.get(k).and_then(|i| link(i));
                linked.map(|l| {
                    items[k] = l;
                    Value::List { items, block }
                })
            }
            _ => None,
        };
        if let Some(value) = value {
            tab.set_property(entry, &value);
        }
    }

    fn accept_completion(&mut self) -> Task<Message> {
        let Some(c) = self.completion.take() else { return Task::none() };
        let Some(item) = c.items.get(c.selected) else { return Task::none() };
        let Some(tab) = self.tabs.get_mut(self.active) else { return Task::none() };
        let cursor = tab.content.cursor();
        if cursor.position.line != c.line {
            return Task::none();
        }
        tab.checkpoint(sage_editor::undo::Kind::Other);
        // Select what was typed, then paste the full value over it.
        tab.content.move_to(Cursor {
            position: cursor.position,
            selection: Some(Position { line: c.line, column: c.ctx.start }),
        });
        let mut ctx = c.ctx.clone();
        if item.date {
            ctx.kind = complete::Kind::Date;
        }
        let insert = complete::insertion(&ctx, &item.value);
        tab.content.perform(text_editor::Action::Edit(Edit::Paste(Arc::new(insert))));
        // Step past a closing bracket that was already there.
        if c.ctx.closed {
            let skip = if matches!(c.ctx.kind, complete::Kind::Path) { 1 } else { 2 };
            for _ in 0..skip {
                tab.content.perform(text_editor::Action::Move(text_editor::Motion::Right));
            }
        }
        tab.mark_edited();
        iced::widget::operation::focus(EDITOR)
    }

    /// Fills the palette: commands after `>`, otherwise notes by name, then
    /// full-text matches, then an entry to create a note with that name.
    fn search(&mut self) {
        // Asking for a Quick Add value: what Enter will create or add to.
        if let Some(Palette { mode: PaletteMode::QuickAddValue(i), query, .. }) = &self.palette {
            let hits = self.value_hits(*i, query.trim());
            if let Some(palette) = &mut self.palette {
                palette.hits = hits;
            }
            return;
        }
        if let Some(hits) = self.flow_hits() {
            if let Some(palette) = &mut self.palette {
                palette.hits = hits;
            }
            return;
        }
        if let Some(Palette { mode: PaletteMode::NewFolder, query, .. }) = &self.palette {
            let hits = new_folder_hits(query, &self.tree_folders);
            if let Some(palette) = &mut self.palette {
                palette.hits = hits;
            }
            return;
        }
        let entries = self.quick_entries();
        let folders = if matches!(self.palette, Some(Palette { mode: PaletteMode::MoveTo(_), .. })) { self.tree_folders.clone() } else { Vec::new() };
        let Some(palette) = &mut self.palette else { return };
        palette.hits.clear();
        match &palette.mode {
            PaletteMode::Search => {}
            PaletteMode::QuickAdd => {
                let filter = palette.query.trim().to_lowercase();
                palette.hits = entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| entry.name().to_lowercase().contains(&filter))
                    .map(|(i, entry)| Hit { label: entry.name().to_owned(), detail: describe(entry), action: HitAction::QuickAdd(i) })
                    .collect();
                if filter.is_empty() || "log to project".contains(&filter) {
                    palette.hits.push(Hit {
                        label: "Log to project…".into(),
                        detail: "a dated line in a project's log".into(),
                        action: HitAction::Run(Command::LogToProject),
                    });
                }
                return;
            }
            PaletteMode::QuickAddValue(_) => return,
            PaletteMode::Project(_) | PaletteMode::LogEntry { .. } | PaletteMode::Triage(_) | PaletteMode::TriageAction(_) | PaletteMode::Person(..) => return,
            PaletteMode::Rename(from) => {
                palette.hits = rename_hits(from, &palette.query, &self.files, self.reader.as_ref());
                return;
            }
            PaletteMode::MoveTo(from) => {
                palette.hits = move_hits(from, &palette.query, &folders, &self.files);
                return;
            }
            PaletteMode::RenameFolder(from) => {
                palette.hits = folder_hits(from, &palette.query, &self.files);
                return;
            }
            PaletteMode::Template(config) => {
                palette.hits = template_hits(config, &palette.query, &self.files);
                return;
            }
            PaletteMode::NewFolder => return,
            PaletteMode::NewNote => {
                let name = sanitize_note_name(palette.query.trim());
                if !name.is_empty() {
                    palette.hits = vec![Hit { label: format!("Create {name}"), detail: String::new(), action: HitAction::Create(name) }];
                }
                return;
            }
            PaletteMode::Everywhere(None) => {
                palette.hits = vec![Hit { label: "Reading the vault…".into(), detail: String::new(), action: HitAction::Nothing }];
                return;
            }
            PaletteMode::Everywhere(Some(corpus)) => {
                palette.hits = corpus
                    .search(&palette.query, 50)
                    .into_iter()
                    .map(|found| Hit {
                        label: found.name,
                        detail: found.detail,
                        action: match found.line {
                            Some(line) => HitAction::OpenAt(found.path, line),
                            None => HitAction::Open(found.path),
                        },
                    })
                    .collect();
                if palette.query.trim().is_empty() {
                    palette.hits.push(Hit {
                        label: "Names, titles, aliases, tags, properties, headings and text".into(),
                        detail: "typos are forgiven".into(),
                        action: HitAction::Nothing,
                    });
                }
                return;
            }
        }
        let Some(reader) = &self.reader else { return };

        let can_undo = !self.renames.is_empty();
        if let Some(filter) = palette.query.strip_prefix('>') {
            palette.hits = command_hits(&filter.trim().to_lowercase(), &self.settings, &entries, can_undo);
            return;
        }

        let push = |hits: &mut Vec<Hit>, sql: &str, arg: String| {
            let Ok(mut stmt) = reader.prepare_cached(sql) else { return };
            let Ok(rows) = stmt.query_map([arg], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            }) else {
                return;
            };
            for (path, title, detail) in rows.flatten() {
                let seen = hits
                    .iter()
                    .any(|h| matches!(&h.action, HitAction::Open(p) if *p == path));
                if !seen {
                    // FTS snippets span lines; a result row is one line.
                    let detail = detail.split_whitespace().collect::<Vec<_>>().join(" ");
                    // Show the H1 or frontmatter title only when it adds something.
                    let name = file_stem(&path).to_owned();
                    let detail = if title != name && detail == path { format!("{title} · {path}") } else { detail };
                    hits.push(Hit { label: name, detail, action: HitAction::Open(path) });
                }
            }
        };

        let query = palette.query.trim().to_owned();
        if query.is_empty() {
            // Notes opened lately first, then those changed lately.
            let active = self.tabs.get(self.active).map(|t| t.path.as_str());
            let recent = self.recent.iter().filter(|p| Some(p.as_str()) != active && self.files.iter().any(|f| &f.path == *p));
            for path in recent.take(8) {
                palette.hits.push(Hit {
                    label: file_stem(path).to_owned(),
                    detail: format!("opened lately · {path}"),
                    action: HitAction::Open(path.clone()),
                });
            }
            push(
                &mut palette.hits,
                "SELECT path, title, path FROM v_notes WHERE ?1 = '' ORDER BY mtime_ns DESC LIMIT 12",
                String::new(),
            );
            return;
        }
        push(
            &mut palette.hits,
            "SELECT path, title, path FROM v_notes
             WHERE instr(lower(title), lower(?1)) > 0 OR instr(lower(path), lower(?1)) > 0
             ORDER BY length(path) LIMIT 6",
            query.clone(),
        );
        // Commands and Quick Add entries are found by name too, without `>`.
        if query.chars().count() >= 3 {
            palette.hits.extend(command_hits(&query.to_lowercase(), &self.settings, &entries, can_undo));
        }
        if let Some(fts) = palette_fts(&query) {
            push(
                &mut palette.hits,
                "SELECT f.path, f.title, snippet(fts, 1, '', '', ' … ', 10)
                 FROM fts JOIN files f ON f.id = fts.rowid
                 WHERE fts MATCH ?1 ORDER BY rank LIMIT 8",
                fts,
            );
        }
        let name = sanitize_note_name(&query);
        let exists = palette.hits.iter().any(|h| h.label.eq_ignore_ascii_case(&name));
        if !name.is_empty() && !exists {
            palette.hits.push(Hit {
                label: format!("Create note \u{201c}{name}\u{201d}"),
                detail: format!("{name}.md"),
                action: HitAction::Create(name),
            });
        }
    }

    // ---- view -------------------------------------------------------------

    fn view(&self) -> Element<'_, Message> {
        let t = self.tokens();
        let workspace = if self.focus {
            column![self.workspace(t)]
        } else {
            column![
                row![self.sidebar(t), divider(t, Divider::Sidebar), self.workspace(t)].height(Fill),
                hline(t),
                self.status_bar(t),
            ]
        };
        let base = container(workspace).width(Fill).height(Fill).style(move |_| theme::page(t));
        let base: Element<'_, Message> = match self.context_menu(t) {
            Some(menu) => stack![base, menu].into(),
            None => base.into(),
        };
        let base: Element<'_, Message> = match self.toast_overlay(t) {
            Some(toast) => stack![base, toast].into(),
            None => base,
        };

        if self.help {
            return stack![base, self.help_overlay(t)].into();
        }
        if let Some(doomed) = &self.doomed {
            return stack![base, self.confirm_overlay(t, doomed)].into();
        }
        if let Some(form) = &self.task_form {
            return stack![base, self.task_form_overlay(t, form)].into();
        }
        match &self.palette {
            Some(palette) => stack![base, self.palette_overlay(t, palette)].into(),
            None => base.into(),
        }
    }

    /// The menu a right click on a note, a folder or a tab opens, floating at
    /// the pointer. Any click outside it closes it.
    fn context_menu(&self, t: Tokens) -> Option<Element<'_, Message>> {
        let (_, folder) = self.menu.as_ref()?;
        let choice = |label: &str, message: Message| -> Element<'_, Message> {
            button(text(label.to_owned()).size(13))
                .width(Fill)
                .padding([4, 10])
                .style(move |_, status| theme::nav_button(t, false, status))
                .on_press(message)
                .into()
        };
        let reveal = if cfg!(windows) { "Show in Explorer" } else { "Show in file manager" };
        let mut items = Column::new().spacing(1);
        let mut count = 0;
        if let Some(i) = self.tab_menu {
            items = items
                .push(choice("Close", Message::CloseTab(i)))
                .push(choice("Close others", Message::CloseOtherTabs(i)))
                .push(choice("Close tabs to the right", Message::CloseTabsRight(i)))
                .push(choice("Close all", Message::CloseAllTabs))
                .push(container(hline(t)).padding([3, 0]));
            count += 5;
        }
        items = items.push(choice("Rename or move", Message::MenuRename));
        if let Some(i) = self.tab_menu {
            let pinned = self.tabs.get(i).is_some_and(|t| self.pinned.contains(&t.path));
            items = items
                .push(choice(if pinned { "Unpin" } else { "Pin" }, Message::PinTab(i)))
                .push(choice("Move left", Message::MoveTab(i, -1)))
                .push(choice("Move right", Message::MoveTab(i, 1)));
            count += 3;
        }
        if self.tab_menu.is_none() {
            items = items.push(choice("Delete", Message::MenuDelete));
            count += 1;
        }
        items = items
            .push(choice(reveal, Message::MenuReveal))
            .push(choice(if *folder { "Copy path" } else { "Copy link" }, Message::MenuCopyLink));
        count += 3;

        // Kept inside the window: the menu opens up or to the left where it
        // would run off the bottom or the right edge.
        let (width, height) = (200.0, count as f32 * 27.0 + 12.0);
        let window_height = f32::from_bits(WINDOW_HEIGHT.load(std::sync::atomic::Ordering::Relaxed));
        let window_height = if window_height > 0.0 { window_height } else { 4000.0 };
        let x = self.menu_at.x.min(self.window_width - width - 4.0).max(0.0);
        let y = self.menu_at.y.min(window_height - height - 4.0).max(0.0);
        let menu = container(items.width(width))
            .padding(4)
            .style(move |_| theme::panel(t));
        let outside = iced::widget::mouse_area(container(space()).width(Fill).height(Fill))
            .on_press(Message::CloseMenu)
            .on_right_press(Message::CloseMenu);
        Some(
            stack![
                outside,
                container(menu).width(Fill).height(Fill).padding(iced::Padding { top: y, left: x, right: 0.0, bottom: 0.0 }),
            ]
            .into(),
        )
    }

    fn sidebar(&self, t: Tokens) -> Element<'_, Message> {
        let current = self.tabs.get(self.active).map(|tab| tab.path.as_str());
        let vault = self.vault.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        column![
            container(
                row![
                    text(vault.into_owned()).size(18).font(theme::SERIF).color(t.text).wrapping(text::Wrapping::None),
                    space().width(Fill),
                    header_button("+", "Quick add (Ctrl+Shift+N)", Message::Key(Shortcut::Command(Command::QuickAdd)), t),
                    header_button(
                        if self.outline_open { "▤" } else { "≡" },
                        if self.outline_open { "Show folders (Ctrl+Shift+O)" } else { "Show outline (Ctrl+Shift+O)" },
                        Message::Key(Shortcut::Command(Command::ToggleOutline)),
                        t
                    ),
                    header_button("✓", "Tasks", Message::Key(Shortcut::Command(Command::ToggleTasks)), t),
                    header_button("⚙", "Settings", Message::ToggleSettings, t),
                ]
                .spacing(2)
                .align_y(iced::Center),
            )
            .padding(iced::Padding { top: 10.0, bottom: 4.0, left: 16.0, right: 8.0 }),
            self.files_or_outline(t, current),
        ]
        .width(self.sidebar_width())
        .into()
    }

    /// The folder tree, or the active note's headings while the outline is open.
    fn files_or_outline<'a>(&'a self, t: Tokens, current: Option<&'a str>) -> Element<'a, Message> {
        if self.outline_open {
            return column![hline(t), scrollable(container(self.outline(t)).padding([8, 8])).height(Fill)].into();
        }
        column![
            container(tree::toolbar(
                self.tree_order,
                self.tree_order_menu,
                // A lookup per folder, nothing made: this runs on every frame.
                self.tree_folders.iter().any(|f| self.expanded.contains(f)),
                t
            ))
            .padding(iced::Padding { bottom: 6.0, ..iced::Padding::ZERO }),
            hline(t),
            scrollable(
                container(tree::view(
                    &self.files,
                    &self.disk_folders,
                    &self.expanded,
                    self.tree_order,
                    current,
                    t
                ))
                .padding([8, 8])
            )
            .height(Fill),
        ]
        .into()
    }

    /// The active note's headings, indented by level; the one the cursor is
    /// under is marked. A click puts the cursor on it.
    fn outline(&self, t: Tokens) -> Element<'_, Message> {
        let quiet = |s: &'static str| -> Element<'_, Message> { container(text(s).size(13).color(t.text_2)).padding([4, 8]).into() };
        let Some(tab) = self.tabs.get(self.active).filter(|tab| tab.loaded) else {
            return quiet("Open a note to see its headings");
        };
        if tab.outline.is_empty() {
            return quiet("This note has no headings");
        }
        let top = tab.outline.iter().map(|h| h.0).min().unwrap_or(1);
        let line = tab.content.cursor().position.line;
        let here = tab.outline.iter().rposition(|h| h.2 <= line);
        let mut list = Column::new().spacing(1);
        for (i, (level, heading, at)) in tab.outline.iter().enumerate() {
            let indent = f32::from(level - top) * 14.0;
            let current = here == Some(i);
            list = list.push(
                button(text(heading.as_str()).size(13).wrapping(text::Wrapping::WordOrGlyph))
                    .width(Fill)
                    .padding(iced::Padding { top: 3.0, bottom: 3.0, left: 8.0 + indent, right: 8.0 })
                    .style(move |_, status| theme::nav_button(t, current, status))
                    .on_press(Message::OutlineJump(*at)),
            );
        }
        list.into()
    }

    fn workspace(&self, t: Tokens) -> Element<'_, Message> {
        if self.settings_open {
            let ctx = settings::Ctx {
                settings: &self.settings,
                current_note: self.tabs.get(self.active).map(|t| t.path.as_str()),
                obsidian_folder: &self.obsidian_templates.folder,
                folders: self.note_folders.clone(),
                templates: template_hits(&self.template_config(), "", &self.files)
                    .into_iter()
                    .filter_map(|hit| match hit.action {
                        HitAction::InsertTemplate(path) => Some(path),
                        _ => None,
                    })
                    .collect(),
                error: self.settings_error.as_deref(),
                theme: settings::ThemeChoice::of(self.dark_override),
                recording: self.recording,
                built_in: (0..BUILT_IN.len())
                    .map(|i| settings::BuiltIn {
                        keys: built_in_chord(i, &self.settings).map(|c| c.to_string()),
                        what: command_name(BUILT_IN[i].1),
                        changed: self.settings.shortcuts.contains_key(command_name(BUILT_IN[i].1)),
                    })
                    .collect(),
                recording_built_in: self.recording_built_in,
                fixed: &FIXED,
                vault: &self.vault,
                filter: &self.shortcut_filter,
                t,
            };
            return column![self.tab_bar(t), hline(t), settings::view(ctx)].width(Fill).into();
        }
        if let Some(list) = &self.tasks {
            return column![self.tab_bar(t), hline(t), tasks::view(list, t)].width(Fill).into();
        }
        let Some(tab) = self.tabs.get(self.active) else {
            let action = |label: &'static str, message: Message| {
                button(text(label).size(14))
                    .padding([6, 14])
                    .style(move |_, status| theme::nav_button(t, false, status))
                    .on_press(message)
            };
            let mut recent = Column::new().spacing(2);
            for path in self.recent.iter().filter(|p| self.files.iter().any(|f| &f.path == *p)).take(6) {
                recent = recent.push(
                    button(text(file_stem(path).to_owned()).size(15))
                        .padding([2, 0])
                        .style(move |_, status| theme::link_button(t, status))
                        .on_press(Message::Open(path.clone())),
                );
            }
            let mut start = column![
                text("No note open").size(26).font(theme::SERIF).color(t.text),
                row![
                    action("New note", Message::Run(Command::NewNote)),
                    action("Quick add", Message::Run(Command::QuickAdd)),
                    action("Today's note", Message::Run(Command::DailyNote)),
                    action("Search", Message::Run(Command::SearchEverywhere)),
                ]
                .spacing(8),
            ]
            .spacing(12)
            .align_x(iced::Center);
            // A vault without notes is offered the starter vault, once indexing shows it is empty.
            if self.indexed && self.files.is_empty() {
                start = start.push(
                    column![
                        text("This vault has no notes yet").size(12).font(theme::SANS_BOLD).color(t.text_2),
                        action("Create starter vault", Message::Run(Command::CreateStarterVault)),
                        text("Folders, templates with Quick Add shortcuts, and the Sage tutorial").size(13).color(t.text_2),
                    ]
                    .spacing(6)
                    .align_x(iced::Center),
                );
            }
            return container(
                start
                    .push(text("Recent").size(12).font(theme::SANS_BOLD).color(t.text_2))
                    .push(recent)
                    .push(text("Ctrl+P finds or creates a note · F1 lists every shortcut").size(13).color(t.text_2)),
            )
            .center(Fill)
            .into();
        };
        if tab.path.is_empty() {
            let action = |label: &'static str, message: Message| {
                button(text(label).size(15))
                    .padding([10, 22])
                    .style(move |_, status| theme::nav_button(t, false, status))
                    .on_press(message)
            };
            return column![
                self.tab_bar(t),
                hline(t),
                container(
                    column![
                        text("New tab").size(26).font(theme::SERIF).color(t.text),
                        row![
                            action("New note  Ctrl+N", Message::Run(Command::NewNote)),
                            action("Open file  Ctrl+O", Message::Run(Command::OpenNote)),
                        ]
                        .spacing(12),
                    ]
                    .spacing(16)
                    .align_x(iced::Center),
                )
                .center(Fill)
            ]
            .width(Fill)
            .into();
        }
        if !tab.loaded {
            return column![
                self.tab_bar(t),
                hline(t),
                container(text("Loading…").size(14).color(t.text_2)).center(Fill)
            ]
            .into();
        }
        if let Some(resolve) = &tab.merge {
            return column![self.tab_bar(t), hline(t), self.resolve_panel(t, tab, resolve)].width(Fill).into();
        }

        // While suggestions are showing, navigation keys drive the popup
        // instead of the text, in either editor.
        let popup_open = self.completion.is_some();
        let key_binding = move |press: text_editor::KeyPress| {
            if popup_open && matches!(press.status, text_editor::Status::Focused { .. }) {
                let message = match press.key.as_ref() {
                    Key::Named(Named::ArrowUp) => Some(Message::CompleteMove(-1)),
                    Key::Named(Named::ArrowDown) => Some(Message::CompleteMove(1)),
                    Key::Named(Named::Enter | Named::Tab) => Some(Message::CompleteAccept),
                    Key::Named(Named::Escape) => Some(Message::CompleteDismiss),
                    _ => None,
                };
                if let Some(message) = message {
                    return Some(Binding::Custom(message));
                }
            }
            if matches!(press.status, text_editor::Status::Focused { .. }) {
                if let Some(op) = edit_op(&press) {
                    return Some(Binding::Custom(Message::EditOp(op)));
                }
                let m = press.modifiers;
                // Back and Forward, run by the global shortcuts; not a cursor move.
                if m.alt() && !m.command() && matches!(press.key.as_ref(), Key::Named(Named::ArrowLeft | Named::ArrowRight)) {
                    return None;
                }
                // Text pastes as always; a picture, if that is all there is, follows.
                if m.command() && !m.alt() && press.key.to_latin(press.physical_key) == Some('v') {
                    return Some(Binding::Sequence(vec![Binding::Paste, Binding::Custom(Message::PasteImage)]));
                }
            }
            Binding::from_key_press(press)
        };
        let source = || -> Element<'_, Message> {
            let editor = text_editor(&tab.content)
                .id(EDITOR)
                .on_action(Message::Edit)
                .font(theme::MONO)
                .size(14)
                .line_height(1.6)
                .padding(24)
                .height(Fill)
                .key_binding(key_binding)
                .highlight_with::<highlight::Markdown>(
                    highlight::Settings { tokens: t, mark: self.find_mark() },
                    highlight::to_format,
                )
                .style(move |_, status| theme::editor(t, status));
            self.with_overlay(t, editor.into(), false)
        };
        let live = || -> Element<'_, Message> {
            // Tables, embeds, views and the properties, drawn by the preview.
            let ctx = preview::Ctx::new(t, tab, self.drag.as_ref(), self.folds.get(&tab.path));
            // The properties are edited in place, not drawn read-only.
            let props = tab.islands.iter().find(|i| matches!(i.blocks.first(), Some(sage_markdown::Block::Properties(_)))).map(|i| i.lines.clone());
            let inner = if props.is_some() { tab.frontmatter_lines().unwrap_or_default() } else { Vec::new() };
            let pending = self.prop_complete.as_ref().filter(|c| c.tab == tab.path);
            let islands = tab
                .islands
                .iter()
                .map(|i| {
                    let element = if props.as_ref() == Some(&i.lines) {
                        let popup = pending.map(|c| (c.entry, suggestion_list(t, c.ctx.kind, &c.items, c.selected, Message::PropertyPick)));
                        let types = sage_editor::properties::parse(&inner)
                            .into_iter()
                            .map(|e| {
                                let chosen = self.chosen_type(&e.key);
                                let ty = sage_editor::properties::type_of(&e.value, chosen);
                                (e, ty)
                            })
                            .collect();
                        let editing = self.editing_prop.as_ref().filter(|(p, _)| *p == tab.path).map(|(_, e)| *e);
                        let template = tab.path.starts_with(&format!("{}/", self.template_config().folder));
                        preview::property_editor(types, popup, self.type_menu, self.chip_hover, editing, template, self.new_property.as_ref().map(|(ty, s)| (*ty, s.as_str())), t)
                    } else if let (Some(sage_markdown::Block::Table { .. }), Some(table)) = (i.blocks.first(), tab.table_at(i.lines.start)) {
                        tables::editor(i.lines.start, &table, self.table_draft.as_ref(), t)
                    } else {
                        preview::blocks(&i.blocks, ctx)
                    };
                    (i.lines.clone(), element)
                })
                .collect();
            let popup = self.completion.as_ref().map(|c| self.completion_popup(t, c));
            let mut editable: Vec<std::ops::Range<usize>> = props.into_iter().collect();
            editable.extend(tab.islands.iter().filter(|i| matches!(i.blocks.first(), Some(sage_markdown::Block::Table { .. }))).map(|i| i.lines.clone()));
            self.with_overlay(
                t,
                live_editor::LiveEditor::new(&tab.content, tab.revision, EDITOR, t, self.find_mark(), Message::Live, key_binding)
                    .islands(islands)
                    .editable(editable)
                    .popup(popup)
                    .into(),
                true,
            )
        };

        let panes: Element<'_, Message> = match self.view_mode {
            ViewMode::Source => source(),
            ViewMode::Live => live(),
            ViewMode::Preview => self.preview(t, tab),
            ViewMode::Split => {
                // Shares out of a thousand, as the divider left them.
                let source_share = (self.panes.source * 1000.0).round() as u16;
                row![
                    container(source()).width(Length::FillPortion(source_share)),
                    divider(t, Divider::Split),
                    container(self.preview(t, tab)).width(Length::FillPortion(1000 - source_share))
                ]
                .into()
            }
        };

        let panes: Element<'_, Message> = if tab.followups.is_empty() && tab.timeline.is_empty() {
            panes
        } else {
            let mut stacked = column![container(panes).height(Fill)];
            if !tab.timeline.is_empty() {
                stacked = stacked.push(hline(t)).push(timeline::panel(&tab.timeline, self.timeline_kind, &self.day, t));
            }
            if !tab.followups.is_empty() {
                stacked = stacked.push(hline(t)).push(tasks::followup_panel(&tab.followups, t));
            }
            stacked.into()
        };

        // A OneDrive conflict copy waits above the note until it is merged.
        let banner: Element<'_, Message> = match tab.copies.first() {
            Some(copy) => column![
                container(
                    row![
                        text(format!(
                            "OneDrive kept a conflicting copy of this note: {}",
                            copy.rsplit('/').next().unwrap_or(copy)
                        ))
                        .size(13)
                        .color(t.text),
                        space().width(Fill),
                        button(text("Compare and merge").size(13))
                            .padding([3, 10])
                            .style(move |_, status| theme::nav_button(t, true, status))
                            .on_press(Message::CompareCopy(copy.clone())),
                    ]
                    .align_y(iced::Center),
                )
                .padding([6, 16])
                .style(move |_| theme::surface(t)),
                hline(t),
            ]
            .into(),
            None => space().into(),
        };
        // The note's name above the page, in every view mode, with where it
        // lives and the way to switch how it is shown.
        let folder = tab.path.rsplit_once('/').map(|(f, _)| f.replace('/', "  ›  ")).unwrap_or_default();
        let mode_button = |label: &'static str, hint: &'static str, mode: ViewMode| -> Element<'_, Message> {
            let on = self.view_mode == mode;
            iced::widget::tooltip(
                button(text(label).size(12))
                    .padding([2, 8])
                    .style(move |_, status| theme::nav_button(t, on, status))
                    .on_press(Message::Run(Command::View(mode))),
                container(text(hint).size(12)).padding([3, 8]).style(move |_| theme::panel(t)),
                iced::widget::tooltip::Position::Bottom,
            )
            .into()
        };
        let modes = row![
            mode_button("Source", "Markdown source", ViewMode::Source),
            mode_button("Split", "Source and preview side by side", ViewMode::Split),
            mode_button("Preview", "Rendered note", ViewMode::Preview),
            mode_button("Live", "Formatted as you type", ViewMode::Live),
        ]
        .spacing(2);
        let heading: Element<'_, Message> = container(
            column![
                row![
                    text(folder).size(12).color(t.text_2).wrapping(text::Wrapping::None),
                    space().width(Fill),
                    modes,
                ]
                .align_y(iced::Center),
                text(&tab.title).size(22).font(theme::SERIF).color(t.text).wrapping(text::Wrapping::None),
            ]
            .spacing(2),
        )
        .padding([8, 16])
        .width(Fill)
        .into();
        if self.focus {
            // A comfortable reading width; side by side keeps the whole window.
            let panes = if self.view_mode == ViewMode::Split {
                panes
            } else {
                container(container(panes).max_width(FOCUS_WIDTH)).center_x(Fill).into()
            };
            return column![banner, heading, panes].width(Fill).into();
        }
        column![self.tab_bar(t), hline(t), banner, heading, hline(t), panes].width(Fill).into()
    }

    /// The editor always sits in the same two-layer stack. Swapping it
    /// between a bare editor and a stack changes the widget tree, and iced
    /// would drop the editor's state, focus included. The find bar floats at
    /// the top right, the completion popup at the bottom unless the editor
    /// draws it at its caret (`at_caret`).
    fn with_overlay<'a>(&'a self, t: Tokens, editor: Element<'a, Message>, at_caret: bool) -> Element<'a, Message> {
        let find: Element<'_, Message> = match &self.find {
            Some(find) => container(self.find_bar(t, find)).width(Fill).align_right(Fill).into(),
            None => space().into(),
        };
        let popup: Element<'_, Message> = match &self.completion {
            Some(completion) if !at_caret => self.completion_popup(t, completion),
            _ => space().into(),
        };
        let overlay = column![find, space().height(Fill), popup].padding(16);
        stack![editor, overlay].into()
    }

    /// Anchored to the bottom of the source pane. iced does not expose the
    /// editor's scroll offset, so placing it at the caret would drift once the
    /// note scrolls; a fixed anchor is always correct. Live preview knows its
    /// caret and puts the same list under it.
    fn completion_popup<'a>(&'a self, t: Tokens, completion: &'a Completion) -> Element<'a, Message> {
        suggestion_list(t, completion.ctx.kind, &completion.items, completion.selected, Message::CompletePick)
    }

    /// A merge waiting for choices, in place of the editor: each section both
    /// sides changed, with a little context, the other version beside yours.
    fn resolve_panel<'a>(&'a self, t: Tokens, tab: &'a Tab, resolve: &'a Resolve) -> Element<'a, Message> {
        let name = file_stem(&tab.path);
        let (title, intro, theirs_label, take_theirs) = match &resolve.source {
            ResolveSource::Disk { .. } => (
                format!("{name} changed on disk"),
                "It was edited elsewhere while you were editing it here. Everything that does not overlap is \
                 merged already; choose for the sections both sides changed."
                    .to_owned(),
                "On disk",
                "Take disk",
            ),
            ResolveSource::Copy { path } => (
                format!("A conflict copy of {name}"),
                format!(
                    "OneDrive kept {} when two edits collided. Lines both versions have stay as they are; choose \
                     for the sections that differ. The copy then goes to .trash.",
                    path.rsplit('/').next().unwrap_or(path)
                ),
                "The copy",
                "Take copy",
            ),
        };
        let total = resolve.choices.len();
        let decided = resolve.choices.iter().filter(|c| c.is_some()).count();
        let choice = |label: String, on: bool, message: Message| {
            button(text(label).size(13))
                .padding([4, 10])
                .style(move |_, status| theme::nav_button(t, on, status))
                .on_press(message)
        };

        let mut page = Column::new().spacing(16).padding([28, 36]).max_width(980);
        page = page.push(
            column![text(title).size(24).font(theme::SERIF).color(t.text), text(intro).size(14).color(t.text_2)]
                .spacing(6),
        );
        page = page.push(
            row![
                choice("Keep all yours".into(), false, Message::ChooseAll(Choice::Ours)),
                choice(format!("{take_theirs} everywhere"), false, Message::ChooseAll(Choice::Theirs)),
                choice("Keep both everywhere".into(), false, Message::ChooseAll(Choice::Both)),
            ]
            .spacing(8),
        );

        let chunks = &resolve.merge.chunks;
        let (mut n, mut line) = (0, 0);
        for (i, chunk) in chunks.iter().enumerate() {
            let (ours, theirs) = match chunk {
                Chunk::Kept(text) => {
                    line += line_count(text);
                    continue;
                }
                Chunk::Changed { replaces, .. } => {
                    line += replaces;
                    continue;
                }
                Chunk::Conflict { ours, theirs, .. } => (ours, theirs),
            };
            let context = |chunk: Option<&'a Chunk>, last: bool| -> Element<'a, Message> {
                let text_of = |c: &'a Chunk| match c {
                    Chunk::Kept(text) | Chunk::Changed { text, .. } => Some(text.as_str()),
                    Chunk::Conflict { .. } => None,
                };
                let lines: Vec<&str> = chunk.and_then(text_of).map(|s| s.lines().collect()).unwrap_or_default();
                let shown = if last { &lines[lines.len().saturating_sub(2)..] } else { &lines[..lines.len().min(2)] };
                text(shown.join("\n")).size(13).font(theme::MONO).color(t.text_2).into()
            };
            let side = |label: &'a str, content: &'a str| {
                let body: Element<'a, Message> = if content.is_empty() {
                    text("Nothing here").size(13).color(t.text_2).into()
                } else {
                    text(content.trim_end_matches(['\r', '\n'])).size(13).font(theme::MONO).into()
                };
                container(column![text(label).size(12).font(theme::SANS_BOLD).color(t.text_2), body].spacing(6))
                    .padding(10)
                    .width(Fill)
                    .style(move |_| theme::panel(t))
            };
            let chosen = resolve.choices[n];
            page = page.push(
                column![
                    text(format!("Section {} of {total} · line {}", n + 1, line + 1))
                        .size(12)
                        .font(theme::SANS_BOLD)
                        .color(t.text_2),
                    context(i.checked_sub(1).map(|p| &chunks[p]), true),
                    row![side("Yours", ours), side(theirs_label, theirs)].spacing(10),
                    context(chunks.get(i + 1), false),
                    row![
                        choice("Keep yours".into(), chosen == Some(Choice::Ours), Message::Choose(n, Choice::Ours)),
                        choice(take_theirs.into(), chosen == Some(Choice::Theirs), Message::Choose(n, Choice::Theirs)),
                        choice("Keep both".into(), chosen == Some(Choice::Both), Message::Choose(n, Choice::Both)),
                    ]
                    .spacing(8),
                ]
                .spacing(8),
            );
            line += line_count(ours);
            n += 1;
        }

        let mut footer = row![button(text(format!("Apply · {decided} of {total} decided")).size(14))
            .padding([6, 14])
            .style(move |_, status| theme::nav_button(t, decided == total, status))
            .on_press_maybe((decided == total).then_some(Message::ApplyResolve))]
        .spacing(8);
        if matches!(resolve.source, ResolveSource::Copy { .. }) {
            footer = footer.push(choice("Not now".into(), false, Message::CancelResolve));
        }
        page = page.push(container(footer).padding(iced::Padding { top: 8.0, ..iced::Padding::ZERO }));
        scrollable(container(page).width(Fill)).height(Fill).into()
    }

    /// Floats over the source pane. Enter goes to the next match, Shift+Enter
    /// to the previous one, Esc closes it.
    fn find_bar<'a>(&'a self, t: Tokens, find: &'a Find) -> Element<'a, Message> {
        let count = match (find.current, find.matches.len()) {
            (_, 0) if find.query.is_empty() => String::new(),
            (_, 0) => "No matches".to_owned(),
            (Some(i), n) => format!("{} of {n}", i + 1),
            (None, n) => format!("{n} found"),
        };
        container(
            row![
                text_input("Find in note", &find.query)
                    .id(FIND_INPUT)
                    .on_input(Message::FindQuery)
                    .on_submit(Message::FindSubmit)
                    .size(14)
                    .padding([5, 8])
                    .width(240)
                    .style(move |_, status| theme::input(t, status)),
                text(count).size(12).color(t.text_2).width(72),
                button(text("×").size(14))
                    .padding([4, 8])
                    .style(move |_, status| theme::nav_button(t, false, status))
                    .on_press(Message::FindClose),
            ]
            .spacing(8)
            .align_y(iced::Center),
        )
        .padding(6)
        .style(move |_| theme::panel(t))
        .into()
    }

    fn tab_bar(&self, t: Tokens) -> Element<'_, Message> {
        let mut bar = Row::new().height(38);
        for (i, tab) in self.tabs.iter().enumerate() {
            // While the task list is showing, no tab is current.
            let active = i == self.active && (self.tasks.is_none() || tab.tasks);
            let (label, color) = if tab.conflict {
                (format!("{} !", tab.title), t.error)
            } else {
                (tab.title.clone(), if active { t.text } else { t.text_2 })
            };
            let pinned = self.pinned.contains(&tab.path);
            let tab_button = row![
                button(text(if pinned { format!("▪ {label}") } else { label }).size(14).color(color))
                    .padding([8, 12])
                    .style(move |_, status| theme::nav_button(t, false, status))
                    .on_press(Message::SelectTab(i)),
                button(text(if tab.dirty { "•" } else { "×" }).size(14))
                    .padding([8, 8])
                    .style(move |_, status| theme::nav_button(t, false, status))
                    .on_press(Message::CloseTab(i)),
            ];
            bar = bar.push(
                column![
                    // A middle click closes the tab, as in a browser.
                    iced::widget::mouse_area(container(tab_button).height(Fill))
                        .on_middle_press(Message::CloseTab(i))
                        .on_right_press(Message::TabMenu(i)),
                    // The highlight rule under the current tab.
                    container(space()).height(2).width(Fill).style(move |_| theme::accent_rule(t, active)),
                ]
                .width(iced::Shrink),
            );
            bar = bar.push(vline(t));
        }
        bar = bar.push(
            container(
                button(text("+").size(16))
                    .padding([6, 12])
                    .style(move |_, status| theme::nav_button(t, false, status))
                    .on_press(Message::Run(Command::NewTab)),
            )
            .height(Fill)
            .center_y(Fill),
        );
        scrollable(bar)
            .id(TABS)
            .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(0).scroller_width(0)))
            .into()
    }

    fn preview<'a>(&'a self, t: Tokens, tab: &'a Tab) -> Element<'a, Message> {
        let ctx = preview::Ctx::new(t, tab, self.drag.as_ref(), self.folds.get(&tab.path));
        let mut page = Column::new()
            .spacing(10)
            .padding([28, 36])
            .max_width(760)
            .push(preview::blocks(&tab.preview, ctx));

        if !tab.backlinks.is_empty() || !tab.outgoing.is_empty() {
            page = page.push(container(hline(t)).padding(iced::Padding {
                top: 24.0,
                ..iced::Padding::ZERO
            }));
        }
        if !tab.backlinks.is_empty() {
            page = page.push(text("Linked from").size(12).font(theme::SANS_BOLD).color(t.text_2));
            for (path, title) in &tab.backlinks {
                page = page.push(
                    button(text(title).size(15))
                        .padding([2, 0])
                        .style(move |_, status| theme::link_button(t, status))
                        .on_press(Message::Open(path.clone())),
                );
            }
        }
        if !tab.outgoing.is_empty() {
            let gap = if tab.backlinks.is_empty() { 0.0 } else { 12.0 };
            page = page.push(
                container(text("Links to").size(12).font(theme::SANS_BOLD).color(t.text_2))
                    .padding(iced::Padding { top: gap, ..iced::Padding::ZERO }),
            );
            for out in &tab.outgoing {
                let label = text(&out.label).size(15);
                let message = match &out.kind {
                    OutKind::Note(path) => Some(Message::Open(path.clone())),
                    OutKind::Missing(target) => Some(Message::FollowLink(LinkTarget::Note {
                        target: target.clone(),
                        heading: None,
                        block: None,
                    })),
                    OutKind::Attachment => None,
                };
                // Missing notes and attachments recede, as unresolved links do in Obsidian.
                let label = if matches!(out.kind, OutKind::Note(_)) { label } else { label.color(t.text_2) };
                page = page.push(
                    button(label)
                        .padding([2, 0])
                        .style(move |_, status| theme::link_button(t, status))
                        .on_press_maybe(message),
                );
            }
        }

        scrollable(container(page).width(Fill)).height(Fill).into()
    }

    fn status_bar(&self, t: Tokens) -> Element<'_, Message> {
        let right = match self.backend {
            Some(backend) => format!(
                "{}{} · {}",
                backend.label(),
                if self.onedrive { " · OneDrive" } else { "" },
                if self.dark() { "dark" } else { "light" }
            ),
            None => String::new(),
        };
        let showing_note = !self.settings_open && self.tasks.is_none() && self.view_mode != ViewMode::Preview;
        let counts = match self.tabs.get(self.active).filter(|tab| tab.loaded && showing_note) {
            Some(tab) => match tab.content.selection().filter(|s| !s.is_empty()) {
                Some(selected) => {
                    let (words, chars) = counts(&selected);
                    format!("{} · {} selected", plural(words, "word"), plural(chars, "character"))
                }
                None => format!("{} · {}", plural(tab.counts.0, "word"), plural(tab.counts.1, "character")),
            },
            None => String::new(),
        };
        container(
            row![
                text(&self.status).size(12).color(t.text_2),
                space().width(Fill),
                text(counts).size(12).color(t.text_2),
                space().width(20),
                text(right).size(12).color(t.text_2),
            ]
            .padding([5, 12]),
        )
        .into()
    }

    /// The newest notice floating at the bottom right, with Undo when a note
    /// was just moved to `.trash`.
    fn toast_overlay(&self, t: Tokens) -> Option<Element<'_, Message>> {
        let toast = self.toast.as_ref()?;
        let small = |label: &'static str, message: Message| {
            button(text(label).size(13))
                .padding([3, 10])
                .style(move |_, status| theme::nav_button(t, false, status))
                .on_press(message)
        };
        let mut line = Row::new().spacing(10).align_y(iced::Center).push(text(&toast.text).size(13).color(t.text));
        if toast.undo.is_some() {
            line = line.push(small("Undo", Message::UndoTrash));
        }
        if let Some((path, line_no)) = &toast.open {
            let open = match line_no {
                Some(n) => Message::OpenAt(path.clone(), *n as usize),
                None => Message::Open(path.clone()),
            };
            line = line.push(small("Open", open));
        }
        line = line.push(small("×", Message::DismissToast));
        let card = container(line).padding([8, 12]).max_width(560).style(move |_| theme::panel(t));
        Some(container(card).width(Fill).height(Fill).align_right(Fill).align_bottom(Fill).padding([40, 16]).into())
    }

    /// Every shortcut in one place: Sage's own as the vault has them set, then
    /// what the editor does, then the mouse and the `@` and `[[` popups.
    fn help_overlay(&self, t: Tokens) -> Element<'_, Message> {
        let line = |keys: String, what: String| -> Element<'_, Message> {
            row![text(keys).size(13).font(theme::MONO).color(t.text).width(150), text(what).size(13).color(t.text_2)]
                .spacing(10)
                .into()
        };
        let heading = |title: &'static str| text(title).size(12).font(theme::SANS_BOLD).color(t.text_2);
        let mut own = Column::new().spacing(3);
        for i in 0..BUILT_IN.len() {
            if let Some(keys) = built_in_chord(i, &self.settings) {
                own = own.push(line(keys.to_string(), command_name(BUILT_IN[i].1).to_owned()));
            }
        }
        let mut editing = Column::new().spacing(3);
        for (keys, what) in EDITING {
            editing = editing.push(line(keys.to_owned(), what.to_owned()));
        }
        let tricks = column![
            line("[[".into(), "links to a note; Ctrl+click follows one, creating it if missing".into()),
            line("@".into(), "dates such as tomorrow or next friday; people in a task".into()),
            line("#".into(), "tags".into()),
            line("Right click".into(), "menu on a note, folder or tab".into()),
            line("Middle click".into(), "closes a tab".into()),
        ]
        .spacing(3);
        let panel = container(
            column![
                row![
                    text("Keyboard shortcuts").size(20).font(theme::SERIF).color(t.text),
                    space().width(Fill),
                    button(text("Close").size(13))
                        .padding([3, 10])
                        .style(move |_, status| theme::nav_button(t, false, status))
                        .on_press(Message::Run(Command::Shortcuts)),
                ]
                .align_y(iced::Center),
                scrollable(
                    column![heading("SAGE"), own, heading("EDITING"), editing, heading("TIPS"), tricks].spacing(8).padding([0, 12])
                )
                .height(Fill),
            ]
            .spacing(12),
        )
        .padding(18)
        .width(560)
        .max_height(520)
        .style(move |_| theme::panel(t));
        iced::widget::opaque(
            container(panel)
                .width(Fill)
                .height(Fill)
                .align_x(iced::Center)
                .padding(iced::Padding { top: 60.0, ..iced::Padding::ZERO })
                .style(move |_| theme::scrim(t)),
        )
    }

    /// Asks before a note or folder goes to `.trash`. Esc cancels.
    fn confirm_overlay<'a>(&'a self, t: Tokens, doomed: &'a Doomed) -> Element<'a, Message> {
        let (title, detail) = if doomed.folder {
            let prefix = format!("{}/", doomed.path);
            let notes = match self.files.iter().filter(|f| f.path.starts_with(&prefix)).count() {
                1 => "1 note".to_owned(),
                n => format!("{n} notes"),
            };
            (
                format!("Delete the folder {}?", doomed.path),
                format!("It moves to the vault's .trash folder with everything in it, {notes} included. Move it back from there to restore it."),
            )
        } else {
            (
                format!("Delete {}?", file_stem(&doomed.path)),
                format!("{} moves to the vault's .trash folder. Move it back from there to restore it.", doomed.path),
            )
        };
        let choice = |label: &'a str, current: bool, message: Message| {
            button(text(label).size(14))
                .padding([6, 14])
                .style(move |_, status| theme::nav_button(t, current, status))
                .on_press(message)
        };
        let panel = container(
            column![
                text(title).size(20).font(theme::SERIF).color(t.text),
                text(detail).size(14).color(t.text_2),
                row![
                    space().width(Fill),
                    choice("Cancel", false, Message::CancelDelete),
                    choice("Move to .trash", true, Message::ConfirmDelete),
                ]
                .spacing(8),
            ]
            .spacing(12),
        )
        .padding(18)
        .width(480)
        .style(move |_| theme::panel(t));
        iced::widget::opaque(
            container(panel)
                .width(Fill)
                .height(Fill)
                .align_x(iced::Center)
                .padding(iced::Padding { top: 140.0, ..iced::Padding::ZERO })
                .style(move |_| theme::scrim(t)),
        )
    }

    fn palette_overlay<'a>(&'a self, t: Tokens, palette: &'a Palette) -> Element<'a, Message> {
        let mut results = Column::new().spacing(1);
        for (i, hit) in palette.hits.iter().enumerate() {
            let selected = i == palette.selected;
            let message = Message::PaletteClick(i);
            results = results.push(
                button(
                    row![
                        container(space()).width(2).height(20).style(move |_| theme::accent_rule(t, selected)),
                        text(&hit.label).size(15).wrapping(text::Wrapping::None),
                        space().width(Fill),
                        text(&hit.detail).size(12).color(t.text_2).wrapping(text::Wrapping::None),
                    ]
                    .spacing(10)
                    .align_y(iced::Center),
                )
                .width(Fill)
                .padding([6, 8])
                .style(move |_, status| theme::nav_button(t, selected, status))
                .on_press(message),
            );
        }

        let placeholder = match palette.mode {
            PaletteMode::Search => "Find a note or a command".to_owned(),
            PaletteMode::Rename(_) => "New name, with folders to move the note".to_owned(),
            PaletteMode::MoveTo(_) => "Move the note to a folder".to_owned(),
            PaletteMode::RenameFolder(_) => "New name or path for the folder".to_owned(),
            PaletteMode::Template(_) => "Insert a template".to_owned(),
            PaletteMode::QuickAdd => "Quick add".to_owned(),
            PaletteMode::Everywhere(_) => "Search everywhere".to_owned(),
            PaletteMode::NewFolder => "Name of the new folder, with / for folders in folders".to_owned(),
            PaletteMode::NewNote => "Name of the new note, with / for folders".to_owned(),
            PaletteMode::Project(_) => "Choose a project".to_owned(),
            PaletteMode::LogEntry { .. } => "What happened? Start with meeting:, email:, decision: or info:".to_owned(),
            PaletteMode::Triage(_) => "Choose a line to sort".to_owned(),
            PaletteMode::TriageAction(_) => "What should this line become?".to_owned(),
            PaletteMode::Person(..) => "Choose a person".to_owned(),
            PaletteMode::QuickAddValue(i) => match self.quick_entries().get(i) {
                Some(QuickAdd::Capture { name, .. }) => format!("{name}: what to add"),
                Some(entry) => format!("{}: name of the new note", entry.name()),
                None => String::new(),
            },
        };
        let panel = container(
            column![
                text_input(&placeholder, &palette.query)
                    .id(PALETTE_INPUT)
                    .on_input(Message::PaletteQuery)
                    .on_submit(Message::PaletteSubmit)
                    .size(16)
                    .padding(10)
                    .style(move |_, status| theme::input(t, status)),
                // A long list scrolls instead of running off a short window.
                scrollable(results).height(iced::Shrink),
            ]
            .spacing(8),
        )
        .padding(10)
        .width(620)
        .style(move |_| theme::panel(t));

        // The scrim is opaque to the mouse so clicks do not reach the workspace.
        iced::widget::opaque(
            container(panel)
                .width(Fill)
                .height(Fill)
                .align_x(iced::Center)
                .padding(iced::Padding { top: 90.0, bottom: 40.0, ..iced::Padding::ZERO })
                .style(move |_| theme::scrim(t)),
        )
    }
}

fn hline<'a>(t: Tokens) -> Element<'a, Message> {
    container(space()).width(Fill).height(1).style(move |_| theme::hairline(t)).into()
}

fn vline<'a>(t: Tokens) -> Element<'a, Message> {
    container(space()).width(1).height(Fill).style(move |_| theme::hairline(t)).into()
}

/// A small button beside the vault's name, with its meaning in a tooltip.
fn header_button<'a>(label: &'a str, tip: &'a str, message: Message, t: Tokens) -> Element<'a, Message> {
    iced::widget::tooltip(
        button(text(label).size(16).color(t.text_2))
            .padding([2, 8])
            .style(move |_, status| theme::nav_button(t, false, status))
            .on_press(message),
        container(text(tip).size(12)).padding([4, 8]).style(move |_| theme::panel(t)),
        iced::widget::tooltip::Position::Bottom,
    )
    .into()
}

/// A divider between panes: a hairline with room around it to grab and drag.
fn divider<'a>(t: Tokens, which: Divider) -> Element<'a, Message> {
    iced::widget::mouse_area(container(vline(t)).center_x(DIVIDER).height(Fill))
        .interaction(iced::mouse::Interaction::ResizingHorizontally)
        .on_press(Message::ResizeStart(which))
        .into()
}

fn file_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

fn is_note_path(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md"))
}

/// The OneDrive conflict copies of the note at `path`: notes in its folder
/// whose names are a copy's name for it. A copy belongs to the most specific
/// original that exists, so `Note-TPT14.md` does not claim a copy of itself.
fn conflict_copies(files: &[FileRow], path: &str) -> Vec<String> {
    let folder = path.rsplit_once('/').map_or("", |(folder, _)| folder);
    let name_of = |p: &str| p.rsplit('/').next().unwrap_or(p).to_owned();
    let siblings: Vec<String> = files.iter().filter(|f| f.folder == folder).map(|f| name_of(&f.path)).collect();
    let name = name_of(path);
    files
        .iter()
        .filter(|f| f.folder == folder && f.path != path)
        .filter(|f| {
            sage_onedrive::originals_of(&name_of(&f.path)).into_iter().find(|o| siblings.contains(o)) == Some(name.clone())
        })
        .map(|f| f.path.clone())
        .collect()
}

/// The frontmatter's own lines, between the `---` fences, from a note read
/// a line at a time by `line`, or `None` when it has none. Live preview asks
/// on every frame, so only the lines up to the closing fence are read rather
/// than the whole note.
fn frontmatter_in(line: impl Fn(usize) -> Option<String>) -> Option<Vec<String>> {
    // A byte-order mark may sit before the opening fence.
    if line(0)?.trim_start_matches('\u{feff}').trim_end() != "---" {
        return None;
    }
    let mut inner = Vec::new();
    loop {
        let next = line(inner.len() + 1)?;
        if matches!(next.trim_end(), "---" | "...") {
            return Some(inner);
        }
        inner.push(next);
    }
}

/// The folders that hold notes, with the folders above them, and those with
/// `disk`'s folders added as well: what the settings and the tree offer.
/// Each sorted without regard to case, with duplicates gone.
fn folder_lists(files: &[FileRow], disk: &[String]) -> (Vec<String>, Vec<String>) {
    let sorted = |mut folders: Vec<String>| {
        // Case breaks ties, so that the same name twice ends up side by side.
        folders.sort_by_cached_key(|f| (f.to_lowercase(), f.clone()));
        folders.dedup();
        folders
    };
    let mut seen = HashSet::new();
    let mut notes = Vec::new();
    // Notes come a folder at a time, so most are passed over by one lookup.
    for file in files.iter().filter(|f| !f.folder.is_empty()) {
        if seen.insert(file.folder.as_str()) {
            let folder = format!("{}/", file.folder);
            notes.extend(tree::ancestors(&folder).map(str::to_owned));
        }
    }
    let notes = sorted(notes);
    let all = sorted(notes.iter().chain(disk).cloned().collect());
    (notes, all)
}

/// A note's headings, for the outline: level, text and line.
fn outline(text: &str) -> Vec<(u8, String, usize)> {
    sage_core::parse(text).headings.into_iter().map(|h| (h.level, h.text, h.line as usize)).collect()
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

/// What `[[…]]` calls the note at `path`: its name when no other note shares
/// it, else its path without `.md`, as Obsidian writes links.
fn link_name(path: &str, files: &[FileRow]) -> String {
    let stem = file_stem(path);
    let shared = files.iter().any(|f| f.path != path && file_stem(&f.path).eq_ignore_ascii_case(stem));
    if shared { path.strip_suffix(".md").unwrap_or(path).to_owned() } else { stem.to_owned() }
}

/// Shows a file or folder in the system's file manager: selected in
/// Explorer on Windows; elsewhere its folder is opened.
#[cfg(windows)]
fn reveal(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let path = path.to_string_lossy().replace('/', "\\");
    std::process::Command::new("explorer")
        .raw_arg(format!("/select,\"{path}\""))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Explorer did not start: {e}"))
}

#[cfg(not(windows))]
fn reveal(path: &Path) -> Result<(), String> {
    let folder = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
    open_link(&folder.to_string_lossy())
}

/// Words and characters, as the status bar counts them.
/// The editor's text and cursor, for undo.
fn snapshot_of(content: &text_editor::Content) -> sage_editor::undo::Snapshot {
    let at = content.cursor().position;
    sage_editor::undo::Snapshot { text: content.text(), line: at.line, col: at.column }
}

fn counts(text: &str) -> (usize, usize) {
    (text.split_whitespace().count(), text.chars().filter(|c| !matches!(c, '\n' | '\r')).count())
}

/// The picture on the clipboard as PNG, unless the clipboard holds text too:
/// copying from an office program puts both there, and the text is wanted.
fn clipboard_picture() -> Result<Option<Vec<u8>>, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    if clipboard.get_text().is_ok_and(|t| !t.is_empty()) {
        return Ok(None);
    }
    let Ok(picture) = clipboard.get_image() else { return Ok(None) };
    let rgba = image::RgbaImage::from_raw(picture.width as u32, picture.height as u32, picture.bytes.into_owned())
        .ok_or("the clipboard's picture could not be read")?;
    let mut png = Vec::new();
    rgba.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(Some(png))
}

fn line_count(text: &str) -> usize {
    text.split_inclusive('\n').count()
}

/// A file's modification time and size, for noticing writes made elsewhere.
fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((modified.as_nanos() as i64, meta.len()))
}

/// The picture formats the preview decodes; see the `image` features in Cargo.toml.
fn is_picture(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| ["png", "jpg", "jpeg", "gif", "webp"].iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

/// Whether a link target names an attachment rather than a note: a short file
/// extension with a letter in it, other than `.md`. `[[v1.2 plan]]` is a note.
/// What a link written in the note, such as `[[Note#Heading|alias]]`, leads
/// to, read as the preview reads it. Attachments lead nowhere yet.
/// The `[[link]]` as written whose brackets or text hold byte column `col`.
fn wikilink_at(line: &str, col: usize) -> Option<&str> {
    let mut from = 0;
    while let Some(open) = line[from..].find("[[").map(|i| i + from) {
        let close = open + line[open..].find("]]")? + 2;
        if (open..=close).contains(&col) {
            return Some(&line[open..close]);
        }
        from = close;
    }
    None
}

fn link_target(source: &str) -> Option<LinkTarget> {
    let blocks = sage_markdown::parse(source);
    let Some(sage_markdown::Block::Paragraph(inline)) = blocks.first() else { return None };
    match inline.spans.iter().find_map(|s| s.link.clone())? {
        sage_markdown::Link::Internal { target, .. } if is_attachment(&target) => None,
        sage_markdown::Link::Internal { target, heading, block } => Some(LinkTarget::Note { target, heading, block }),
        sage_markdown::Link::External(url) => Some(LinkTarget::Url(url)),
    }
}

fn is_attachment(target: &str) -> bool {
    let name = target.rsplit('/').next().unwrap_or(target);
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && ext.len() <= 5
            && !ext.eq_ignore_ascii_case("md")
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && ext.chars().any(|c| c.is_ascii_alphabetic())
    })
}

/// What a link shows in reading view, as in Obsidian: the alias or link text,
/// else the target, with `> Heading` for a section.
fn link_label(link: &sage_core::Link) -> String {
    if let Some(alias) = &link.alias {
        return alias.clone();
    }
    let section = link.heading.clone().or_else(|| link.block.as_ref().map(|b| format!("^{b}")));
    match (link.target.as_str(), section) {
        ("", Some(section)) => section,
        (target, Some(section)) => format!("{target} > {section}"),
        (target, None) => target.to_owned(),
    }
}

/// Text with every link replaced by what it shows, for plain-text lists.
fn plain_links(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut at = 0;
    for link in sage_core::links_in_line(content) {
        out.push_str(&content[at..link.col as usize]);
        out.push_str(&link_label(&link));
        at = (link.col + link.len) as usize;
    }
    out.push_str(&content[at..]);
    out
}

/// Selects a find match with the caret at its end; the editor scrolls to it.
fn select_match(tab: &mut Tab, (line, range): &(usize, Range<usize>)) {
    tab.content.move_to(Cursor {
        position: Position { line: *line, column: range.end },
        selection: Some(Position { line: *line, column: range.start }),
    });
}

/// The palette while renaming the note at `from`: where it would go and how
/// many notes have links to rewrite.
fn rename_hits(from: &str, query: &str, files: &[FileRow], reader: Option<&Connection>) -> Vec<Hit> {
    let name = sanitize_note_name(query);
    if name.is_empty() {
        return Vec::new();
    }
    let to = format!("{name}.md");
    let hit = if to == from {
        Hit { label: "Type a new name, or a folder path to move it".into(), detail: from.to_owned(), action: HitAction::Nothing }
    } else if files.iter().any(|f| f.path == to) {
        Hit { label: format!("{to} already exists"), detail: String::new(), action: HitAction::Nothing }
    } else {
        let notes: i64 = reader
            .and_then(|db| {
                db.prepare_cached(
                    "SELECT count(DISTINCT l.src_file_id) FROM links l JOIN files f ON f.id = l.dst_file_id
                     WHERE f.path = ?1 AND l.src_file_id <> l.dst_file_id",
                )
                .ok()?
                .query_row([from], |r| r.get(0))
                .ok()
            })
            .unwrap_or(0);
        let detail = match notes {
            0 => "no links to update".to_owned(),
            1 => "updates links in 1 note".to_owned(),
            n => format!("updates links in {n} notes"),
        };
        Hit { label: format!("Rename to {to}"), detail, action: HitAction::Rename(to) }
    };
    vec![hit]
}

/// The palette while moving the note at `from`: the folders matching the
/// query, each moving the note there under its own name.
fn move_hits(from: &str, query: &str, folders: &[String], files: &[FileRow]) -> Vec<Hit> {
    let (current, name) = from.rsplit_once('/').unwrap_or(("", from));
    let filter = query.trim().to_lowercase();
    let root = std::iter::once(String::new());
    root.chain(folders.iter().cloned())
        .filter(|f| f.to_lowercase().contains(&filter) || (f.is_empty() && "vault root".contains(&filter)))
        .map(|folder| {
            let label = if folder.is_empty() { "Vault root".to_owned() } else { format!("{folder}/") };
            let to = if folder.is_empty() { name.to_owned() } else { format!("{folder}/{name}") };
            if folder == current {
                Hit { label, detail: "the note is here".into(), action: HitAction::Nothing }
            } else if files.iter().any(|f| f.path == to) {
                Hit { label, detail: format!("{name} already exists there"), action: HitAction::Nothing }
            } else {
                Hit { label, detail: "move here, links are updated".into(), action: HitAction::Rename(to) }
            }
        })
        .collect()
}

/// The palette while moving the folder at `from`: where it would go and how
/// many notes go with it.
fn folder_hits(from: &str, query: &str, files: &[FileRow]) -> Vec<Hit> {
    let to = sanitize_note_name(query);
    if to.is_empty() {
        return Vec::new();
    }
    let inside = |folder: &str, f: &FileRow| f.folder == folder || f.folder.starts_with(&format!("{folder}/"));
    let hit = if to == from {
        Hit { label: "Type a new name, or a path to move it".into(), detail: from.to_owned(), action: HitAction::Nothing }
    } else if to.starts_with(&format!("{from}/")) {
        Hit { label: format!("{from} cannot move into itself"), detail: String::new(), action: HitAction::Nothing }
    } else if files.iter().any(|f| inside(&to, f) || f.path == format!("{to}.md")) {
        Hit { label: format!("{to} already exists"), detail: String::new(), action: HitAction::Nothing }
    } else {
        let detail = match files.iter().filter(|f| inside(from, f)).count() {
            1 => "1 note moves with it, links are updated".to_owned(),
            n => format!("{n} notes move with it, links are updated"),
        };
        Hit { label: format!("Move folder to {to}"), detail, action: HitAction::RenameFolder(to) }
    };
    vec![hit]
}

/// Commands and Quick Add entries whose name contains `filter`, in lowercase.
fn command_hits(filter: &str, settings: &sage_editor::settings::Settings, entries: &[QuickAdd], can_undo: bool) -> Vec<Hit> {
    let commands = COMMANDS
        .iter()
        .filter(|(name, ..)| name.to_lowercase().contains(filter))
        .filter(|(.., command)| can_undo || *command != Command::UndoRename)
        .map(|&(name, command)| Hit { label: name.into(), detail: keys(command, settings), action: HitAction::Run(command) });
    let entries = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| format!("quick add: {}", entry.name().to_lowercase()).contains(filter))
        .map(|(i, entry)| Hit { label: format!("Quick add: {}", entry.name()), detail: describe(entry), action: HitAction::QuickAdd(i) });
    commands.chain(entries).collect()
}

/// The palette while naming a new folder: the folder Enter creates, or why not.
fn new_folder_hits(query: &str, folders: &[String]) -> Vec<Hit> {
    let folder = sanitize_note_name(query);
    if folder.is_empty() {
        return vec![Hit { label: "Type a name for the folder".into(), detail: String::new(), action: HitAction::Nothing }];
    }
    if folders.iter().any(|f| f.eq_ignore_ascii_case(&folder)) {
        return vec![Hit { label: format!("{folder}/ exists already"), detail: String::new(), action: HitAction::Nothing }];
    }
    vec![Hit { label: format!("Create folder \u{201c}{folder}\u{201d}"), detail: format!("{folder}/"), action: HitAction::CreateFolder(folder) }]
}

/// The full-text query the palette runs for what is typed, if any. Every key
/// runs it, and a word of one or two letters as a prefix matches nearly every
/// note, all of which FTS then ranks: nothing below two letters, and a last
/// word under three matches only as a whole word. Names still match by part.
fn palette_fts(query: &str) -> Option<String> {
    if query.trim().chars().count() < 2 {
        return None;
    }
    let fts = sage_db::fts_query(query)?;
    let last = query.split_whitespace().last()?;
    if last.chars().count() < 3 { fts.strip_suffix('*').map(str::to_owned) } else { Some(fts) }
}

/// What a Quick Add entry does, in a few words, and its shortcut.
fn describe(entry: &QuickAdd) -> String {
    let what = match entry {
        QuickAdd::Template { folder, .. } if folder.trim().is_empty() => "new note".into(),
        QuickAdd::Template { folder, .. } => format!("new note in {}/", folder.trim_matches('/')),
        QuickAdd::Capture { target, .. } => format!("adds to {target}"),
    };
    match entry.shortcut() {
        Some(chord) => format!("{what} · {chord}"),
        None => what,
    }
}

/// Whether running a Quick Add entry asks for a value first. An empty file
/// name or capture text stands for `{{value}}`.
fn needs_value(entry: &QuickAdd) -> bool {
    use sage_editor::quick_add::needs_value as has;
    match entry {
        QuickAdd::Template { folder, file_name, .. } => file_name.trim().is_empty() || has(file_name) || has(folder),
        QuickAdd::Capture { target, format, .. } => format.trim().is_empty() || has(format) || has(target),
    }
}

/// Whether a rename moves the note at `path`.
fn moves(request: &Move, path: &str) -> bool {
    match path.strip_prefix(&request.from) {
        Some(rest) if request.folder => rest.starts_with('/'),
        _ => path == request.from,
    }
}

/// Every embed a preview shows, as `(note it is written in, embed)`: those in
/// `blocks`, which belong to the last note of `chain`, then those in the notes
/// they embed, as far down as the preview draws them. A note already in
/// `chain` embeds itself and is not followed.
fn embeds_in(
    blocks: &[sage_markdown::Block],
    chain: &mut Vec<String>,
    loaded: &HashMap<String, Embedded>,
    out: &mut Vec<(String, sage_markdown::Embed)>,
) {
    if chain.len() > preview::MAX_DEPTH {
        return;
    }
    let (mut embeds, mut views, mut queries) = (Vec::new(), Vec::new(), Vec::new());
    preview::collect(blocks, &mut embeds, &mut views, &mut queries);
    let src = chain.last().cloned().unwrap_or_default();
    for embed in embeds {
        out.push((src.clone(), embed.clone()));
        if let Some(Embedded::Note { path, blocks }) = loaded.get(&preview::embed_key(&src, embed)) {
            if !chain.contains(path) {
                chain.push(path.clone());
                embeds_in(blocks, chain, loaded, out);
                chain.pop();
            }
        }
    }
}

/// Notes in the template folder whose name contains `query`.
fn template_hits(config: &TemplateConfig, query: &str, files: &[FileRow]) -> Vec<Hit> {
    let prefix = format!("{}/", config.folder);
    let query = query.trim().to_lowercase();
    files
        .iter()
        .filter(|f| f.path.starts_with(&prefix) && f.title.to_lowercase().contains(&query))
        .take(12)
        .map(|f| Hit {
            label: f.title.clone(),
            detail: f.path[prefix.len()..].to_owned(),
            action: HitAction::InsertTemplate(f.path.clone()),
        })
        .collect()
}

/// The suggestions as a list in a panel; `pick` makes the message for a row.
fn suggestion_list<'a>(
    t: Tokens,
    kind: complete::Kind,
    items: &'a [Suggestion],
    selected: usize,
    pick: fn(usize) -> Message,
) -> Element<'a, Message> {
    let heading = match kind {
        complete::Kind::Link => "Link to note",
        complete::Kind::Embed => "Embed",
        complete::Kind::Tag => "Tag",
        complete::Kind::Path => "Path",
        complete::Kind::Person => "Person",
        complete::Kind::Date => "Date",
    };
    let mut list = Column::new().spacing(1);
    for (i, item) in items.iter().enumerate() {
        let current = i == selected;
        list = list.push(
            button(
                row![
                    container(space()).width(2).height(18).style(move |_| theme::accent_rule(t, current)),
                    text(&item.label).size(14).wrapping(text::Wrapping::None),
                    space().width(Fill),
                    text(&item.detail).size(12).color(t.text_2).wrapping(text::Wrapping::None),
                ]
                .spacing(10)
                .align_y(iced::Center),
            )
            .width(Fill)
            .padding([4, 8])
            .style(move |_, status| theme::nav_button(t, current, status))
            .on_press(pick(i)),
        );
    }
    container(
        column![
            row![
                text(heading).size(12).font(theme::SANS_BOLD).color(t.text_2),
                space().width(Fill),
                text("↑↓ select · Enter accept · Esc close").size(12).color(t.text_2),
            ]
            .padding([2, 8]),
            list,
        ]
        .spacing(4),
    )
    .padding(6)
    .max_width(560)
    .style(move |_| theme::panel(t))
    .into()
}

/// Puts the editor's cursor, and its selection if `cursor` has one. iced's
/// own `move_to` sets a selection but never takes one away, so after any
/// selection, even an empty one a click leaves, every later click would
/// select from its old end. A motion is what clears it.
fn place(content: &mut text_editor::Content, cursor: Cursor) {
    if cursor.selection.is_none() && content.cursor().selection.is_some() {
        content.perform(text_editor::Action::Move(text_editor::Motion::Left));
    }
    content.move_to(cursor);
}

/// `text` with every line ending as `ending`.
fn with_ending(text: &str, ending: &str) -> String {
    let text = text.replace("\r\n", "\n");
    if ending == "\n" { text } else { text.replace('\n', ending) }
}

/// Runs editing command `op` on only the lines it can touch: those the
/// selection from `anchor` to `head` spans, and one on either side, which
/// moving lines swaps with. The note has `count` lines and `line` reads one.
/// Enter, Backspace and the brackets run a command on every key, and the
/// whole note copied out line by line each time would cost more than the
/// command. Positions go in and come back as in the whole note.
fn windowed(
    count: usize,
    line: impl Fn(usize) -> String,
    anchor: Pos,
    head: Pos,
    op: impl FnOnce(&[String], Pos, Pos) -> Option<edit::Change>,
) -> Option<edit::Change> {
    let first = anchor.line.min(head.line).saturating_sub(1);
    let last = (anchor.line.max(head.line) + 1).min(count.saturating_sub(1)).max(first);
    let lines: Vec<String> = (first..=last).map(line).collect();
    let local = |p: Pos| Pos::new(p.line - first, p.col);
    let mut change = op(&lines, local(anchor), local(head))?;
    change.first += first;
    change.last += first;
    change.anchor.line += first;
    change.head.line += first;
    Some(change)
}

/// Whether a note's preview and outline should wait rather than be parsed
/// again now: only for a note slow to parse (`cost`), while typing goes on
/// (`since_edit`), and not for longer than `PARSE_STALE` (`since_parse`).
fn parse_waits(cost: Duration, since_edit: Duration, since_parse: Duration) -> bool {
    cost > PARSE_FAST && since_edit < PARSE_SETTLE && since_parse < PARSE_STALE
}

/// The active tab once the tabs for which `removed` holds are closed: the
/// same note if it stays open, else the one that takes its place, else the
/// last. Closing a tab to the left of the active one must not switch notes.
/// Closes the task notes that `targets`, link targets written in the note
/// `from`, name. A target that is no task note is left alone, as is a note
/// that fails to write: the checkbox has been checked either way. Returns the
/// vault paths of the notes closed.
fn close_linked_notes(db_path: &Path, vault: &Path, from: &str, targets: &[String], backend: Backend) -> Vec<String> {
    let Ok(db) = sage_db::open_reader(db_path) else { return Vec::new() };
    let mut closed = Vec::new();
    for target in targets {
        let full = sage_core::target_key(target, from);
        let name = sage_core::name_key(target);
        let found: Option<String> = db
            .query_row(
                "SELECT f.path FROM files f JOIN v_documents d ON d.id = f.id
                 WHERE d.type = 'task' AND (f.name_key = ?1 OR f.id_key = ?1)
                   AND (?2 = ?1 OR lower(f.path) LIKE '%' || ?2 || '.md')",
                [&name, &full],
                |r| r.get(0),
            )
            .ok();
        if let Some(path) = found {
            if sage_editor::tasks::close_document(&vault.join(&path), backend) == Ok(true) {
                closed.push(path);
            }
        }
    }
    closed
}

fn active_after<T>(active: usize, tabs: &[T], removed: impl Fn(&T) -> bool) -> usize {
    let before = tabs.iter().take(active).filter(|t| removed(t)).count();
    let left = tabs.iter().filter(|t| !removed(t)).count();
    (active - before).min(left.saturating_sub(1))
}

fn load_session(db_path: &Path) -> Session {
    std::fs::read(session_path(db_path)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

impl Session {
    /// The saved zoom, if it is one Sage offers; else 100%.
    fn zoom(&self) -> f32 {
        self.zoom.filter(|z| (ZOOM[0]..=ZOOM[ZOOM.len() - 1]).contains(z)).unwrap_or(1.0)
    }
}

fn session_path(db_path: &Path) -> PathBuf {
    db_path.with_file_name("session.json")
}

/// Turns palette input into a safe vault-relative note name: characters that
/// Windows (and so OneDrive) rejects are dropped, `/` makes folders, and a
/// typed `.md` is not doubled.
fn sanitize_note_name(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .filter(|c| !matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control())
        .collect();
    let parts: Vec<String> = cleaned
        .split('/')
        .map(|part| part.trim().trim_end_matches('.'))
        .filter(|part| !part.is_empty() && *part != "." && *part != ".." && !part.starts_with('.'))
        // Windows keeps device names for itself, extension or not.
        .map(|part| match part.split_once('.') {
            _ if !is_device_name(part) => part.to_owned(),
            Some((stem, rest)) => format!("{stem}_.{rest}"),
            None => format!("{part}_"),
        })
        .collect();
    let joined = parts.join("/");
    joined.strip_suffix(".md").map(str::to_owned).unwrap_or(joined)
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9` and `LPT1`–`LPT9`, with or without
/// an extension: names Windows, and so OneDrive, refuses for files.
fn is_device_name(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).trim_end().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

/// Runs the indexer for as long as the subscription is alive and forwards its
/// events. The indexer thread calls back from outside the async runtime, so the
/// events cross over on an unbounded channel.
fn indexer_events(key: &(PathBuf, PathBuf)) -> impl Stream<Item = Message> + use<> {
    let (vault, db_path) = key.clone();
    iced::stream::channel(256, async move |mut output| {
        let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
        let handle = sage_indexer::spawn(vault, db_path, move |event| {
            let _ = tx.unbounded_send(event);
        });
        let handle = match handle {
            Ok(handle) => Arc::new(handle),
            Err(error) => {
                let event = IndexEvent::Error(format!("{error:#}"));
                let _ = output.send(Message::Index(event)).await;
                return;
            }
        };
        // The app shares the handle to ask for passes; the stream keeps its own.
        let _ = output.send(Message::IndexerReady(IndexerLink(handle.clone()))).await;
        while let Some(event) = rx.next().await {
            if output.send(Message::Index(event)).await.is_err() {
                break;
            }
        }
    })
}

/// The window got focus back, maybe after the note was edited elsewhere, or
/// lost it.
fn window_focus(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    match event {
        iced::Event::Window(iced::window::Event::Focused) => Some(Message::WindowFocused),
        iced::Event::Window(iced::window::Event::Unfocused) => Some(Message::WindowUnfocused),
        _ => None,
    }
}

/// Opens a web or mail link with the program the desktop has for it: the
/// shell's "open" on Windows; under WSL `wslview`, else Windows' own
/// `explorer.exe`; elsewhere `xdg-open`.
#[cfg(windows)]
fn open_link(url: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(url));
    // SAFETY: both strings are valid and NUL-terminated; the rest may be null.
    let code = unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    };
    // Above 32 is success; below is an error code.
    if code as isize > 32 { Ok(()) } else { Err(format!("Windows could not open the link (error {})", code as isize)) }
}

#[cfg(not(windows))]
fn open_link(url: &str) -> Result<(), String> {
    let wsl = std::env::var_os("WSL_DISTRO_NAME").is_some();
    let openers: &[&str] = if wsl { &["wslview", "explorer.exe"] } else { &["xdg-open"] };
    // Each returns as soon as the handler has the link, and its exit code
    // says little: explorer.exe reports 1 on success.
    let opened = openers.iter().any(|opener| {
        std::process::Command::new(opener)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    });
    if opened { Ok(()) } else { Err(format!("Nothing to open links with: install {}", openers[0])) }
}

/// Reads the vault's Sage settings and Obsidian's Templates settings off the
/// UI thread.
fn load_settings(vault: PathBuf) -> Task<Message> {
    let other = vault.clone();
    Task::batch([
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || sage_editor::settings::load(&vault))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::SettingsLoaded,
        ),
        Task::perform(
            async move { tokio::task::spawn_blocking(move || TemplateConfig::load(&other)).await.unwrap_or_default() },
            Message::ObsidianTemplates,
        ),
    ])
}

/// The window's width, as it opens and whenever it is resized.
fn window_width(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    match event {
        iced::Event::Window(iced::window::Event::Opened { size, .. } | iced::window::Event::Resized(size)) => {
            WINDOW_HEIGHT.store(size.height.to_bits(), std::sync::atomic::Ordering::Relaxed);
            Some(Message::WindowWidth(size.width))
        }
        _ => None,
    }
}

/// Remembers where the pointer is, for the menu a right click opens.
fn track_pointer(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    if let iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) = event {
        let bits = (u64::from(position.x.to_bits()) << 32) | u64::from(position.y.to_bits());
        POINTER.store(bits, std::sync::atomic::Ordering::Relaxed);
    }
    None
}

fn pointer() -> iced::Point {
    let bits = POINTER.load(std::sync::atomic::Ordering::Relaxed);
    iced::Point::new(f32::from_bits((bits >> 32) as u32), f32::from_bits(bits as u32))
}

/// Where the pointer is across the window, while a divider is dragged.
fn pointer_x(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => Some(Message::ResizeMove(position.x)),
        _ => None,
    }
}

/// Ends a Kanban drag wherever the button comes up, even outside the board.
/// The mouse's back and forward buttons.
fn side_buttons(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Back)) => {
            Some(Message::Key(Shortcut::Command(Command::Back)))
        }
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Forward)) => {
            Some(Message::Key(Shortcut::Command(Command::Forward)))
        }
        _ => None,
    }
}

fn file_dropped(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    match event {
        iced::Event::Window(iced::window::Event::FileDropped(path)) => Some(Message::FileDropped(path)),
        _ => None,
    }
}

fn mouse_up(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    matches!(event, iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)))
        .then_some(Message::MouseUp)
}

/// Global shortcuts. Uses the raw event stream because `keyboard::listen` only
/// reports keys that no widget captured, and the editor captures most of them.
fn shortcuts(event: iced::Event, status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    let iced::Event::Keyboard(event) = event else { return None };
    let (key, physical_key, modifiers) = match event {
        keyboard::Event::KeyPressed { key, physical_key, modifiers, .. } => (key, physical_key, modifiers),
        keyboard::Event::ModifiersChanged(modifiers) => return Some(Message::Modifiers(modifiers)),
        _ => return None,
    };
    let shortcut = match key.as_ref() {
        Key::Named(Named::Escape) => Shortcut::Escape,
        Key::Named(Named::ArrowUp) => Shortcut::Up,
        Key::Named(Named::ArrowDown) => Shortcut::Down,
        Key::Named(Named::Tab) => Shortcut::Tab,
        Key::Named(Named::ArrowLeft) if modifiers.alt() && !modifiers.command() && !modifiers.shift() => {
            Shortcut::Command(Command::Back)
        }
        Key::Named(Named::ArrowRight) if modifiers.alt() && !modifiers.command() && !modifiers.shift() => {
            Shortcut::Command(Command::Forward)
        }
        // Only when no editor or text field took the key: there it deletes text.
        Key::Named(Named::Delete) if status == iced::event::Status::Ignored && !modifiers.command() => {
            Shortcut::Command(Command::DeleteNote)
        }
        // Everything else that may be a shortcut; the app knows which are.
        _ => return chord_of(&key, physical_key, modifiers).map(Message::Chord),
    };
    Some(Message::Key(shortcut))
}

/// The chord a key press makes, if it can be a shortcut: one with Ctrl, or a
/// function key. Others type text. Modifiers count exactly, so AltGr, which
/// arrives as Ctrl+Alt, never runs Ctrl's shortcuts while it types ń or ę.
fn chord_of(key: &Key, physical_key: keyboard::key::Physical, modifiers: keyboard::Modifiers) -> Option<Chord> {
    use sage_editor::chord::Key as ChordKey;
    let pressed = match key.as_ref() {
        Key::Named(named) => ChordKey::F(function_key(named)?),
        _ => ChordKey::Char(key.to_latin(physical_key)?.to_ascii_lowercase()),
    };
    let chord = Chord { ctrl: modifiers.command(), alt: modifiers.alt(), shift: modifiers.shift(), key: pressed };
    (chord.ctrl || matches!(pressed, ChordKey::F(_))).then_some(chord)
}

fn function_key(named: Named) -> Option<u8> {
    Some(match named {
        Named::F1 => 1,
        Named::F2 => 2,
        Named::F3 => 3,
        Named::F4 => 4,
        Named::F5 => 5,
        Named::F6 => 6,
        Named::F7 => 7,
        Named::F8 => 8,
        Named::F9 => 9,
        Named::F10 => 10,
        Named::F11 => 11,
        Named::F12 => 12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::sanitize_note_name as s;
    use super::{
        active_after, folder_lists, frontmatter_in, move_hits, new_folder_hits, palette_fts, parse_waits, place, windowed, with_ending, FileRow,
        HitAction,
    };
    use sage_editor::edit::{self, Format, Pos};
    use iced::widget::text_editor::{Content, Cursor, Position};

    #[test]
    fn moving_a_note_offers_the_other_folders() {
        let folders = ["Archive".to_owned(), "Work/Docs".to_owned()];
        let hits = move_hits("Work/Docs/a.md", "", &folders, &[]);
        let targets: Vec<Option<String>> =
            hits.iter().map(|h| if let HitAction::Rename(to) = &h.action { Some(to.clone()) } else { None }).collect();
        assert_eq!(targets, [Some("a.md".to_owned()), Some("Archive/a.md".to_owned()), None], "the folder it is in is no choice");
        let hits = move_hits("a.md", "arch", &folders, &[]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].label, "Archive/");
    }

    #[test]
    fn placing_the_cursor_forgets_an_old_selection() {
        let at = |column| Position { line: 0, column };
        for anchor in [at(4), at(2)] {
            // A drag that ended where it began leaves an empty selection; a
            // real one leaves its anchor. iced's own move keeps either, so the
            // next click would select from there.
            let mut content = Content::with_text("one two three");
            content.move_to(Cursor { position: at(4), selection: Some(anchor) });
            let mut kept: Content = Content::with_text("one two three");
            kept.move_to(Cursor { position: at(4), selection: Some(anchor) });
            kept.move_to(Cursor { position: at(8), selection: None });
            assert_eq!(kept.cursor().selection, Some(anchor), "iced keeps the anchor");

            place(&mut content, Cursor { position: at(8), selection: None });
            assert_eq!((content.cursor().position, content.cursor().selection), (at(8), None));
            assert_eq!(content.selection(), None);
        }
        // A selection asked for is still made.
        let mut content = Content::with_text("one two three");
        place(&mut content, Cursor { position: at(7), selection: Some(at(4)) });
        assert_eq!(content.selection().as_deref(), Some("two"));
    }
    use std::time::Duration;

    #[test]
    fn only_slow_notes_wait_to_parse_and_never_long() {
        let ms = Duration::from_millis;
        assert!(!parse_waits(ms(2), ms(0), ms(0)), "a short note parses on every key");
        assert!(parse_waits(ms(17), ms(100), ms(300)), "a long one waits while typing goes on");
        assert!(!parse_waits(ms(17), ms(300), ms(300)), "until typing pauses");
        assert!(!parse_waits(ms(17), ms(100), ms(800)), "or it has waited long enough");
    }

    #[test]
    fn inserted_text_takes_the_notes_line_endings() {
        assert_eq!(with_ending("---\ntags: [a]\n---\n", "\r\n"), "---\r\ntags: [a]\r\n---\r\n");
        assert_eq!(with_ending("a\r\nb\nc", "\r\n"), "a\r\nb\r\nc");
        assert_eq!(with_ending("a\r\nb\nc", "\n"), "a\nb\nc");
    }

    #[test]
    fn closing_tabs_keeps_the_note_in_view() {
        let tabs = ["A", "B", "C", "D"];
        // B is active: closing A to its left keeps B, closing D keeps B.
        assert_eq!(tabs[1..][active_after(1, &tabs, |t| *t == "A")], "B");
        assert_eq!(active_after(1, &tabs, |t| *t == "D"), 1);
        // Closing B itself shows C, which takes its place; closing the last shows the one before.
        assert_eq!(active_after(1, &tabs, |t| *t == "B"), 1);
        assert_eq!(active_after(3, &tabs, |t| *t == "D"), 2);
        // A folder moved to the trash with A and C in it, D active.
        assert_eq!(active_after(3, &tabs, |t| *t == "A" || *t == "C"), 1);
        assert_eq!(active_after(0, &tabs, |_| true), 0);
    }

    #[test]
    fn new_folders_are_named_safely_and_never_twice() {
        let folders = ["Projects".to_owned()];
        let hits = new_folder_hits("Projects/Q4: plan?", &folders);
        assert!(matches!(&hits[..], [hit] if matches!(&hit.action, HitAction::CreateFolder(f) if f == "Projects/Q4 plan")));
        let hits = new_folder_hits("projects", &folders);
        assert!(matches!(&hits[..], [hit] if matches!(hit.action, HitAction::Nothing)), "exists already, any case");
        let hits = new_folder_hits(".sage", &folders);
        assert!(matches!(&hits[..], [hit] if matches!(hit.action, HitAction::Nothing)), "hidden names are dropped");
    }

    #[test]
    fn editing_commands_see_the_same_on_a_few_lines_as_on_the_whole_note() {
        type Op = Box<dyn Fn(&[String], Pos, Pos) -> Option<edit::Change>>;
        let lines: Vec<String> = ["- one", "  - two", "", "plain (x) **", "> quote", "- [ ] last"].map(String::from).to_vec();
        let ops: Vec<Op> = vec![
            Box::new(|l, a, h| Some(edit::toggle(l, a, h, Format::Bold))),
            Box::new(|l, a, h| Some(edit::link(l, a, h))),
            Box::new(|l, _, h| edit::enter(l, h, "  ")),
            Box::new(|l, a, h| edit::indent(l, a, h, false, "  ")),
            Box::new(|l, a, h| edit::indent(l, a, h, true, "  ")),
            Box::new(|l, a, h| edit::move_lines(l, a, h, true)),
            Box::new(|l, a, h| edit::move_lines(l, a, h, false)),
            Box::new(|l, a, h| Some(edit::duplicate(l, a, h))),
            Box::new(|l, a, h| edit::type_char(l, a, h, '(')),
            Box::new(|l, _, h| edit::backspace(l, h)),
        ];
        // Carets and selections at the top, in the middle, over lines and at the bottom.
        let places = [
            (Pos::new(0, 2), Pos::new(0, 2)),
            (Pos::new(3, 7), Pos::new(3, 7)),
            (Pos::new(1, 0), Pos::new(4, 3)),
            (Pos::new(5, 10), Pos::new(5, 6)),
        ];
        for op in &ops {
            for (anchor, head) in places {
                let few = windowed(lines.len(), |i| lines[i].clone(), anchor, head, |l, a, h| op(l, a, h));
                assert_eq!(few, op(&lines, anchor, head), "at {anchor:?}..{head:?}");
            }
        }
    }

    #[test]
    fn frontmatter_is_read_up_to_its_closing_fence() {
        let read = |text: &str| {
            let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
            frontmatter_in(|i| lines.get(i).cloned())
        };
        let owned = |lines: &[&str]| Some(lines.iter().map(|l| l.to_string()).collect::<Vec<_>>());
        assert_eq!(read("---\ntags: [a]\nup: x\n---\nbody"), owned(&["tags: [a]", "up: x"]));
        assert_eq!(read("\u{feff}---\na: 1\n...\nbody"), owned(&["a: 1"]), "a byte-order mark, and YAML's other end");
        assert_eq!(read("---\n---\nbody"), owned(&[]), "empty");
        assert_eq!(read("---\na: 1\nno end"), None, "never closed");
        assert_eq!(read("# Title\n---\na: 1\n---"), None, "not at the top");
    }

    #[test]
    fn folder_lists_hold_every_ancestor_once() {
        let row = |path: &str| {
            let folder = path.rsplit_once('/').map_or("", |(f, _)| f);
            FileRow { path: path.into(), folder: folder.into(), title: String::new(), cloud: false, modified: 0 }
        };
        let files = [row("a.md"), row("Work/b.md"), row("Work/Docs/c.md"), row("Work/Docs/d.md"), row("archive/e.md")];
        let disk = ["Empty".to_owned(), "Work".to_owned()];
        let (notes, all) = folder_lists(&files, &disk);
        assert_eq!(notes, ["archive", "Work", "Work/Docs"], "sorted without regard to case");
        assert_eq!(all, ["archive", "Empty", "Work", "Work/Docs"]);
    }

    #[test]
    fn the_palette_searches_text_only_for_two_letters_or_more() {
        assert_eq!(palette_fts("a"), None);
        assert_eq!(palette_fts(" a "), None);
        assert_eq!(palette_fts("ab").as_deref(), Some("\"ab\""), "a whole word, not every word starting so");
        assert_eq!(palette_fts("abc").as_deref(), Some("\"abc\"*"));
        assert_eq!(palette_fts("plan q").as_deref(), Some("\"plan\" \"q\""), "a short last word is whole too");
        assert_eq!(palette_fts("plan qu4").as_deref(), Some("\"plan\" \"qu4\"*"));
    }

    #[test]
    fn note_names_are_safe() {
        assert_eq!(s("Meeting: Q4 plan?"), "Meeting Q4 plan");
        assert_eq!(s("Projects/Beta/kickoff.md"), "Projects/Beta/kickoff");
        assert_eq!(s("../../etc/passwd"), "etc/passwd");
        assert_eq!(s(".obsidian/x"), "x");
        assert_eq!(s("  "), "");
        assert_eq!(s("con"), "con_");
        assert_eq!(s("Projects/aux.md"), "Projects/aux_");
        assert_eq!(s("LPT1.draft"), "LPT1_.draft");
        assert_eq!(s("Console"), "Console");
    }
}
