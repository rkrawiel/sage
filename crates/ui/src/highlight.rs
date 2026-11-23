//! Markdown syntax highlighting for the source pane.
//!
//! iced calls the highlighter line by line, top to bottom, and tells it when a
//! line changed. Fenced code and frontmatter span lines, so the block state at
//! the start of every line is remembered and replayed from the first edited line.

use crate::live::{fence_closes, fence_open, Fence};
use crate::theme::Tokens;
use iced::advanced::text::highlighter::{Format, Highlighter};
use iced::{Color, Font, Theme};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Block {
    Body,
    Frontmatter,
    /// Inside fenced code, opened by this fence.
    Fence(Fence),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub tokens: Tokens,
    /// The current find match as `(line, byte range)`. iced draws the editor's
    /// selection only while the editor has focus, and the find bar keeps it, so
    /// the match is marked by colour instead.
    pub mark: Option<(usize, Range<usize>)>,
}

pub struct Markdown {
    tokens: Tokens,
    mark: Option<(usize, Range<usize>)>,
    /// `starts[i]` is the block state at the beginning of line `i`.
    starts: Vec<Block>,
    line: usize,
}

/// The colour for a span. Carried by value because iced's format callback is a
/// plain `fn` and cannot capture the tokens.
pub struct Paint(Color);

pub fn to_format(paint: &Paint, _theme: &Theme) -> Format<Font> {
    Format { color: Some(paint.0), font: None }
}

impl Highlighter for Markdown {
    type Settings = Settings;
    type Highlight = Paint;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, Paint)>;

    fn new(settings: &Settings) -> Self {
        Self { tokens: settings.tokens, mark: settings.mark.clone(), starts: vec![Block::Body], line: 0 }
    }

    fn update(&mut self, settings: &Settings) {
        // A moved find mark needs repainting from the first line it was or is
        // on; a new theme needs everything.
        let from = if settings.tokens != self.tokens {
            0
        } else {
            [&self.mark, &settings.mark].into_iter().flatten().map(|(line, _)| *line).min().unwrap_or(0)
        };
        self.tokens = settings.tokens;
        self.mark = settings.mark.clone();
        self.change_line(from);
    }

    fn change_line(&mut self, line: usize) {
        let line = line.min(self.starts.len() - 1);
        self.starts.truncate(line + 1);
        self.line = line;
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        let t = self.tokens;
        let state = self.starts[self.line];
        let opens = fence_open(line).map(|(fence, _)| fence);
        let whole = |color| vec![(0..line.len(), Paint(color))];

        let (mut spans, next) = match state {
            Block::Frontmatter => {
                let end = line.trim_end() == "---" || line.trim_end() == "...";
                (whole(t.text_2), if end { Block::Body } else { Block::Frontmatter })
            }
            // Only the same mark, at least as long, closes it, so a fence shown
            // inside a longer one stays code.
            Block::Fence(open) => (whole(t.text_2), if fence_closes(line, open) { Block::Body } else { Block::Fence(open) }),
            Block::Body if self.line == 0 && line.trim_end() == "---" => {
                (whole(t.text_2), Block::Frontmatter)
            }
            Block::Body => match opens {
                Some(fence) => (whole(t.text_2), Block::Fence(fence)),
                None => (inline_spans(line, t), Block::Body),
            },
        };
        // Last, so it paints over the syntax colours. Clamped, because the mark
        // can be a frame behind the text.
        if let Some((_, range)) = self.mark.as_ref().filter(|(at, _)| *at == self.line) {
            let end = range.end.min(line.len());
            let start = range.start.min(end);
            if line.is_char_boundary(start) && line.is_char_boundary(end) {
                spans.push((start..end, Paint(t.warning)));
            }
        }

        self.line += 1;
        self.starts.truncate(self.line);
        self.starts.push(next);
        spans.into_iter()
    }

    fn current_line(&self) -> usize {
        self.line
    }
}

fn inline_spans(line: &str, t: Tokens) -> Vec<(Range<usize>, Paint)> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();

    if trimmed.starts_with('#') && trimmed.trim_start_matches('#').starts_with(' ') {
        return vec![(0..line.len(), Paint(t.accent_2))];
    }
    if trimmed.starts_with('>') {
        return vec![(0..line.len(), Paint(t.text_2))];
    }

    let mut spans = Vec::new();
    let mut from = indent;
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 6 && matches!(bytes[0], b'-' | b'*' | b'+') && &bytes[1..3] == b" [" && bytes[4] == b']' {
        if bytes[3] != b' ' {
            // A completed task recedes as a whole.
            return vec![(0..line.len(), Paint(t.text_2))];
        }
        spans.push((indent..indent + 5, Paint(t.accent)));
        from = indent + 5;
    }

    let bytes = line.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                let end = line[i + 1..].find('`').map_or(line.len(), |n| i + n + 2);
                spans.push((i..end, Paint(t.text_2)));
                i = end;
            }
            b'[' if bytes.get(i + 1) == Some(&b'[') => {
                let Some(n) = line[i..].find("]]") else { break };
                spans.push((i..i + n + 2, Paint(t.accent)));
                i += n + 2;
            }
            b'#' if i == 0 || bytes[i - 1].is_ascii_whitespace() => {
                let len = line[i + 1..]
                    .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '/')))
                    .unwrap_or(line.len() - i - 1);
                if len > 0 {
                    spans.push((i..i + 1 + len, Paint(t.text_2)));
                }
                i += 1 + len;
            }
            _ => i += 1,
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether each line starts inside fenced code.
    fn in_code(text: &str) -> Vec<bool> {
        let mut h = Markdown::new(&Settings { tokens: Tokens::LIGHT, mark: None });
        for line in text.lines() {
            let _ = h.highlight_line(line).count();
        }
        h.starts.iter().map(|b| *b != Block::Body).collect()
    }

    #[test]
    fn a_fence_closes_only_on_its_own_kind_and_length() {
        // A fence shown inside a longer one stays code until the longer closes.
        let nested = "````markdown\n```view\ntype: tasks\n```\n````\nafter";
        assert_eq!(in_code(nested), [false, true, true, true, true, false, false]);
        // Tildes do not close backticks, nor a fence with a language.
        let mixed = "```\n~~~\n```rust\n```\nafter";
        assert_eq!(in_code(mixed), [false, true, true, true, false, false]);
    }
}
