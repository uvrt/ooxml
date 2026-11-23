//! Owns: what a field update writes for a field (docx-agent's `_value`, `_sequence_value`,
//! `format_date`): page numbers and `STYLEREF` from the layout ([`super::LaidFields`]),
//! `REF` from its bookmark's text, `SEQ` counted through the document, `DATE` from the
//! call's date in its picture (and back: a date typed in a picture, `day_in_picture`); a table
//! of contents is rebuilt elsewhere (`super::toc`).
//!
//! Must not: write anything.

use super::pages::{FieldPlace, FieldRun, LaidFields, Unknown};
use super::scan::{instruction_run, paragraph_of, Located, Marks};
use crate::names::W;
use crate::text_range::Span;
use crate::WordDoc;
use ooxml_word_model::address::bookmark_range;
use ooxml_word_model::docx::cells::story_model_path;
use ooxml_word_model::docx::sections::section_story;
use ooxml_word_model::docx::text::OBJECT;
use ooxml_word_model::docx::Docx;
use ooxml_word_model::fieldtext::argument;
use ooxml_word_model::fieldtext::sequence::{sequence_step, sequence_switches};
use std::collections::{HashMap, HashSet};

/// A day: year, month (1-12), day (1-31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Day {
    /// The year.
    pub year: i64,
    /// The month, 1-12.
    pub month: u32,
    /// The day of the month, 1-31.
    pub day: u32,
}

impl Day {
    /// The day an ISO 8601 date or date and time (`2026-11-17`, `2026-11-17T10:00Z`)
    /// starts with.
    #[must_use]
    pub fn parse(text: &str) -> Option<Day> {
        let date = text.get(..10)?;
        let mut parts = date.split('-');
        let year = parts.next()?.parse().ok()?;
        let month = parts.next()?.parse().ok().filter(|m| (1..=12).contains(m))?;
        let day = parts.next()?.parse().ok().filter(|d| (1..=31).contains(d))?;
        Some(Day { year, month, day })
    }

    /// Monday 0 .. Sunday 6.
    fn weekday(self) -> usize {
        // days since 1970-01-01 (a Thursday), by Howard Hinnant's civil-from-days inverse
        let (y, m) = if self.month <= 2 {
            (self.year - 1, i64::from(self.month) + 9)
        } else {
            (self.year, i64::from(self.month) - 3)
        };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * m + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        usize::try_from((days + 3).rem_euclid(7)).unwrap_or(0)
    }
}

/// English month names (`DATE`'s pictures, as measured in an en-GB document).
const MONTHS: [&str; 12] = [
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
];
/// English day names, Monday first.
const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// The picture's tokens as docx-agent's pattern takes them: a quoted literal, a run of one
/// letter (`d`, `M` up to four, `y` two to four, `H` `h` `m` `s` up to two), `AM/PM`,
/// `am/pm`, or one character.
fn picture_tokens(picture: &str) -> Vec<String> {
    let chars: Vec<char> = picture.chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while let Some(&c) = chars.get(k) {
        let rest: String = chars.get(k..).unwrap_or_default().iter().collect();
        if c == '\'' {
            if let Some(end) = chars.iter().skip(k + 1).position(|d| *d == '\'') {
                out.push(chars.get(k..=k + 1 + end).unwrap_or_default().iter().collect());
                k += end + 2;
                continue;
            }
        }
        let limit = match c {
            'd' | 'M' => Some((1, 4)),
            'y' => Some((2, 4)),
            'H' | 'h' | 'm' | 's' => Some((1, 2)),
            _ => None,
        };
        if let Some((least, most)) = limit {
            let count = chars.iter().skip(k).take(most).take_while(|d| **d == c).count();
            if count >= least {
                out.push(c.to_string().repeat(count));
                k += count;
                continue;
            }
        }
        if rest.starts_with("AM/PM") || rest.starts_with("am/pm") {
            out.push(rest.chars().take(5).collect());
            k += 5;
            continue;
        }
        out.push(c.to_string());
        k += 1;
    }
    out
}

