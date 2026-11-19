//! Owns: direct formatting as docx-agent's `edit/formatting.py` writes it -- the run and
//! paragraph properties `word_format` sets or clears ([`Setting`], in the order given,
//! which is the order attributes are written in), their checks with docx-agent's
//! messages ([`check`]), their writing on a run, a paragraph mark or a paragraph
//! ([`write_run`], [`write_paragraph`]) and the clearing of direct formatting
//! ([`clear_run`], [`clear_paragraph`]).
//!
//! Must not: record revisions (the property tracker does), or read outside the element
//! it writes; a theme colour's value is given ([`ThemeColour`]).

use crate::marks::insert_in_order;
use crate::names::W;
use crate::validate::Orders;
use crate::work::{Id, Work};
use crate::{EditError, Result};

/// A direct property `word_format` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Property {
    /// `w:b` and `w:bCs`.
    Bold,
    /// `w:i` and `w:iCs`.
    Italic,
    /// `w:u`.
    Underline,
    /// `w:strike`.
    Strike,
    /// `w:sz` and `w:szCs`, points.
    Size,
    /// `w:rFonts`' ASCII, high-ANSI and complex-script faces.
    Font,
    /// `w:color`: `RRGGBB`, `auto` or a theme colour.
    Color,
    /// `w:highlight`.
    Highlight,
    /// `w:jc`: `left`, `center`, `right`, `justify`.
    Alignment,
    /// `w:spacing/@w:before`, points.
    SpaceBefore,
    /// `w:spacing/@w:after`, points.
    SpaceAfter,
    /// `w:spacing/@w:line`, a multiple.
    LineSpacing,
    /// `w:ind/@w:left`, points.
    IndentLeft,
    /// `w:ind/@w:right`, points.
    IndentRight,
    /// `w:ind/@w:firstLine`, points.
    FirstLine,
    /// `w:ind/@w:hanging`, points.
    Hanging,
    /// `w:keepNext`.
    KeepWithNext,
    /// `w:keepLines`.
    KeepTogether,
    /// `w:pageBreakBefore`.
    PageBreakBefore,
}

impl Property {
    /// Whether it is a run property.
    #[must_use]
    pub fn is_run(self) -> bool {
        matches!(
            self,
            Self::Bold
                | Self::Italic
                | Self::Underline
                | Self::Strike
                | Self::Size
                | Self::Font
                | Self::Color
                | Self::Highlight
        )
    }

    /// docx-agent's keyword for it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Underline => "underline",
            Self::Strike => "strike",
            Self::Size => "size",
            Self::Font => "font",
            Self::Color => "color",
            Self::Highlight => "highlight",
            Self::Alignment => "alignment",
            Self::SpaceBefore => "space_before",
            Self::SpaceAfter => "space_after",
            Self::LineSpacing => "line_spacing",
            Self::IndentLeft => "indent_left",
            Self::IndentRight => "indent_right",
            Self::FirstLine => "first_line",
            Self::Hanging => "hanging",
            Self::KeepWithNext => "keep_with_next",
            Self::KeepTogether => "keep_together",
            Self::PageBreakBefore => "page_break_before",
        }
    }

    /// The `w:pPr` toggle it is.
    fn paragraph_toggle(self) -> Option<&'static str> {
        match self {
            Self::KeepWithNext => Some("keepNext"),
            Self::KeepTogether => Some("keepLines"),
            Self::PageBreakBefore => Some("pageBreakBefore"),
            _ => None,
        }
    }
}

/// A property's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// On or off.
    Flag(bool),
    /// A number.
    Number(f64),
    /// A name or colour.
    Text(String),
}

/// A property set to a value, or cleared (`None`: inherit again).
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    /// Which.
    pub property: Property,
    /// Its value; `None` removes it.
    pub value: Option<Value>,
}

/// A theme colour's value, as Word writes it in `w:val`.
pub type ThemeColour<'a> = &'a dyn Fn(&str) -> Option<String>;

