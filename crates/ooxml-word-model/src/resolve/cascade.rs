//! Owns: the property cascade as Word 16.106 applies it (measured, docx2svg's
//! `resolve.cascade`; ROADMAP "Style inheritance"), lowest precedence first:
//! `w:docDefaults`; the table style of the innermost table with its regions for the cell;
//! numbering (here when the style gives the `w:numPr`); the paragraph style; numbering
//! (here when it is direct formatting); the character style (runs; the default character
//! style contributes nothing); direct formatting. Within a style's `w:basedOn` chain every
//! property is a plain override. Toggles (`w:b`, `w:i`, `w:caps`, `w:vanish`...): each
//! style level resolves its own value, then every level whose value differs from
//! `w:docDefaults`' flips the result; direct formatting sets it absolutely.
//!
//! Must not: memoise (`crate::resolve::Resolver` does), or read XML.

use super::origin::{Holder, Level, Origin, Trace};
use crate::numbering::{Level as ListLevelDef, Numbering};

use crate::settings::Settings;
use crate::styles::{Style, StyleKind, StyleSheet};
use crate::value::{key, Key, Props, TabStop, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

/// What a space `w:beforeAutospacing` / `w:afterAutospacing` makes: 14 pt.
pub const AUTOSPACING_TWIPS: i64 = 280;
/// The size Word treated as "no size stated" before Word 2013, half points.
pub const LEGACY_DEFAULT_SIZE: i64 = 24;
/// A `w:docDefaults` size that counts as unset under the same rule.
pub const LEGACY_UNSET_DEFAULT: i64 = 20;

/// The parts a cascade reads.
#[derive(Debug, Clone, Copy)]
pub struct Sources<'a> {
    /// The styles part, where the package has one.
    pub styles: Option<&'a StyleSheet>,
    /// The numbering part.
    pub numbering: &'a Numbering,
    /// The settings part.
    pub settings: &'a Settings,
}

/// What a paragraph's resolution reads of the paragraph.
#[derive(Debug, Clone, Copy)]
pub struct ParagraphInput<'a> {
    /// `w:pStyle`.
    pub style_id: Option<&'a str>,
    /// The innermost table's style (`""`: none named), `None` outside tables.
    pub table_style: Option<&'a str>,
    /// The cell's regions (band positions unnamed).
    pub conditions: &'a [crate::value::Text],
    /// The paragraph's direct formatting.
    pub direct: &'a Props,
}

/// Override `acc` with `layer`: `ind.firstLine` and `ind.hanging` are one quantity (either
/// removes the other), and tab stops accumulate (a `clear` stop removes one).
pub fn merge(acc: &mut Props, layer: &Props) {
    for (k, value) in layer {
        match &**k {
            "ind.firstLine" => {
                acc.remove("ind.hanging");
            }
            "ind.hanging" => {
                acc.remove("ind.firstLine");
            }
            _ => {}
        }
        if let (true, Value::Tabs(stops)) = (&**k == "tabs", value) {
            let mut by_position: std::collections::BTreeMap<i64, TabStop> = match acc.get("tabs") {
                Some(Value::Tabs(inherited)) => inherited.iter().map(|s| (s.position, s.clone())).collect(),
                _ => std::collections::BTreeMap::new(),
            };
            for stop in stops.iter() {
                if &*stop.alignment == "clear" {
                    by_position.remove(&stop.position);
                } else {
                    by_position.insert(stop.position, stop.clone());
                }
            }
            acc.insert(k.clone(), Value::Tabs(by_position.into_values().collect()));
            continue;
        }
        acc.insert(k.clone(), value.clone());
    }
}

/// Which part of a style a level reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// `w:pPr`.
    Paragraph,
    /// `w:rPr`.
    Run,
}

impl Part {
    /// The properties element it reads.
    #[must_use]
    pub fn holder(self) -> Holder {
        match self {
            Part::Paragraph => Holder::Paragraph,
            Part::Run => Holder::Run,
        }
    }
}