/// A `DATE`'s `\@` picture applied to `day`, English names (`format_date`); the time of
/// day is midnight.
#[must_use]
pub fn format_date(picture: &str, day: Day) -> String {
    let mut out = String::new();
    let month = usize::try_from(day.month).unwrap_or(1).saturating_sub(1);
    let month_name = MONTHS.get(month).copied().unwrap_or("");
    let day_name = DAYS.get(day.weekday()).copied().unwrap_or("");
    let short = |name: &str| name.chars().take(3).collect::<String>();
    for token in picture_tokens(picture) {
        let first = token.chars().next().unwrap_or(' ');
        let len = token.chars().count();
        match first {
            '\'' => out.push_str(token.trim_matches('\'')),
            'd' => out.push_str(&match len {
                1 => day.day.to_string(),
                2 => format!("{:02}", day.day),
                3 => short(day_name),
                _ => day_name.to_owned(),
            }),
            'M' => out.push_str(&match len {
                1 => day.month.to_string(),
                2 => format!("{:02}", day.month),
                3 => short(month_name),
                _ => month_name.to_owned(),
            }),
            'y' => out.push_str(&if len == 2 {
                format!("{:02}", day.year.rem_euclid(100))
            } else {
                day.year.to_string()
            }),
            'A' | 'a' if len == 5 => out.push_str(if first == 'A' { "AM" } else { "am" }),
            'h' => out.push_str("12"),
            'H' | 'm' | 's' => out.push_str(if len == 2 { "00" } else { "0" }),
            _ => out.push_str(&token),
        }
    }
    out
}

/// The day `text` names as written in `picture` (`5-10-2026` in `d-M-yyyy`), the way back
/// from [`format_date`]: the day and month in one or two digits, or a month or day name in
/// full or its first three letters (English, any case); the year in four digits (two name
/// no century, so they are not read); a time of day read and left; any run of spaces and
/// punctuation where the picture has one. `None` for text the picture does not read, or a
/// day its month lacks.
#[must_use]
pub fn day_in_picture(picture: &str, text: &str) -> Option<Day> {
    let mut reader = Reader {
        chars: text.trim().chars().collect(),
        at: 0,
    };
    let (mut year, mut month, mut day) = (None, None, None);
    let mut gap = false;
    for token in picture_tokens(picture) {
        let first = token.chars().next()?;
        let len = token.chars().count();
        let was_gap = std::mem::take(&mut gap);
        match first {
            'd' if len <= 2 => day = Some(reader.number(2)?),
            'd' => {
                reader.name(&DAYS)?;
            }
            'M' if len <= 2 => month = Some(reader.number(2)?),
            'M' => month = Some(u32::try_from(reader.name(&MONTHS)? + 1).ok()?),
            'y' if len >= 2 => {
                let from = reader.at;
                let read = reader.number(4)?;
                if reader.at - from != 4 {
                    return None;
                }
                year = Some(i64::from(read));
            }
            'H' | 'h' | 'm' | 's' => {
                reader.number(2)?;
            }
            'A' | 'a' if len == 5 => reader.letters(),
            '\'' => reader.literal(token.trim_matches('\''))?,
            c if c.is_alphanumeric() => reader.literal(&token)?,
            _ => {
                gap = true;
                if !was_gap {
                    reader.gap()?;
                }
            }
        }
    }
    let (year, month, day) = (year?, month?, day?);
    let whole = reader.at == reader.chars.len();
    (whole && (1..=days_in(year, month)).contains(&day)).then_some(Day { year, month, day })
}

/// Text read from its start, as [`day_in_picture`] reads it.
struct Reader {
    chars: Vec<char>,
    at: usize,
}

impl Reader {
    /// A number of one to `most` digits.
    fn number(&mut self, most: usize) -> Option<u32> {
        let rest = self.chars.get(self.at..).unwrap_or_default();
        let digits: String = rest.iter().take(most).take_while(|c| c.is_ascii_digit()).collect();
        self.at += digits.len();
        digits.parse().ok()
    }

    /// A run of letters.
    fn letters(&mut self) {
        while self.chars.get(self.at).is_some_and(|c| c.is_alphabetic()) {
            self.at += 1;
        }
    }

    /// Which of `names` the next letters are, in full or their first three, in any case.
    fn name(&mut self, names: &[&str]) -> Option<usize> {
        let from = self.at;
        self.letters();
        let word = self.chars.get(from..self.at)?.iter().collect::<String>().to_lowercase();
        names.iter().position(|name| {
            let name = name.to_lowercase();
            word == name || (word.chars().count() == 3 && name.starts_with(&word))
        })
    }

