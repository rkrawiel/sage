//! The preview's Markdown block tree: CommonMark parsed with pulldown-cmark,
//! plus what Obsidian adds on top: wikilinks, `![[embeds]]`, `#tags`, task
//! lists, fenced `view` and `tasks` blocks, callouts, `==highlights==` and
//! the frontmatter's properties.
//!
//! References are nodes of the tree, never text patterns: `[[target]]` is a
//! span whose [`Link::Internal`] names the document, and `![[target]]` is a
//! [`Block::Embed`]. What a target is, a plain note or a typed document such
//! as a task, is a fact of the vault and not of the syntax, so it is looked up
//! when the tree is drawn; [`references`] lists what to look up.
//!
//! No UI types here, so the tree is tested headless. Source positions are kept
//! where the preview acts on the source: the line of every task checkbox.

mod tree;

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading { level: u8, text: Inline },
    Paragraph(Inline),
    /// `start` is the first number of an ordered list.
    List { start: Option<u64>, items: Vec<Item> },
    Quote(Vec<Block>),
    /// `> [!kind]± Title`, an Obsidian callout. `kind` is lowercase; `title`
    /// is the kind's name when none is written. `fold` is `Some(true)` for a
    /// foldable callout that starts open (`+`), `Some(false)` for one that
    /// starts folded (`-`). `key` names the callout by kind, title and how
    /// many like it come before, so it stays the same when the note is edited
    /// around it, and its folding can be remembered.
    Callout { kind: String, title: Inline, fold: Option<bool>, key: String, blocks: Vec<Block> },
    /// The frontmatter, as Obsidian shows it at the top of a note.
    Properties(Vec<Property>),
    /// A fenced or indented code block other than `view`.
    Code { language: Option<String>, code: String },
    /// A fenced block with the language `view`: its body, for sage-query.
    View { source: String, line: u32 },
    /// A fenced block with the language `tasks`: a query over task documents,
    /// for `sage_query::documents`.
    TaskQuery { source: String, line: u32 },
    Table { alignments: Vec<Align>, header: Vec<Inline>, rows: Vec<Vec<Inline>> },
    /// `![[target]]` or `![alt](target)`. Always a block of its own: an embed
    /// inside a paragraph splits it.
    Embed(Embed),
    Rule,
    /// Raw HTML, shown as text.
    Html(String),
}

/// One frontmatter key with its values: one for a scalar, several for a list.
/// Wikilinks in values are links, and the values of `tags` are tags.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub key: String,
    pub values: Vec<Inline>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Set for task list items.
    pub task: Option<Task>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Task {
    pub done: bool,
    /// 0-based line of the checkbox, counted from the top of the file.
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Inline {
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    pub link: Option<Link>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    pub code: bool,
    pub tag: bool,
    /// `==text==`
    pub highlight: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    /// A note or attachment in the vault, from `[[target#heading|alias]]` or
    /// `[text](target.md#heading)`. `target` is as written, percent-decoded,
    /// and empty for a link into the same note.
    Internal { target: String, heading: Option<String>, block: Option<String> },
    /// `https://…`, `mailto:…` and any other URL with a scheme.
    External(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Embed {
    /// As written, percent-decoded, without heading or block.
    pub target: String,
    pub heading: Option<String>,
    pub block: Option<String>,
    /// From `![[pic.png|300]]`, `![[pic.png|300x200]]` or `![alt|300](pic.png)`.
    pub width: Option<u32>,
    /// Alt text or alias, without the size.
    pub alt: String,
    /// `![](https://…)`, which is never fetched.
    pub external: bool,
}

/// Parses a note. Frontmatter becomes a [`Block::Properties`] at the top.
pub fn parse(src: &str) -> Vec<Block> {
    tree::parse(src)
}

/// The targets of the links in `blocks` that name a whole document, as
/// written: `[[task-id]]` and `[text](Note.md)`, not `[[Note#Heading]]` or a
/// link into the same note. Each once, in order. Embeds are not included;
/// they are blocks of their own.
pub fn references(blocks: &[Block]) -> Vec<&str> {
    fn inline<'a>(text: &'a Inline, out: &mut Vec<&'a str>) {
        for span in &text.spans {
            if let Some(Link::Internal { target, heading: None, block: None }) = &span.link {
                if !target.is_empty() && !out.contains(&target.as_str()) {
                    out.push(target);
                }
            }
        }
    }
    fn walk<'a>(blocks: &'a [Block], out: &mut Vec<&'a str>) {
        for block in blocks {
            match block {
                Block::Heading { text, .. } | Block::Paragraph(text) => inline(text, out),
                Block::List { items, .. } => items.iter().for_each(|item| walk(&item.blocks, out)),
                Block::Quote(inner) => walk(inner, out),
                Block::Callout { title, blocks, .. } => {
                    inline(title, out);
                    walk(blocks, out);
                }
                Block::Properties(properties) => properties.iter().flat_map(|p| &p.values).for_each(|v| inline(v, out)),
                Block::Table { header, rows, .. } => header.iter().chain(rows.iter().flatten()).for_each(|c| inline(c, out)),
                Block::Code { .. } | Block::View { .. } | Block::TaskQuery { .. } | Block::Embed(_) | Block::Rule | Block::Html(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut out);
    out
}

/// The part of a note that `![[Note#Heading]]` embeds: from that heading to
/// the next heading of the same or a higher level. Headings compare by their
/// letters and digits, ignoring case. `None` if there is no such heading.
pub fn section(src: &str, heading: &str) -> Option<String> {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    // `[[Note#Part#Section]]` names the last heading of the path.
    let wanted = norm(heading.rsplit('#').next().unwrap_or(heading));
    if wanted.is_empty() {
        return None;
    }
    let headings = sage_core::parse(src).headings;
    let i = headings.iter().position(|h| norm(&h.text) == wanted)?;
    let level = headings[i].level;
    let start = headings[i].line as usize;
    let end = headings[i + 1..].iter().find(|h| h.level <= level).map(|h| h.line as usize);
    let lines: Vec<&str> = src.strip_prefix('\u{feff}').unwrap_or(src).lines().collect();
    let end = end.unwrap_or(lines.len()).min(lines.len());
    Some(lines[start..end].join("\n").trim_end().to_owned())
}

/// The line that `![[Note#^id]]` embeds, without its ` ^id` marker. A marker
/// on a line of its own, as Obsidian writes them after lists and tables,
/// stands for the lines right above it.
pub fn block(src: &str, id: &str) -> Option<String> {
    if id.is_empty() {
        return None;
    }
    let marker = format!("^{id}");
    let lines: Vec<&str> = src.strip_prefix('\u{feff}').unwrap_or(src).lines().collect();
    let (n, rest) = lines.iter().enumerate().find_map(|(n, line)| {
        let rest = line.trim_end().strip_suffix(&marker)?;
        (rest.is_empty() || rest.ends_with(char::is_whitespace)).then_some((n, rest.trim_end()))
    })?;
    if !rest.is_empty() {
        return Some(rest.to_owned());
    }
    let above = lines[..n].iter().rev().take_while(|line| !line.trim().is_empty()).count();
    (above > 0).then(|| lines[n - above..n].join("\n"))
}

#[cfg(test)]
mod tests;
