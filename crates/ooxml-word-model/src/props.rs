//! Owns: reading `w:rPr`, `w:pPr`, `w:tblPr`, `w:trPr` and `w:tcPr` into declared
//! properties ([`Props`]) -- every property the element declares and nothing else -- with
//! docx2svg's keys and its reading of each (`docx2svg.parse.properties`), and a theme
//! colour read as Word draws it where the package's theme is known ([`ThemeColours`]).
//!
//! Must not: merge levels or apply defaults; `rStyle` / `pStyle` are structure, read by the
//! callers, not properties.

use crate::theme::ThemeColours;
use crate::value::{joined, known, text, Border, FontRef, Key, Props, Shading, TabStop, Text, Value};
use crate::xmlread::{attr, attr_int, attr_on, child, elements, is_on, local};
use ooxml_xml::Element;
use std::sync::Arc;

/// The run properties ECMA-376 calls toggle properties (17.7.3); how Word combines them is
/// measured (`crate::resolve`).
pub const TOGGLES: &[&str] = &[
    "b",
    "bCs",
    "i",
    "iCs",
    "caps",
    "smallCaps",
    "strike",
    "dstrike",
    "outline",
    "shadow",
    "emboss",
    "imprint",
    "vanish",
];
const RUN_BOOLS: &[&str] = &["webHidden", "specVanish", "noProof", "snapToGrid", "rtl", "cs", "oMath"];
const RUN_INTS: &[&str] = &["sz", "szCs", "kern", "spacing", "w", "position"];
const RUN_STRINGS: &[&str] = &["color", "u", "vertAlign", "highlight", "effect", "em"];
const PARA_BOOLS: &[&str] = &[
    "keepNext",
    "keepLines",
    "pageBreakBefore",
    "widowControl",
    "contextualSpacing",
    "bidi",
    "snapToGrid",
    "suppressAutoHyphens",
    "mirrorIndents",
    "suppressLineNumbers",
    "adjustRightInd",
    "wordWrap",
    "kinsoku",
    "overflowPunct",
    "autoSpaceDE",
    "autoSpaceDN",
];
const SLOTS: [(&str, &str, &str); 4] = [
    ("ascii", "asciiTheme", "rFonts.ascii"),
    ("hAnsi", "hAnsiTheme", "rFonts.hAnsi"),
    ("eastAsia", "eastAsiaTheme", "rFonts.eastAsia"),
    ("cs", "cstheme", "rFonts.cs"),
];

/// Whether `name` is a toggle property.
pub fn is_toggle(name: &str) -> bool {
    TOGGLES.contains(&name)
}

/// The colour of a `w:color` (or any element with `w:themeColor`): its theme colour where
/// it names one the theme has, else its `w:val`.
pub fn text_colour(element: &Element, colours: &ThemeColours) -> Option<Text> {
    let theme = attr(element, "themeColor").filter(|t| !t.is_empty());
    theme
        .and_then(|t| colours.resolve(t, attr(element, "themeTint"), attr(element, "themeShade")))
        .or_else(|| attr(element, "val").map(text))
}

fn shading(element: &Element, colours: &ThemeColours) -> Value {
    let theme = attr(element, "themeFill").filter(|t| !t.is_empty());
    let fill = theme
        .and_then(|t| colours.resolve(t, attr(element, "themeFillTint"), attr(element, "themeFillShade")))
        .or_else(|| attr(element, "fill").map(text));
    Value::Shading(Shading {
        kind: attr(element, "val").map(text),
        color: attr(element, "color").map(text),
        fill,
    })
}

fn border(element: &Element, with_colour: bool) -> Border {
    Border {
        style: attr(element, "val").map(text),
        size: attr_int(element, "sz").unwrap_or(0),
        space: attr_int(element, "space").unwrap_or(0),
        color: if with_colour {
            attr(element, "color").map(text)
        } else {
            None
        },
    }
}

fn attributes(element: &Element) -> Value {
    let mut out: Vec<(Text, Text)> = Vec::new();
    for a in element.attributes() {
        let name = a.name.local();
        match out.iter_mut().find(|(k, _)| &**k == name) {
            Some(slot) => slot.1 = a.value.clone(),
            None => out.push((text(name), a.value.clone())),
        }
    }
    Value::Attrs(Arc::from(out))
}

fn read_fonts(node: &Element, out: &mut Props) {
    for (slot, theme, name) in SLOTS {
        let face = attr(node, slot);
        let theme = attr(node, theme);
        if face.is_some() || theme.is_some() {
            out.insert(
                Key::Borrowed(name),
                Value::Font(FontRef {
                    face: face.map(text),
                    theme: theme.map(text),
                }),
            );
        }
    }
    if let Some(hint) = attr(node, "hint") {
        out.insert(Key::Borrowed("rFonts.hint"), Value::Str(text(hint)));
    }
}

