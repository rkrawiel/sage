//! Search everywhere (`Ctrl+Shift+F`): fuzzy matching over every note's
//! metadata (name, title, aliases, tags, properties, headings, path) and text.
//!
//! The index is read once when the search opens, off the UI thread with a
//! reader of its own; each keystroke then scores in memory. Text is matched word by word through the vault's vocabulary, so
//! a typo still finds the note while a long body cannot match a query letter
//! by letter. Every word of the query has to match somewhere in a note.
//! Each query word's text matches are kept until the next search, so typing
//! only walks the vocabulary again for the word being typed.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::Connection;

#[derive(Default)]
pub(crate) struct Corpus {
    notes: Vec<Note>,
    /// Each lowercase word of the notes' text: the notes it is in, with the
    /// first line it is on.
    words: HashMap<String, Vec<(u32, u32)>>,
    /// The last search's text matches per query word. The corpus never
    /// changes once loaded, so they stay right until the next search drops
    /// the words it no longer has; a mutex, as the corpus is shared in an `Arc`.
    matched: Mutex<HashMap<String, Matches>>,
}

/// The notes whose text matches a query word: best score and its line.
type Matches = HashMap<u32, (i32, u32)>;

/// Buffers reused across the many words one search scores, so matching a
/// word allocates nothing.
#[derive(Default)]
struct Scratch {
    word: Vec<char>,
    hay: Vec<char>,
    /// The rows `distance` keeps: two back, one back and the current one.
    rows: [Vec<usize>; 3],
}

struct Note {
    path: String,
    name: String,
    text: String,
    fields: Vec<Field>,
}

struct Field {
    kind: Kind,
    /// As shown in the result row.
    shown: String,
    lower: String,
    line: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Kind {
    Name,
    Title,
    Alias,
    Tag,
    Property,
    Heading,
    Path,
}

impl Kind {
    /// How much a match here counts against a match in the text, in tenths.
    fn weight(self) -> i32 {
        match self {
            Kind::Name | Kind::Title => 15,
            Kind::Alias => 14,
            Kind::Tag => 13,
            Kind::Property | Kind::Heading => 12,
            Kind::Path => 10,
        }
    }
}

// Kept out of `Message`'s debug output, which would print the whole vault.
impl std::fmt::Debug for Corpus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Corpus({} notes)", self.notes.len())
    }
}

/// A note that matched: what to show and where Enter goes.
#[derive(Debug)]
pub(crate) struct Found {
    pub path: String,
    pub name: String,
    pub detail: String,
    /// 0-based line to open the note at.
    pub line: Option<usize>,
}

/// Where a note's best match was.
#[derive(Clone, Copy)]
enum Place {
    Field(usize),
    Line(u32),
}

