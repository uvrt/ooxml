//! Owns: how a VML shape is painted, as docx2svg's `parse.vml` reads it and `drawing.py`
//! draws it: a rectangle, rounded rectangle, oval or text box as the DrawingML preset it is
//! ([`paint`]: its fill -- `fillcolor`, `filled`, `v:fill/@opacity`, white by default -- and
//! its outline -- `strokecolor`, `strokeweight`, `stroked`, black 0.75 pt by default), and a
//! text path ([`TextPath`], a watermark): its string, face, weight and slant, fill and
//! opacity, stroke, and whether its shape type fits it to the box -- written as SVG text
//! stretched to the box less a text box's insets, at its face's kerned pens
//! ([`TextPath::markup`]; measured in Word, `drawing.TEXT_PATH_INSETS`), or, where the
//! outlines are not at hand, stretched by its advances (an approximation, said).
//!
//! Must not: place the shape (the layout has), or choose a face's file (the caller's
//! [`Inks`]).

use super::shapes::{to_f64, Paint};
use super::write::PxBox;
use ooxml_base::Q;
use ooxml_colour::{SourceColour, Transform, TransformKind};
use ooxml_drawingml::source::{Fill, GeometrySpec, Outline};
use ooxml_fonts::ink::TextInk;
use ooxml_fonts::outlines::InkBox;
use ooxml_scene::drawing_num as num;
use ooxml_scene::exact::{escape, quote};
use ooxml_scene::python_g;
use ooxml_word_model::read::vml::{length_emu, preset_of, style_of, vml_shape};
use ooxml_xml::view::{attr, child, elements, local};
use ooxml_xml::Element;
use std::collections::BTreeMap;

/// VML's colour names (ECMA-376 Part 4, 19.1.2.3: the sixteen HTML 4 names).
const COLOURS: [(&str, &str); 16] = [
    ("black", "000000"),
    ("silver", "C0C0C0"),
    ("gray", "808080"),
    ("white", "FFFFFF"),
    ("maroon", "800000"),
    ("red", "FF0000"),
    ("purple", "800080"),
    ("fuchsia", "FF00FF"),
    ("green", "008000"),
    ("lime", "00FF00"),
    ("olive", "808000"),
    ("yellow", "FFFF00"),
    ("navy", "000080"),
    ("blue", "0000FF"),
    ("teal", "008080"),
    ("aqua", "00FFFF"),
];

/// The shape type Word writes for a watermark's text: fitted where its `v:shapetype` is
/// not in the part (measured, `parse.vml._FITTED_TYPES`).
const FITTED_TYPES: [&str; 1] = ["_x0000_t136"];

/// The slant of a text path in italic whose face has no italic (`TEXT_PATH_OBLIQUE`).
pub const TEXT_PATH_OBLIQUE: f64 = 0.34;

/// What a text path's face has to say of its text: its outlines set at its kerned pens,
/// in the face Word draws `family` in that style ([`TextInk`]), slanted by `oblique` where
/// italic is asked of a face with none.
pub trait Inks: Send + Sync {
    /// The ink of `text`, or `None` where the face or its outlines are not at hand.
    fn ink(&self, family: &str, bold: bool, italic: bool, text: &str, oblique: f64) -> Option<TextInk>;
}

/// A VML colour (`#C0C0C0`, `silver`, `#ccc`, `black [3213]`) as `RRGGBB`.
pub fn colour(value: Option<&str>) -> Option<String> {
    let value = value.filter(|v| !v.is_empty())?;
    let value = value.split_whitespace().next()?.trim().to_lowercase();
    if let Some(digits) = value.strip_prefix('#') {
        let digits = if digits.chars().count() == 3 {
            digits.chars().flat_map(|c| [c, c]).collect()
        } else {
            digits.to_owned()
        };
        let hex = digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        return hex.then(|| digits.to_uppercase());
    }
    COLOURS
        .iter()
        .find(|(name, _)| *name == value)
        .map(|(_, hex)| (*hex).to_owned())
}

/// A VML fraction: `.5`, `50%` or `32768f` (65536ths); `default` where unstated or not one.
pub fn fraction(value: Option<&str>, default: f64) -> f64 {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return default;
    };
    let value = value.trim().to_lowercase();
    let parse = |t: &str| t.trim().parse::<f64>().ok();
    let found = if let Some(units) = value.strip_suffix('f') {
        parse(units).map(|v| v / 65536.0)
    } else if let Some(percent) = value.strip_suffix('%') {
        parse(percent).map(|v| v / 100.0)
    } else {
        parse(&value)
    };
    found.unwrap_or(default)
}

