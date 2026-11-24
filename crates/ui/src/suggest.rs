//! Autocomplete suggestions, read from the index.

use sage_editor::complete::{Context, Kind};
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    /// Text inserted for this suggestion, before closing brackets are added.
    pub value: String,
    /// A natural-language date, written as a link to its day whatever the
    /// popup it is in.
    pub date: bool,
}

const LIMIT: usize = 8;

pub fn for_context(db: &Connection, ctx: &Context) -> Vec<Suggestion> {
    let result = match ctx.kind {
        Kind::Link => links(db, &ctx.query, true),
        Kind::Embed => links(db, &ctx.query, false),
        Kind::Tag => tags(db, &ctx.query),
        Kind::Path => paths(db, &ctx.query),
        Kind::Person => people(db, &ctx.query).map(|mut found| {
            // Dates follow the people, so `@tomorrow` works in a task too.
            found.truncate(5);
            let room = LIMIT - found.len();
            found.extend(dates(&ctx.query).into_iter().take(room));
            found
        }),
        Kind::Date => Ok(dates(&ctx.query)),
    };
    result.unwrap_or_default()
}

/// The days a phrase such as `tomorrow` can mean, labelled with the date.
fn dates(query: &str) -> Vec<Suggestion> {
    let today = sage_editor::now().date();
    sage_editor::dates::suggest(query, today)
        .into_iter()
        .take(LIMIT)
        .map(|p| Suggestion {
            label: p.label,
            detail: p.date.format("%a %d %b %Y").to_string(),
            value: p.date.format("%Y-%m-%d").to_string(),
            date: true,
        })
        .collect()
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
        // The file's own name is what a link says, so it is the label; a differing title goes beside the folder.
        let file = path.rsplit('/').next().unwrap_or(&path);
        let stem = if ext == "md" { file.strip_suffix(".md").unwrap_or(file) } else { file }.to_owned();
        let detail = if title != stem && ext == "md" { format!("{title} · {path}") } else { path };
        out.push(Suggestion { label: stem, detail, value, date: false });
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
                date: false,
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

/// Notes that are people: a `type: person` property, or a folder called
/// `People`. Matched by name, title or alias; the most recently changed first.
fn people(db: &Connection, query: &str) -> rusqlite::Result<Vec<Suggestion>> {
    let mut stmt = db.prepare_cached(
        "SELECT f.path, f.title, f.name_key FROM files f
         WHERE f.ext = 'md'
           AND (lower(f.parent) = 'people' OR lower(f.parent) LIKE '%/people'
                OR EXISTS (SELECT 1 FROM frontmatter p WHERE p.file_id = f.id AND p.key = 'type' AND lower(trim(p.value)) = 'person'))
           AND (?1 = '' OR instr(f.name_key, lower(?1)) > 0 OR instr(lower(f.title), lower(?1)) > 0
                OR EXISTS (SELECT 1 FROM aliases a WHERE a.file_id = f.id AND instr(a.alias_lc, lower(?1)) > 0))
         ORDER BY (substr(f.name_key, 1, length(?1)) = lower(?1)) DESC, f.mtime_ns DESC
         LIMIT 8",
    )?;
    let rows = stmt.query_map([query], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (path, title, name_key) = row?;
        let value = link_text(db, &path, "md", &name_key)?;
        let file = path.rsplit('/').next().unwrap_or(&path);
        let stem = file.strip_suffix(".md").unwrap_or(file).to_owned();
        let detail = if title != stem { format!("{title} · {path}") } else { path };
        out.push(Suggestion { label: stem, detail, value, date: false });
    }
    Ok(out)
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
            date: false,
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
        Ok(Suggestion { label: path.clone(), detail: title, value: path, date: false })
    })
    .collect()
}
