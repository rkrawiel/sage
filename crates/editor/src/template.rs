//! Templates in Obsidian's core syntax: `{{title}}`, `{{date}}`, `{{time}}`,
//! and `{{date:FORMAT}}` / `{{time:FORMAT}}` with Moment.js format tokens.
//! The commonest Templater tags are read too, for vaults that use that plugin:
//! `<% tp.date.now("FORMAT") %>`, `<% tp.file.creation_date("FORMAT") %>`,
//! `<% tp.file.title %>`. Anything else of Templater's is left as written.

use chrono::{Datelike, NaiveDateTime, Timelike};
use regex::{Captures, Regex};
use std::path::Path;
use std::sync::LazyLock;

/// The settings of Obsidian's Templates plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Vault-relative folder holding the templates.
    pub folder: String,
    pub date_format: String,
    pub time_format: String,
}

/// Obsidian's defaults, with a `Templates` folder.
impl Default for Config {
    fn default() -> Self {
        Self { folder: "Templates".into(), date_format: "YYYY-MM-DD".into(), time_format: "HH:mm".into() }
    }
}

impl Config {
    /// Reads `.obsidian/templates.json`. Missing values fall back to the defaults.
    pub fn load(vault: &Path) -> Self {
        let json: serde_json::Value = std::fs::read(vault.join(".obsidian/templates.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let get = |key: &str| json.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
        let default = Self::default();
        Self {
            folder: get("folder").map(|f| f.trim_matches('/').to_owned()).unwrap_or(default.folder),
            date_format: get("dateFormat").map(str::to_owned).unwrap_or(default.date_format),
            time_format: get("timeFormat").map(str::to_owned).unwrap_or(default.time_format),
        }
    }
}

static VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\{\{\s*(title|date|time)\s*(?::([^}]*))?\}\}").unwrap());

static TEMPLATER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<%[-_]?\s*tp\.(?:(date)\.now|file\.(creation_date|last_modified_date)|(file)\.title)\s*(?:\(\s*(?:"([^"]*)"|'([^']*)')?[^)]*\))?\s*[-_]?%>"#).unwrap()
});

/// Fills in the variables of `template` for the note called `title`.
pub fn expand(template: &str, title: &str, now: NaiveDateTime, config: &Config) -> String {
    let template = TEMPLATER.replace_all(template, |c: &Captures| {
        if c.get(3).is_some() {
            return title.to_owned();
        }
        let format = c.get(4).or_else(|| c.get(5)).map_or("YYYY-MM-DD", |m| m.as_str());
        moment(format, now)
    });
    VARIABLE
        .replace_all(&template, |c: &Captures| {
            let format = c.get(2).map(|m| m.as_str().trim());
            match c[1].to_lowercase().as_str() {
                "title" => title.to_owned(),
                "date" => moment(format.unwrap_or(&config.date_format), now),
                _ => moment(format.unwrap_or(&config.time_format), now),
            }
        })
        .into_owned()
}

/// The frontmatter YAML of a note after inserting a template whose
/// frontmatter is `template`: keys the note lacks are added as the template
/// writes them, list items the note lacks join its lists, and every value the
/// note has already stays. `note` is the note's frontmatter, if it has one.
pub fn merge_properties(note: Option<&str>, template: &str) -> String {
    use serde_yaml_ng::{Mapping, Value};
    let values = |yaml: &str| match serde_yaml_ng::from_str::<Value>(yaml) {
        Ok(Value::Mapping(map)) => map,
        _ => Mapping::new(),
    };
    let (mine, theirs) = (note.map(values).unwrap_or_default(), values(template));
    let mut blocks = note.map(key_blocks).unwrap_or_default();
    for (key, text) in key_blocks(template) {
        // Comments in the template stay there.
        let Some(key) = key else { continue };
        let Some((_, block)) = blocks.iter_mut().find(|(k, _)| k.as_deref() == Some(key.as_str())) else {
            blocks.push((Some(key), text));
            continue;
        };
        let (Some(Value::Sequence(have)), Some(Value::Sequence(add))) = (mine.get(key.as_str()), theirs.get(key.as_str())) else {
            continue;
        };
        let new: Vec<Value> = add.iter().filter(|v| !have.contains(v)).cloned().collect();
        if new.is_empty() {
            continue;
        }
        let mut map = Mapping::new();
        map.insert(Value::String(key), Value::Sequence(have.iter().cloned().chain(new).collect()));
        if let Ok(yaml) = serde_yaml_ng::to_string(&map) {
            *block = yaml;
        }
    }
    let mut yaml = String::new();
    for (_, block) in blocks {
        yaml.push_str(&block);
        if !yaml.ends_with('\n') {
            yaml.push('\n');
        }
    }
    yaml
}

