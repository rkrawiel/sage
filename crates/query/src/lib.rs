//! `view` blocks: the query language, its compiler to parameterized SQL over
//! the index's stable views (`v_notes`, `v_tasks`, `v_tags`, `v_properties`,
//! `v_backlinks`, and `fts`), and running a block into rows for the preview.
//! No raw SQL ever comes from a note. `tasks` blocks list typed task
//! documents; see [`documents`].

pub mod documents;
pub mod dsl;
mod sql;
mod view;

pub use view::{parse_spec, run, same_status, Board, BoardColumn, NoteRow, Outcome, Sort, Table, TaskRow, Tasks, ViewKind, ViewSpec};