/// `conditions` with the band positions named against the table style chain: a row `n`
/// rows into the banded ones is in `band1Horz` when `n / size` is even, `band2Horz` when
/// odd (or negative); no size stated (or 0): no band.
pub fn banded(chain: &[Arc<Style>], conditions: &[crate::value::Text]) -> Vec<crate::value::Text> {
    let (mut h, mut v) = (0, 0);
    for style in chain {
        if let Some(value) = style.table.get("tblStyleRowBandSize") {
            h = value.int().unwrap_or(0);
        }
        if let Some(value) = style.table.get("tblStyleColBandSize") {
            v = value.int().unwrap_or(0);
        }
    }
    let mut out = Vec::new();
    for condition in conditions {
        let (kind, rest) = (condition.get(..1).unwrap_or(""), condition.get(1..6).unwrap_or(""));
        if rest != "Band:" || !matches!(kind, "h" | "v") {
            out.push(condition.clone());
            continue;
        }
        let size = if kind == "h" { h } else { v };
        if size <= 0 {
            continue;
        }
        let Some(index) = condition.get(6..).and_then(crate::xmlread::py_int) else {
            continue;
        };
        let odd = index >= 0 && (index.div_euclid(size)) % 2 == 0;
        let name = format!(
            "{}{}",
            if odd { "band1" } else { "band2" },
            if kind == "h" { "Horz" } else { "Vert" }
        );
        out.push(key(&name));
    }
    out
}

/// One style level: its chain merged root first, each style's regions for `conditions`
/// after its own properties; with where each key came from at `level`.
pub fn flatten_traced<T: Trace>(
    chain: &[Arc<Style>],
    part: Part,
    conditions: &[crate::value::Text],
    level: Level,
) -> (Props, T) {
    let holder = part.holder();
    let mut layer = Props::new();
    let mut trace = T::default();
    for style in chain {
        let own = if part == Part::Paragraph {
            &style.paragraph
        } else {
            &style.run
        };
        merge(&mut layer, own);
        trace.declare(own, || Origin::style(level, style, holder));
        for condition in conditions {
            if let Some(region) = style.conditional.get(condition) {
                let props = if part == Part::Paragraph {
                    &region.paragraph
                } else {
                    &region.run
                };
                merge(&mut layer, props);
                trace.declare(props, || Origin::region(level, style, condition, holder));
            }
        }
    }
    (layer, trace)
}

impl Sources<'_> {
    /// The paragraph style that applies: the named one, or the default where it names
    /// none or no paragraph style.
    pub fn paragraph_style(&self, declared: Option<&str>) -> Option<crate::value::Text> {
        let sheet = self.styles?;
        if let Some(id) = declared {
            if sheet.get(id).is_some_and(|s| s.kind == StyleKind::Paragraph) {
                return Some(key(id));
            }
        }
        sheet.default_of(StyleKind::Paragraph).map(|s| s.style_id.clone())
    }

    /// The table style that applies to a cell's paragraph (`table_style` `None`: not in
    /// a table).
    pub fn table_style(&self, table_style: Option<&str>) -> Option<crate::value::Text> {
        let sheet = self.styles?;
        let named = table_style?;
        if !named.is_empty() && sheet.get(named).is_some_and(|s| s.kind == StyleKind::Table) {
            return Some(key(named));
        }
        sheet.default_of(StyleKind::Table).map(|s| s.style_id.clone())
    }

    fn chain(&self, id: Option<&str>, kind: StyleKind) -> Vec<Arc<Style>> {
        self.styles.map(|s| s.chain(id, kind)).unwrap_or_default()
    }

    fn level(&self, num_id: Option<i64>, ilvl: i64) -> Option<&Arc<ListLevelDef>> {
        self.numbering.level(num_id.filter(|n| *n != 0)?, ilvl)
    }

    fn table_layer<T: Trace>(
        &self,
        input_table: Option<&str>,
        conditions: &[crate::value::Text],
        part: Part,
    ) -> Option<(Props, T)> {
        let id = self.table_style(input_table)?;
        let chain = self.chain(Some(&id), StyleKind::Table);
        let named = banded(&chain, conditions);
        Some(flatten_traced(&chain, part, &named, Level::TableStyle))
    }
}

/// How a paragraph's list level stands (docx2svg's `_list_level`, measured by
/// `make_list_level_probe.py`).
#[derive(Default)]
struct ListLevel {
    single: bool,
    missing: bool,
    unnumbered: bool,
}

