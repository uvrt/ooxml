//! Owns: what a chart's formatting writes (ooxml-edit's `charts/format.py`): a colour
//! specification read ([`Colour`], `color_element`), a shape's fill and line, a marker,
//! gridlines, and one text-properties element's characters -- each where its schema
//! sequence puts it.
//!
//! Must not: find what it formats (the axes, series and text: `axes`, `looks`, `text_style`).

use super::make::Make;
use super::owned::{Order, Owned};
use super::DataError;
use crate::pyfmt::round_half_even;
use ooxml_base::pyfmt::{repr_str, PyNumber};
use ooxml_xml::view::{py_flat, py_int};

/// The theme colours a chart may name (`SCHEME_COLORS`), sorted as Python sorts them.
const SCHEME: [&str; 16] = [
    "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "bg1", "bg2", "dk1", "dk2", "folHlink", "hlink",
    "lt1", "lt2", "tx1", "tx2",
];
/// Office's own names for the theme colours, folded (`THEME_COLOR_ALIASES`).
const ALIASES: [(&str, &str); 10] = [
    ("text1", "tx1"),
    ("text2", "tx2"),
    ("background1", "bg1"),
    ("background2", "bg2"),
    ("dark1", "dk1"),
    ("light1", "lt1"),
    ("dark2", "dk2"),
    ("light2", "lt2"),
    ("hyperlink", "hlink"),
    ("followedhyperlink", "folHlink"),
];
/// Colour modifiers with a value (`_VALUED`), and without (`_FLAGS`).
const VALUED: [&str; 23] = [
    "tint", "shade", "alpha", "alphaOff", "alphaMod", "hue", "hueOff", "hueMod", "sat", "satOff", "satMod", "lum",
    "lumOff", "lumMod", "red", "redOff", "redMod", "green", "greenOff", "greenMod", "blue", "blueOff", "blueMod",
];
const FLAGS: [&str; 5] = ["comp", "inv", "gray", "gamma", "invGamma"];

/// The fills (`_FILL_TAGS`).
pub(crate) const FILLS: &[&str] = &["noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill"];
/// `c:spPr`'s sequence (`_SHAPE_ORDER`).
pub(crate) const SHAPE_ORDER: Order = &[
    &["xfrm"],
    &["custGeom", "prstGeom"],
    FILLS,
    &["ln"],
    &["effectLst", "effectDag"],
    &["scene3d"],
    &["sp3d"],
    &["extLst"],
];
/// `a:ln`'s fills, and its sequence (`_LINE_ORDER`).
const LINE_FILLS: &[&str] = &["noFill", "solidFill", "gradFill", "pattFill"];
const LINE_ORDER: Order = &[
    LINE_FILLS,
    &["prstDash", "custDash"],
    &["round", "bevel", "miter"],
    &["headEnd"],
    &["tailEnd"],
    &["extLst"],
];
/// A text run's character properties' sequence (`_CHARACTER_ORDER`).
const CHARACTER_ORDER: Order = &[
    &["ln"],
    FILLS,
    &["effectLst", "effectDag"],
    &["highlight"],
    &["uLnTx", "uLn"],
    &["uFillTx", "uFill"],
    &["latin"],
    &["ea"],
    &["cs"],
    &["sym"],
    &["hlinkClick"],
    &["hlinkMouseOver"],
    &["rtl"],
    &["extLst"],
];
/// `c:marker`'s sequence (`MARKER_ORDER`).
const MARKER_ORDER: Order = &[&["symbol"], &["size"], &["spPr"], &["extLst"]];
/// EMU in a point.
const EMU_PER_POINT: f64 = 12700.0;

/// A colour specification read (`color_element`): a theme colour or a hex one, with its
/// modifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Colour {
    /// `schemeClr` or `srgbClr`.
    kind: &'static str,
    /// Its `val`.
    value: String,
    /// Each modifier's name and value (`None`: a flag).
    modifiers: Vec<(String, Option<String>)>,
}

