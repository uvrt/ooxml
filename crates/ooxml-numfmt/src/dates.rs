//! Owns: a date serial as a calendar date (the 1900 system with its phantom 29 February, and
//! the 1904 system) and written through a date code -- the code's day, month and year
//! tokens, its literals, and the month and day names of the language it states, else the
//! locale's -- equal to ooxml-common's `dates.py`. Office's own names per language are a
//! table here; a code with a time, an elapsed time or a token no name table covers writes
//! nothing (`None`), and the caller falls back to a number.
//!
//! Must not: write a time of day, or guess a language the tables do not hold.

use crate::code::{is_date_code, sections, tokens, TokenKind};
use crate::locale::{canonical_language, Locale};

/// A language's month and day names, Monday first.
struct Names {
    months: [&'static str; 12],
    short_months: [&'static str; 12],
    days: [&'static str; 7],
    short_days: [&'static str; 7],
}

const ENGLISH: Names = Names {
    months: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    short_months: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    days: [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ],
    short_days: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
};
const DUTCH: Names = Names {
    months: [
        "januari",
        "februari",
        "maart",
        "april",
        "mei",
        "juni",
        "juli",
        "augustus",
        "september",
        "oktober",
        "november",
        "december",
    ],
    short_months: [
        "jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec",
    ],
    days: [
        "maandag",
        "dinsdag",
        "woensdag",
        "donderdag",
        "vrijdag",
        "zaterdag",
        "zondag",
    ],
    short_days: ["ma", "di", "wo", "do", "vr", "za", "zo"],
};
const GERMAN: Names = Names {
    months: [
        "Januar",
        "Februar",
        "März",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ],
    short_months: [
        "Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
    ],
    days: [
        "Montag",
        "Dienstag",
        "Mittwoch",
        "Donnerstag",
        "Freitag",
        "Samstag",
        "Sonntag",
    ],
    short_days: ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"],
};
const FRENCH_MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];
const FRENCH_DAYS: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
const FRENCH_SHORT_DAYS: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];
const FRENCH: Names = Names {
    months: FRENCH_MONTHS,
    short_months: [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc.",
    ],
    days: FRENCH_DAYS,
    short_days: FRENCH_SHORT_DAYS,
};
const SPANISH_MONTHS: [&str; 12] = [
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];
const SPANISH_DAYS: [&str; 7] = ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"];
const SPANISH: Names = Names {
    months: SPANISH_MONTHS,
    short_months: [
        "ene.", "feb.", "mar.", "abr.", "may.", "jun.", "jul.", "ago.", "sep.", "oct.", "nov.", "dic.",
    ],
    days: SPANISH_DAYS,
    short_days: ["lu.", "ma.", "mi.", "ju.", "vi.", "sá.", "do."],
};
const SPANISH_MEXICO: Names = Names {
    short_days: ["lun.", "mar.", "mié.", "jue.", "vie.", "sáb.", "dom."],
    ..SPANISH
};
const SPANISH_SHORT_DAYS: [&str; 7] = ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"];
const SPANISH_ANY: Names = Names {
    short_months: [
        "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "set", "oct", "nov", "dic",
    ],
    short_days: SPANISH_SHORT_DAYS,
    ..SPANISH
};
const ITALIAN: Names = Names {
    months: [
        "gennaio",
        "febbraio",
        "marzo",
        "aprile",
        "maggio",
        "giugno",
        "luglio",
        "agosto",
        "settembre",
        "ottobre",
        "novembre",
        "dicembre",
    ],
    short_months: [
        "gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic",
    ],
    days: [
        "lunedì",
        "martedì",
        "mercoledì",
        "giovedì",
        "venerdì",
        "sabato",
        "domenica",
    ],
    short_days: ["lun", "mar", "mer", "gio", "ven", "sab", "dom"],
};
/// PowerPoint's own French names where a code states no language.
const FRENCH_INTERFACE: Names = Names {
    short_months: [
        "janv", "févr", "mars", "avr", "mai", "juin", "juil", "août", "sept", "oct", "nov", "déc",
    ],
    ..FRENCH
};
const SPANISH_INTERFACE: Names = Names {
    short_months: [
        "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic",
    ],
    short_days: SPANISH_SHORT_DAYS,
    ..SPANISH
};

/// The name tables by language tag.
fn names(tag: &str) -> Option<&'static Names> {
    Some(match tag {
        "en-US" | "en-GB" => &ENGLISH,
        "nl-NL" | "nl-BE" => &DUTCH,
        "de-DE" => &GERMAN,
        "fr-FR" => &FRENCH,
        "es-ES" => &SPANISH,
        "it-IT" => &ITALIAN,
        "es-MX" => &SPANISH_MEXICO,
        "es" => &SPANISH_ANY,
        _ => return None,
    })
}

/// A primary language's default tag.
fn default_tag(primary: &str) -> Option<&'static str> {
    Some(match primary {
        "en" => "en-US",
        "nl" => "nl-NL",
        "de" => "de-DE",
        "fr" => "fr-FR",
        "it" => "it-IT",
        _ => return None,
    })
}