fn off(value: Option<&str>) -> bool {
    matches!(value, Some("f" | "false"))
}

/// How a VML rectangle, rounded rectangle, oval or text box is painted, as the DrawingML
/// shape it is (`parse.vml._shape`); `None` where `object` draws no preset shape.
pub fn paint(object: &Element) -> Option<Paint> {
    let (shape, _) = vml_shape(object)?;
    let preset = preset_of(shape)?;
    let fill_element = child(shape, "fill");
    let fill = if off(attr(shape, "filled")) || off(fill_element.and_then(|f| attr(f, "on"))) {
        Fill::None
    } else {
        let alpha = fraction(fill_element.and_then(|f| attr(f, "opacity")), 1.0);
        let transforms = if alpha < 1.0 {
            vec![Transform {
                kind: TransformKind::Alpha,
                value: alpha * 100_000.0,
            }]
        } else {
            Vec::new()
        };
        Fill::Solid(SourceColour::Srgb {
            hex: colour(attr(shape, "fillcolor")).unwrap_or_else(|| "FFFFFF".into()),
            transforms,
            linear: None,
        })
    };
    let stroke = child(shape, "stroke");
    let line = if off(attr(shape, "stroked")) || off(stroke.and_then(|s| attr(s, "on"))) {
        Outline {
            fill: Some(Fill::None),
            ..Outline::default()
        }
    } else {
        let stated = attr(shape, "strokeweight").filter(|w| !w.is_empty()).unwrap_or("0.75pt");
        let weight = length_emu(Some(stated)).filter(|w| *w != 0).unwrap_or(9525);
        let named = attr(shape, "strokecolor")
            .filter(|c| !c.is_empty())
            .or_else(|| stroke.and_then(|s| attr(s, "color")));
        #[allow(clippy::cast_precision_loss)]
        Outline {
            width: Some(weight as f64),
            fill: Some(Fill::Solid(SourceColour::Srgb {
                hex: colour(named).unwrap_or_else(|| "000000".into()),
                transforms: Vec::new(),
                linear: None,
            })),
            ..Outline::default()
        }
    };
    Some(Paint {
        geometry: Some(GeometrySpec::Preset {
            name: preset.to_owned(),
            adjust: Vec::new(),
        }),
        fill: Some(fill),
        line: Some(line),
        ..Paint::default()
    })
}

/// A VML text path (`v:textpath/@string`: a watermark) as docx2svg reads it (its `body`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextPath {
    /// The text.
    pub string: String,
    /// The face it is drawn in (`drawing.text_path_face`: the theme's minor face for none
    /// or Times New Roman).
    pub face: String,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Its fill colour, `RRGGBB` (`None`: not filled).
    pub fill: Option<String>,
    /// Its fill's opacity.
    pub opacity: f64,
    /// Its stroke colour.
    pub stroke: Option<String>,
    /// Whether its shape type fits it to its box (`None`: not known).
    pub fit: Option<bool>,
}

/// Whether a text path is stretched to fill its shape, by its `v:shapetype`'s
/// `v:textpath` alone (`_fits`); `types` are the object's shape types by id.
fn fits(shape: &Element, types: &BTreeMap<String, &Element>) -> Option<bool> {
    let kind = attr(shape, "type").unwrap_or("").trim_start_matches('#');
    if let Some(found) = types.get(kind) {
        let stated = child(found, "textpath").and_then(|t| attr(t, "fitshape"));
        return Some(matches!(stated, Some("t" | "true" | "1" | "on")));
    }
    FITTED_TYPES.contains(&kind).then_some(true)
}

