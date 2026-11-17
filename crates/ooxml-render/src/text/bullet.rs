//! Owns: a paragraph's bullet (ooxml-common's `textbody` bullet functions): its text -- a
//! character, or an automatic number in its scheme, counted per scheme -- its picture, its
//! size, and its span, a symbol face the drawing has no copy of drawn as the Unicode it
//! stands for.
//!
//! Must not: place the bullet's line (the body does).

use super::line::{leading, mapped_symbol_face, Dy};
use crate::context::Painter;
use crate::measure::PX_PER_PT;
use ooxml_drawingml::drawable::{Bullet, ParagraphProperties, RunProperties};
use ooxml_fonts::font_family_value;
use ooxml_fonts::symbols::{self, FALLBACK_FAMILIES};
use ooxml_scene::markup::{Attr, Element, Tag, Value};
use std::collections::HashMap;

pub(super) fn picture_bullet(props: &ParagraphProperties) -> Option<(&str, &str)> {
    match &props.bullet {
        Some(Bullet::Picture { data, mime }) => Some((data.as_str(), mime.as_str())),
        _ => None,
    }
}

pub(super) fn bullet_text(props: &ParagraphProperties, counters: &mut HashMap<String, i64>) -> Option<String> {
    match props.bullet.as_ref()? {
        Bullet::Char(c) => Some(c.clone()),
        Bullet::AutoNumber { scheme, start } => {
            let key = format!("{scheme}-{}", props.level);
            let count = counters.entry(key).or_insert(0);
            *count += 1;
            Some(format_auto_num(scheme, start + *count - 1))
        }
        Bullet::None | Bullet::Picture { .. } => None,
    }
}

/// An automatic number as its scheme writes it.
pub fn format_auto_num(scheme: &str, index: i64) -> String {
    match scheme {
        "arabicParenR" => format!("{index})"),
        "arabicPlain" => index.to_string(),
        "romanUcPeriod" => format!("{}.", roman(index)),
        "romanLcPeriod" => format!("{}.", roman(index).to_lowercase()),
        "alphaUcPeriod" => format!("{}.", alpha(index)),
        "alphaLcPeriod" => format!("{}.", alpha(index).to_lowercase()),
        "alphaUcParenR" => format!("{})", alpha(index)),
        "alphaLcParenR" => format!("{})", alpha(index).to_lowercase()),
        _ => format!("{index}."),
    }
}

fn roman(number: i64) -> String {
    const NUMERALS: [(i64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    let mut remaining = number;
    for (value, symbol) in NUMERALS {
        while remaining >= value {
            out.push_str(symbol);
            remaining -= value;
        }
    }
    out
}

fn alpha(number: i64) -> String {
    let mut out = Vec::new();
    let mut remaining = number;
    while remaining > 0 {
        remaining -= 1;
        out.push(char::from(b'A' + u8::try_from(remaining.rem_euclid(26)).unwrap_or(0)));
        remaining = remaining.div_euclid(26);
    }
    out.iter().rev().collect()
}

pub(super) fn bullet_size_pt(props: &ParagraphProperties, text_size: f64) -> f64 {
    if let Some(points) = props.bullet_size_points {
        return points;
    }
    if let Some(percent) = props.bullet_size_percent {
        return text_size * (percent / 100_000.0);
    }
    text_size
}

pub(super) fn bullet_span(
    props: &ParagraphProperties,
    text_size: f64,
    first: &RunProperties,
    text: &str,
    x: f64,
    dy: Dy,
    painter: &Painter<'_>,
) -> Element {
    let mut styles: Vec<Attr> = Vec::new();
    let size = bullet_size_pt(props, text_size);
    if size != 0.0 {
        styles.push(Attr {
            name: "font-size",
            value: Value::Num(size * PX_PER_PT),
        });
    }
    // A bullet with no `buFont` is drawn in the first run's face.
    let face = mapped_symbol_face(props.bullet_font.as_deref().or(first.font_family.as_deref()), painter);
    let chain: Vec<&str> = match face {
        Some(_) => FALLBACK_FAMILIES.to_vec(),
        None => [
            props.bullet_font.as_deref(),
            first.font_family.as_deref(),
            first.font_family_ea.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect(),
    };
    if let Some(family) = font_family_value(&chain, &painter.font_mapping) {
        styles.push(Attr {
            name: "font-family",
            value: Value::Text(family),
        });
    }
    if let Some(c) = &props.bullet_colour {
        styles.push(Attr {
            name: "fill",
            value: Value::Word(c.hex.clone()),
        });
        if c.alpha < 1.0 {
            styles.push(Attr {
                name: "fill-opacity",
                value: Value::Num(c.alpha),
            });
        }
    } else if let Some(c) = &first.colour {
        styles.push(Attr {
            name: "fill",
            value: Value::Word(c.hex.clone()),
        });
    }
    let mut span = Element::open(Tag::Tspan);
    span.attrs = leading(x, Some(dy), "start");
    span.attrs.extend(styles);
    match face {
        Some(face) => span.push_text(&symbols::translate(face, text).0),
        None => span.push_text(text),
    }
    span
}