impl Colour {
    /// The colour `spec` gives: `tx2`, `accent1 lumMod=60% lumOff=40%`, `1F4E79`, Office's
    /// names (`Text 2`, `background1`) read as the theme's.
    ///
    /// # Errors
    /// [`DataError`] (ooxml-edit's `FormatError`) for text that is not a colour.
    pub fn parse(spec: &str) -> Result<Colour, DataError> {
        let flat = py_flat(spec);
        if flat.is_empty() {
            return Err(DataError(format!(
                "{} is not a colour; give a theme colour such as accent1 or tx2 (with modifiers like lumMod=75%), or a hex colour such as 1F4E79",
                repr_str(spec)
            )));
        }
        let mut parts: Vec<&str> = flat.split(' ').collect();
        let mut base = parts.remove(0).to_owned();
        if let Some(first) = parts.first() {
            let joined = format!("{base}{first}");
            if first.chars().all(|c| c.is_ascii_digit()) && scheme_colour(&joined).is_some() {
                base = joined;
                parts.remove(0);
            }
        }
        let (kind, value) = if let Some(scheme) = scheme_colour(&base) {
            ("schemeClr", scheme.to_owned())
        } else {
            let hex = base.strip_prefix('#').unwrap_or(&base);
            if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(DataError(format!(
                    "{} is neither a hex colour nor a theme colour ({}; Office's names such as text1 or background2 are read as these)",
                    repr_str(&base),
                    SCHEME.join(", ")
                )));
            }
            ("srgbClr", hex.to_ascii_uppercase())
        };
        let modifiers = parts.into_iter().map(modifier).collect::<Result<_, _>>()?;
        Ok(Colour { kind, value, modifiers })
    }

    /// The colour element (`a:schemeClr`, `a:srgbClr`).
    pub(crate) fn element(&self, make: &Make) -> Owned {
        let mut out = make.a(self.kind).with_attr("val", &self.value);
        for (name, value) in &self.modifiers {
            let modifier = make.a(name);
            out.push(match value {
                Some(value) => modifier.with_attr("val", value),
                None => modifier,
            });
        }
        out
    }

    /// `a:solidFill` of this colour.
    fn solid(&self, make: &Make) -> Owned {
        make.a("solidFill").with(self.element(make))
    }
}

/// The scheme name `name` means (`scheme_color`), ignoring case and spaces, or an Office name
/// for one.
fn scheme_colour(name: &str) -> Option<&'static str> {
    if let Some(exact) = SCHEME.iter().find(|s| **s == name) {
        return Some(exact);
    }
    let folded: String = py_flat(name).replace(' ', "").to_lowercase();
    if let Some(found) = SCHEME.iter().find(|s| s.to_lowercase() == folded) {
        return Some(found);
    }
    ALIASES.iter().find(|(k, _)| *k == folded).map(|(_, v)| *v)
}

/// One modifier of a specification: `lumMod=75%` (thousandths of a percent), `alpha=50000`,
/// a flag (`inv`).
fn modifier(text: &str) -> Result<(String, Option<String>), DataError> {
    let (name, raw) = match text.split_once('=') {
        Some((name, raw)) => (name, Some(raw)),
        None => (text, None),
    };
    let Some(raw) = raw else {
        if FLAGS.contains(&name) {
            return Ok((name.to_owned(), None));
        }
        return Err(DataError(format!("unknown colour modifier {}", repr_str(text))));
    };
    if !VALUED.contains(&name) {
        return Err(DataError(format!("unknown colour modifier {}", repr_str(text))));
    }
    let raw = py_flat(raw);
    let value = match raw.strip_suffix('%') {
        Some(number) => py_float(number).filter(|v| v.is_finite()).map(|v| {
            #[allow(clippy::cast_possible_truncation)] // a percentage, whole after rounding
            let whole = round_half_even(v * 1000.0) as i64;
            whole
        }),
        None => py_int(&raw),
    };
    let Some(value) = value else {
        return Err(DataError(format!(
            "colour modifier {}: a percentage or an integer",
            repr_str(text)
        )));
    };
    Ok((name.to_owned(), Some(value.to_string())))
}

/// Python's `float(text)` for the numbers a specification gives (`_` between digits
/// allowed, as Python allows it).
fn py_float(text: &str) -> Option<f64> {
    let text = text.trim();
    let chars: Vec<char> = text.chars().collect();
    let digit = |at: Option<usize>| at.and_then(|k| chars.get(k)).is_some_and(char::is_ascii_digit);
    let joined = chars
        .iter()
        .enumerate()
        .all(|(i, c)| *c != '_' || (digit(i.checked_sub(1)) && digit(Some(i + 1))));
    if !joined {
        return None;
    }
    text.replace('_', "").parse().ok()
}

/// A line width in points as EMU (`points_to_emu`).
///
/// # Errors
/// [`DataError`] past 0-1584 points.
pub fn points_to_emu(given: PyNumber) -> Result<i64, DataError> {
    let points = given.value();
    if !(0.0..=1584.0).contains(&points) {
        return Err(DataError(format!("a line width is in points, 0-1584 (got {given})")));
    }
    #[allow(clippy::cast_possible_truncation)] // at most 1584 * 12700
    Ok(round_half_even(points * EMU_PER_POINT) as i64)
}

/// A text size in points as hundredths (`text_size`).
///
/// # Errors
/// [`DataError`] past 1-400 points.
pub fn text_size(given: PyNumber) -> Result<i64, DataError> {
    let points = given.value();
    if !(1.0..=400.0).contains(&points) {
        return Err(DataError(format!("a text size is in points, 1-400 (got {given})")));
    }
    #[allow(clippy::cast_possible_truncation)] // at most 40000
    Ok(round_half_even(points * 100.0) as i64)
}