const HIGHLIGHTS: [&str; 17] = [
    "black",
    "blue",
    "cyan",
    "green",
    "magenta",
    "red",
    "yellow",
    "white",
    "darkBlue",
    "darkCyan",
    "darkGreen",
    "darkMagenta",
    "darkRed",
    "darkYellow",
    "darkGray",
    "lightGray",
    "none",
];
const THEME_COLORS: [&str; 16] = [
    "dark1",
    "light1",
    "dark2",
    "light2",
    "accent1",
    "accent2",
    "accent3",
    "accent4",
    "accent5",
    "accent6",
    "hyperlink",
    "followedHyperlink",
    "background1",
    "text1",
    "background2",
    "text2",
];
const THEME_ALIASES: [(&str, &str); 10] = [
    ("tx1", "text1"),
    ("tx2", "text2"),
    ("bg1", "background1"),
    ("bg2", "background2"),
    ("dk1", "dark1"),
    ("lt1", "light1"),
    ("dk2", "dark2"),
    ("lt2", "light2"),
    ("hlink", "hyperlink"),
    ("folhlink", "followedHyperlink"),
];

/// The theme colour `value` names, as Word writes it in `w:themeColor` (docx-agent's
/// `theme_color_name`: ignoring case and spaces, or a DrawingML name for one).
#[must_use]
pub fn theme_color_name(value: &str) -> Option<&'static str> {
    if let Some(name) = THEME_COLORS.iter().find(|n| **n == value) {
        return Some(name);
    }
    let folded: String = value.split_whitespace().collect::<String>().to_lowercase();
    if let Some(name) = THEME_COLORS.iter().find(|n| n.to_lowercase() == folded) {
        return Some(name);
    }
    THEME_ALIASES.iter().find(|(a, _)| *a == folded).map(|(_, n)| *n)
}

fn sorted(names: &[&str]) -> String {
    let mut names = names.to_vec();
    names.sort_unstable();
    names.join(", ")
}