impl TextPath {
    /// The text path `object` draws, its face as Word draws it with `minor` the theme's
    /// minor Latin face; `None` where it draws none.
    pub fn of(object: &Element, minor: Option<&str>) -> Option<TextPath> {
        let (shape, _) = vml_shape(object)?;
        let path = child(shape, "textpath")?;
        let string = attr(path, "string").filter(|s| !s.is_empty())?;
        let text_style = style_of(path);
        let mut types = BTreeMap::new();
        for element in elements(object).filter(|e| local(e) == "shapetype") {
            if let Some(id) = attr(element, "id").filter(|i| !i.is_empty()) {
                types.insert(id.to_owned(), element);
            }
        }
        let fill = child(shape, "fill");
        let font = text_style.get("font-family").filter(|f| !f.is_empty()).cloned();
        let stroke = (!off(attr(shape, "stroked")))
            .then(|| colour(attr(shape, "strokecolor")).unwrap_or_else(|| "000000".to_owned()));
        Some(TextPath {
            string: string.to_owned(),
            face: text_path_face(font.as_deref(), minor),
            bold: matches!(
                text_style.get("font-weight").map(String::as_str),
                Some("bold" | "700" | "800" | "900")
            ),
            italic: text_style.get("font-style").map(String::as_str) == Some("italic"),
            fill: (!off(attr(shape, "filled")))
                .then(|| colour(attr(shape, "fillcolor")).unwrap_or_else(|| "FFFFFF".into())),
            opacity: fill.map_or(1.0, |f| fraction(attr(f, "opacity"), 1.0)),
            stroke,
            fit: fits(shape, &types),
        })
    }

    /// Its outlines in its face (`text_path_outlines`): only where its shape type fits it to
    /// the box.
    pub fn ink(&self, inks: Option<&dyn Inks>) -> Option<TextInk> {
        inks.filter(|_| self.fit == Some(true))?
            .ink(&self.face, self.bold, self.italic, &self.string, TEXT_PATH_OBLIQUE)
    }

    /// The face its text is drawn in, for a rasteriser: `(face, bold, italic)`, italic only
    /// where the face has an italic (a slanted upright face is drawn upright, slanted).
    pub fn drawn_face(&self, ink: Option<&TextInk>) -> (String, bool, bool) {
        let oblique = ink.is_some_and(|o| o.slant != 0.0);
        (self.face.clone(), self.bold, self.italic && !oblique)
    }

    /// The text as SVG in the box `at` (`text_path_markup`): at its pens and stretched to
    /// the box less the insets where its outlines are given, else stretched by its
    /// advances.
    pub fn markup(&self, at: &PxBox, outlines: Option<&TextInk>) -> String {
        let mut paint = match &self.fill {
            Some(fill) => format!("fill=\"#{fill}\""),
            None => "fill=\"none\"".to_owned(),
        };
        if self.fill.is_some() && self.opacity < 1.0 {
            paint.push_str(&format!(" fill-opacity=\"{}\"", num(self.opacity)));
        }
        let family = quote(&format!("{}, sans-serif", self.face));
        let weight = if self.bold { " font-weight=\"bold\"" } else { "" };
        let (x, y, width, height) = (at.x.value(), at.y.value(), at.width.value(), at.height.value());
        let Some(ink) = outlines else {
            let style = if self.italic { " font-style=\"italic\"" } else { "" };
            let em = height * Q::new(5, 6);
            if let Some(stroke) = &self.stroke {
                paint.push_str(&format!(" stroke=\"#{stroke}\" stroke-width=\"{}\"", num(to_f64(height) / 200.0)));
            }
            let baseline = y + em * Q::new(4, 5) + (height - em) / Q::from_integer(2);
            return format!(
                "<text x=\"{}\" y=\"{}\" font-family={family} font-size=\"{}\"{weight}{style} textLength=\"{}\" \
                 lengthAdjust=\"spacingAndGlyphs\" {paint}>{}</text>",
                num(to_f64(x)),
                num(to_f64(baseline)),
                num(to_f64(em)),
                num(to_f64(width)),
                escape(&self.string)
            );
        };
        let style = if self.italic && ink.slant == 0.0 {
            " font-style=\"italic\""
        } else {
            ""
        };
        let one = Q::from_integer(1);
        let inner_w = (width - Q::from_integer(60)).max(one);
        let inner_h = (height - Q::from_integer(30)).max(one);
        let (sx, tx) = axis(inner_w, x + Q::from_integer(30), ink.ink, 0, false);
        let (sy, ty) = axis(inner_h, y + Q::from_integer(15), ink.ink, 1, true);
        if let Some(stroke) = &self.stroke {
            let w = match sy {
                Exact::Q(s) => to_f64(height / Q::from_integer(200) / s),
                Exact::F(s) => to_f64(height / Q::from_integer(200)) / s,
            };
            paint.push_str(&format!(" stroke=\"#{stroke}\" stroke-width=\"{}\"", num(w)));
        }
        let skew = -ink.slant * sx.float();
        let pens: Vec<String> = ink.pens.iter().map(ToString::to_string).collect();
        let pens = if pens.is_empty() { "0".to_owned() } else { pens.join(" ") };
        format!(
            "<text x=\"{pens}\" y=\"0\" transform=\"matrix({} 0 {} {} {} {})\" font-family={family} font-size=\"{}\" \
             style=\"font-kerning:none;font-variant-ligatures:none\"{weight}{style} {paint}>{}</text>",
            python_g(sx.float()),
            python_g(skew),
            python_g(sy.float()),
            num(tx.float()),
            num(ty.float()),
            ink.units_per_em,
            escape(&self.string)
        )
    }
}

