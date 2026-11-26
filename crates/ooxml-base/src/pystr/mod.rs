//! Owns: Python's `str` predicates the ported code must read exactly as Python reads them,
//! where Rust's standard library reads a different property: `str.isalpha` (general
//! category L*; Rust's `char::is_alphabetic` is Unicode's Alphabetic, which also takes
//! combining vowel signs and letter numbers), `str.isdecimal`, `str.isdigit` and
//! `str.isnumeric` (Numeric_Type Decimal; or Digit; or Numeric -- Rust's `char::is_numeric`
//! is the general categories Nd, Nl and No, which differ from all three: it takes vulgar
//! fractions and Roman numerals but not the CJK number ideographs), `str.isalnum`, and the
//! word character of `re`'s `\w` in a `str` pattern (`isalnum` or `_`).
//!
//! The tables ([`tables`]) are generated from CPython 3.12 (Unicode 15.0.0, the version the
//! gates run) by tools/pystr_tables.py.
//!
//! Must not: be used where the Python it replaces reads another predicate.

mod tables;

use tables::{ALPHA, DECIMAL, DIGITS, NUMERIC};

/// Whether `c` is in `table` (sorted inclusive ranges).
fn within(table: &[(char, char)], c: char) -> bool {
    table
        .binary_search_by(|&(low, high)| {
            if high < c {
                std::cmp::Ordering::Less
            } else if low > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Python's `str.isalpha` for one character.
#[must_use]
pub fn is_alpha(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphabetic();
    }
    within(ALPHA, c)
}

/// Python's `str.isdecimal` for one character (`re`'s `\d` in a `str` pattern).
#[must_use]
pub fn is_decimal(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_digit();
    }
    within(DECIMAL, c)
}

/// The value of a decimal digit as Python's `int` and `Decimal` read it (`unicodedata.decimal`),
/// any script's; `None` for a character that is no decimal digit. (Each run of decimal digits
/// in the table starts at a zero and spans whole tens.)
#[must_use]
pub fn decimal_value(c: char) -> Option<u32> {
    if c.is_ascii() {
        return c.to_digit(10);
    }
    let run = DECIMAL.iter().find(|(low, high)| (*low..=*high).contains(&c))?;
    Some((u32::from(c) - u32::from(run.0)) % 10)
}

/// Python's `str.isdigit` for one character.
#[must_use]
pub fn is_digit(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_digit();
    }
    within(DIGITS, c)
}

/// Python's `str.isnumeric` for one character.
#[must_use]
pub fn is_numeric(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_digit();
    }
    within(NUMERIC, c)
}

/// Python's `str.isalnum` for one character: `isalpha`, `isdecimal`, `isdigit` or
/// `isnumeric` (the last takes the other two).
#[must_use]
pub fn is_alnum(c: char) -> bool {
    is_alpha(c) || is_numeric(c)
}

/// A word character of `re`'s `\w` in a `str` pattern: `isalnum`, or `_`.
#[must_use]
pub fn is_word(c: char) -> bool {
    c == '_' || is_alnum(c)
}

/// Python's `str.isdigit` for a string: not empty, and every character a digit.
#[must_use]
pub fn str_is_digit(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_python_reads_as_digits() {
        // ASCII, superscripts, circled and Arabic-Indic digits, a mathematical digit.
        for c in [
            '0', '9', '²', '³', '¹', '⁰', '₉', '①', '⑨', '⓪', '❶', '٣', '۵', '０', '𝟎', '🄀',
        ] {
            assert!(is_digit(c), "{c:?} is a digit in Python");
            assert!(c.is_numeric());
        }
    }

    #[test]
    fn numeric_characters_python_reads_as_no_digit() {
        // Where `isnumeric` and `isdigit` differ: vulgar fractions, Roman numerals, circled
        // numbers past nine, Bengali currency numerators, the CJK zero and the letter numbers.
        for c in ['½', '¼', '¾', '⅓', 'Ⅻ', 'ⅰ', '⑩', '⑳', '৴', '〇', '〡', 'ᛮ', '𐅀'] {
            assert!(c.is_numeric() && is_numeric(c), "{c:?} is numeric");
            assert!(!is_digit(c), "{c:?} is no digit in Python");
        }
    }

    #[test]
    fn a_decimal_digit_s_value_is_python_s() {
        for (c, value) in [('7', 7), ('٣', 3), ('۵', 5), ('０', 0), ('𝟗', 9), ('𝟎', 0)] {
            assert_eq!(decimal_value(c), Some(value), "{c:?}");
        }
        assert_eq!(decimal_value('²'), None);
        assert_eq!(decimal_value('a'), None);
    }

    #[test]
    fn other_characters_are_no_digits() {
        for c in ['a', ' ', '-', '五', '\u{10ffff}'] {
            assert!(!is_digit(c));
        }
    }

    #[test]
    fn strings() {
        assert!(str_is_digit("2024"));
        assert!(str_is_digit("²٣"));
        assert!(!str_is_digit(""));
        assert!(!str_is_digit("1½"));
    }

    #[test]
    fn letters_as_python_reads_them() {
        // general category L*: the feminine ordinal, micro, a title-case digraph, a modifier
        // letter, a CJK number ideograph
        for c in ['a', 'Z', 'ª', 'µ', 'ǅ', 'ʰ', '五'] {
            assert!(is_alpha(c) && is_alnum(c) && is_word(c), "{c:?} is a letter in Python");
        }
        // Rust's Alphabetic, not Python's letters: a combining vowel sign, a spacing mark,
        // the Greek iota subscript, the letter numbers
        for c in ['\u{93e}', '\u{903}', '\u{345}', 'ⅰ', 'ᛮ'] {
            assert!(c.is_alphabetic() && !is_alpha(c), "{c:?} is no letter in Python");
        }
        assert!(!is_alpha('_') && !is_alnum('_') && is_word('_'));
        assert!(!is_word('\u{93e}') && !is_word('-'));
    }

    #[test]
    fn decimals_and_numerics_as_python_reads_them() {
        for c in ['0', '٣', '０', '𝟎'] {
            assert!(is_decimal(c) && is_digit(c) && is_numeric(c), "{c:?} is decimal");
        }
        for c in ['²', '①'] {
            assert!(!is_decimal(c) && is_digit(c), "{c:?} is a digit, not decimal");
        }
        // CJK number ideographs: numeric in Python, not in Rust (general category Lo)
        for c in ['五', '〇', '兆'] {
            assert!(is_numeric(c) && is_alnum(c), "{c:?} is numeric in Python");
        }
        assert!(!'五'.is_numeric());
        assert!(!is_numeric('a') && !is_numeric(' '));
    }

    #[test]
    fn tables_are_sorted_and_disjoint() {
        for table in [ALPHA, DECIMAL, DIGITS, NUMERIC] {
            for pair in table.windows(2) {
                assert!(pair[0].0 <= pair[0].1 && pair[0].1 < pair[1].0);
            }
            assert!(table.first().is_some_and(|(low, _)| !low.is_ascii()));
        }
    }
}
