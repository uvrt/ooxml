//! The `date` and `date-time` formats a string argument may have, checked as today's
//! validator checks them (Python's `date.fromisoformat` on ten characters, and
//! `datetime.fromisoformat` on a value of at least 19 with `T` at index 10).
//!
//! Owns: those two checks.
//!
//! Must not: parse dates for anything else. Python's `fromisoformat` also takes rarer
//! forms (ISO week dates in a date-time, compact times); those are refused here and noted
//! as the one known difference.

/// Whether `text` is a date, `YYYY-MM-DD` (or the ISO week form `YYYY-Www-D`).
#[must_use]
pub fn is_date(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() != 10 {
        return false;
    }
    calendar_date(&chars) || week_date(&chars)
}

fn digits(chars: &[char]) -> Option<u32> {
    if chars.is_empty() || !chars.iter().all(char::is_ascii_digit) {
        return None;
    }
    chars.iter().try_fold(0u32, |n, c| Some(n * 10 + c.to_digit(10)?))
}

fn calendar_date(chars: &[char]) -> bool {
    let (Some(year), Some('-'), Some(month), Some('-'), Some(day)) = (
        chars.get(0..4).and_then(digits),
        chars.get(4),
        chars.get(5..7).and_then(digits),
        chars.get(7),
        chars.get(8..10).and_then(digits),
    ) else {
        return false;
    };
    year >= 1 && (1..=12).contains(&month) && day >= 1 && day <= days_in(year, month)
}

fn week_date(chars: &[char]) -> bool {
    let (Some(year), Some('-'), Some('W'), Some(week), Some('-'), Some(day)) = (
        chars.get(0..4).and_then(digits),
        chars.get(4),
        chars.get(5),
        chars.get(6..8).and_then(digits),
        chars.get(8),
        chars.get(9..10).and_then(digits),
    ) else {
        return false;
    };
    year >= 1 && (1..=53).contains(&week) && (1..=7).contains(&day)
}

fn days_in(year: u32, month: u32) -> u32 {
    match month {
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Whether `text` is a date-time: `YYYY-MM-DDThh:mm:ss`, an optional fraction, and an
/// optional offset (`Z`, `+hh:mm`, `+hhmm`).
#[must_use]
pub fn is_date_time(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 19 || !matches!(chars.get(10), Some('T' | 't')) {
        return false;
    }
    let (Some(date), Some(time)) = (chars.get(..10), chars.get(11..19)) else {
        return false;
    };
    if !calendar_date(date) || !clock(time) {
        return false;
    }
    let mut rest = chars.get(19..).unwrap_or(&[]);
    if let Some(('.' | ',', tail)) = rest.split_first() {
        let count = tail.iter().take_while(|c| c.is_ascii_digit()).count();
        if count == 0 {
            return false;
        }
        rest = tail.get(count..).unwrap_or(&[]);
    }
    offset(rest)
}

fn clock(time: &[char]) -> bool {
    let (Some(h), Some(':'), Some(m), Some(':'), Some(s)) = (
        time.get(0..2).and_then(digits),
        time.get(2),
        time.get(3..5).and_then(digits),
        time.get(5),
        time.get(6..8).and_then(digits),
    ) else {
        return false;
    };
    h < 24 && m < 60 && s < 60
}

fn offset(rest: &[char]) -> bool {
    match rest {
        [] | ['Z' | 'z'] => true,
        ['+' | '-', h1, h2, ':', m1, m2] | ['+' | '-', h1, h2, m1, m2] => {
            matches!((digits(&[*h1, *h2]), digits(&[*m1, *m2])), (Some(h), Some(m)) if h < 24 && m < 60)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_date_times() {
        assert!(is_date("2026-10-07") && is_date("2024-02-29") && is_date("2026-W01-1"));
        assert!(!is_date("2025-02-29") && !is_date("2026-13-01") && !is_date("20261007"));
        assert!(is_date_time("2026-10-07T09:00:00Z") && is_date_time("2026-10-07t09:00:00.5+02:00"));
        assert!(!is_date_time("2026-10-07 09:00:00") && !is_date_time("2026-10-07T25:00:00"));
    }
}