impl Corpus {
    /// Reads every note's metadata and text from the index.
    pub(crate) fn load(db: &Connection) -> rusqlite::Result<Self> {
        let mut corpus = Corpus::default();
        let mut index = HashMap::new();
        let mut stmt = db.prepare(
            "SELECT f.id, f.path, f.title, coalesce(t.text, '')
             FROM files f LEFT JOIN texts t ON t.file_id = f.id WHERE f.ext = 'md'",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        for row in rows {
            let (id, path, title, text): (i64, String, String, String) = row?;
            index.insert(id, corpus.add(path, &title, text));
        }

        type Add<'a> = &'a mut dyn FnMut(&mut Corpus, usize, &rusqlite::Row) -> rusqlite::Result<()>;
        let mut each = |sql: &str, add: Add| -> rusqlite::Result<()> {
            let mut stmt = db.prepare(sql)?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                if let Some(&i) = index.get(&row.get::<_, i64>(0)?) {
                    add(&mut corpus, i, row)?;
                }
            }
            Ok(())
        };
        each("SELECT file_id, alias FROM aliases", &mut |c, i, r| {
            c.field(i, Kind::Alias, r.get(1)?, None);
            Ok(())
        })?;
        each("SELECT file_id, tag, min(line) FROM tags GROUP BY file_id, tag", &mut |c, i, r| {
            let line: i64 = r.get(2)?;
            c.field(i, Kind::Tag, format!("#{}", r.get::<_, String>(1)?), (line > 0).then_some(line as usize));
            Ok(())
        })?;
        // Tags, aliases and the title have fields of their own.
        each(
            "SELECT file_id, key, value FROM frontmatter
             WHERE value IS NOT NULL AND value <> '' AND key NOT IN ('tags', 'tag', 'aliases', 'alias', 'title')",
            &mut |c, i, r| {
                c.field(i, Kind::Property, format!("{}: {}", r.get::<_, String>(1)?, r.get::<_, String>(2)?), None);
                Ok(())
            },
        )?;
        each("SELECT file_id, line, level, text FROM headings", &mut |c, i, r| {
            let (line, level, heading): (i64, i64, String) = (r.get(1)?, r.get(2)?, r.get(3)?);
            c.field(i, Kind::Heading, format!("{} {heading}", "#".repeat(level.clamp(1, 6) as usize)), Some(line as usize));
            Ok(())
        })?;
        Ok(corpus)
    }

    /// Adds a note with its name, title and path; returns its index.
    fn add(&mut self, path: String, title: &str, text: String) -> usize {
        let i = self.notes.len();
        for (line, content) in text.lines().enumerate() {
            for word in words(content) {
                let postings = self.words.entry(word).or_default();
                if postings.last().is_none_or(|&(note, _)| note != i as u32) {
                    postings.push((i as u32, line as u32));
                }
            }
        }
        let name = crate::file_stem(&path).to_owned();
        self.notes.push(Note { path: path.clone(), name: name.clone(), text, fields: Vec::new() });
        self.field(i, Kind::Name, name.clone(), None);
        if !title.is_empty() && title != name {
            self.field(i, Kind::Title, title.to_owned(), None);
        }
        if path.contains('/') {
            self.field(i, Kind::Path, path, None);
        }
        i
    }

    fn field(&mut self, note: usize, kind: Kind, shown: String, line: Option<usize>) {
        let lower = match kind {
            // The `#` and the heading marks are not part of what is searched.
            Kind::Tag | Kind::Heading => shown.trim_start_matches('#').trim_start().to_lowercase(),
            _ => shown.to_lowercase(),
        };
        self.notes[note].fields.push(Field { kind, shown, lower, line });
    }

    /// The notes that match every word of `query`, best first.
    pub(crate) fn search(&self, query: &str, limit: usize) -> Vec<Found> {
        // `#tag` searches for the tag; the `#` is not part of what is indexed.
        let tokens: Vec<String> = query
            .split_whitespace()
            .map(|t| t.trim_start_matches('#').to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.is_empty() {
            return Vec::new();
        }
        let chars: Vec<Vec<char>> = tokens.iter().map(|t| t.chars().collect()).collect();
        let mut scratch = Scratch::default();
        let mut matched = self.matched.lock().unwrap_or_else(|e| e.into_inner());
        let mut last = std::mem::take(&mut *matched);
        for (token, chars) in tokens.iter().zip(&chars) {
            if !matched.contains_key(token) {
                let found = last.remove(token).unwrap_or_else(|| self.text_matches(token, chars, &mut scratch));
                matched.insert(token.clone(), found);
            }
        }
        let in_text: Vec<&Matches> = tokens.iter().map(|t| &matched[t]).collect();

        let mut scored: Vec<(i32, usize, Place)> = Vec::new();
        'notes: for (i, note) in self.notes.iter().enumerate() {
            let mut total = 0;
            let mut best: Option<(i32, Place)> = None;
            for ((token, chars), text) in tokens.iter().zip(&chars).zip(&in_text) {
                let field = note
                    .fields
                    .iter()
                    .enumerate()
                    .filter_map(|(f, field)| {
                        Some((field_score(token, chars, &field.lower, &mut scratch)? * field.kind.weight() / 10, Place::Field(f)))
                    })
                    .max_by_key(|&(score, _)| score);
                let line = text.get(&(i as u32)).map(|&(score, line)| (score, Place::Line(line)));
                let Some((score, place)) = [field, line].into_iter().flatten().max_by_key(|&(score, _)| score) else {
                    continue 'notes;
                };
                total += score;
                if best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, place));
                }
            }
            if let Some((_, place)) = best {
                scored.push((total, i, place));
            }
        }
        // Only the best `limit` are shown, so only they are sorted; the note's
        // index breaks the last ties, as a stable sort of them all would.
        let order = |a: &(i32, usize, Place), b: &(i32, usize, Place)| {
            b.0.cmp(&a.0).then_with(|| self.notes[a.1].path.len().cmp(&self.notes[b.1].path.len())).then(a.1.cmp(&b.1))
        };
        if limit > 0 && scored.len() > limit {
            scored.select_nth_unstable_by(limit - 1, order);
            scored.truncate(limit);
        }
        scored.sort_by(order);
        scored
            .into_iter()
            .take(limit)
            .map(|(_, i, place)| {
                let note = &self.notes[i];
                let (detail, line) = match place {
                    Place::Field(f) => {
                        let field = &note.fields[f];
                        let detail = match field.kind {
                            Kind::Name | Kind::Path => note.path.clone(),
                            Kind::Alias => format!("alias: {}", field.shown),
                            _ => field.shown.clone(),
                        };
                        (detail, field.line)
                    }
                    Place::Line(line) => {
                        let content = note.text.lines().nth(line as usize).unwrap_or_default();
                        (snippet(content, &tokens), Some(line as usize))
                    }
                };
                Found { path: note.path.clone(), name: note.name.clone(), detail: flatten(&detail), line }
            })
            .collect()
    }

    /// The notes whose text has a word matching `token`: best score and its line.
    fn text_matches(&self, token: &str, chars: &[char], scratch: &mut Scratch) -> Matches {
        let mut found: Matches = HashMap::new();
        for (word, postings) in &self.words {
            let Some(score) = word_score(token, chars, word, scratch) else { continue };
            for &(note, line) in postings {
                let entry = found.entry(note).or_insert((score, line));
                if score > entry.0 {
                    *entry = (score, line);
                }
            }
        }
        found
    }
}

