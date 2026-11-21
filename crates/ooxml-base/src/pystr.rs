//! Owns: Python's `str` predicates the ported code must read exactly as Python reads them,
//! where Rust's standard library reads a different property: `str.isdigit` (Unicode
//! Numeric_Type Decimal or Digit; Rust's `char::is_numeric` is Python's `isnumeric`, which
//! also takes vulgar fractions, Roman numerals and the CJK number letters).
//!
//! The table is generated from CPython 3.12 (Unicode 15.0.0, the version the gates run):
//! every code point `c` with `chr(c).isdigit()`, as inclusive ranges.
//!
//! Must not: be used where the Python it replaces reads another predicate.

/// Python's `str.isdigit` for one character.
#[must_use]
pub fn is_digit(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_digit();
    }
    DIGITS
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

/// Python's `str.isdigit` for a string: not empty, and every character a digit.
#[must_use]
pub fn str_is_digit(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_digit)
}

/// The code points CPython 3.12 reads as digits, as sorted inclusive ranges.
const DIGITS: &[(char, char)] = &[
    ('\u{30}', '\u{39}'),
    ('\u{b2}', '\u{b3}'),
    ('\u{b9}', '\u{b9}'),
    ('\u{660}', '\u{669}'),
    ('\u{6f0}', '\u{6f9}'),
    ('\u{7c0}', '\u{7c9}'),
    ('\u{966}', '\u{96f}'),
    ('\u{9e6}', '\u{9ef}'),
    ('\u{a66}', '\u{a6f}'),
    ('\u{ae6}', '\u{aef}'),
    ('\u{b66}', '\u{b6f}'),
    ('\u{be6}', '\u{bef}'),
    ('\u{c66}', '\u{c6f}'),
    ('\u{ce6}', '\u{cef}'),
    ('\u{d66}', '\u{d6f}'),
    ('\u{de6}', '\u{def}'),
    ('\u{e50}', '\u{e59}'),
    ('\u{ed0}', '\u{ed9}'),
    ('\u{f20}', '\u{f29}'),
    ('\u{1040}', '\u{1049}'),
    ('\u{1090}', '\u{1099}'),
    ('\u{1369}', '\u{1371}'),
    ('\u{17e0}', '\u{17e9}'),
    ('\u{1810}', '\u{1819}'),
    ('\u{1946}', '\u{194f}'),
    ('\u{19d0}', '\u{19da}'),
    ('\u{1a80}', '\u{1a89}'),
    ('\u{1a90}', '\u{1a99}'),
    ('\u{1b50}', '\u{1b59}'),
    ('\u{1bb0}', '\u{1bb9}'),
    ('\u{1c40}', '\u{1c49}'),
    ('\u{1c50}', '\u{1c59}'),
    ('\u{2070}', '\u{2070}'),
    ('\u{2074}', '\u{2079}'),
    ('\u{2080}', '\u{2089}'),
    ('\u{2460}', '\u{2468}'),
    ('\u{2474}', '\u{247c}'),
    ('\u{2488}', '\u{2490}'),
    ('\u{24ea}', '\u{24ea}'),
    ('\u{24f5}', '\u{24fd}'),
    ('\u{24ff}', '\u{24ff}'),
    ('\u{2776}', '\u{277e}'),
    ('\u{2780}', '\u{2788}'),
    ('\u{278a}', '\u{2792}'),
    ('\u{a620}', '\u{a629}'),
    ('\u{a8d0}', '\u{a8d9}'),
    ('\u{a900}', '\u{a909}'),
    ('\u{a9d0}', '\u{a9d9}'),
    ('\u{a9f0}', '\u{a9f9}'),
    ('\u{aa50}', '\u{aa59}'),
    ('\u{abf0}', '\u{abf9}'),
    ('\u{ff10}', '\u{ff19}'),
    ('\u{104a0}', '\u{104a9}'),
    ('\u{10a40}', '\u{10a43}'),
    ('\u{10d30}', '\u{10d39}'),
    ('\u{10e60}', '\u{10e68}'),
    ('\u{11052}', '\u{1105a}'),
    ('\u{11066}', '\u{1106f}'),
    ('\u{110f0}', '\u{110f9}'),
    ('\u{11136}', '\u{1113f}'),
    ('\u{111d0}', '\u{111d9}'),
    ('\u{112f0}', '\u{112f9}'),
    ('\u{11450}', '\u{11459}'),
    ('\u{114d0}', '\u{114d9}'),
    ('\u{11650}', '\u{11659}'),
    ('\u{116c0}', '\u{116c9}'),
    ('\u{11730}', '\u{11739}'),
    ('\u{118e0}', '\u{118e9}'),
    ('\u{11950}', '\u{11959}'),
    ('\u{11c50}', '\u{11c59}'),
    ('\u{11d50}', '\u{11d59}'),
    ('\u{11da0}', '\u{11da9}'),
    ('\u{11f50}', '\u{11f59}'),
    ('\u{16a60}', '\u{16a69}'),
    ('\u{16ac0}', '\u{16ac9}'),
    ('\u{16b50}', '\u{16b59}'),
    ('\u{1d7ce}', '\u{1d7ff}'),
    ('\u{1e140}', '\u{1e149}'),
    ('\u{1e2f0}', '\u{1e2f9}'),
    ('\u{1e4f0}', '\u{1e4f9}'),
    ('\u{1e950}', '\u{1e959}'),
    ('\u{1f100}', '\u{1f10a}'),
    ('\u{1fbf0}', '\u{1fbf9}'),
];

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
        // Where Rust's `is_numeric` (Python's `isnumeric`) and `isdigit` differ: vulgar
        // fractions, Roman numerals, circled numbers past nine, Bengali currency numerators,
        // the CJK zero and the letter numbers.
        for c in ['½', '¼', '¾', '⅓', 'Ⅻ', 'ⅰ', '⑩', '⑳', '৴', '〇', '〡', 'ᛮ', '𐅀'] {
            assert!(c.is_numeric(), "{c:?} is numeric");
            assert!(!is_digit(c), "{c:?} is no digit in Python");
        }
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
    fn table_is_sorted_and_disjoint() {
        for pair in DIGITS.windows(2) {
            assert!(pair[0].0 <= pair[0].1 && pair[0].1 < pair[1].0);
        }
    }
}
