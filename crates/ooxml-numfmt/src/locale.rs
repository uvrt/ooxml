//! Owns: the locale a number or date is written for -- its decimal, thousands and list
//! separators, date order and language (a canonical BCP 47 tag) -- equal to ooxml-common's
//! `locales.py`, with en-US the default ([`EN_US`]).
//!
//! Must not: hold a table of every locale (a caller builds the one it wants), or read the
//! machine's settings.

use std::borrow::Cow;

/// Why a locale or language tag is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocaleError {
    /// Not a BCP 47 tag.
    #[error("language is a BCP 47 tag such as 'en-US', not {0:?}")]
    Language(String),
    /// A separator is not one non-alphanumeric character, or clashes with another.
    #[error("{0}")]
    Separator(String),
    /// Not one of `dmy`, `mdy`, `ymd`.
    #[error("date_order is one of ('dmy', 'mdy', 'ymd'), not {0:?}")]
    DateOrder(String),
}

/// How numbers and dates are written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    /// The decimal separator.
    pub decimal_separator: char,
    /// The thousands separator.
    pub thousands_separator: char,
    /// `dmy`, `mdy` or `ymd`.
    pub date_order: Cow<'static, str>,
    /// The language, canonical.
    pub language: Cow<'static, str>,
    /// The list separator.
    pub list_separator: char,
}

/// en-US: `.` and `,`, month first.
pub const EN_US: Locale = Locale {
    decimal_separator: '.',
    thousands_separator: ',',
    date_order: Cow::Borrowed("mdy"),
    language: Cow::Borrowed("en-US"),
    list_separator: ',',
};

impl Default for Locale {
    fn default() -> Self {
        EN_US
    }
}

fn separator(name: &str, value: char) -> Result<char, LocaleError> {
    if value.is_alphanumeric() {
        return Err(LocaleError::Separator(format!(
            "{name} is one character, not a letter or digit: {value:?}"
        )));
    }
    Ok(value)
}

impl Locale {
    /// A locale, checked as the Python's constructor checks it.
    ///
    /// # Errors
    /// When a separator is a letter or digit, the decimal one is white space or shared,
    /// the date order is not one of the three, or the language is not a tag.
    pub fn new(
        decimal: char,
        thousands: char,
        date_order: &str,
        language: &str,
        list: char,
    ) -> Result<Locale, LocaleError> {
        let decimal = separator("decimal_separator", decimal)?;
        if decimal.is_whitespace() {
            return Err(LocaleError::Separator(format!(
                "decimal_separator is not whitespace: {decimal:?}"
            )));
        }
        if separator("thousands_separator", thousands)? == decimal {
            return Err(LocaleError::Separator(
                "thousands_separator is not the decimal_separator".into(),
            ));
        }
        if separator("list_separator", list)? == decimal {
            return Err(LocaleError::Separator(
                "list_separator is not the decimal_separator".into(),
            ));
        }
        if !matches!(date_order, "dmy" | "mdy" | "ymd") {
            return Err(LocaleError::DateOrder(date_order.into()));
        }
        Ok(Locale {
            decimal_separator: decimal,
            thousands_separator: thousands,
            date_order: Cow::Owned(date_order.into()),
            language: Cow::Owned(canonical_language(language)?),
            list_separator: list,
        })
    }

    /// The language's primary subtag (`en`).
    pub fn primary_language(&self) -> &str {
        self.language.split('-').next().unwrap_or("")
    }

    /// Number text written with `.` and `,` turned into this locale's separators.
    pub fn number_text(&self, text: &str) -> String {
        if self.decimal_separator == '.' && self.thousands_separator == ',' {
            return text.to_owned();
        }
        text.chars()
            .map(|c| match c {
                '.' => self.decimal_separator,
                ',' => self.thousands_separator,
                other => other,
            })
            .collect()
    }
}

fn alpha(part: &str, sizes: std::ops::RangeInclusive<usize>) -> bool {
    sizes.contains(&part.len()) && part.chars().all(|c| c.is_ascii_alphabetic())
}

/// A language tag in canonical case (`en-us` as `en-US`, `zh_hant_tw` as `zh-Hant-TW`).
///
/// # Errors
/// When `tag` is not a BCP 47 tag of the shape the Python accepts.
pub fn canonical_language(tag: &str) -> Result<String, LocaleError> {
    let refuse = || LocaleError::Language(tag.into());
    let normal = tag.replace('_', "-");
    let mut parts = normal.split('-').peekable();
    let language = parts.next().filter(|p| alpha(p, 2..=3)).ok_or_else(refuse)?;
    let mut out = vec![language.to_ascii_lowercase()];
    if let Some(script) = parts.next_if(|p| alpha(p, 4..=4)) {
        let mut chars = script.chars();
        let first = chars.next().map(|c| c.to_ascii_uppercase()).unwrap_or_default();
        out.push(format!("{first}{}", chars.as_str().to_ascii_lowercase()));
    }
    if let Some(region) = parts.next_if(|p| alpha(p, 2..=2) || (p.len() == 3 && p.chars().all(|c| c.is_ascii_digit())))
    {
        out.push(region.to_ascii_uppercase());
    }
    for variant in parts {
        let alnum = variant.chars().all(|c| c.is_ascii_alphanumeric());
        let long = (5..=8).contains(&variant.len());
        let digit_led = variant.len() == 4 && variant.starts_with(|c: char| c.is_ascii_digit());
        if !alnum || !(long || digit_led) {
            return Err(refuse());
        }
        out.push(variant.to_ascii_lowercase());
    }
    Ok(out.join("-"))
}

#[cfg(test)]
mod tests {
    use super::{canonical_language, Locale, EN_US};

    #[test]
    fn tags_are_canonical_and_numbers_localised() {
        assert_eq!(canonical_language("nl_be").unwrap(), "nl-BE");
        assert_eq!(canonical_language("zh-hant-tw").unwrap(), "zh-Hant-TW");
        assert!(canonical_language("english").is_err());
        let dutch = Locale::new(',', '.', "dmy", "nl-NL", ';').unwrap();
        assert_eq!(dutch.number_text("1,234.5"), "1.234,5");
        assert_eq!(EN_US.number_text("1,234.5"), "1,234.5");
        assert_eq!(EN_US.primary_language(), "en");
    }
}