/// The lowercase words of a line: runs of letters and digits.
fn words(line: &str) -> impl Iterator<Item = String> + '_ {
    line.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase)
}

/// How well a query word matches a word of a note's text. Exact beats prefix
/// beats substring beats a typo; a word of two letters or less must start the
/// word, as anything shorter matches nearly everything.
/// `chars` is `token` as characters, made once per search rather than per word.
fn word_score(token: &str, chars: &[char], word: &str, scratch: &mut Scratch) -> Option<i32> {
    if word == token {
        return Some(100);
    }
    if word.starts_with(token) {
        return Some(85);
    }
    if chars.len() >= 3 && word.contains(token) {
        return Some(65);
    }
    typo_score(chars, word, scratch)
}

/// A word within one or two edits of `token` (two from nine letters), or a
/// word that starts with such a prefix: what a typo still finds.
fn typo_score(token: &[char], word: &str, scratch: &mut Scratch) -> Option<i32> {
    if token.len() < 4 {
        return None;
    }
    let most = if token.len() >= 9 { 2 } else { 1 };
    // Both kinds of match need at least `token.len() - most` letters, and a
    // word has no more letters than bytes: most of the vocabulary stops here.
    if word.len() + most < token.len() {
        return None;
    }
    // One letter past the longest whole match is enough to tell the two
    // kinds apart, however long the word is.
    scratch.word.clear();
    scratch.word.extend(word.chars().take(token.len() + most + 1));
    let word = &scratch.word;
    if word.len().abs_diff(token.len()) <= most {
        if let Some(d) = distance(token, word, most, &mut scratch.rows) {
            return Some(55 - 15 * d as i32);
        }
    }
    if word.len() > token.len() {
        if let Some(d) = distance(token, &word[..token.len()], most, &mut scratch.rows) {
            return Some(45 - 15 * d as i32);
        }
    }
    None
}

