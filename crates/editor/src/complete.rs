//! Autocomplete context detection: what is being typed at the cursor, and what
//! text replaces it when a suggestion is accepted.
//!
//! Pure string logic, so it is tested here without a UI. Columns are byte
//! offsets within the line, matching iced's editor cursor.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// After `[[`.
    Link,
    /// After `![[`.
    Embed,
    /// After a `#` that starts a word.
    Tag,
    /// Inside the `(...)` of a Markdown link: `[text](path`.
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub kind: Kind,
    /// What has been typed so far.
    pub query: String,
    /// Byte column where `query` starts. Accepting replaces `start..cursor`.
    pub start: usize,
    /// Whether the closing `]]` or `)` already follows the cursor.
    pub closed: bool,
}

/// Finds the completion context for the cursor at byte `col` in `line`.
pub fn context(line: &str, col: usize) -> Option<Context> {
    if col > line.len() || !line.is_char_boundary(col) {
        return None;
    }
    let before = &line[..col];
    let after = &line[col..];

    // Wikilinks. The nearest `[[` must not be closed before the cursor, and
    // completion stops once a heading (`#`) or alias (`|`) part begins.
    if let Some(open) = before.rfind("[[") {
        let query = &before[open + 2..];
        if !query.contains("]]") && !query.contains(['#', '|', '^']) {
            let embed = before[..open].ends_with('!');
            return Some(Context {
                kind: if embed { Kind::Embed } else { Kind::Link },
                query: query.to_owned(),
                start: open + 2,
                closed: after.starts_with("]]"),
            });
        }
    }

    // Markdown link target.
    if let Some(open) = before.rfind("](") {
        let query = &before[open + 2..];
        if !query.contains([')', ' ']) {
            return Some(Context {
                kind: Kind::Path,
                query: query.to_owned(),
                start: open + 2,
                closed: after.starts_with(')'),
            });
        }
    }

    // Tags: `#` at the start of a word, followed by at least one tag character.
    // A bare `#` is not offered, so typing a heading never opens the popup.
    let word_start = before
        .char_indices()
        .rev()
        .find(|(_, c)| !is_tag_char(*c) && *c != '#')
        .map_or(0, |(i, c)| i + c.len_utf8());
    let word = &before[word_start..];
    if let Some(query) = word.strip_prefix('#') {
        if !query.is_empty() && !query.contains('#') && !query.chars().all(|c| c.is_ascii_digit()) {
            return Some(Context {
                kind: Kind::Tag,
                query: query.to_owned(),
                start: word_start + 1,
                closed: false,
            });
        }
    }
    None
}

/// The text that replaces `start..cursor` when `value` is accepted.
pub fn insertion(ctx: &Context, value: &str) -> String {
    match ctx.kind {
        Kind::Link | Kind::Embed if ctx.closed => value.to_owned(),
        Kind::Link | Kind::Embed => format!("{value}]]"),
        // Markdown links need encoded spaces, as Obsidian writes them.
        Kind::Path => {
            let encoded = value.replace(' ', "%20");
            if ctx.closed { encoded } else { format!("{encoded})") }
        }
        Kind::Tag => format!("{value} "),
    }
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line_with_cursor: &str) -> Option<Context> {
        let col = line_with_cursor.find('|').unwrap();
        let line = line_with_cursor.replacen('|', "", 1);
        context(&line, col)
    }

    #[test]
    fn links_and_embeds() {
        let c = at("see [[Proj|").unwrap();
        assert_eq!((c.kind, c.query.as_str(), c.start, c.closed), (Kind::Link, "Proj", 6, false));
        assert_eq!(insertion(&c, "Project Alpha"), "Project Alpha]]");

        let c = at("![[pi|]]").unwrap();
        assert_eq!((c.kind, c.closed), (Kind::Embed, true));
        assert_eq!(insertion(&c, "pic.png"), "pic.png");

        assert_eq!(at("[[|").unwrap().query, "");
        assert!(at("[[Note]] then|").is_none());
        assert!(at("[[Note#Head|").is_none());
    }

    #[test]
    fn tags() {
        let c = at("text #proj|").unwrap();
        assert_eq!((c.kind, c.query.as_str(), c.start), (Kind::Tag, "proj", 6));
        assert_eq!(at("#status/in|").unwrap().query, "status/in");
        assert!(at("# |").is_none(), "a heading is not a tag");
        assert!(at("#|").is_none());
        assert!(at("issue #12|").is_none());
        assert!(at("a#b|").is_none(), "tags start a word");
        assert_eq!(at("zażółć #gęś|").unwrap().query, "gęś");
    }

    #[test]
    fn paths() {
        let c = at("[spec](docs/Sp|").unwrap();
        assert_eq!((c.kind, c.query.as_str()), (Kind::Path, "docs/Sp"));
        assert_eq!(insertion(&c, "docs/Spec v2.md"), "docs/Spec%20v2.md)");
        assert!(at("[spec](done) more|").is_none());
    }
}