fn hex(value: &str) -> bool {
    value.len() == 6 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn refused(message: String) -> EditError {
    EditError::Refused(message)
}

/// Refuse a value Word would not accept (docx-agent's `check_run` and `check_paragraph`).
///
/// # Errors
/// docx-agent's message for the first value refused.
pub fn check(settings: &[Setting]) -> Result<()> {
    for setting in settings {
        let Some(value) = &setting.value else { continue };
        check_one(setting.property, value)?;
    }
    let given = |p: Property| settings.iter().any(|s| s.property == p && s.value.is_some());
    if given(Property::FirstLine) && given(Property::Hanging) {
        return Err(refused(
            "a paragraph has a first-line indent or a hanging one, not both".to_owned(),
        ));
    }
    Ok(())
}

fn check_one(property: Property, value: &Value) -> Result<()> {
    let number = |low: f64, high: f64| matches!(value, Value::Number(n) if (low..=high).contains(n));
    let name = property.name();
    let ok = match property {
        Property::Bold | Property::Italic | Property::Strike => matches!(value, Value::Flag(_)),
        Property::KeepWithNext | Property::KeepTogether | Property::PageBreakBefore => {
            matches!(value, Value::Flag(_))
        }
        Property::Underline => matches!(value, Value::Flag(_)),
        Property::Size => number(1.0, 1638.0),
        Property::Font => matches!(value, Value::Text(t) if !t.trim().is_empty() && t.chars().count() <= 31),
        Property::Color => {
            matches!(value, Value::Text(t) if hex(t) || t == "auto" || theme_color_name(t).is_some())
        }
        Property::Highlight => matches!(value, Value::Text(t) if HIGHLIGHTS.contains(&t.as_str())),
        Property::Alignment => {
            matches!(value, Value::Text(t) if ["left", "center", "right", "justify", "distribute"].contains(&t.as_str()))
        }
        Property::IndentLeft | Property::IndentRight => number(-1584.0, 1584.0),
        Property::FirstLine | Property::Hanging | Property::SpaceBefore | Property::SpaceAfter => number(0.0, 1584.0),
        Property::LineSpacing => number(0.06, 132.0),
    };
    if ok {
        return Ok(());
    }
    Err(refused(match property {
        Property::Bold
        | Property::Italic
        | Property::Strike
        | Property::KeepWithNext
        | Property::KeepTogether
        | Property::PageBreakBefore => format!("{name} is True, False or None"),
        Property::Underline => "underline is True, False, None or one of dash, dashDotDotHeavy, dashDotHeavy, \
             dashLong, dashLongHeavy, dashedHeavy, dotDash, dotDotDash, dotted, dottedHeavy, double, none, single, \
             thick, wave, wavyDouble, wavyHeavy, words"
            .to_owned(),
        Property::Size => "size is in points, 1 to 1638".to_owned(),
        Property::Font => "font is a face name of at most 31 characters".to_owned(),
        Property::Color => format!(
            "color is RRGGBB, 'auto' or a theme colour ({}; tx1, bg1... are read as text1, background1...)",
            sorted(&THEME_COLORS)
        ),
        Property::Highlight => format!("highlight is one of {}", sorted(&HIGHLIGHTS)),
        Property::Alignment => "alignment is one of left, center, right, justify, distribute".to_owned(),
        Property::IndentLeft | Property::IndentRight => format!("{name} is in points, -1584 to 1584"),
        Property::FirstLine | Property::Hanging | Property::SpaceBefore | Property::SpaceAfter => {
            format!("{name} is in points, 0 to 1584")
        }
        Property::LineSpacing => {
            "line_spacing is a multiple (1.0, 1.5...), or ('exact' | 'at_least', points)".to_owned()
        }
    }))
}

/// Python's `str(int(round(value * scale)))`: half to even.
fn scaled(value: f64, scale: f64) -> String {
    // `+ 0.0` makes a negative zero positive, as `int()` does
    let rounded = (value * scale).round_ties_even() + 0.0;
    format!("{rounded:.0}")
}

/// `owner`'s `w:{local}` (`rPr`, `pPr`), made in schema order when asked.
pub(crate) fn properties_of(work: &mut Work, owner: Id, local: &str, create: bool, orders: &Orders) -> Option<Id> {
    if let Some(found) = work.find(owner, W, local) {
        return Some(found);
    }
    if !create {
        return None;
    }
    let made = work.new_element(work.element_name(owner, W, local, "w"));
    insert_in_order(work, owner, made, orders);
    Some(made)
}

/// docx-agent's `_set_child`: `parent`'s `w:{local}` removed (`None`), or kept in its place
/// (made in schema order) with each attribute set, or removed where its value is `None`.
pub(crate) fn set_child(
    work: &mut Work,
    parent: Id,
    local: &str,
    attributes: Option<&[(&str, Option<&str>)]>,
    orders: &Orders,
) -> Option<Id> {
    let node = work.find(parent, W, local);
    let Some(attributes) = attributes else {
        if let Some(node) = node {
            work.remove(node);
        }
        return None;
    };
    let node = node.unwrap_or_else(|| {
        let made = work.new_element(work.element_name(parent, W, local, "w"));
        insert_in_order(work, parent, made, orders);
        made
    });
    for (name, value) in attributes {
        match value {
            Some(value) => {
                let qname = work.qname(W, name, "w");
                work.set(node, qname, value);
            }
            None => work.remove_attr(node, Some(W), name),
        }
    }
    Some(node)
}

/// A toggle as Word writes it: `None` removed, on without `w:val`, off `w:val="0"`.
fn toggle(work: &mut Work, parent: Id, local: &str, value: Option<bool>, orders: &Orders) {
    let Some(on) = value else {
        set_child(work, parent, local, None, orders);
        return;
    };
    let value = if on { None } else { Some("0") };
    set_child(work, parent, local, Some(&[("val", value)]), orders);
}

fn flag(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Flag(on)) => Some(*on),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> Option<&str> {
    match value {
        Some(Value::Text(t)) => Some(t),
        _ => None,
    }
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(n)) => Some(*n),
        _ => None,
    }
}

