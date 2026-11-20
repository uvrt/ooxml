//! Owns: a 1900-system date serial written as ISO 8601, as ooxml-edit's `from_serial` and
//! Python's `isoformat()` write it: `2026-01-05` for a whole day, `2026-01-05T12:00:00`
//! (with `.ffffff` where there are microseconds) for a time of day, the fraction rounded to
//! microseconds as `datetime.timedelta(days=...)` rounds it.
//!
//! Must not: read a format code (the caller decides the serial is a date).

use ooxml_numfmt::date_of_serial;

/// The serial `number` as an ISO date or date and time; `None` before 1 March 1900 (61) or
/// past the year 9999.
#[must_use]
pub fn iso_of_serial(number: f64) -> Option<String> {
    if !number.is_finite() || number < 61.0 {
        return None;
    }
    // timedelta(days=d): the whole days, the day's fraction's whole seconds, and its
    // remaining fraction rounded half to even to microseconds, carried upward.
    let days = number.trunc();
    let seconds = number.fract() * 86_400.0;
    let mut whole_seconds = seconds.trunc();
    let mut micros = ((seconds - whole_seconds) * 1e6).round_ties_even();
    let mut days = days;
    if micros >= 1e6 {
        micros -= 1e6;
        whole_seconds += 1.0;
    }
    if whole_seconds >= 86_400.0 {
        whole_seconds -= 86_400.0;
        days += 1.0;
    }
    let date = date_of_serial(days, false)?;
    if date.year > 9999 {
        return None;
    }
    let day = format!("{:04}-{:02}-{:02}", date.year, date.month, date.day);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (seconds, micros) = (whole_seconds as u64, micros as u64);
    if seconds == 0 && micros == 0 {
        return Some(day);
    }
    let time = format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60);
    Some(if micros == 0 {
        format!("{day}T{time}")
    } else {
        format!("{day}T{time}.{micros:06}")
    })
}

#[cfg(test)]
mod tests {
    use super::iso_of_serial;

    #[test]
    fn serials_read_as_python_writes_them() {
        assert_eq!(iso_of_serial(46027.0).as_deref(), Some("2026-01-05"));
        assert_eq!(iso_of_serial(46027.5).as_deref(), Some("2026-01-05T12:00:00"));
        assert_eq!(iso_of_serial(61.0).as_deref(), Some("1900-03-01"));
        // what Python's datetime(1899, 12, 30) + timedelta(days=...) writes
        assert_eq!(
            iso_of_serial(46027.123456789).as_deref(),
            Some("2026-01-05T02:57:46.666569")
        );
        assert_eq!(
            iso_of_serial(46027.99999999999).as_deref(),
            Some("2026-01-05T23:59:59.999999")
        );
        assert_eq!(
            iso_of_serial(61.000005787037).as_deref(),
            Some("1900-03-01T00:00:00.500000")
        );
        assert_eq!(iso_of_serial(60.0), None);
        assert_eq!(iso_of_serial(3e6), None);
    }
}