/// A value exact where docx2svg's is a `Fraction`, a float where it is one.
#[derive(Debug, Clone, Copy)]
enum Exact {
    Q(Q),
    F(f64),
}

impl Exact {
    fn float(self) -> f64 {
        match self {
            Exact::Q(q) => to_f64(q),
            Exact::F(f) => f,
        }
    }
}

/// One axis of a fitted text path: its scale (`inner / (max - min)`) and its translation
/// (`start - min * scale` across, `start + max * scale` down).
fn axis(inner: Q, start: Q, ink: InkBox, axis: usize, down: bool) -> (Exact, Exact) {
    let (low, high) = (axis, axis + 2);
    match ink {
        InkBox::Units(v) => {
            let (lo, hi) = (v.get(low).copied().unwrap_or(0), v.get(high).copied().unwrap_or(0));
            let span = Q::from_integer(i128::from(hi - lo));
            if span == Q::from_integer(0) {
                return (Exact::F(f64::INFINITY), Exact::F(f64::NAN));
            }
            let scale = inner / span;
            let at = if down {
                start + Q::from_integer(i128::from(hi)) * scale
            } else {
                start - Q::from_integer(i128::from(lo)) * scale
            };
            (Exact::Q(scale), Exact::Q(at))
        }
        InkBox::Float(v) => {
            let (lo, hi) = (v.get(low).copied().unwrap_or(0.0), v.get(high).copied().unwrap_or(0.0));
            let scale = to_f64(inner) / (hi - lo);
            let at = if down {
                to_f64(start) + hi * scale
            } else {
                to_f64(start) - lo * scale
            };
            (Exact::F(scale), Exact::F(at))
        }
    }
}

/// The faces Word draws a text path in its document's body face instead (measured,
/// `TEXT_PATH_DEFAULT_FACES`).
const TEXT_PATH_DEFAULT_FACES: [&str; 2] = ["", "times new roman"];

/// The face Word draws a text path whose `font-family` is `font` in, the document's minor
/// face being `minor` (`text_path_face`).
pub fn text_path_face(font: Option<&str>, minor: Option<&str>) -> String {
    let stated = font.unwrap_or("").trim().to_lowercase();
    if let Some(minor) = minor.filter(|m| !m.is_empty()) {
        if TEXT_PATH_DEFAULT_FACES.contains(&stated.as_str()) {
            return minor.to_owned();
        }
    }
    font.filter(|f| !f.is_empty()).unwrap_or("Calibri").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_and_fractions_read_as_docx2svg_reads_them() {
        assert_eq!(colour(Some("#ccc")), Some("CCCCCC".into()));
        assert_eq!(colour(Some("silver")), Some("C0C0C0".into()));
        assert_eq!(colour(Some("black [3213]")), Some("000000".into()));
        assert_eq!(colour(Some("#12345g")), None);
        assert!((fraction(Some("32768f"), 1.0) - 0.5).abs() < 1e-12);
        assert!((fraction(Some("50%"), 1.0) - 0.5).abs() < 1e-12);
        assert!((fraction(Some("x"), 1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_text_path_in_no_face_or_times_is_drawn_in_the_minor_face() {
        assert_eq!(text_path_face(None, Some("Aptos")), "Aptos");
        assert_eq!(text_path_face(Some("Times New Roman"), Some("Aptos")), "Aptos");
        assert_eq!(text_path_face(Some("Georgia"), Some("Aptos")), "Georgia");
        assert_eq!(text_path_face(None, None), "Calibri");
    }

    #[test]
    fn minus_zero_is_written_as_python_writes_it() {
        assert_eq!(python_g(-0.0), "-0");
    }
}