/// Take out every child of `parent` named one of `names` (`_replace`), then put `new` in
/// where `order` wants it.
pub(crate) fn replace(parent: &mut Owned, names: &[&str], new: Option<Owned>, order: Order) {
    let mut places: Vec<usize> = parent
        .elements()
        .into_iter()
        .filter(|(_, name)| names.contains(&name.as_str()))
        .map(|(at, _)| at)
        .collect();
    places.reverse();
    for at in places {
        parent.remove(at);
    }
    if let Some(new) = new {
        parent.insert_ordered(Some(order), new);
    }
}

/// The child `name` of `parent`, made by `new` where `order` puts it where there is none.
pub(crate) fn made<'a>(parent: &'a mut Owned, name: &str, order: Order, new: impl FnOnce() -> Owned) -> &'a mut Owned {
    parent.child_or_insert(name, Some(order), new)
}

/// A solid fill of `colour`, or none (`set_fill`).
pub(crate) fn set_fill(shape: &mut Owned, make: &Make, colour: Option<&Colour>) {
    let new = colour.map_or_else(|| make.a("noFill"), |c| c.solid(make));
    replace(shape, FILLS, Some(make.top(new)), SHAPE_ORDER);
}

/// The line (`set_line`): its colour (`keep_colour`: as it is; `None`: none), its width in
/// EMU and its dash (`ST_PresetLineDashVal`), where given.
pub(crate) fn set_line(
    shape: &mut Owned,
    make: &Make,
    colour: Option<&Colour>,
    keep_colour: bool,
    width: Option<i64>,
    dash: Option<&str>,
) {
    let line = made(shape, "ln", SHAPE_ORDER, || make.top(make.a("ln")));
    if !keep_colour {
        let new = colour.map_or_else(|| make.a("noFill"), |c| c.solid(make));
        replace(line, LINE_FILLS, Some(new), LINE_ORDER);
    }
    if let Some(width) = width {
        line.set_attr("w", &width.to_string());
    }
    if let Some(dash) = dash {
        let new = make.a("prstDash").with_attr("val", dash);
        replace(line, &["prstDash", "custDash"], Some(new), LINE_ORDER);
    }
}

/// An empty `a:effectLst` where the shape has none (as Office writes one).
pub(crate) fn effects(shape: &mut Owned, make: &Make) {
    if shape.place("effectLst").is_none() {
        shape.insert_ordered(Some(SHAPE_ORDER), make.top(make.a("effectLst")));
    }
}

/// What a marker is given (`set_marker`): each `None` stays as it is.
#[derive(Debug, Clone, Default)]
pub(crate) struct MarkerLook<'a> {
    pub(crate) symbol: Option<&'a str>,
    pub(crate) size: Option<i64>,
    /// Its fill: a colour or (`Some(None)`) none.
    pub(crate) fill: Option<Option<&'a Colour>>,
    /// Its outline: a colour or none.
    pub(crate) line: Option<Option<&'a Colour>>,
}

/// `owner`'s `c:marker` (made where `order` puts it), given `look`.
pub(crate) fn set_marker(owner: &mut Owned, order: Order, make: &Make, look: &MarkerLook<'_>) {
    let marker = made(owner, "marker", order, || make.c("marker"));
    if let Some(symbol) = look.symbol {
        replace(
            marker,
            &["symbol"],
            Some(make.c("symbol").with_attr("val", symbol)),
            MARKER_ORDER,
        );
    }
    if let Some(size) = look.size {
        let new = make.c("size").with_attr("val", &size.to_string());
        replace(marker, &["size"], Some(new), MARKER_ORDER);
    }
    if look.fill.is_none() && look.line.is_none() {
        return;
    }
    let shape = made(marker, "spPr", MARKER_ORDER, || make.c("spPr"));
    if let Some(fill) = look.fill {
        set_fill(shape, make, fill);
    }
    if let Some(line) = look.line {
        let unset = shape
            .place("ln")
            .and_then(|at| shape.read(at))
            .is_none_or(|ln| ooxml_xml::view::attr(&ln, "w").is_none());
        let width = (line.is_some() && unset).then_some(9525);
        set_line(shape, make, line, false, width, None);
    }
    effects(shape, make);
}