    /// `literal`, in any case.
    fn literal(&mut self, literal: &str) -> Option<()> {
        for want in literal.chars() {
            let got = self.chars.get(self.at)?;
            if !got.to_lowercase().eq(want.to_lowercase()) {
                return None;
            }
            self.at += 1;
        }
        Some(())
    }

    /// A run of one or more spaces and punctuation.
    fn gap(&mut self) -> Option<()> {
        let from = self.at;
        while self.chars.get(self.at).is_some_and(|c| !c.is_alphanumeric()) {
            self.at += 1;
        }
        (self.at > from).then_some(())
    }
}

/// The days of `month` in `year` (Gregorian); none for no month.
fn days_in(year: i64, month: u32) -> u32 {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => 0,
    }
}

/// Every `SEQ` field's text, counted through the document in story order whether
/// updated or not, by its part and number: the text, or `None` where Word draws an error.
pub(crate) fn sequence_values(fields: &[Located]) -> HashMap<(String, usize), Option<String>> {
    let mut counts = HashMap::new();
    let restarted = HashSet::new();
    let mut out = HashMap::new();
    for located in fields {
        if located.found.keyword() != "SEQ" {
            continue;
        }
        let instruction = &located.found.instruction;
        let (name, found) = sequence_switches(instruction);
        let name = name.unwrap_or_default().to_uppercase();
        let value = sequence_step(instruction, &name, &found, &mut counts, &restarted).ok();
        out.insert((located.part.clone(), located.found.number), value);
    }
    out
}

/// Where `located` is, as the layout is asked: the body paragraph it begins in by id, or
/// for a header's or footer's field the first page that shows its story (a page not blank
/// whose section states it for its kind, or inherits it: `_story_page`); `None` for
/// another story, or a story no page shows.
pub(crate) fn place_of(docx: &Docx, located: &Located, laid: &dyn LaidFields) -> Option<Placed> {
    let part = docx.part(&located.part)?;
    if located.part == docx.main {
        let path = paragraph_of(part.root(), &located.found.begin)?;
        return part.ids.paragraph_at(&path).map(|p| Placed::Body(p.id));
    }
    let story = &part.story;
    // A header or footer by its root (`w:hdr`, `w:ftr`), not its name: a part may be named
    // anything (`word/hrIdH1.xml`).
    let root = part.root().name();
    let which = if root.is(Some(W), "hdr") {
        "header"
    } else if root.is(Some(W), "ftr") {
        "footer"
    } else {
        return None;
    };
    let sections = docx.sections();
    let mut shown: HashMap<(usize, &'static str), Option<String>> = HashMap::new();
    for (k, page) in laid.pages().into_iter().enumerate() {
        if page.blank {
            continue;
        }
        let found = shown
            .entry((page.section, page.kind))
            .or_insert_with(|| section_story(docx, sections, page.section, which, page.kind));
        if found.as_deref() == Some(story.as_str()) {
            return Some(Placed::Page(k));
        }
    }
    None
}

/// A place the layout is asked about.
pub(crate) enum Placed {
    /// A body paragraph by id.
    Body(String),
    /// A page.
    Page(usize),
}

impl Placed {
    fn place(&self) -> FieldPlace<'_> {
        match self {
            Placed::Body(id) => FieldPlace::Body(id),
            Placed::Page(k) => FieldPlace::Page(*k),
        }
    }
}

