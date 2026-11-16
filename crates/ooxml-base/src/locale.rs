//! Owns: the locale as a value (a BCP 47 tag; en-US unless a caller says otherwise).
//! Must not: read the process's locale or environment.

use crate::BaseError;

/// A locale, by its BCP 47 tag (`en-US`, `fr-FR`, `nl`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Locale {
    tag: String,
}

impl Default for Locale {
    fn default() -> Self {
        Locale {
            tag: "en-US".to_owned(),
        }
    }
}

impl Locale {
    /// A locale from its tag: subtags of 1 to 8 ASCII letters or digits joined by `-`, the
    /// first of 2 to 8 letters. `_` is read as `-` (`en_US`).
    pub fn new(tag: &str) -> Result<Self, BaseError> {
        let tag = tag.replace('_', "-");
        let mut subtags = tag.split('-');
        let first_ok = subtags
            .next()
            .is_some_and(|s| (2..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphabetic()));
        let rest_ok = subtags.all(|s| (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric()));
        if first_ok && rest_ok {
            Ok(Locale { tag })
        } else {
            Err(BaseError::InvalidLocale(tag))
        }
    }

    /// The tag, as given (with `_` read as `-`).
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// The primary language subtag, lower-cased (`en` of `en-US`).
    pub fn language(&self) -> String {
        self.tag.split('-').next().unwrap_or_default().to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(Locale::default().tag(), "en-US");
        assert_eq!(Locale::new("fr_FR").unwrap().tag(), "fr-FR");
        assert_eq!(Locale::new("NL-be").unwrap().language(), "nl");
        assert!(Locale::new("").is_err());
        assert!(Locale::new("e").is_err());
        assert!(Locale::new("en--US").is_err());
        assert!(Locale::new("en-US!").is_err());
    }
}