/// Write run `settings` on `owner` (a `w:r`, or a `w:pPr` for its mark): its `w:rPr`
/// made as needed and removed when left empty (docx-agent's `write_run`).
pub(crate) fn write_run(work: &mut Work, owner: Id, settings: &[Setting], colour: ThemeColour<'_>, orders: &Orders) {
    let create = settings.iter().any(|s| s.value.is_some());
    let Some(properties) = properties_of(work, owner, "rPr", create, orders) else {
        return;
    };
    for setting in settings {
        let value = setting.value.as_ref();
        match setting.property {
            Property::Bold => {
                toggle(work, properties, "b", flag(value), orders);
                toggle(work, properties, "bCs", flag(value), orders);
            }
            Property::Italic => {
                toggle(work, properties, "i", flag(value), orders);
                toggle(work, properties, "iCs", flag(value), orders);
            }
            Property::Strike => toggle(work, properties, "strike", flag(value), orders),
            Property::Underline => {
                let val = flag(value).map(|on| if on { "single" } else { "none" });
                set_child(
                    work,
                    properties,
                    "u",
                    val.map(|v| [("val", Some(v))]).as_ref().map(|a| &a[..]),
                    orders,
                );
            }
            Property::Size => {
                let half = number(value).map(|v| scaled(v, 2.0));
                for local in ["sz", "szCs"] {
                    let attrs = half.as_deref().map(|h| [("val", Some(h))]);
                    set_child(work, properties, local, attrs.as_ref().map(|a| &a[..]), orders);
                }
            }
            Property::Font => write_font(work, properties, text(value), orders),
            Property::Color => write_colour(work, properties, text(value), colour, orders),
            Property::Highlight => {
                let attrs = text(value).map(|v| [("val", Some(v))]);
                set_child(work, properties, "highlight", attrs.as_ref().map(|a| &a[..]), orders);
            }
            _ => {}
        }
    }
    if work.children(properties).is_empty() {
        work.remove(properties);
    }
}

fn write_font(work: &mut Work, properties: Id, face: Option<&str>, orders: &Orders) {
    let Some(face) = face else {
        if let Some(node) = work.find(properties, W, "rFonts") {
            for local in ["ascii", "hAnsi", "cs", "asciiTheme", "hAnsiTheme", "cstheme"] {
                work.remove_attr(node, Some(W), local);
            }
            if work.attributes(node).is_empty() {
                work.remove(node);
            }
        }
        return;
    };
    let attrs = [
        ("ascii", Some(face)),
        ("hAnsi", Some(face)),
        ("cs", Some(face)),
        ("asciiTheme", None),
        ("hAnsiTheme", None),
        ("cstheme", None),
    ];
    set_child(work, properties, "rFonts", Some(&attrs), orders);
}

fn write_colour(work: &mut Work, properties: Id, value: Option<&str>, colour: ThemeColour<'_>, orders: &Orders) {
    let Some(value) = value else {
        set_child(work, properties, "color", None, orders);
        return;
    };
    if let Some(name) = theme_color_name(value) {
        let rgb = colour(name).unwrap_or_else(|| "000000".to_owned());
        let attrs = [
            ("val", Some(rgb.as_str())),
            ("themeColor", Some(name)),
            ("themeTint", None),
            ("themeShade", None),
        ];
        set_child(work, properties, "color", Some(&attrs), orders);
        return;
    }
    let val = if value == "auto" {
        value.to_owned()
    } else {
        value.to_uppercase()
    };
    let attrs = [
        ("val", Some(val.as_str())),
        ("themeColor", None),
        ("themeTint", None),
        ("themeShade", None),
    ];
    set_child(work, properties, "color", Some(&attrs), orders);
}

/// Write paragraph `settings` on a `w:p` (its `w:pPr`, removed when left empty;
/// docx-agent's `write_paragraph`).
pub(crate) fn write_paragraph(work: &mut Work, paragraph: Id, settings: &[Setting], orders: &Orders) {
    let create = settings.iter().any(|s| s.value.is_some());
    let Some(properties) = properties_of(work, paragraph, "pPr", create, orders) else {
        return;
    };
    for setting in settings {
        let value = setting.value.as_ref();
        if let Some(local) = setting.property.paragraph_toggle() {
            toggle(work, properties, local, flag(value), orders);
            continue;
        }
        match setting.property {
            Property::Alignment => {
                let jc = text(value).map(|v| if v == "justify" { "both" } else { v });
                let attrs = jc.map(|v| [("val", Some(v))]);
                set_child(work, properties, "jc", attrs.as_ref().map(|a| &a[..]), orders);
            }
            Property::IndentLeft | Property::IndentRight | Property::FirstLine | Property::Hanging => {
                write_indent(work, properties, setting.property, number(value), orders);
            }
            Property::SpaceBefore | Property::SpaceAfter => {
                write_space(work, properties, setting.property, number(value), orders);
            }
            Property::LineSpacing => write_line(work, properties, number(value), orders),
            _ => {}
        }
    }
    if work.children(properties).is_empty() {
        work.remove(properties);
    }
}

