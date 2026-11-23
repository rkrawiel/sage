//! Find in note: literal, case-insensitive matches, positioned in byte columns
//! as the editor cursor expects.

use regex::RegexBuilder;
use std::ops::Range;

/// Every match of `query` in `lines`, as `(line, byte range)`.
pub fn find<'a>(lines: impl IntoIterator<Item = &'a str>, query: &str) -> Vec<(usize, Range<usize>)> {
    if query.is_empty() {
        return Vec::new();
    }
    let Ok(re) = RegexBuilder::new(&regex::escape(query)).case_insensitive(true).build() else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for (n, line) in lines.into_iter().enumerate() {
        matches.extend(re.find_iter(line).map(|m| (n, m.range())));
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_case_insensitive_byte_ranges() {
        assert_eq!(find(["Alpha alpha", "beta", "ALPHA"], "alpha"), [(0, 0..5), (0, 6..11), (2, 0..5)]);
        assert_eq!(find(["Zażółć gęślą"], "ŻÓŁ"), [(0, 2..8)]);
        assert_eq!(find(["a.b axb"], "a.b"), [(0, 0..3)], "not a regex");
        assert!(find(["text"], "").is_empty());
    }
}