/// YAML split into top-level entries: a `key:` line with the indented or
/// `- ` lines under it. Comment lines and lines before the first key are
/// entries without a key, so rewriting a key never drops them.
fn key_blocks(yaml: &str) -> Vec<(Option<String>, String)> {
    let mut out: Vec<(Option<String>, String)> = Vec::new();
    for line in yaml.split_inclusive('\n') {
        let nested = line.starts_with([' ', '\t', '-']) || line.trim().is_empty();
        match out.last_mut() {
            Some((_, block)) if nested => block.push_str(line),
            _ if line.starts_with('#') || nested => out.push((None, line.to_owned())),
            _ => {
                let key = line.split_once(':').map_or(line, |(key, _)| key).trim().trim_matches(['"', '\'']);
                out.push((Some(key.to_owned()), line.to_owned()));
            }
        }
    }
    out
}

/// Formats `t` with Moment.js tokens, the format language Obsidian uses.
/// `[text]` is copied literally, as are characters that are not tokens.
pub fn moment(format: &str, t: NaiveDateTime) -> String {
    // Longer tokens first, so `MMMM` is not read as four `M`s.
    const TOKENS: [&str; 30] = [
        "YYYY", "YY", "Q", "MMMM", "MMM", "MM", "M", "DDDD", "DDD", "Do", "DD", "D", "dddd", "ddd", "dd", "d",
        "E", "GGGG", "WW", "W", "HH", "H", "hh", "h", "mm", "m", "ss", "s", "A", "a",
    ];
    let mut out = String::new();
    let mut rest = format;
    while let Some(ch) = rest.chars().next() {
        if let Some(end) = rest.strip_prefix('[').and_then(|r| r.find(']')) {
            out.push_str(&rest[1..=end]);
            rest = &rest[end + 2..];
        } else if let Some(token) = TOKENS.iter().find(|token| rest.starts_with(*token)) {
            out.push_str(&field(token, t));
            rest = &rest[token.len()..];
        } else {
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

fn field(token: &str, t: NaiveDateTime) -> String {
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
        "November", "December",
    ];
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    let month = MONTHS[t.month0() as usize];
    let weekday = t.weekday().num_days_from_sunday();
    let day = DAYS[weekday as usize];
    let hour12 = match t.hour() % 12 {
        0 => 12,
        h => h,
    };
    match token {
        "YYYY" => format!("{:04}", t.year()),
        "YY" => format!("{:02}", t.year().rem_euclid(100)),
        "Q" => (t.month0() / 3 + 1).to_string(),
        "MMMM" => month.to_owned(),
        "MMM" => month[..3].to_owned(),
        "MM" => format!("{:02}", t.month()),
        "M" => t.month().to_string(),
        "DDDD" => format!("{:03}", t.ordinal()),
        "DDD" => t.ordinal().to_string(),
        "Do" => {
            let n = t.day();
            let suffix = match (n % 10, n % 100) {
                (_, 11..=13) => "th",
                (1, _) => "st",
                (2, _) => "nd",
                (3, _) => "rd",
                _ => "th",
            };
            format!("{n}{suffix}")
        }
        "DD" => format!("{:02}", t.day()),
        "D" => t.day().to_string(),
        "dddd" => day.to_owned(),
        "ddd" => day[..3].to_owned(),
        "dd" => day[..2].to_owned(),
        "d" => weekday.to_string(),
        "E" => t.weekday().number_from_monday().to_string(),
        "GGGG" => format!("{:04}", t.iso_week().year()),
        "WW" => format!("{:02}", t.iso_week().week()),
        "W" => t.iso_week().week().to_string(),
        "HH" => format!("{:02}", t.hour()),
        "H" => t.hour().to_string(),
        "hh" => format!("{hour12:02}"),
        "h" => hour12.to_string(),
        "mm" => format!("{:02}", t.minute()),
        "m" => t.minute().to_string(),
        "ss" => format!("{:02}", t.second()),
        "s" => t.second().to_string(),
        "A" => (if t.hour() < 12 { "AM" } else { "PM" }).to_owned(),
        _ => (if t.hour() < 12 { "am" } else { "pm" }).to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn moment_tokens() {
        let t = at("2026-01-02 15:04:05");
        assert_eq!(moment("YYYY-MM-DD HH:mm:ss", t), "2026-01-02 15:04:05");
        assert_eq!(moment("dddd, MMMM Do YYYY h:mm A", t), "Friday, January 2nd 2026 3:04 PM");
        assert_eq!(moment("ddd D MMM YY, [week] W, GGGG-[W]WW-E", t), "Fri 2 Jan 26, week 1, 2026-W01-5");
        assert_eq!(moment("DDDD Q hh a [open", t), "002 1 03 pm [open");
        // 2027-01-01 is a Friday in ISO week 53 of 2026.
        assert_eq!(moment("GGGG-[W]WW", at("2027-01-01 00:00:00")), "2026-W53");
    }

    #[test]
    fn expands_obsidian_variables() {
        let config = Config { folder: "Templates".into(), date_format: "DD.MM.YYYY".into(), time_format: "HH:mm".into() };
        let out = expand(
            "# {{title}}\n{{date}} {{time}} {{DATE:YYYY-[W]WW}} {{ time : h A }} {{other}}",
            "Kickoff",
            at("2026-11-13 09:30:00"),
            &config,
        );
        assert_eq!(out, "# Kickoff\n13.11.2026 09:30 2026-W46 9 AM {{other}}");
    }

    #[test]
    fn expands_the_common_templater_tags() {
        let out = expand(
            "Date: <% tp.file.creation_date(\"YYYY-MM-DD\") %>\n<%tp.date.now('DD.MM')%> <% tp.file.title %> <% tp.system.prompt() %>",
            "Sync",
            at("2026-11-13 09:30:00"),
            &Config::default(),
        );
        assert_eq!(out, "Date: 2026-11-13\n13.11 Sync <% tp.system.prompt() %>");
    }

    #[test]
    fn merges_template_properties_into_the_note() {
        let note = "title: Kickoff\ntags:\n  - meeting\n# keep me\nstatus: draft\n";
        let template = "# from the template\ntags: [meeting, work]\nstatus: todo\ncreated: 2026-11-13\nattendees:\n  - Ann\n";
        assert_eq!(
            merge_properties(Some(note), template),
            "title: Kickoff\ntags:\n- meeting\n- work\n# keep me\nstatus: draft\ncreated: 2026-11-13\nattendees:\n  - Ann\n"
        );
        assert_eq!(merge_properties(None, "a: 1\nb:\n- x"), "a: 1\nb:\n- x\n");
        assert_eq!(merge_properties(Some("tags: [a]"), "tags: [a]\n"), "tags: [a]\n", "nothing new, nothing rewritten");
        assert_eq!(merge_properties(Some("tags: one"), "tags: [two]\n"), "tags: one\n", "a scalar stays as it is");
    }

    #[test]
    fn reads_obsidian_settings() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Config::load(dir.path()).folder, "Templates");
        std::fs::create_dir(dir.path().join(".obsidian")).unwrap();
        std::fs::write(dir.path().join(".obsidian/templates.json"), r#"{"folder":"/Meta/Templates/","dateFormat":"D MMM"}"#)
            .unwrap();
        let config = Config::load(dir.path());
        assert_eq!((config.folder.as_str(), config.date_format.as_str(), config.time_format.as_str()), ("Meta/Templates", "D MMM", "HH:mm"));
    }
}
