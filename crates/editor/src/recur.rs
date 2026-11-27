//! Recurring tasks: the next occurrence of a `🔁 every …` task, which the
//! Tasks plugin inserts above a recurring task when it is checked.
//!
//! The rule is read from `🔁 every week` or a Dataview `[repeat:: every week]`
//! field. The next date follows from the due date, else the scheduled date,
//! else the start date, or from today for a rule ending in `when done`; every
//! date on the line, a reminder's `(@2026-12-01 09:30)` included, moves by the
//! same number of days, so the next occurrence does not remind at once.

use chrono::{Datelike, Days, Months, NaiveDate, TimeDelta, Weekday};
use regex::{Captures, Regex};
use std::sync::LazyLock;

/// The rule after `🔁`, up to the next signifier or block id.
static RULE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"🔁\s*([^📅⏳🛫✅➕🔺⏫🔼🔽⏬\[\(\^]+)|[\[\(]repeat::\s*([^\]\)]+)[\]\)]").unwrap()
});
/// A date the next occurrence moves, in either format.
static DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"((?:📅|⏳|🛫)\s*|[\[\(](?:due|scheduled|start)::\s*|\(@)(\d{4}-\d{2}-\d{2})").unwrap()
});
/// A completion date, with the space before it.
static DONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s*(?:✅\s*\d{4}-\d{2}-\d{2}|[\[\(](?:completion|done)::[^\]\)]*[\]\)])").unwrap()
});
/// A trailing block id, which must stay unique to the line it marks.
static BLOCK_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s\^[A-Za-z0-9\-]+\s*$").unwrap());

