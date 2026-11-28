//! Owns: how `word_design_facts` says a look (docx-agent's `edit/design.py`, "values" and
//! "theme colours"): points, borders and fills as text; a style's look after inheritance
//! ([`Formatting::style_look`]) as its text and paragraph facts; and a colour Word draws
//! from the theme labelled by the element that declared it ("accent1 shade BF"), found from
//! where the resolver says the value came from ([`Origin`]), not searched for again.
//!
//! Must not: change a document, or choose what is reported (the kinds' modules do).

use ooxml_base::pyfmt::round2;
use ooxml_tools::pyfmt::repr_float;
use ooxml_word_model::docx::formatting::{Cell, EffectiveRun, Formatting, StyleLook};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::W;
use ooxml_word_model::markdown::read::Shared;
use ooxml_word_model::resolve::{Level, Origin, Origins};
use ooxml_word_model::styles::StyleSheet;
use ooxml_word_model::value::{Border, Props, Value as Prop};
use ooxml_xml::Element;
use serde_json::{Map, Value};
use std::sync::Arc;

/// The theme colour attributes of a colour or border element.
const COLOR_ATTRIBUTES: [&str; 3] = ["themeColor", "themeTint", "themeShade"];
/// Of a shading element: its fill's.
const FILL_ATTRIBUTES: [&str; 3] = ["themeFill", "themeFillTint", "themeFillShade"];

/// One version as the design facts read it: its reading, what formatting resolves against
/// and the styles part's root (where `w:docDefaults` is).
pub(crate) struct Design<'d> {
    /// The version.
    pub(crate) docx: &'d Docx,
    shared: Arc<Shared>,
    styles_root: Option<Arc<Element>>,
}

impl<'d> Design<'d> {
    /// The design of `docx`.
    pub(crate) fn new(docx: &'d Docx) -> Self {
        let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
        let styles_root = ooxml_word_model::parts::related(&docx.package, &docx.main, rel)
            .ok()
            .flatten()
            .and_then(|part| docx.package.tree(&part).ok().flatten())
            .map(|tree| Arc::clone(tree.root()));
        Design {
            docx,
            shared: Shared::of(docx),
            styles_root,
        }
    }

    /// What formatting resolves against.
    pub(crate) fn formatting(&self) -> &Formatting {
        self.shared.formatting()
    }

    /// The styles as the resolver reads them.
    pub(crate) fn sheet(&self) -> Option<&StyleSheet> {
        self.formatting().context().styles.as_deref()
    }

    /// A style's look after inheritance: `style` (`None`: the default paragraph style) in
    /// `cell` where given, a run in it in `character_style`.
    pub(crate) fn style_look(
        &self,
        style: Option<&str>,
        cell: Option<&Cell>,
        character_style: Option<&str>,
    ) -> StyleLook {
        self.formatting().style_look(style, cell, character_style)
    }

    /// The element that declared `key` where `origin` says it came from: docDefaults' or a
    /// style's (its region's where it is one) properties element, then the key's element
    /// (`pBdr.top` is `w:pBdr/w:top`; a side read from an inside line, `insideH`/`insideV`).
    fn declaring(&self, origin: &Origin, key: &str) -> Option<Arc<Element>> {
        let container = match origin.level {
            Level::DocDefaults => {
                let local = if origin.holder.local() == "pPr" {
                    "pPrDefault"
                } else {
                    "rPrDefault"
                };
                Arc::clone(
                    self.styles_root
                        .as_ref()?
                        .find(Some(W), "docDefaults")?
                        .find(Some(W), local)?,
                )
            }
            Level::Direct | Level::Application => return None,
            _ => {
                let style = origin.declared_in(self.sheet()?)?;
                let element = Arc::clone(self.docx.style(&style.style_id)?);
                match origin.region_type(style) {
                    Some(kind) => element
                        .child_elements()
                        .find(|c| c.name().is(Some(W), "tblStylePr") && c.attribute(Some(W), "type") == Some(&**kind))
                        .map(Arc::clone)?,
                    None => element,
                }
            }
        };
        let holder = Arc::clone(container.find(Some(W), origin.holder.local())?);
        let mut steps = key.split('.');
        let first = steps.next()?;
        let mut node = Arc::clone(holder.find(Some(W), first)?);
        if let Some(side) = steps.next() {
            let side = match side {
                "top" | "bottom" if origin.inside => "insideH",
                "left" | "right" if origin.inside => "insideV",
                other => other,
            };
            node = Arc::clone(node.find(Some(W), side)?);
        }
        Some(node)
    }

