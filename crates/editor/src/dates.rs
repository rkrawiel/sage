//! Natural-language dates for the `@` popup: words such as `tomorrow`,
//! `friday` or `in 2 weeks` and the day each one means.
//!
//! Pure functions of the query and today's date, so they are tested here
//! without a UI. A weekday means its next occurrence, never today.

use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phrase {
    pub label: String,
    pub date: NaiveDate,
}

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("monday", Weekday::Mon),
    ("tuesday", Weekday::Tue),
    ("wednesday", Weekday::Wed),
    ("thursday", Weekday::Thu),
    ("friday", Weekday::Fri),
    ("saturday", Weekday::Sat),
    ("sunday", Weekday::Sun),
];

/// The phrases that begin with `query`, then a counted one such as `in 3 days`
/// when the query is one. An empty query lists the common ones.
pub fn suggest(query: &str, today: NaiveDate) -> Vec<Phrase> {
    let query = query.trim().to_lowercase();
    let mut all = vec![
        Phrase { label: "today".into(), date: today },
        Phrase { label: "tomorrow".into(), date: today + Duration::days(1) },
        Phrase { label: "yesterday".into(), date: today - Duration::days(1) },
        Phrase { label: "next week".into(), date: today + Duration::days(7) },
    ];
    if let Some(date) = today.checked_add_months(Months::new(1)) {
        all.push(Phrase { label: "next month".into(), date });
    }
    let mut days: Vec<Phrase> = WEEKDAYS
        .iter()
        .map(|(name, weekday)| {
            let ahead = (weekday.num_days_from_monday() + 6 - today.weekday().num_days_from_monday()) % 7 + 1;
            Phrase { label: (*name).to_owned(), date: today + Duration::days(i64::from(ahead)) }
        })
        .collect();
    days.sort_by_key(|p| p.date);
    all.extend(days);

    let mut found: Vec<Phrase> = all.into_iter().filter(|p| p.label.starts_with(&query)).collect();
    found.extend(counted(&query, today));
    found
}

/// `in 3 days`, `2w`, `in 1 month`: a number and the start of a unit.
fn counted(query: &str, today: NaiveDate) -> Option<Phrase> {
    let rest = query.strip_prefix("in ").unwrap_or(query).trim_start();
    let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    let n: u32 = rest[..digits].parse().ok().filter(|n| (1..=3650).contains(n))?;
    let unit = rest[digits..].trim();
    if unit.is_empty() {
        return None;
    }
    let (singular, plural) = ["day", "week", "month"]
        .iter()
        .map(|s| (*s, format!("{s}s")))
        .find(|(_, plural)| plural.starts_with(unit))?;
    let date = match singular {
        "day" => today.checked_add_signed(Duration::days(i64::from(n)))?,
        "week" => today.checked_add_signed(Duration::weeks(i64::from(n)))?,
        _ => today.checked_add_months(Months::new(n))?,
    };
    let name = if n == 1 { singular.to_owned() } else { plural };
    Some(Phrase { label: format!("in {n} {name}"), date })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A Tuesday.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 24).unwrap()
    }

    fn first(query: &str) -> Option<(String, String)> {
        suggest(query, today()).into_iter().next().map(|p| (p.label, p.date.to_string()))
    }

    #[test]
    fn common_words_and_weekdays_are_found_by_prefix() {
        assert_eq!(first(""), Some(("today".into(), "2026-11-24".into())));
        assert_eq!(first("tom"), Some(("tomorrow".into(), "2026-11-25".into())));
        assert_eq!(first("next w"), Some(("next week".into(), "2026-12-01".into())));
        assert_eq!(first("fri"), Some(("friday".into(), "2026-11-27".into())));
        assert_eq!(first("tuesday"), Some(("tuesday".into(), "2026-12-01".into())), "a weekday is never today");
        assert_eq!(first("zzz"), None);
    }

    #[test]
    fn counted_dates_take_days_weeks_and_months() {
        assert_eq!(first("in 3 days"), Some(("in 3 days".into(), "2026-11-27".into())));
        assert_eq!(first("2w"), Some(("in 2 weeks".into(), "2026-12-08".into())));
        assert_eq!(first("in 1 mo"), Some(("in 1 month".into(), "2026-12-24".into())));
        assert_eq!(first("in 3"), None, "a number alone is not a date");
    }
}