/// The locale ids a code may state, by tag.
const LCIDS: [(&str, i64); 19] = [
    ("en-US", 0x409),
    ("en-GB", 0x809),
    ("nl-NL", 0x413),
    ("nl-BE", 0x813),
    ("de-DE", 0x407),
    ("fr-FR", 0x40C),
    ("es-ES", 0xC0A),
    ("it-IT", 0x410),
    ("en-IE", 0x1809),
    ("fr-BE", 0x80C),
    ("fr-CH", 0x100C),
    ("en", 0x9),
    ("nl", 0x13),
    ("de", 0x7),
    ("fr", 0xC),
    ("es-MX", 0x80A),
    ("it-CH", 0x810),
    ("es", 0xA),
    ("it", 0x10),
];

/// The tag whose names a language is written with.
fn names_tag(language: &str) -> Option<&'static str> {
    const TAGS: [&str; 10] = [
        "en-US", "en-GB", "nl-NL", "nl-BE", "de-DE", "fr-FR", "es-ES", "it-IT", "es-MX", "es",
    ];
    TAGS.iter()
        .copied()
        .find(|t| *t == language)
        .or_else(|| default_tag(language.split('-').next().unwrap_or("")))
}

/// Python's `int(text, 16)`: surrounding white space, a sign and a `0x` prefix allowed.
fn hexadecimal(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let body = body
        .strip_prefix("0x")
        .or_else(|| body.strip_prefix("0X"))
        .unwrap_or(body);
    if body.is_empty() || body.starts_with('_') || body.ends_with('_') || body.contains("__") {
        return None;
    }
    let value = i64::from_str_radix(&body.replace('_', ""), 16).ok()?;
    Some(if negative { -value } else { value })
}

/// A date code's own token kinds (ooxml-common's `dates._tokens`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Literal(String),
    Locale(String),
    Token(String),
    Time,
    Other,
}