/// `c:majorGridlines` (`major`) or `c:minorGridlines` as Office draws them (`gridline`):
/// 0.75 pt at 15% or 5% of the text colour.
pub(crate) fn gridline(make: &Make, major: bool) -> Owned {
    let (name, level) = if major {
        ("majorGridlines", 15000)
    } else {
        ("minorGridlines", 5000)
    };
    let colour = make
        .a("schemeClr")
        .with_attr("val", "tx1")
        .with(make.a("lumMod").with_attr("val", &level.to_string()))
        .with(make.a("lumOff").with_attr("val", &(100_000 - level).to_string()));
    let line = make
        .a("ln")
        .with_attr("w", "9525")
        .with_attr("cap", "flat")
        .with_attr("cmpd", "sng")
        .with_attr("algn", "ctr")
        .with(make.a("solidFill").with(colour))
        .with(make.a("round"));
    let shape = make.top(make.c("spPr").with(line).with(make.a("effectLst")));
    make.c(name).with(shape)
}

/// What a text's characters are given (`set_text_style`): each `None` stays as it is.
#[derive(Debug, Clone, Default)]
pub struct TextLook {
    /// A typeface, or `+mn-lt` / `+mj-lt` for the theme's.
    pub font: Option<String>,
    /// Hundredths of a point.
    pub size: Option<i64>,
    /// Bold or not.
    pub bold: Option<bool>,
    /// Its colour.
    pub colour: Option<Colour>,
}

/// Write `look` into one `a:defRPr` / `a:rPr` / `a:endParaRPr` (`style_characters`).
///
/// # Errors
/// [`DataError`] for a font that is not a typeface name.
pub(crate) fn style_characters(properties: &mut Owned, make: &Make, look: &TextLook) -> Result<(), DataError> {
    if let Some(size) = look.size {
        properties.set_attr("sz", &size.to_string());
    }
    if let Some(bold) = look.bold {
        properties.set_attr("b", if bold { "1" } else { "0" });
    }
    if let Some(colour) = &look.colour {
        replace(properties, FILLS, Some(colour.solid(make)), CHARACTER_ORDER);
    }
    if let Some(font) = &look.font {
        let name = py_flat(font);
        let trimmed = font.trim_matches(|c: char| c.is_whitespace());
        if name.is_empty() || font.chars().count() > 31 {
            return Err(DataError(
                "a font is a typeface name of at most 31 characters, or +mn-lt / +mj-lt for the theme's".to_owned(),
            ));
        }
        let new = make.a("latin").with_attr("typeface", trimmed);
        replace(properties, &["latin"], Some(new), CHARACTER_ORDER);
        let theme = match trimmed {
            "+mn-lt" => Some("mn"),
            "+mj-lt" => Some("mj"),
            _ => None,
        };
        if let Some(theme) = theme {
            for script in ["ea", "cs"] {
                let face = format!("+{theme}-{script}");
                let new = make.a(script).with_attr("typeface", &face);
                replace(properties, &[script], Some(new), CHARACTER_ORDER);
            }
        }
    }
    Ok(())
}

/// The `a:defRPr` of a `c:txPr` (its first paragraph's), made where missing (`defaults_of`).
pub(crate) fn defaults_of<'a>(text: &'a mut Owned, make: &Make) -> &'a mut Owned {
    let paragraph = text.child_or_insert("p", None, || make.top(make.a("p")));
    paragraph_defaults(paragraph, make)
}

/// A paragraph's `a:pPr/a:defRPr`, each made where missing (`a:pPr` first).
fn paragraph_defaults<'a>(paragraph: &'a mut Owned, make: &Make) -> &'a mut Owned {
    let properties = paragraph.child_or_put("pPr", || make.a("pPr"), |_| 0);
    properties.child_or_insert("defRPr", None, || make.a("defRPr"))
}

/// Every character-properties element of a `c:rich` body (`rich_character_properties`):
/// each paragraph's `a:defRPr` (made), its runs' and fields' `a:rPr` (made) and its
/// `a:endParaRPr`, given to `style`; how many there were.
///
/// # Errors
/// What `style` returns.
pub(crate) fn rich_properties(
    rich: &mut Owned,
    make: &Make,
    style: &mut dyn FnMut(&mut Owned) -> Result<(), DataError>,
) -> Result<usize, DataError> {
    let mut count = 0;
    for at in rich.places("p") {
        let Some(paragraph) = rich.at(at) else { continue };
        style(paragraph_defaults(paragraph, make))?;
        count += 1;
        let runs: Vec<usize> = paragraph
            .elements()
            .into_iter()
            .filter(|(_, name)| name == "r" || name == "fld")
            .map(|(at, _)| at)
            .collect();
        for run in runs {
            let Some(run) = paragraph.at(run) else { continue };
            if run.place("rPr").is_none() {
                run.insert(0, make.a("rPr"));
            }
            if let Some(properties) = run.child("rPr") {
                style(properties)?;
                count += 1;
            }
        }
        if let Some(end) = paragraph.child("endParaRPr") {
            style(end)?;
            count += 1;
        }
    }
    Ok(count)
}