fn numbering_layer(
    src: &Sources<'_>,
    style_id: Option<&str>,
    styled: &Props,
    direct: &Props,
) -> (Props, bool, ListLevel) {
    let mut direct_numbering = direct.contains_key("numPr.numId") || direct.contains_key("numPr.ilvl");
    let source = if direct.contains_key("numPr.numId") {
        direct
    } else {
        styled
    };
    let num_id = source.get("numPr.numId").and_then(Value::int);
    let ilvl = direct
        .get("numPr.ilvl")
        .or_else(|| source.get("numPr.ilvl"))
        .and_then(Value::int)
        .unwrap_or(0);
    let mut level = src.level(num_id, ilvl).cloned();
    let levels = num_id
        .filter(|n| *n != 0)
        .and_then(|n| src.numbering.levels.get(&n))
        .filter(|l| !l.is_empty());
    let mut state = ListLevel::default();
    match (&level, levels) {
        (None, Some(levels)) => match levels.get(&0).filter(|l| l.single) {
            Some(first) => {
                level = Some(Arc::clone(first));
                state.single = true;
            }
            None => state.missing = true,
        },
        (Some(l), _) => {
            if let Some(tied) = l.style.as_deref().filter(|s| !s.is_empty()) {
                let chain: BTreeSet<_> = src
                    .chain(style_id, StyleKind::Paragraph)
                    .iter()
                    .map(|s| s.style_id.clone())
                    .collect();
                if !direct_numbering && !chain.contains(tied) {
                    state.unnumbered = true;
                }
            }
        }
        _ => {}
    }
    let mut layer = Props::new();
    if let Some(l) = &level {
        layer = l.paragraph.clone();
    } else if state.missing && direct_numbering {
        let styled_num = styled.get("numPr.numId").and_then(Value::int);
        let styled_ilvl = styled.get("numPr.ilvl").and_then(Value::int).unwrap_or(0);
        if let Some(l) = src.level(styled_num, styled_ilvl) {
            layer = l.paragraph.clone();
        }
        direct_numbering = false;
    }
    (layer, direct_numbering, state)
}

/// A paragraph's own properties (`w:pPr`), through the whole hierarchy.
pub fn resolve_paragraph(src: &Sources<'_>, input: &ParagraphInput<'_>) -> Props {
    resolve_paragraph_traced::<()>(src, input).0
}

/// [`resolve_paragraph`], with where each value came from.
pub fn resolve_paragraph_traced<T: Trace>(src: &Sources<'_>, input: &ParagraphInput<'_>) -> (Props, T) {
    let mut layer = Props::new();
    let mut trace = T::default();
    if let Some(sheet) = src.styles {
        merge(&mut layer, &sheet.paragraph_defaults);
        trace.declare(&sheet.paragraph_defaults, || {
            Origin::at(Level::DocDefaults, Holder::Paragraph)
        });
    }
    if let Some((table, from)) = src.table_layer::<T>(input.table_style, input.conditions, Part::Paragraph) {
        merge(&mut layer, &table);
        trace.absorb(&from, &table);
    }
    let style_id = src.paragraph_style(input.style_id);
    let (style_layer, style_from) = flatten_traced::<T>(
        &src.chain(style_id.as_deref(), StyleKind::Paragraph),
        Part::Paragraph,
        &[],
        Level::ParagraphStyle,
    );
    let (numbering, direct_numbering, state) = numbering_layer(src, style_id.as_deref(), &style_layer, input.direct);
    let numbered = Origin::at(Level::Numbering, Holder::Paragraph);
    if !direct_numbering {
        merge(&mut layer, &numbering);
        trace.declare(&numbering, || numbered);
    }
    merge(&mut layer, &style_layer);
    trace.absorb(&style_from, &style_layer);
    if direct_numbering {
        merge(&mut layer, &numbering);
        trace.declare(&numbering, || numbered);
    }
    merge(&mut layer, input.direct);
    trace.declare(input.direct, || Origin::at(Level::Direct, Holder::Paragraph));
    let mut set = |layer: &mut Props, k: &'static str, v: Value, origin: Origin| {
        layer.insert(Key::Borrowed(k), v);
        trace.set(&Key::Borrowed(k), origin);
    };
    if state.single {
        set(&mut layer, "numPr.ilvl", Value::Int(0), numbered);
    }
    if state.missing {
        set(&mut layer, "ind.firstLine", Value::Int(0), numbered);
        set(&mut layer, "ind.hanging", Value::Int(0), numbered);
    }
    if state.unnumbered {
        set(&mut layer, "numPr.numId", Value::Int(0), numbered);
    }
    if !src.settings.option("doNotUseHTMLParagraphAutoSpacing") {
        for (flag, side) in [
            ("spacing.beforeAutospacing", "spacing.before"),
            ("spacing.afterAutospacing", "spacing.after"),
        ] {
            if layer.get(flag).is_some_and(Value::truthy) {
                set(
                    &mut layer,
                    side,
                    Value::Int(AUTOSPACING_TWIPS),
                    Origin::at(Level::Application, Holder::Paragraph),
                );
            }
        }
    }
    (layer, trace)
}