fn parts(section: &str) -> Vec<Part> {
    let mut out = Vec::new();
    for token in tokens(section) {
        let text = token.text;
        match token.kind {
            TokenKind::Quoted | TokenKind::Escaped | TokenKind::Literal => out.push(Part::Literal(text)),
            TokenKind::Pad => out.push(Part::Literal(" ".into())),
            TokenKind::Locale => {
                let (symbol, language) = text.split_once('-').unwrap_or((&text, ""));
                if !symbol.is_empty() {
                    out.push(Part::Literal(symbol.into()));
                }
                if !language.is_empty() {
                    out.push(Part::Locale(language.into()));
                }
            }
            TokenKind::Date | TokenKind::Time => {
                let letter = text
                    .chars()
                    .next()
                    .map(|c| c.to_lowercase().to_string())
                    .unwrap_or_default();
                if matches!(letter.as_str(), "d" | "m" | "y") {
                    out.push(Part::Token(text.to_lowercase()));
                } else if matches!(letter.as_str(), "h" | "s") {
                    out.push(Part::Time);
                } else {
                    out.push(Part::Other);
                }
            }
            TokenKind::Elapsed | TokenKind::AmPm => out.push(Part::Time),
            TokenKind::Condition => out.push(Part::Other),
            TokenKind::Bracket => {
                let alpha = !text.is_empty() && text.chars().all(char::is_alphabetic);
                if !(alpha || text.to_lowercase().starts_with("color")) {
                    out.push(Part::Other);
                }
            }
            TokenKind::Fill | TokenKind::Colour => {}
            _ => out.push(Part::Other),
        }
    }
    out
}

/// The language a code states (`[$-413]`, `[$-nl-NL]`), where the tables know it.
fn language_of_code(code: &str) -> Option<String> {
    for section in sections(code) {
        for part in parts(&section) {
            let Part::Locale(text) = part else { continue };
            let stated = hexadecimal(&text)
                .and_then(|n| LCIDS.iter().find(|(_, id)| *id == n & 0xFFFF))
                .map(|(t, _)| (*t).to_owned());
            if stated.is_some() {
                return stated;
            }
            let tag = canonical_language(&text).ok()?;
            let known = LCIDS.iter().any(|(t, _)| *t == tag) || names_tag(&tag).is_some();
            return known.then_some(tag);
        }
    }
    None
}

fn date_names(code: &str, locale: &Locale) -> &'static Names {
    let stated = language_of_code(code);
    if stated.as_deref().and_then(names_tag).is_none() {
        match locale.primary_language() {
            "fr" => return &FRENCH_INTERFACE,
            "es" => return &SPANISH_INTERFACE,
            "it" => return &ITALIAN,
            _ => {}
        }
    }
    let shown = stated
        .as_deref()
        .and_then(names_tag)
        .or_else(|| names_tag(&locale.language))
        .unwrap_or("en-US");
    names(shown).unwrap_or(&ENGLISH)
}

/// The month and day names a date pattern writes in a language (ooxml-common's
/// `dates.NAMES`, measured): January first, Monday first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateNames {
    /// The months' names.
    pub months: &'static [&'static str; 12],
    /// Their abbreviations.
    pub short_months: &'static [&'static str; 12],
    /// The weekdays' names.
    pub days: &'static [&'static str; 7],
    /// Their abbreviations.
    pub short_days: &'static [&'static str; 7],
}

/// The measured names `language` (a canonical tag) shows (ooxml-common's `names_for`): its
/// own tag's where the tables hold it, else its language's in any region (fr-BE shows
/// fr-FR's); `None` for a language with none measured.
#[must_use]
pub fn measured_names(language: &str) -> Option<DateNames> {
    let found = names(names_tag(language)?)?;
    Some(DateNames {
        months: &found.months,
        short_months: &found.short_months,
        days: &found.days,
        short_days: &found.short_days,
    })
}

/// A calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    /// The year.
    pub year: i64,
    /// The month, 1-12.
    pub month: u32,
    /// The day, 1-31.
    pub day: u32,
    /// Monday 0 to Sunday 6.
    pub weekday: usize,
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> Date {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    // 1970-01-01 was a Thursday (3).
    let weekday = usize::try_from((days + 3).rem_euclid(7)).unwrap_or(0);
    Date {
        year,
        month: u32::try_from(month).unwrap_or(1),
        day: u32::try_from(day).unwrap_or(1),
        weekday,
    }
}