fn write_indent(work: &mut Work, properties: Id, property: Property, value: Option<f64>, orders: &Orders) {
    let twips = value.map(|v| scaled(v, 20.0));
    if work.find(properties, W, "ind").is_none() && twips.is_none() {
        return;
    }
    let (own, other) = match property {
        Property::IndentLeft => ("left", Some("start")),
        Property::IndentRight => ("right", Some("end")),
        Property::FirstLine => ("firstLine", twips.as_ref().map(|_| "hanging")),
        _ => ("hanging", twips.as_ref().map(|_| "firstLine")),
    };
    let mut attrs = vec![(own, twips.as_deref())];
    if let Some(other) = other {
        attrs.push((other, None));
    }
    if let Some(node) = set_child(work, properties, "ind", Some(&attrs), orders) {
        if work.attributes(node).is_empty() {
            work.remove(node);
        }
    }
}

fn write_space(work: &mut Work, properties: Id, property: Property, value: Option<f64>, orders: &Orders) {
    let twips = value.map(|v| scaled(v, 20.0));
    if work.find(properties, W, "spacing").is_none() && twips.is_none() {
        return;
    }
    let (side, auto) = if property == Property::SpaceBefore {
        ("before", "beforeAutospacing")
    } else {
        ("after", "afterAutospacing")
    };
    let attrs = [(side, twips.as_deref()), (auto, None)];
    if let Some(node) = set_child(work, properties, "spacing", Some(&attrs), orders) {
        if work.attributes(node).is_empty() {
            work.remove(node);
        }
    }
}

fn write_line(work: &mut Work, properties: Id, value: Option<f64>, orders: &Orders) {
    let node = work.find(properties, W, "spacing");
    let Some(value) = value else {
        if let Some(node) = node {
            work.remove_attr(node, Some(W), "line");
            work.remove_attr(node, Some(W), "lineRule");
            if work.attributes(node).is_empty() {
                work.remove(node);
            }
        }
        return;
    };
    let line = scaled(value, 240.0);
    let attrs = [("line", Some(line.as_str())), ("lineRule", Some("auto"))];
    set_child(work, properties, "spacing", Some(&attrs), orders);
}

/// What clearing direct formatting keeps of a run's properties.
const KEPT_RUN: [&str; 11] = [
    "rStyle",
    "lang",
    "noProof",
    "rtl",
    "cs",
    "oMath",
    "rPrChange",
    "ins",
    "del",
    "moveFrom",
    "moveTo",
];
/// ... and of a paragraph's.
const KEPT_PARAGRAPH: [&str; 8] = [
    "pStyle",
    "numPr",
    "sectPr",
    "pPrChange",
    "cnfStyle",
    "divId",
    "framePr",
    "rPr",
];

/// Remove `properties`' element children but those `kept` names (in `w:`).
fn strip(work: &mut Work, properties: Id, kept: &[&str]) {
    let children: Vec<Id> = work.children(properties).to_vec();
    for child in children {
        let keep = work
            .name(child)
            .is_none_or(|n| n.namespace() == Some(W) && kept.contains(&n.local()));
        if work.is_element(child) && !keep {
            work.remove(child);
        }
    }
    if work.children(properties).is_empty() {
        work.remove(properties);
    }
}

/// Remove a run's (or a mark's `w:rPr` owner's) direct formatting (docx-agent's
/// `clear_run`).
pub(crate) fn clear_run(work: &mut Work, run: Id) {
    if let Some(properties) = work.find(run, W, "rPr") {
        strip(work, properties, &KEPT_RUN);
    }
}

/// Remove a paragraph's direct paragraph formatting and its mark's (docx-agent's
/// `clear_paragraph`).
pub(crate) fn clear_paragraph(work: &mut Work, paragraph: Id) {
    let Some(properties) = work.find(paragraph, W, "pPr") else {
        return;
    };
    let children: Vec<Id> = work.children(properties).to_vec();
    for child in children {
        let keep = work
            .name(child)
            .is_none_or(|n| n.namespace() == Some(W) && KEPT_PARAGRAPH.contains(&n.local()));
        if work.is_element(child) && !keep {
            work.remove(child);
        }
    }
    if let Some(mark) = work.find(properties, W, "rPr") {
        strip(work, mark, &KEPT_RUN);
    }
    if work.children(properties).is_empty() {
        work.remove(properties);
    }
}