/// Edits between `a` and `b`, a swap of neighbours counting as one, if no
/// more than `most`. Only the last three rows of the table are kept, in
/// `rows`, which a search reuses for every word.
fn distance(a: &[char], b: &[char], most: usize, rows: &mut [Vec<usize>; 3]) -> Option<usize> {
    for row in rows.iter_mut() {
        row.clear();
        row.resize(b.len() + 1, 0);
    }
    for (j, d) in rows[1].iter_mut().enumerate() {
        *d = j;
    }
    for i in 1..=a.len() {
        let [two, one, row] = &mut *rows;
        row[0] = i;
        let mut least = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (one[j] + 1).min(row[j - 1] + 1).min(one[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(two[j - 2] + 1);
            }
            row[j] = d;
            least = least.min(d);
        }
        if least > most {
            return None;
        }
        // The current row becomes the one back, and the oldest is reused next.
        rows.rotate_left(1);
    }
    let d = rows[1][b.len()];
    (d <= most).then_some(d)
}

/// How well a query word matches a short metadata value: whole, start,
/// substring, letters in order close together (`pjal` finds
/// `projects/alpha`), or a word of it with a typo.
/// `chars` is `token` as characters and `field` is already lowercase.
fn field_score(token: &str, chars: &[char], field: &str, scratch: &mut Scratch) -> Option<i32> {
    if field == token {
        return Some(100);
    }
    if field.starts_with(token) {
        return Some(90);
    }
    if let Some(at) = field.find(token) {
        let boundary = !field[..at].chars().next_back().is_some_and(char::is_alphanumeric);
        return Some(if boundary { 80 } else { 70 });
    }
    let letters = subsequence_score(chars, field, &mut scratch.hay);
    let typo = field.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).filter_map(|w| typo_score(chars, w, scratch)).max();
    letters.max(typo)
}

/// The letters of `token` in order in `field`, scored by how tightly they
/// sit and how many start a word. Too loose a match is no match.
fn subsequence_score(token: &[char], field: &str, hay: &mut Vec<char>) -> Option<i32> {
    if token.len() < 2 {
        return None;
    }
    hay.clear();
    hay.extend(field.chars());
    let hay = &*hay;
    let starts_word = |k: usize| k == 0 || !hay[k - 1].is_alphanumeric();
    let mut best: Option<i32> = None;
    // Greedy from every place the first letter occurs; the tightest wins.
    for start in (0..hay.len()).filter(|&k| hay[k] == token[0]) {
        let (mut k, mut gaps, mut bonus) = (start, 0usize, i32::from(starts_word(start)) * 6);
        let mut ok = true;
        for &c in &token[1..] {
            let Some(next) = hay[k + 1..].iter().position(|&h| h == c) else {
                ok = false;
                break;
            };
            gaps += next;
            k += next + 1;
            if starts_word(k) {
                bonus += 6;
            }
        }
        if !ok {
            break;
        }
        if gaps > token.len() * 3 {
            continue;
        }
        let score = 60 - gaps as i32 * 3 + bonus.min(15);
        best = best.max(Some(score));
    }
    best.filter(|&s| s >= 25)
}

/// The part of a line around the first query word in it, for a one-line row.
fn snippet(line: &str, tokens: &[String]) -> String {
    const WIDTH: usize = 90;
    let line = line.trim();
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= WIDTH {
        return line.to_owned();
    }
    let lower = line.to_lowercase();
    let at = tokens
        .iter()
        .filter_map(|t| lower.find(t.as_str()))
        .min()
        .map(|byte| lower[..byte].chars().count())
        .unwrap_or(0);
    let start = at.saturating_sub(WIDTH / 3).min(chars.len() - WIDTH);
    let end = start + WIDTH;
    let mut out: String = chars[start..end].iter().collect();
    if start > 0 {
        out.insert_str(0, "… ");
    }
    if end < chars.len() {
        out.push_str(" …");
    }
    out
}

fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Corpus {
        let mut c = Corpus::default();
        let alpha = c.add(
            "Projects/Alpha.md".into(),
            "Project Alpha",
            "---\nstatus: active\n---\n# Alpha\n\n## Architecture\nThe reconnect logic retries with backoff.\n".into(),
        );
        c.field(alpha, Kind::Tag, "#project/alpha".into(), None);
        c.field(alpha, Kind::Property, "status: active".into(), None);
        c.field(alpha, Kind::Heading, "## Architecture".into(), Some(5));
        let reading = c.add("Reading list.md".into(), "Reading list", "Books about distributed systems.\n".into());
        c.field(reading, Kind::Alias, "Books".into(), None);
        c.add("Meeting notes.md".into(), "Meeting notes", "We talked about the alpha launch.\n".into());
        c
    }

    fn names(found: &[Found]) -> Vec<&str> {
        found.iter().map(|f| f.name.as_str()).collect()
    }

    #[test]
    fn a_name_match_beats_a_mention_in_text() {
        let found = corpus().search("alpha", 10);
        assert_eq!(names(&found), ["Alpha", "Meeting notes"]);
        assert_eq!(found[0].detail, "Projects/Alpha.md");
        assert_eq!(found[1].line, Some(0));
    }

    #[test]
    fn letters_in_order_find_a_path() {
        assert_eq!(names(&corpus().search("pjalp", 10)), ["Alpha"]);
    }

    #[test]
    fn typos_find_text_and_metadata() {
        let found = corpus().search("recconect", 10);
        assert_eq!(names(&found), ["Alpha"]);
        assert_eq!(found[0].line, Some(6));
        assert_eq!(found[0].detail, "The reconnect logic retries with backoff.");
        assert_eq!(names(&corpus().search("distribted", 10)), ["Reading list"]);
        assert_eq!(names(&corpus().search("archtecture", 10))[0], "Alpha");
    }

    #[test]
    fn metadata_is_searched() {
        let c = corpus();
        let found = c.search("#project/alpha", 10);
        assert_eq!((found[0].name.as_str(), found[0].detail.as_str()), ("Alpha", "#project/alpha"));
        let found = c.search("books", 10);
        assert_eq!(found[0].name, "Reading list");
        let found = c.search("architecture", 10);
        assert_eq!((found[0].detail.as_str(), found[0].line), ("## Architecture", Some(5)));
    }

    #[test]
    fn every_word_must_match() {
        let c = corpus();
        assert_eq!(names(&c.search("alpha launch", 10)), ["Meeting notes"]);
        assert_eq!(names(&c.search("active backoff", 10)), ["Alpha"]);
        assert!(c.search("alpha zebra", 10).is_empty());
    }

    #[test]
    fn the_index_is_read_with_its_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(vault.join("Projects")).unwrap();
        std::fs::write(
            vault.join("Projects/Beta.md"),
            "---\ntitle: Beta launch\naliases: [Bravo]\ntags: [launch]\nowner: Dana\n---\n# Plan\n\n## Rollout steps\nShip the #feature flag on Friday.\n",
        )
        .unwrap();
        let db = dir.path().join("index.db");
        let mut ix = sage_indexer::Indexer::open(vault.canonicalize().unwrap(), &db).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        let c = Corpus::load(&sage_db::open_reader(&db).unwrap()).unwrap();

        let first = |q: &str| c.search(q, 5).into_iter().next().map(|f| (f.name, f.detail, f.line));
        let beta = |detail: &str, line: Option<usize>| Some(("Beta".to_owned(), detail.to_owned(), line));
        assert_eq!(first("bravo"), beta("alias: Bravo", None));
        assert_eq!(first("#feature"), beta("#feature", Some(9)));
        // The frontmatter line itself scores as high, and opens on the property.
        assert_eq!(first("dana"), beta("owner: Dana", Some(4)));
        assert_eq!(first("rollout"), beta("## Rollout steps", Some(8)));
        assert_eq!(first("frday"), beta("Ship the #feature flag on Friday.", Some(9)));
        assert_eq!(first("beta launch"), beta("Projects/Beta.md", None));
    }

    #[test]
    fn the_vaults_own_settings_are_never_indexed_or_found() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(vault.join(".sage")).unwrap();
        std::fs::write(vault.join("Note.md"), "Visible text.\n").unwrap();
        std::fs::write(vault.join(".sage/settings.json"), r#"{"quick_add": [{"name": "Zanzibar"}]}"#).unwrap();
        std::fs::write(vault.join(".sage/stray.md"), "Zanzibar\n").unwrap();
        let db = dir.path().join("index.db");
        let mut ix = sage_indexer::Indexer::open(vault.canonicalize().unwrap(), &db).unwrap();
        ix.reconcile(&mut |_| {}).unwrap();
        let reader = sage_db::open_reader(&db).unwrap();

        let paths: Vec<String> =
            reader.prepare("SELECT path FROM files").unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect();
        assert_eq!(paths, ["Note.md"], "the tree and every search read this table");
        let c = Corpus::load(&reader).unwrap();
        assert!(c.search("zanzibar", 5).is_empty());
        assert!(c.search("sage", 5).is_empty());
        assert_eq!(c.search("visible", 5).len(), 1);
    }

    #[test]
    fn short_words_do_not_match_everything() {
        assert!(typo_score(&['a', 'b', 'c'], "abd", &mut Scratch::default()).is_none());
        assert!(subsequence_score(&['a', 'z'], "a very long value without the letter z at all", &mut Vec::new()).is_none());
    }

    /// The whole table, as `distance` first kept it, to check the rolling rows against.
    fn full_distance(a: &[char], b: &[char]) -> usize {
        let mut rows = vec![(0..=b.len()).collect::<Vec<_>>(); a.len() + 1];
        for i in 1..=a.len() {
            rows[i][0] = i;
            for j in 1..=b.len() {
                let cost = usize::from(a[i - 1] != b[j - 1]);
                let mut d = (rows[i - 1][j] + 1).min(rows[i][j - 1] + 1).min(rows[i - 1][j - 1] + cost);
                if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                    d = d.min(rows[i - 2][j - 2] + 1);
                }
                rows[i][j] = d;
            }
        }
        rows[a.len()][b.len()]
    }

    #[test]
    fn rolling_rows_count_the_same_edits_as_the_whole_table() {
        // One set of rows for every pair, as a search reuses them across words of different lengths.
        let mut rows = Default::default();
        let pairs = [
            ("reconnect", "recconect"),
            ("friday", "frday"),
            ("form", "from"),
            ("abcdef", "badcfe"),
            ("über", "ubre"),
            ("", "abc"),
            ("abc", ""),
            ("kitten", "sitting"),
            ("distributed", "distribted"),
        ];
        for (a, b) in pairs {
            let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
            let full = full_distance(&a, &b);
            for most in 0..4 {
                assert_eq!(distance(&a, &b, most, &mut rows), (full <= most).then_some(full), "{a:?} {b:?} {most}");
            }
        }
        assert_eq!(distance(&['f', 'o', 'r', 'm'], &['f', 'r', 'o', 'm'], 1, &mut rows), Some(1), "a swap is one edit");
    }

    #[test]
    fn repeated_searches_reuse_matches_and_still_agree() {
        let c = corpus();
        for q in ["alpha", "alpha lau", "alpha launch", "recconect", "alpha launch", "books"] {
            assert_eq!(names(&c.search(q, 10)), names(&corpus().search(q, 10)), "{q}");
        }
        assert_eq!(c.matched.lock().unwrap().len(), 1, "only the last search's words are kept");
    }

    #[test]
    fn a_limit_keeps_the_best_in_order() {
        let c = corpus();
        let all = c.search("a", 10);
        assert!(all.len() > 1);
        assert_eq!(names(&c.search("a", 1)), names(&all[..1]));
        assert!(c.search("a", 0).is_empty());
    }

    #[test]
    fn long_lines_are_cut_around_the_match() {
        let line = format!("{} needle {}", "x ".repeat(80), "y ".repeat(80));
        let s = snippet(&line, &["needle".into()]);
        assert!(s.contains("needle") && s.starts_with("… ") && s.ends_with(" …"));
    }
}