    /// `(RRGGBB, "accent1 shade BF")` where the element that declared `key` names a theme
    /// colour the theme has: its fill's (`fill`) or its colour's.
    pub(crate) fn themed(&self, origin: Option<&Origin>, key: &str, fill: bool) -> Option<(String, String)> {
        let element = self.declaring(origin?, key)?;
        let [name, tint, shade] = if fill { FILL_ATTRIBUTES } else { COLOR_ATTRIBUTES }
            .map(|a| element.attribute(Some(W), a).filter(|v| !v.is_empty()));
        let name = name?;
        let rgb = self.formatting().context().colours.resolve(name, tint, shade)?;
        let mut label = name.to_owned();
        for (kind, value) in [("tint", tint), ("shade", shade)] {
            if let Some(value) = value {
                label.push_str(&format!(" {kind} {}", value.to_uppercase()));
            }
        }
        Some((rgb.to_string(), label))
    }

    /// `{name: value}`, or the theme's colour with `theme_<name>` where the element that
    /// declared `key` names one.
    pub(crate) fn colour(&self, origins: &Origins, key: &str, value: Value, name: &str, out: &mut Map<String, Value>) {
        match self.themed(origins.of(key), key, key == "shd") {
            Some((rgb, label)) => {
                out.insert(name.into(), Value::from(rgb));
                out.insert(format!("theme_{name}"), Value::from(label));
            }
            None => {
                out.insert(name.into(), value);
            }
        }
    }

    /// A run's text look: face, size, colour, the flags it has on, its underline.
    pub(crate) fn text_look(&self, run: &EffectiveRun, origins: &Origins) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("font".into(), run.font.clone().map_or(Value::Null, Value::from));
        out.insert("size".into(), pt(run.size));
        self.colour(origins, "color", Value::from(run.color.clone()), "color", &mut out);
        for (name, on) in [
            ("bold", run.bold),
            ("italic", run.italic),
            ("caps", run.caps),
            ("small_caps", run.small_caps),
        ] {
            if on {
                out.insert(name.into(), Value::Bool(true));
            }
        }
        if let Some(underline) = &run.underline {
            out.insert("underline".into(), Value::from(underline.as_str()));
        }
        out
    }

    /// A style's whole look: its text, then its paragraph's alignment, spacing, indents,
    /// keeps, outline level, list, shading and borders.
    pub(crate) fn look(&self, look: &StyleLook) -> Map<String, Value> {
        let mut out = self.text_look(&look.run, &look.run_origins);
        let paragraph = &look.paragraph;
        out.insert(
            "alignment".into(),
            paragraph.alignment.clone().map_or(Value::Null, Value::from),
        );
        out.insert("space_before".into(), pt(paragraph.space_before));
        out.insert("space_after".into(), pt(paragraph.space_after));
        if let Some(line) = paragraph.line_spacing {
            let value = if paragraph.line_rule == "auto" {
                pt(line)
            } else {
                Value::from(format!("{}pt {}", py_number(line), paragraph.line_rule))
            };
            out.insert("line_spacing".into(), value);
        }
        out.insert("indent_left".into(), pt(paragraph.indent_left));
        if paragraph.indent_right != 0.0 {
            out.insert("indent_right".into(), pt(paragraph.indent_right));
        }
        let first = paragraph.first_line - paragraph.hanging;
        if first != 0.0 {
            out.insert("indent_first".into(), pt(first));
        }
        for (name, on) in [
            ("keep_with_next", paragraph.keep_with_next),
            ("keep_together", paragraph.keep_together),
            ("page_break_before", paragraph.page_break_before),
        ] {
            if on {
                out.insert(name.into(), Value::Bool(true));
            }
        }
        if let Some(level) = paragraph.outline_level.filter(|l| *l < 9) {
            out.insert("outline_level".into(), Value::from(level + 1));
        }
        if let Some(numbering) = self.numbering(&paragraph.resolved) {
            out.insert("numbering".into(), numbering);
        }
        let resolved = &paragraph.resolved;
        if let Some(fill) = fill(resolved.get("shd")) {
            self.colour(&look.paragraph_origins, "shd", Value::from(fill), "shading", &mut out);
        }
        let mut borders = Map::new();
        for side in ["top", "left", "bottom", "right", "between"] {
            let key = format!("pBdr.{side}");
            if let Some(Prop::Border(border)) = resolved.get(key.as_str()) {
                if drawn(border) {
                    let themed = self.themed(look.paragraph_origins.of(&key), &key, false);
                    borders.insert(side.into(), Value::from(border_text(Some(border), themed.as_ref())));
                }
            }
        }
        if !borders.is_empty() {
            out.insert("borders".into(), Value::Object(borders));
        }
        out
    }

    /// The list a paragraph's resolved properties put it in: its `w:numId`, level, and the
    /// level's format and label text.
    fn numbering(&self, resolved: &Props) -> Option<Value> {
        let num_id = resolved.get("numPr.numId").and_then(Prop::int).filter(|n| *n != 0)?;
        let level = resolved.get("numPr.ilvl").and_then(Prop::int).unwrap_or(0);
        let mut out = Map::new();
        out.insert("num_id".into(), Value::from(num_id));
        out.insert("level".into(), Value::from(level));
        if let Some(found) = self.formatting().numbering().level(num_id, level) {
            let format = found.format.as_deref().filter(|f| !f.is_empty()).unwrap_or("decimal");
            out.insert("format".into(), Value::from(format));
            if let Some(text) = &found.text {
                out.insert("label".into(), Value::from(&**text));
            }
        }
        Some(Value::Object(out))
    }
}