fn read_run_child(node: &Element, colours: &ThemeColours, out: &mut Props) {
    let name = local(node);
    match name {
        "rFonts" => read_fonts(node, out),
        "lang" => {
            for (k, name) in [
                ("val", "lang.val"),
                ("eastAsia", "lang.eastAsia"),
                ("bidi", "lang.bidi"),
            ] {
                if let Some(v) = attr(node, k) {
                    out.insert(Key::Borrowed(name), Value::Str(text(v)));
                }
            }
        }
        "bdr" => {
            out.insert(known(name), Value::RunBorder(border(node, false)));
        }
        "shd" => {
            out.insert(known(name), shading(node, colours));
        }
        _ if is_toggle(name) || RUN_BOOLS.contains(&name) => {
            out.insert(known(name), Value::Bool(attr_on(node, "val")));
        }
        _ if RUN_INTS.contains(&name) => {
            if let Some(n) = attr_int(node, "val") {
                out.insert(known(name), Value::Int(n));
            }
        }
        _ if RUN_STRINGS.contains(&name) => {
            let value = if name == "color" {
                text_colour(node, colours)
            } else {
                attr(node, "val").map(text)
            };
            if let Some(v) = value {
                out.insert(known(name), Value::Str(v));
            }
        }
        "textFill" | "textOutline" => {
            out.insert(known(name), Value::TextEffect(text(name)));
        }
        _ => {}
    }
}

/// Every property a `w:rPr` declares.
pub fn run_properties(element: Option<&Element>, colours: &ThemeColours) -> Props {
    let mut out = Props::new();
    for node in element.into_iter().flat_map(elements) {
        read_run_child(node, colours, &mut out);
    }
    out
}

fn tab_stops(node: &Element) -> Value {
    let stops: Vec<TabStop> = elements(node)
        .filter(|t| local(t) == "tab")
        .filter_map(|t| {
            Some(TabStop {
                position: attr_int(t, "pos")?,
                alignment: text(attr(t, "val").unwrap_or("left")),
                leader: attr(t, "leader").map(text),
            })
        })
        .collect();
    Value::Tabs(Arc::from(stops))
}

fn read_spacing(node: &Element, out: &mut Props) {
    for (k, name) in [
        ("before", "spacing.before"),
        ("after", "spacing.after"),
        ("line", "spacing.line"),
        ("beforeLines", "spacing.beforeLines"),
        ("afterLines", "spacing.afterLines"),
    ] {
        if let Some(v) = attr_int(node, k) {
            out.insert(Key::Borrowed(name), Value::Int(v));
        }
    }
    if let Some(rule) = attr(node, "lineRule") {
        out.insert(Key::Borrowed("spacing.lineRule"), Value::Str(text(rule)));
    }
    for (k, name) in [
        ("beforeAutospacing", "spacing.beforeAutospacing"),
        ("afterAutospacing", "spacing.afterAutospacing"),
    ] {
        if let Some(v) = attr(node, k) {
            out.insert(Key::Borrowed(name), Value::Bool(is_on(v)));
        }
    }
}

fn read_indent(node: &Element, out: &mut Props) {
    for (name, aliases) in [
        ("ind.left", &["left", "start"][..]),
        ("ind.right", &["right", "end"][..]),
        ("ind.firstLine", &["firstLine"][..]),
        ("ind.hanging", &["hanging"][..]),
    ] {
        if let Some(v) = aliases.iter().find_map(|a| attr_int(node, a)) {
            out.insert(Key::Borrowed(name), Value::Int(v));
        }
    }
}

fn read_paragraph_child(node: &Element, colours: &ThemeColours, out: &mut Props) {
    let name = local(node);
    match name {
        "spacing" => read_spacing(node, out),
        "ind" => read_indent(node, out),
        "numPr" => {
            if let Some(n) = child(node, "numId").and_then(|e| attr_int(e, "val")) {
                out.insert(Key::Borrowed("numPr.numId"), Value::Int(n));
            }
            if let Some(n) = child(node, "ilvl").and_then(|e| attr_int(e, "val")) {
                out.insert(Key::Borrowed("numPr.ilvl"), Value::Int(n));
            }
        }
        "tabs" => {
            out.insert(Key::Borrowed("tabs"), tab_stops(node));
        }
        "framePr" => {
            out.insert(Key::Borrowed("framePr"), attributes(node));
        }
        "pBdr" => {
            for side in elements(node) {
                out.insert(joined("pBdr", local(side)), Value::Border(border(side, true)));
            }
        }
        "shd" => {
            out.insert(known(name), shading(node, colours));
        }
        "jc" | "textAlignment" => {
            if let Some(v) = attr(node, "val") {
                out.insert(known(name), Value::Str(text(v)));
            }
        }
        "outlineLvl" => {
            if let Some(v) = attr_int(node, "val") {
                out.insert(known(name), Value::Int(v));
            }
        }
        _ if PARA_BOOLS.contains(&name) => {
            out.insert(known(name), Value::Bool(attr_on(node, "val")));
        }
        _ => {}
    }
}

