//! Autocomplete suggestions, read from the index.

use sage_editor::complete::{Context, Kind};
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    /// Text inserted for this suggestion, before closing brackets are added.
    pub value: String,
}

const LIMIT: usize = 8;

pub fn for_context(db: &Connection, ctx: &Context) -> Vec<Suggestion> {
    let result = match ctx.kind {
        Kind::Link => links(db, &ctx.query, true),
        Kind::Embed => links(db, &ctx.query, false),
        Kind::Tag => tags(db, &ctx.query),
        Kind::Path => paths(db, &ctx.query),
    };
    result.unwrap_or_default()
}

/// Notes first, then attachments, then aliases. Inserted text follows
/// Obsidian's "shortest unique" rule: the bare name when it is unique in the
/// vault, the path otherwise.
fn links(db: &Connection, query: &str, notes_first: bool) -> rusqlite::Result<Vec<Suggestion>> {
    let mut out = Vec::new();
    let mut stmt = if query.is_empty() {
        db.prepare_cached(
            "SELECT path, title, ext, name_key FROM files
             WHERE ?1 = '' AND (ext = 'md' OR ?2 = 0)
             ORDER BY mtime_ns DESC LIMIT 8",
        )?
    } else {
        db.prepare_cached(
            "SELECT path, title, ext, name_key FROM files
             WHERE instr(name_key, lower(?1)) > 0 OR instr(lower(title), lower(?1)) > 0
             ORDER BY (ext = 'md' AND ?2 = 1) DESC,
                      (substr(name_key, 1, length(?1)) = lower(?1)) DESC,
                      length(path)
             LIMIT 8",
        )?
    };
    let rows = stmt.query_map((query, notes_first), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
    })?;
    for row in rows {
        let (path, title, ext, name_key) = row?;
        let value = link_text(db, &path, &ext, &name_key)?;
        out.push(Suggestion { label: title, detail: path, value });
    }

    if !query.is_empty() && out.len() < LIMIT {
        let mut stmt = db.prepare_cached(
            "SELECT f.path, f.ext, f.name_key, a.alias FROM aliases a JOIN files f ON f.id = a.file_id
             WHERE instr(a.alias_lc, lower(?1)) > 0 LIMIT 4",
        )?;
        let rows = stmt.query_map([query], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
        })?;
        for row in rows {
            let (path, ext, name_key, alias) = row?;
            let target = link_text(db, &path, &ext, &name_key)?;
            out.push(Suggestion {
                label: alias.clone(),
                detail: format!("alias of {path}"),
                value: format!("{target}|{alias}"),
            });
        }
    }
    out.truncate(LIMIT);
    Ok(out)
}

fn link_text(db: &Connection, path: &str, ext: &str, name_key: &str) -> rusqlite::Result<String> {
    let unique: bool = db
        .prepare_cached("SELECT count(*) = 1 FROM files WHERE name_key = ?1")?
        .query_row([name_key], |r| r.get(0))
        .optional()?
        .unwrap_or(false);
    let shown = if unique { path.rsplit('/').next().unwrap_or(path) } else { path };
    Ok(if ext == "md" { shown.strip_suffix(".md").unwrap_or(shown).to_owned() } else { shown.to_owned() })
}

/// Prefix matches first, then matches anywhere (so `progress` finds
/// `status/in-progress`), most used first. Tags are indexed lowercase.
fn tags(db: &Connection, query: &str) -> rusqlite::Result<Vec<Suggestion>> {
    let mut stmt = db.prepare_cached(
        "SELECT tag, count(DISTINCT file_id) FROM tags
         WHERE instr(tag, lower(?1)) > 0
         GROUP BY tag
         ORDER BY (substr(tag, 1, length(?1)) = lower(?1)) DESC, count(*) DESC, tag
         LIMIT 8",
    )?;
    let rows = stmt.query_map([query], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    rows.map(|row| {
        let (tag, notes) = row?;
        Ok(Suggestion {
            label: format!("#{tag}"),
            detail: if notes == 1 { "1 note".into() } else { format!("{notes} notes") },
            value: tag,
        })
    })
    .collect()
}

fn paths(db: &Connection, query: &str) -> rusqlite::Result<Vec<Suggestion>> {
    let mut stmt = db.prepare_cached(
        "SELECT path, title FROM files WHERE instr(lower(path), lower(?1)) > 0
         ORDER BY length(path) LIMIT 8",
    )?;
    let rows = stmt.query_map([query], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    rows.map(|row| {
        let (path, title) = row?;
        Ok(Suggestion { label: path.clone(), detail: title, value: path })
    })
    .collect()
}