/// Points as reported: two decimals, a whole number as one.
pub(crate) fn pt(value: f64) -> Value {
    let value = round2(value);
    if value.fract() == 0.0 && value.abs() < 1e15 {
        Value::from(value as i64)
    } else {
        Value::from(value)
    }
}

/// [`pt`], `null` for none.
pub(crate) fn pt_or_null(value: Option<f64>) -> Value {
    value.map_or(Value::Null, pt)
}

/// [`pt`] as Python's `str` writes it.
pub(crate) fn py_number(value: f64) -> String {
    match pt(value) {
        Value::Number(n) if n.is_i64() => n.to_string(),
        _ => repr_float(round2(value)),
    }
}

/// Whether a border is drawn: a style that is not none.
pub(crate) fn drawn(border: &Border) -> bool {
    !matches!(border.style.as_deref(), None | Some("nil" | "none"))
}

/// `single 0.5pt 000000`, `double 1.5pt auto`, or `none`; with a theme colour, the theme's
/// colour and `(accent1 shade BF)`.
pub(crate) fn border_text(border: Option<&Border>, themed: Option<&(String, String)>) -> String {
    let Some(border) = border.filter(|b| drawn(b)) else {
        return "none".into();
    };
    let style = border.style.as_deref().unwrap_or("");
    let size = py_number(border.size as f64 / 8.0);
    match themed {
        Some((rgb, label)) => format!("{style} {size}pt {rgb} ({label})"),
        None => format!(
            "{style} {size}pt {}",
            border.color.as_deref().filter(|c| !c.is_empty()).unwrap_or("auto")
        ),
    }
}

/// A shading's fill as reported (`RRGGBB` upper case; `None` for none, `nil` or `auto`).
pub(crate) fn fill(shading: Option<&Prop>) -> Option<String> {
    let Some(Prop::Shading(shading)) = shading else {
        return None;
    };
    if shading.kind.as_deref() == Some("nil") {
        return None;
    }
    shading.fill.as_deref().filter(|f| *f != "auto").map(str::to_uppercase)
}
