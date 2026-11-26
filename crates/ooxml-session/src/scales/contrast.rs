//! Owns: WCAG 2's contrast ratio of two colours and the text colour that reads on a fill
//! (ooxml-edit's `contrast`, ported exactly): a colour read as `RRGGBB` ([`hex_colour`]),
//! its relative luminance, the ratio of two ([`ratio`]) and white or black, whichever reads
//! better on a fill, a tie to black ([`readable_text`]).
//!
//! Must not: read a document's colours.

/// White, as a step's `text` names it.
pub const WHITE: &str = "FFFFFF";
/// Black, as a step's `text` names it.
pub const BLACK: &str = "000000";
/// WCAG 2's minimum contrast for text (AA).
pub const TEXT_MINIMUM: f64 = 4.5;

/// `value` as `RRGGBB` in capitals, `None` when it is not six hex digits (a leading `#`
/// allowed; spaces around it aside).
#[must_use]
pub fn hex_colour(value: &str) -> Option<String> {
    let text = crate::facts::py_strip(value);
    let digits = text.strip_prefix('#').unwrap_or(text);
    (digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit())).then(|| digits.to_ascii_uppercase())
}

/// WCAG 2's relative luminance of `RRGGBB`, 0 to 1.
fn luminance(colour: &str) -> f64 {
    let value = colour.trim_start_matches('#');
    let channel = |at: usize| {
        let c = f64::from(
            value
                .get(at..at + 2)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .unwrap_or(0),
        ) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
}

/// WCAG 2's contrast ratio of two colours, 1 to 21.
#[must_use]
pub fn ratio(first: &str, second: &str) -> f64 {
    let (a, b) = (luminance(first), luminance(second));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The text colour for `fill`: white or black, whichever has the higher ratio with it (a tie
/// goes to black), and that ratio rounded to two places.
#[must_use]
pub fn readable_text(fill: &str) -> (&'static str, f64) {
    let (white, black) = (ratio(WHITE, fill), ratio(BLACK, fill));
    if white > black {
        (WHITE, ooxml_base::pyfmt::round2(white))
    } else {
        (BLACK, ooxml_base::pyfmt::round2(black))
    }
}