#[derive(Debug, Clone, PartialEq)]
enum Every {
    Days(u32),
    /// Every n weeks, on these days, or on the same day when empty.
    Weeks(u32, Vec<Weekday>),
    /// Monday to Friday.
    Weekdays,
    Months(u32, Option<MonthDay>),
    Years(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum MonthDay {
    Nth(u32),
    Last,
}

#[derive(Debug, Clone, PartialEq)]
struct Rule {
    every: Every,
    when_done: bool,
}

/// The line of the next occurrence of the open recurring task `line`, as it
/// is about to be checked on `today`: open, its dates moved on, without a
/// completion date or block id. `None` for a line that is no open task or has
/// no rule Sage understands; checking it then just checks it.
pub fn next_occurrence(line: &str, today: NaiveDate) -> Option<String> {
    let task = sage_core::parse(line).tasks.into_iter().next()?;
    if matches!(task.state, 'x' | 'X' | '-') {
        return None;
    }
    let caps = RULE.captures(&task.text)?;
    let rule = parse_rule(caps.get(1).or_else(|| caps.get(2))?.as_str())?;
    let date = |d: &Option<String>| d.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    // A task without dates comes back without dates.
    let shift = match date(&task.due).or(date(&task.scheduled)).or(date(&task.start)) {
        Some(reference) => next(&rule.every, if rule.when_done { today } else { reference })? - reference,
        None => TimeDelta::zero(),
    };

    let moved = DATE.replace_all(line, |c: &Captures| match NaiveDate::parse_from_str(&c[2], "%Y-%m-%d") {
        Ok(d) => format!("{}{}", &c[1], (d + shift).format("%Y-%m-%d")),
        Err(_) => c[0].to_owned(),
    });
    let open = DONE.replace_all(&moved, "");
    let mut out = BLOCK_ID.replace(&open, "").trim_end().to_owned();
    // The checkbox is the first `[` on a task line: back to open.
    let at = out.find('[')? + 1;
    out.replace_range(at..at + task.state.len_utf8(), " ");
    Some(out)
}

/// `every 2 weeks on Monday, Friday`, `every month on the last`, `every
/// weekday when done`, and the other forms the Tasks plugin writes most.
fn parse_rule(text: &str) -> Option<Rule> {
    let text = text.trim().to_lowercase();
    let (text, when_done) = match text.strip_suffix("when done") {
        Some(rest) => (rest.trim_end().to_owned(), true),
        None => (text, false),
    };
    let rest = text.strip_prefix("every")?;
    let mut words: Vec<&str> = rest.split([' ', '\t', ',']).filter(|w| !w.is_empty() && *w != "and").collect();
    let n = match words.first().map(|w| (*w, w.parse::<u32>())) {
        Some((_, Ok(n))) => {
            words.remove(0);
            n
        }
        Some(("other", _)) => {
            words.remove(0);
            2
        }
        _ => 1,
    };
    if n == 0 {
        return None;
    }
    let every = match words.as_slice() {
        ["day" | "days"] => Every::Days(n),
        ["weekday"] if n == 1 => Every::Weekdays,
        ["week" | "weeks"] => Every::Weeks(n, Vec::new()),
        ["week" | "weeks", "on", days @ ..] => Every::Weeks(n, weekdays(days)?),
        ["month" | "months"] => Every::Months(n, None),
        ["month" | "months", "on", "the", day] => Every::Months(n, Some(month_day(day)?)),
        ["year" | "years"] => Every::Years(n),
        days if n == 1 => Every::Weeks(1, weekdays(days)?),
        _ => return None,
    };
    Some(Rule { every, when_done })
}

fn weekdays(words: &[&str]) -> Option<Vec<Weekday>> {
    if words.is_empty() {
        return None;
    }
    words
        .iter()
        .map(|w| {
            Some(match w.trim_end_matches('s') {
                "monday" | "mon" => Weekday::Mon,
                "tuesday" | "tue" => Weekday::Tue,
                "wednesday" | "wed" => Weekday::Wed,
                "thursday" | "thu" => Weekday::Thu,
                "friday" | "fri" => Weekday::Fri,
                "saturday" | "sat" => Weekday::Sat,
                "sunday" | "sun" => Weekday::Sun,
                _ => return None,
            })
        })
        .collect()
}

/// `15th`, `1st`, `15` or `last`.
fn month_day(word: &str) -> Option<MonthDay> {
    if word == "last" {
        return Some(MonthDay::Last);
    }
    let digits = word.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    digits.parse().ok().filter(|d| (1..=31).contains(d)).map(MonthDay::Nth)
}

/// The first date after `from` that the rule gives.
fn next(every: &Every, from: NaiveDate) -> Option<NaiveDate> {
    match every {
        Every::Days(n) => from.checked_add_days(Days::new(u64::from(*n))),
        Every::Weekdays => (1..=3)
            .filter_map(|d| from.checked_add_days(Days::new(d)))
            .find(|d| !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)),
        Every::Weeks(n, days) if days.is_empty() => from.checked_add_days(Days::new(7 * u64::from(*n))),
        Every::Weeks(n, days) => {
            // Later in the same week, else the first wanted day n weeks on.
            let monday = from - Days::new(u64::from(from.weekday().num_days_from_monday()));
            let later = (1..7)
                .filter_map(|d| from.checked_add_days(Days::new(d)))
                .take_while(|d| *d < monday + Days::new(7))
                .find(|d| days.contains(&d.weekday()));
            later.or_else(|| {
                let week = monday.checked_add_days(Days::new(7 * u64::from(*n)))?;
                (0..7).map(|d| week + Days::new(d)).find(|d| days.contains(&d.weekday()))
            })
        }
        Every::Months(n, None) => from.checked_add_months(Months::new(*n)),
        Every::Months(n, Some(day)) => {
            // This month if the day is still to come, else every n months on;
            // a month without that day, like a 31st, is skipped.
            let first = from.with_day(1)?;
            (0..=12u32).find_map(|k| {
                let month = first.checked_add_months(Months::new(k * n))?;
                let date = match day {
                    MonthDay::Nth(d) => month.with_day(*d)?,
                    MonthDay::Last => month.checked_add_months(Months::new(1))?.pred_opt()?,
                };
                (date > from).then_some(date)
            })
        }
        Every::Years(n) => from.checked_add_months(Months::new(12 * n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn after(rule: &str, from: &str) -> String {
        let rule = parse_rule(rule).unwrap_or_else(|| panic!("{rule}"));
        next(&rule.every, d(from)).unwrap().to_string()
    }

    #[test]
    fn rules() {
        // 2026-11-13 is a Friday.
        assert_eq!(after("every day", "2026-11-13"), "2026-11-14");
        assert_eq!(after("every 3 days", "2026-11-13"), "2026-11-16");
        assert_eq!(after("every other day", "2026-11-13"), "2026-11-15");
        assert_eq!(after("every weekday", "2026-11-13"), "2026-11-16");
        assert_eq!(after("every week", "2026-11-13"), "2026-11-20");
        assert_eq!(after("every 2 weeks", "2026-11-13"), "2026-11-27");
        assert_eq!(after("every week on Monday, Friday", "2026-11-13"), "2026-11-16");
        assert_eq!(after("every week on monday and wednesday", "2026-11-09"), "2026-11-11");
        assert_eq!(after("every 2 weeks on Tuesday", "2026-11-13"), "2026-11-24");
        assert_eq!(after("every Saturday", "2026-11-13"), "2026-11-14");
        assert_eq!(after("every month", "2026-01-31"), "2026-02-28");
        assert_eq!(after("every month on the 15th", "2026-11-10"), "2026-11-15");
        assert_eq!(after("every month on the 15th", "2026-11-15"), "2026-12-15");
        assert_eq!(after("every month on the 31st", "2026-01-31"), "2026-03-31");
        assert_eq!(after("every month on the last", "2026-02-10"), "2026-02-28");
        assert_eq!(after("every 3 months", "2026-11-13"), "2027-02-13");
        assert_eq!(after("every year", "2028-02-29"), "2029-02-28");
        assert!(parse_rule("every week when done").unwrap().when_done);
        for bad in ["weekly", "every", "every 0 days", "every fortnight", "every week on payday", "every 2 Mondays"] {
            assert_eq!(parse_rule(bad), None, "{bad}");
        }
    }

    #[test]
    fn next_occurrence_moves_every_date() {
        let today = d("2026-11-16");
        assert_eq!(
            next_occurrence("- [ ] Call mum 🔁 every week 📅 2026-11-13 (@2026-11-13 18:30)", today).as_deref(),
            Some("- [ ] Call mum 🔁 every week 📅 2026-11-20 (@2026-11-20 18:30)"),
            "the reminder moves with the due date and keeps its time"
        );
        assert_eq!(
            next_occurrence("- [ ] Water plants 🔁 every week 🛫 2026-11-10 ⏳ 2026-11-12 📅 2026-11-13 ^w1", today).as_deref(),
            Some("- [ ] Water plants 🔁 every week 🛫 2026-11-17 ⏳ 2026-11-19 📅 2026-11-20")
        );
        assert_eq!(
            next_occurrence("  * [/] Pay rent [repeat:: every month on the 1st] [due:: 2026-11-01] ⏫", today).as_deref(),
            Some("  * [ ] Pay rent [repeat:: every month on the 1st] [due:: 2026-12-01] ⏫"),
        );
        assert_eq!(
            next_occurrence("- [ ] Review 🔁 every 2 days when done 📅 2026-11-01", today).as_deref(),
            Some("- [ ] Review 🔁 every 2 days when done 📅 2026-11-18"),
            "from today, not from the due date"
        );
        assert_eq!(
            next_occurrence("- [ ] Stretch 🔁 every day", today).as_deref(),
            Some("- [ ] Stretch 🔁 every day"),
            "no dates, no dates"
        );
        assert_eq!(next_occurrence("- [x] Done 🔁 every day 📅 2026-11-01 ✅ 2026-11-02", today), None);
        assert_eq!(next_occurrence("- [ ] Odd 🔁 every blue moon 📅 2026-11-01", today), None);
        assert_eq!(next_occurrence("- [ ] Plain 📅 2026-11-01", today), None);
    }
}