/// The calendar date of a serial: `None` for a negative, unrepresentable or phantom one.
pub fn date_of_serial(value: f64, date1904: bool) -> Option<Date> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let whole = value.floor();
    let epoch = if date1904 {
        days_from_civil(1904, 1, 1)
    } else if whole >= 61.0 {
        days_from_civil(1899, 12, 30)
    } else if (1.0..=59.0).contains(&whole) {
        days_from_civil(1899, 12, 31)
    } else {
        return None;
    };
    let last = days_from_civil(9999, 12, 31);
    #[allow(clippy::cast_precision_loss)]
    let room = (last - epoch) as f64;
    if whole > room {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let days = whole as i64;
    Some(civil_from_days(epoch + days))
}

/// A serial written through a date code's day, month and year tokens; `None` where the code
/// shows no date, or shows a time or anything the tables cannot write.
pub fn format_serial(value: f64, code: Option<&str>, locale: &Locale, date1904: bool) -> Option<String> {
    let day = date_of_serial(value, date1904)?;
    let code = code.filter(|c| !c.is_empty())?;
    let all = sections(code);
    let index = if value == 0.0 && all.len() > 2 { 2 } else { 0 };
    if !is_date_code(Some(code), Some(index)) {
        return None;
    }
    let found = parts(all.get(index)?);
    let printed = [
        "d", "dd", "ddd", "dddd", "m", "mm", "mmm", "mmmm", "mmmmm", "yy", "yyyy",
    ];
    if found.is_empty() || found.iter().any(|p| matches!(p, Part::Time | Part::Other)) {
        return None;
    }
    if found
        .iter()
        .any(|p| matches!(p, Part::Token(t) if !printed.contains(&t.as_str())))
    {
        return None;
    }
    if !found.iter().any(|p| matches!(p, Part::Token(_))) {
        return None;
    }
    let names = date_names(code, locale);
    let month = usize::try_from(day.month).unwrap_or(1).saturating_sub(1);
    let mut out = String::new();
    for part in found {
        match part {
            Part::Literal(text) => out.push_str(&text),
            Part::Token(text) => out.push_str(&match text.as_str() {
                "d" => day.day.to_string(),
                "dd" => format!("{:02}", day.day),
                "ddd" => names.short_days.get(day.weekday).copied().unwrap_or("").to_owned(),
                "dddd" => names.days.get(day.weekday).copied().unwrap_or("").to_owned(),
                "m" => day.month.to_string(),
                "mm" => format!("{:02}", day.month),
                "mmm" => names.short_months.get(month).copied().unwrap_or("").to_owned(),
                "mmmm" => names.months.get(month).copied().unwrap_or("").to_owned(),
                "mmmmm" => names
                    .months
                    .get(month)
                    .and_then(|m| m.chars().next())
                    .map(String::from)
                    .unwrap_or_default(),
                "yy" => format!("{:02}", day.year % 100),
                _ => format!("{:04}", day.year),
            }),
            _ => {}
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{date_of_serial, format_serial};
    use crate::locale::{Locale, EN_US};

    #[test]
    fn serials_write_as_office_writes_them() {
        let day = date_of_serial(46027.5, false).unwrap();
        assert_eq!((day.year, day.month, day.day, day.weekday), (2026, 1, 5, 0));
        assert_eq!(
            format_serial(46027.0, Some("yyyy-mm-dd"), &EN_US, false).unwrap(),
            "2026-01-05"
        );
        assert_eq!(
            format_serial(46027.0, Some("d-mmm-yy"), &EN_US, false).unwrap(),
            "5-Jan-26"
        );
        let dutch = Locale::new(',', '.', "dmy", "nl-NL", ';').unwrap();
        assert_eq!(format_serial(46027.0, Some("mmmm"), &dutch, false).unwrap(), "januari");
        assert_eq!(
            format_serial(46027.0, Some("[$-413]mmm"), &EN_US, false).unwrap(),
            "jan"
        );
        assert!(format_serial(46027.0, Some("h:mm"), &EN_US, false).is_none());
        assert!(date_of_serial(60.0, false).is_none());
    }
}
