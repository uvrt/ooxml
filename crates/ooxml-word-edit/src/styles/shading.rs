//! Owns: a style's or a region's shading in words (docx-agent's `shading_attributes` and
//! `read_shading`, `edit/formatting.py`): `RRGGBB`, a theme colour, `accent1 lighter 80%`,
//! `text1 darker 25%`, as `w:shd`'s attributes (the fill worked out from the document's
//! theme, as Word draws a tint or shade) and back; and a style's own shading written where
//! Word draws it (`_write_shading`).
//!
//! Must not: decide which element holds the shading but for a style.

use super::sheet::Sheet;
use crate::format::{properties_of, set_child, theme_color_name, ThemeColours};
use crate::names::W;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_xml::Element;

/// `w:shd`'s attributes, in docx-agent's order (`None`: removed).
pub(crate) type Shd = Vec<(&'static str, Option<String>)>;

fn refused() -> EditError {
    EditError::Refused(
        "shading is RRGGBB, a theme colour (accent1), 'accent1 lighter 80%', 'accent1 darker 25%', or none".to_owned(),
    )
}

fn hex(value: &str) -> bool {
    value.len() == 6 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// `value` read as `name [lighter|darker N%]` (docx-agent's `_SHADING_WORDS`): the name,
/// and whether lighter with the percentage's digits.
fn words(value: &str) -> Option<(String, Option<(bool, String)>)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let n = tokens.len();
    let last = tokens.last().copied().unwrap_or("");
    // the percentage written "80%" or "80 %"
    let (word_at, number) = if last == "%" && n >= 4 {
        (n - 3, tokens.get(n - 2).copied().unwrap_or(""))
    } else if n >= 3 {
        (n - 2, last.strip_suffix('%').unwrap_or(""))
    } else {
        return Some((trimmed.to_owned(), None));
    };
    let word = tokens.get(word_at).copied().unwrap_or("");
    let lighter = word.eq_ignore_ascii_case("lighter");
    if !(lighter || word.eq_ignore_ascii_case("darker")) || !number_like(number) {
        return Some((trimmed.to_owned(), None));
    }
    // the name: the text before the word (its place within `trimmed`)
    let at = (word.as_ptr() as usize).saturating_sub(trimmed.as_ptr() as usize);
    let name = trimmed.get(..at).unwrap_or("").trim_end();
    Some((name.to_owned(), Some((lighter, number.to_owned()))))
}

/// `\d{1,2}(\.\d+)?`.
fn number_like(text: &str) -> bool {
    let (whole, fraction) = match text.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (text, None),
    };
    (1..=2).contains(&whole.len())
        && whole.chars().all(|c| c.is_ascii_digit())
        && fraction.is_none_or(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_digit()))
}

/// Python's `f"{int(round(x)):02X}"` of a share 0-255.
fn byte(value: f64) -> String {
    format!("{:02X}", value.round_ties_even() as i64)
}

/// `w:shd`'s attributes for a shading value (docx-agent's `shading_attributes`).
///
/// # Errors
/// [`EditError::Refused`] for a value none of the forms reads.
pub(crate) fn attributes(value: &str, colours: &ThemeColours) -> Result<Shd> {
    if hex(value.trim()) {
        return Ok(vec![
            ("val", Some("clear".to_owned())),
            ("color", Some("auto".to_owned())),
            ("fill", Some(value.trim().to_uppercase())),
            ("themeFill", None),
            ("themeFillTint", None),
            ("themeFillShade", None),
        ]);
    }
    let (name, shaped) = words(value).ok_or_else(refused)?;
    let name = theme_color_name(&name).ok_or_else(refused)?;
    let (mut tint, mut shade) = (None, None);
    if let Some((lighter, number)) = shaped {
        let percent: f64 = number.parse().map_err(|_| refused())?;
        if !(percent > 0.0 && percent < 100.0) {
            return Err(EditError::Refused(
                "lighter and darker are percentages above 0 and below 100".to_owned(),
            ));
        }
        let made = byte((100.0 - percent) / 100.0 * 255.0);
        if lighter {
            tint = Some(made);
        } else {
            shade = Some(made);
        }
    }
    let fill = match colours.value(name) {
        Some(base) => {
            ooxml_word_model::theme::word_theme_tint(&base, tint.as_deref(), shade.as_deref()).unwrap_or(base)
        }
        None => "auto".to_owned(),
    };
    Ok(vec![
        ("val", Some("clear".to_owned())),
        ("color", Some("auto".to_owned())),
        ("fill", Some(fill)),
        ("themeFill", Some(name.to_owned())),
        ("themeFillTint", tint),
        ("themeFillShade", shade),
    ])
}

/// `properties`' shading in the words a shading is given in (docx-agent's `read_shading`): `RRGGBB`,
/// or a theme colour with `lighter` / `darker` and a percentage; `None` for none.
#[must_use]
pub fn read_shading(properties: Option<&Element>) -> Option<String> {
    let node = properties?.find(Some(W), "shd")?;
    if let Some(name) = node.attribute(Some(W), "themeFill").filter(|n| !n.is_empty()) {
        for (attribute, word) in [("themeFillTint", "lighter"), ("themeFillShade", "darker")] {
            let Some(raw) = node.attribute(Some(W), attribute).filter(|b| !b.is_empty()) else {
                continue;
            };
            let Ok(value) = i64::from_str_radix(raw, 16) else {
                continue;
            };
            let percent = (100.0 - value as f64 / 255.0 * 100.0).round_ties_even() as i64;
            return Some(if percent == 0 {
                name.to_owned()
            } else {
                format!("{name} {word} {percent}%")
            });
        }
        return Some(name.to_owned());
    }
    let fill = node.attribute(Some(W), "fill");
    let val = node.attribute(Some(W), "val");
    if matches!(fill, None | Some("auto")) && matches!(val, None | Some("clear" | "nil")) {
        return None;
    }
    fill.map(str::to_owned)
}

/// `w:shd` under `parent` set to `shd` (removed for `None`), as `_set_child` sets it.
pub(crate) fn set_shd(work: &mut Work, parent: Id, shd: Option<&Shd>, orders: &Orders) {
    let attributes: Option<Vec<(&str, Option<&str>)>> =
        shd.map(|s| s.iter().map(|(k, v)| (*k, v.as_deref())).collect());
    set_child(work, parent, "shd", attributes.as_deref(), orders);
}

/// A style's shading (`_write_shading`): a paragraph style's in its `w:pPr`, a character
/// style's in its `w:rPr`; `None` removes it. A pattern's colour goes with the old one.
pub(crate) fn write_style_shading(work: &mut Work, node: Id, kind: &str, shd: Option<&Shd>, sheet: &Sheet) {
    let orders = sheet.orders();
    let local = if kind == "paragraph" { "pPr" } else { "rPr" };
    let Some(owner) = properties_of(work, node, local, shd.is_some(), orders) else {
        return;
    };
    let full = shd.map(|s| {
        let mut out = s.clone();
        out.extend([("themeColor", None), ("themeTint", None), ("themeShade", None)]);
        out
    });
    set_shd(work, owner, full.as_ref(), orders);
}