/// Every property a `w:pPr` declares, except its `w:rPr` and `w:sectPr`.
pub fn paragraph_properties(element: Option<&Element>, colours: &ThemeColours) -> Props {
    let mut out = Props::new();
    for node in element.into_iter().flat_map(elements) {
        read_paragraph_child(node, colours, &mut out);
    }
    out
}

/// A table width (`w:w`, `w:type`): a `pct` value written `50%` is read in fiftieths;
/// anything unreadable is 0.
fn width(node: &Element) -> Value {
    let kind = attr(node, "type").filter(|k| !k.is_empty()).unwrap_or("dxa");
    let raw = attr(node, "w").filter(|w| !w.is_empty()).unwrap_or("0");
    if let Some(number) = raw.strip_suffix('%') {
        return match py_float(number) {
            Some(f) => Value::Width(round_half_even(f * 50.0), text("pct")),
            None => Value::Width(0, text(kind)),
        };
    }
    Value::Width(py_float(raw).map_or(0, truncate), text(kind))
}

/// A decimal number as Python's `float()` reads the forms a document holds (sign, digits,
/// a point, an exponent; surrounding whitespace).
fn py_float(s: &str) -> Option<f64> {
    let s = s.trim();
    let plain = s
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E' | b'_'));
    if !plain || s.is_empty() {
        return None;
    }
    s.replace('_', "").parse::<f64>().ok().filter(|f| f.is_finite())
}

#[allow(clippy::cast_possible_truncation)]
fn truncate(f: f64) -> i64 {
    f.trunc() as i64
}

#[allow(clippy::cast_possible_truncation)]
fn round_half_even(f: f64) -> i64 {
    f.round_ties_even() as i64
}

const TABLE_WIDTHS: &[&str] = &["tblW", "tblInd", "tblCellSpacing", "tcW", "wBefore", "wAfter"];
const TABLE_STRINGS: &[&str] = &["jc", "vAlign", "textDirection", "hMerge"];
const TABLE_INTS: &[&str] = &[
    "gridSpan",
    "gridBefore",
    "gridAfter",
    "tblStyleRowBandSize",
    "tblStyleColBandSize",
];
const TABLE_BOOLS: &[&str] = &[
    "cantSplit",
    "tblHeader",
    "noWrap",
    "hidden",
    "bidiVisual",
    "tcFitText",
    "hideMark",
];

fn side_name(side: &str) -> &str {
    match side {
        "start" => "left",
        "end" => "right",
        other => other,
    }
}

fn read_table_child(node: &Element, colours: &ThemeColours, out: &mut Props) {
    let name = local(node);
    match name {
        _ if TABLE_WIDTHS.contains(&name) => {
            out.insert(known(name), width(node));
        }
        "tblCellMar" | "tcMar" => {
            for side in elements(node) {
                out.insert(joined(name, side_name(local(side))), width(side));
            }
        }
        "tblBorders" | "tcBorders" => {
            for side in elements(node) {
                out.insert(joined(name, side_name(local(side))), Value::Border(border(side, true)));
            }
        }
        "tblLayout" => {
            out.insert(
                known(name),
                Value::Str(text(attr(node, "type").filter(|t| !t.is_empty()).unwrap_or("autofit"))),
            );
        }
        "trHeight" => {
            let rule = attr(node, "hRule").filter(|r| !r.is_empty()).unwrap_or("atLeast");
            out.insert(
                known(name),
                Value::Height(attr_int(node, "val").unwrap_or(0), text(rule)),
            );
        }
        "vMerge" => {
            out.insert(
                known(name),
                Value::Str(text(attr(node, "val").filter(|v| !v.is_empty()).unwrap_or("continue"))),
            );
        }
        "shd" => {
            out.insert(known(name), shading(node, colours));
        }
        "tblpPr" => {
            out.insert(known(name), attributes(node));
        }
        _ if TABLE_STRINGS.contains(&name) => {
            if let Some(v) = attr(node, "val") {
                out.insert(known(name), Value::Str(text(v)));
            }
        }
        _ if TABLE_INTS.contains(&name) => {
            if let Some(v) = attr_int(node, "val") {
                out.insert(known(name), Value::Int(v));
            }
        }
        _ if TABLE_BOOLS.contains(&name) => {
            out.insert(known(name), Value::Bool(attr_on(node, "val")));
        }
        _ => {}
    }
}

/// Every property a `w:tblPr`, `w:trPr` or `w:tcPr` declares (one shared vocabulary).
pub fn table_properties(element: Option<&Element>, colours: &ThemeColours) -> Props {
    let mut out = Props::new();
    for node in element.into_iter().flat_map(elements) {
        read_table_child(node, colours, &mut out);
    }
    out
}