/// A field's new result (`_value`): `None` for a field whose result is not computed
/// (left as it is).
///
/// # Errors
/// [`Unknown`] where the layout (or the bookmark) cannot say: the field is left as it is.
/// `marks`: `docx`'s bookmarks, as the pass looks them up.
pub(crate) fn value(
    doc: &WordDoc,
    (docx, marks): (&Docx, &Marks<'_>),
    located: &Located,
    laid: Option<&dyn LaidFields>,
    today: Day,
) -> Result<Option<String>, Unknown> {
    let found = &located.found;
    let keyword = found.keyword();
    let unknown = |why: &str| Unknown(why.to_owned());
    match keyword.as_str() {
        "PAGE" | "NUMPAGES" | "SECTIONPAGES" => {
            let laid = laid.ok_or_else(|| unknown("no layout"))?;
            let at = place_of(docx, located, laid).ok_or_else(|| unknown("not on a page"))?;
            laid.page_field(&found.instruction, &keyword, at.place()).map(Some)
        }
        "PAGEREF" => {
            let name = argument(&found.instruction).ok_or_else(|| unknown("no bookmark"))?;
            let at = bookmark_place(doc, docx, marks, &name).ok_or_else(|| unknown("no bookmark"))?;
            let laid = laid.ok_or_else(|| unknown("no layout"))?;
            laid.page_reference(&found.instruction, FieldPlace::Body(&at)).map(Some)
        }
        "STYLEREF" => {
            let laid = laid.ok_or_else(|| unknown("no layout"))?;
            let at = place_of(docx, located, laid).ok_or_else(|| unknown("not on a page"))?;
            let root = docx.part(&located.part).ok_or_else(|| unknown("no part"))?.root();
            let path = instruction_run(root, found)
                .and_then(|run| story_model_path(root, &run))
                .ok_or_else(|| unknown("no instruction run"))?;
            let run = FieldRun {
                part: &located.part,
                path: &path,
            };
            laid.style_reference(&found.instruction, at.place(), run).map(Some)
        }
        "REF" => {
            let Some(name) = argument(&found.instruction) else {
                return Ok(None);
            };
            let text = bookmark_text(doc, docx, marks, &name).ok_or_else(|| unknown("no bookmark"))?;
            let first = text.split('\n').next().unwrap_or("");
            Ok(Some(first.replace(OBJECT, "")))
        }
        "DATE" => {
            let picture = super::switches::switch(&found.instruction, '@');
            Ok(Some(format_date(picture.as_deref().unwrap_or("d-M-yyyy"), today)))
        }
        _ => Ok(None),
    }
}

/// The id of the body paragraph a bookmark starts in (`_bookmark_page`'s target).
fn bookmark_place(doc: &WordDoc, docx: &Docx, marks: &Marks<'_>, name: &str) -> Option<String> {
    let found = bookmark_range(docx, &marks.get(name)?).ok()?;
    let span = Span::of(doc, docx, &found).ok()?;
    (span.part == docx.main).then(|| span.first().map(|f| f.id.clone()))?
}

/// The text a bookmark marks (`Bookmark.text`).
fn bookmark_text(doc: &WordDoc, docx: &Docx, marks: &Marks<'_>, name: &str) -> Option<String> {
    let found = bookmark_range(docx, &marks.get(name)?).ok()?;
    Span::of(doc, docx, &found).ok().map(|s| s.text())
}

#[cfg(test)]
mod tests {
    use super::{day_in_picture, format_date, Day};

    fn day(year: i64, month: u32, day: u32) -> Option<Day> {
        Some(Day { year, month, day })
    }

    #[test]
    fn a_date_is_read_in_its_picture() {
        assert_eq!(day_in_picture("d-M-yyyy", "5-10-2026"), day(2026, 10, 5));
        assert_eq!(day_in_picture("dd/MM/yyyy", " 05/10/2026 "), day(2026, 10, 5));
        assert_eq!(day_in_picture("MMMM d, yyyy", "october 5, 2026"), day(2026, 10, 5));
        assert_eq!(day_in_picture("dddd d MMM yyyy", "Mon 5 Oct 2026"), day(2026, 10, 5));
        assert_eq!(
            day_in_picture("d 'de' MMMM yyyy", "5 de October 2026"),
            day(2026, 10, 5)
        );
    }

    #[test]
    fn a_date_read_back_is_the_date_written() {
        let written = day(2028, 2, 29).unwrap_or(Day {
            year: 0,
            month: 0,
            day: 0,
        });
        for picture in [
            "d-M-yyyy",
            "dd.MM.yyyy",
            "d MMMM yyyy",
            "dddd, MMMM dd, yyyy",
            "yyyy-MM-dd",
        ] {
            assert_eq!(
                day_in_picture(picture, &format_date(picture, written)),
                Some(written),
                "{picture}"
            );
        }
    }

    #[test]
    fn text_the_picture_does_not_read_is_no_date() {
        assert_eq!(day_in_picture("d-M-yyyy", "5-10-26"), None); // a two-digit year: no century
        assert_eq!(day_in_picture("d-M-yyyy", "31-9-2026"), None); // September has 30 days
        assert_eq!(day_in_picture("d-M-yyyy", "5-10-2026 later"), None);
        assert_eq!(day_in_picture("d MMMM yyyy", "5 Octobre 2026"), None);
        assert_eq!(day_in_picture("d-M-yyyy", "soon"), None);
    }
}
