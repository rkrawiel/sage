//! Pasted email or meeting notes, read into the parts of an event: what kind
//! it is, its title and date, the people it names and the actions in it.
//!
//! The reading is by headers and keywords only; nothing is guessed from
//! meaning, so a note without them still imports as a plain meeting.

use regex::Regex;
use std::sync::LazyLock;

static ISO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b([0-9]{4})-([0-9]{2})-([0-9]{2})\b").unwrap());
static LONG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b([0-9]{1,2})\s+(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?,?\s+([0-9]{4})\b").unwrap()
});
static ACTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(?:[-*+]\s*)?(?:\[ \]|todo:?|action(?: item)?s?:|ai:|next step:)\s*(.+\S)\s*$").unwrap());

#[derive(Debug, PartialEq)]
pub struct Imported {
    /// `email` or `meeting`.
    pub kind: &'static str,
    pub title: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// The names from `known` the text mentions.
    pub people: Vec<String>,
    pub tasks: Vec<String>,
    pub body: String,
}

/// Reads `text`. `known` are the names of the people notes in the vault.
pub fn parse(text: &str, known: &[String], today: &str) -> Imported {
    let lines: Vec<&str> = text.lines().collect();
    let header = |name: &str| {
        lines.iter().take(15).find_map(|l| {
            let (key, value) = l.split_once(':')?;
            (key.trim().eq_ignore_ascii_case(name) && !value.trim().is_empty()).then(|| value.trim().to_owned())
        })
    };
    let subject = header("subject");
    let email = subject.is_some() && (header("from").is_some() || header("to").is_some());
    let first = lines.iter().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty());
    let title = match (&subject, email) {
        (Some(s), true) => s.trim_start_matches("Re: ").trim_start_matches("RE: ").trim().to_owned(),
        _ => first.map(|f| f.chars().take(80).collect()).unwrap_or_else(|| "Meeting".to_owned()),
    };
    let dated = if email { header("date") } else { None };
    let date = dated
        .as_deref()
        .and_then(find_date)
        .or_else(|| lines.iter().take(5).find_map(|l| find_date(l)))
        .unwrap_or_else(|| today.to_owned());
    let lower = text.to_lowercase();
    let people = known
        .iter()
        .filter(|name| {
            let name = name.to_lowercase();
            lower.match_indices(&name).any(|(at, _)| {
                let before = lower[..at].chars().next_back();
                let after = lower[at + name.len()..].chars().next();
                !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
            })
        })
        .cloned()
        .collect();
    let tasks = lines.iter().filter_map(|l| ACTION.captures(l).map(|c| c[1].to_owned())).collect();
    Imported { kind: if email { "email" } else { "meeting" }, title, date, people, tasks, body: text.trim().to_owned() }
}

fn find_date(s: &str) -> Option<String> {
    if let Some(c) = ISO.captures(s) {
        return Some(format!("{}-{}-{}", &c[1], &c[2], &c[3]));
    }
    let c = LONG.captures(s)?;
    let month = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        .iter()
        .position(|m| c[2].eq_ignore_ascii_case(m))?;
    Some(format!("{}-{:02}-{:02}", &c[3], month + 1, c[1].parse::<u32>().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_email_is_read_by_its_headers() {
        let mail = "From: Anna Berg <a@x.com>\nTo: me\nDate: Tue, 25 Nov 2026 10:00:00\nSubject: Re: Contract terms\n\nHi,\nAction: send the draft\nThanks, Anna Berg";
        let got = parse(mail, &["Anna Berg".into(), "Bo".into()], "2026-12-01");
        assert_eq!((got.kind, got.title.as_str(), got.date.as_str()), ("email", "Contract terms", "2026-11-25"));
        assert_eq!(got.people, ["Anna Berg"]);
        assert_eq!(got.tasks, ["send the draft"]);
    }

    #[test]
    fn notes_are_a_meeting_with_their_actions() {
        let got = parse("# Weekly sync 2026-11-20\n- [ ] book room\nTODO: ping Bo\nBook", &["Bo".into(), "Boo".into()], "2026-12-01");
        assert_eq!((got.kind, got.title.as_str(), got.date.as_str()), ("meeting", "Weekly sync 2026-11-20", "2026-11-20"));
        assert_eq!(got.tasks, ["book room", "ping Bo"]);
        assert_eq!(got.people, ["Bo"]);
        assert_eq!(parse("", &[], "2026-12-01").date, "2026-12-01");
    }

    #[test]
    fn non_ascii_digits_are_not_read_as_a_date() {
        for text in ["Sync ２０２６-11-26", "Sync ٢٠٢٦-١١-٢٦", "Sync 26 Nov ٢٠٢٦"] {
            assert_eq!(parse(text, &[], "2026-12-01").date, "2026-12-01");
        }
    }
}
