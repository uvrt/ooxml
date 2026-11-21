//! Owns: a series' own point labels written into the chart part (ooxml-edit's
//! `ChartStyling.set_point_labels`, `charts/styling.py`): each point's `c:dLbl` with its
//! text (or, for Value From Cells, a `CELLRANGE` field and the flags PowerPoint writes,
//! measured) in the series' label style, at a position the plot takes, the series' labels
//! made in Office's data-label style where it has none; the labels' range
//! (`c15:datalabelsRange`, with its cache) linked; and the point labels taken away again.
//!
//! Must not: write the workbook or choose the labels' cells (the caller does, with
//! `ooxml-sheets`, and gives the range's formula), resolve the series by name, or hash (the
//! caller gives SHA-256).

use super::look::{value_labels, LabelLook};
use super::make::Make;
use super::owned::Owned;
use super::series::{Hash, SER_ORDER};
use super::text_style::DLBLS_ORDER;
use super::{ChartTree, DataError};
use ooxml_xml::view::{attr, child, children, local};
use ooxml_xml::Element;
use std::sync::Arc;

/// The `c:ext` a series' Value From Cells range is in, and the one a label's flag is in.
const LABEL_RANGE_EXT: &str = "{02D57815-91ED-43cb-92C2-25804820EDAC}";
const LABEL_FLAGS_EXT: &str = "{CE6537A1-D6FC-4f65-9D91-7224C49458BB}";
/// The label positions by name (`LABEL_POSITIONS`), in their order.
pub const LABEL_POSITIONS: [(&str, &str); 9] = [
    ("center", "ctr"),
    ("inside_end", "inEnd"),
    ("inside_base", "inBase"),
    ("outside_end", "outEnd"),
    ("left", "l"),
    ("right", "r"),
    ("top", "t"),
    ("bottom", "b"),
    ("best_fit", "bestFit"),
];
/// The longest label text (characters).
const LONGEST: usize = 255;
/// The show flags of a label, in their order.
const FLAGS: [&str; 6] = [
    "showLegendKey",
    "showVal",
    "showCatName",
    "showSerName",
    "showPercent",
    "showBubbleSize",
];

/// What `set_point_labels` writes.
#[derive(Debug, Clone, Copy)]
pub struct PointLabels<'a> {
    /// One text per point; `None` takes the series' point labels away.
    pub labels: Option<&'a [String]>,
    /// The position's code (`ST_DLblPos`), checked by [`ChartTree::check_point_labels`].
    pub position: Option<&'a str>,
    /// Value From Cells: the formula of the labels' range in the workbook.
    pub cells: Option<&'a str>,
    /// The size of labels made new (`look.data_labels`, hundredths of a point).
    pub size: Option<u32>,
    /// The language of the text.
    pub lang: Option<&'a str>,
    /// The chart part's name (a field's id is drawn from it).
    pub part: &'a str,
}

impl ChartTree {
    /// `set_point_labels`' checks, in its order: one label per point of the series at
    /// `series`, none past 255 characters, and the position (by name) one its plot takes.
    /// The position's code.
    ///
    /// # Errors
    /// [`DataError`] (`ChartDataError`) for each.
    pub fn check_point_labels(
        &self,
        address: &str,
        series: usize,
        labels: Option<&[String]>,
        position: Option<&str>,
    ) -> Result<Option<&'static str>, DataError> {
        let count = self.series_point_count(series);
        if let Some(labels) = labels {
            if labels.len() != count {
                return Err(DataError(format!(
                    "{address}: {} labels for {count} points",
                    labels.len()
                )));
            }
            if labels.iter().any(|t| t.chars().count() > LONGEST) {
                return Err(DataError("a point label is text of at most 255 characters".to_owned()));
            }
        }
        let Some(position) = position else { return Ok(None) };
        let allowed = self.series(series).map_or(&[][..], |(path, _)| {
            let plot = path.split_last().and_then(|(_, p)| super::path::at(&self.root, p));
            plot.map_or(&[][..], positions)
        });
        let code = LABEL_POSITIONS
            .iter()
            .find(|(name, _)| *name == position)
            .map(|(_, c)| *c);
        match code {
            Some(code) if allowed.contains(&code) => Ok(Some(code)),
            _ => {
                let names: Vec<&str> = LABEL_POSITIONS
                    .iter()
                    .filter(|(_, c)| allowed.contains(c))
                    .map(|(n, _)| *n)
                    .collect();
                let takes = if names.is_empty() {
                    "none (leave it out)".to_owned()
                } else {
                    names.join(", ")
                };
                Err(DataError(format!(
                    "{address}: label position '{position}'; this chart takes {takes}"
                )))
            }
        }
    }

    /// Unlink the labels' range of the series at `series` (`_drop_label_range`).
    pub fn drop_label_range(&mut self, series: usize) {
        self.change_series(series, drop_range);
    }

    /// Write the point labels of the series at `series` (`set_point_labels`, past its checks).
    pub fn set_point_labels(&mut self, series: usize, look: &PointLabels<'_>, hash: Hash<'_>) {
        let make = Make::of(&self.root);
        let pie = self
            .series(series)
            .and_then(|(path, _)| path.split_last().and_then(|(_, p)| super::path::at(&self.root, p)))
            .is_some_and(|p| local(p) == "pieChart");
        self.change_series(series, |ser| {
            if let Some(holder) = ser.child("dLbls") {
                remove_named(holder, &["dLbl"]);
            }
            drop_range(ser);
            let Some(labels) = look.labels else {
                if let Some(at) = ser.place("dLbls") {
                    if ser.read(at).is_some_and(|h| !shows_anything(&h)) {
                        ser.remove(at);
                    }
                }
                return;
            };
            let at = match ser.place("dLbls") {
                Some(at) if !ser.read(at).is_some_and(|h| deleted(&h)) => at,
                found => {
                    if let Some(at) = found {
                        ser.remove(at);
                    }
                    let shown = LabelLook {
                        size: look.size,
                        number_format: None,
                        lang: look.lang,
                    };
                    let mut holder = value_labels(&make, &shown, pie);
                    if let Some(flag) = holder.child("showVal") {
                        flag.set_attr("val", "0");
                    }
                    ser.insert_ordered(Some(SER_ORDER), holder)
                }
            };
            if let Some(formula) = look.cells {
                link_range(ser, &make, formula, labels);
            }
            let idx = ser
                .read_child("idx")
                .and_then(|n| attr(&n, "val").map(str::to_owned))
                .unwrap_or_default();
            let Some(holder) = ser.at(at) else { return };
            if let Some(code) = look.position {
                match holder.child("dLblPos") {
                    Some(node) => node.set_attr("val", code),
                    None => {
                        holder.insert_ordered(Some(DLBLS_ORDER), make.c("dLblPos").with_attr("val", code));
                    }
                }
            }
            if look.cells.is_some() {
                range_flag(holder, &make);
            }
            let read = Arc::new(holder.clone().close());
            let style = LabelStyle::of(&read, &make, look, &idx);
            for (index, text) in labels.iter().enumerate() {
                let label = style.label(index, text, hash);
                insert_label(holder, label, index);
            }
        });
    }
}

/// How a series' point labels are drawn: its labels' text style, number format and shape
/// (`_label_text_style`, copied into each).
struct LabelStyle<'a> {
    make: &'a Make,
    look: &'a PointLabels<'a>,
    /// The series' `c:idx`, in a field's seed.
    idx: &'a str,
    body: Owned,
    defaults: Owned,
    format: Option<Owned>,
    shape: Option<Owned>,
}

impl<'a> LabelStyle<'a> {
    fn of(holder: &Element, make: &'a Make, look: &'a PointLabels<'a>, idx: &'a str) -> LabelStyle<'a> {
        let (body, defaults) = text_style(holder, make);
        LabelStyle {
            make,
            look,
            idx,
            body,
            defaults,
            format: child(holder, "numFmt").map(Owned::open),
            shape: child(holder, "spPr").map(Owned::open),
        }
    }

    /// Point `index`'s `c:dLbl` with `text` (a `CELLRANGE` field for Value From Cells).
    fn label(&self, index: usize, text: &str, hash: Hash<'_>) -> Owned {
        let (make, look) = (self.make, self.look);
        let seed = format!("{}\n{}\n{index}", look.part, self.idx);
        let mut paragraph = make.a("p").with(make.a("pPr").with(self.defaults.clone()));
        if look.cells.is_some() {
            let id = guid(&format!("{seed}\nc"), hash);
            paragraph.push(field("CELLRANGE", "[CELLRANGE]", &id, look.lang, make));
        } else {
            paragraph.push(run(text, look.lang, make));
        }
        let mut end = make.a("endParaRPr");
        if let Some(lang) = look.lang.filter(|l| !l.is_empty()) {
            end.set_attr("lang", lang);
        }
        paragraph.push(end);
        let rich = make
            .c("rich")
            .with(make.top(self.body.clone()))
            .with(make.top(make.a("lstStyle")))
            .with(make.top(paragraph));
        let mut label = make
            .c("dLbl")
            .with(make.c("idx").with_attr("val", &index.to_string()))
            .with(make.c("tx").with(rich));
        if let Some(format) = &self.format {
            label.push(format.clone());
        }
        if let Some(shape) = &self.shape {
            label.push(shape.clone());
        }
        if let Some(code) = look.position {
            label.push(make.c("dLblPos").with_attr("val", code));
        }
        for flag in FLAGS {
            let on = flag == "showVal" && look.cells.is_none();
            label.push(make.c(flag).with_attr("val", if on { "1" } else { "0" }));
        }
        if look.cells.is_some() {
            let ext = make
                .c("ext")
                .with_attr("uri", LABEL_FLAGS_EXT)
                .with(make.c15("dlblFieldTable"))
                .with(make.c15("xForSave").with_attr("val", "1"))
                .with(make.c15("showDataLabelsRange").with_attr("val", "1"));
            label.push(make.c("extLst").with(make.top15(ext)));
        }
        label
    }
}

impl Owned {
    /// The first element child named `name`, read.
    fn read_child(&self, name: &str) -> Option<Arc<Element>> {
        self.place(name).and_then(|at| self.read(at))
    }
}

/// The positions a plot's labels take (`_positions`).
fn positions(plot: &Element) -> &'static [&'static str] {
    match local(plot) {
        "barChart" => {
            let grouping = child(plot, "grouping").and_then(|g| attr(g, "val"));
            if matches!(grouping, Some("stacked" | "percentStacked")) {
                &["ctr", "inEnd", "inBase"]
            } else {
                &["ctr", "inEnd", "inBase", "outEnd"]
            }
        }
        "lineChart" | "scatterChart" | "bubbleChart" => &["ctr", "l", "r", "t", "b"],
        "pieChart" => &["ctr", "inEnd", "outEnd", "bestFit"],
        _ => &[],
    }
}

/// Take out `owner`'s children named any of `names`.
fn remove_named(owner: &mut Owned, names: &[&str]) {
    let mut places: Vec<usize> = owner
        .elements()
        .into_iter()
        .filter(|(_, n)| names.contains(&n.as_str()))
        .map(|(at, _)| at)
        .collect();
    places.reverse();
    for at in places {
        owner.remove(at);
    }
}

/// Take out the `c:ext`s of `owner`'s `c:extLst` whose `uri` is `uri`, and the list where
/// it is left empty.
fn remove_ext(owner: &mut Owned, uri: &str) {
    let Some(at) = owner.place("extLst") else { return };
    let Some(list) = owner.at(at) else { return };
    let doomed: Vec<usize> = list
        .elements()
        .into_iter()
        .filter(|(place, name)| name == "ext" && list.read(*place).is_some_and(|e| attr(&e, "uri") == Some(uri)))
        .map(|(place, _)| place)
        .rev()
        .collect();
    for place in doomed {
        list.remove(place);
    }
    if list.elements().is_empty() {
        owner.remove(at);
    }
}

/// `_drop_label_range`: the series' range ext, and its labels' flag ext.
fn drop_range(ser: &mut Owned) {
    remove_ext(ser, LABEL_RANGE_EXT);
    if let Some(holder) = ser.child("dLbls") {
        remove_ext(holder, LABEL_FLAGS_EXT);
    }
}

/// Whether a `c:dLbls` is deleted (`_deleted`: `c:delete` on, its value defaulting to on).
fn deleted(holder: &Element) -> bool {
    child(holder, "delete").is_some_and(|d| matches!(attr(d, "val").unwrap_or("1"), "1" | "true"))
}

/// Whether a `c:dLbls` shows anything (`_shows_anything`: a show flag set on).
fn shows_anything(holder: &Element) -> bool {
    FLAGS
        .iter()
        .any(|flag| child(holder, flag).is_some_and(|f| matches!(attr(f, "val"), Some("1" | "true"))))
}

/// The series' labels' range linked (`_label_cells`' end): first in its `c:extLst`, with
/// the labels as its cache.
fn link_range(ser: &mut Owned, make: &Make, formula: &str, labels: &[String]) {
    let mut cache = make
        .c15("dlblRangeCache")
        .with(make.c("ptCount").with_attr("val", &labels.len().to_string()));
    for (index, text) in labels.iter().enumerate() {
        cache.push(
            make.c("pt")
                .with_attr("idx", &index.to_string())
                .with(make.c("v").with_text(text)),
        );
    }
    let range = make
        .c15("datalabelsRange")
        .with(make.c15("f").with_text(formula))
        .with(cache);
    let ext = make.top15(make.c("ext").with_attr("uri", LABEL_RANGE_EXT).with(range));
    let at = match ser.place("extLst") {
        Some(at) => at,
        None => ser.insert_ordered(Some(SER_ORDER), make.c("extLst")),
    };
    if let Some(list) = ser.at(at) {
        list.insert(0, ext);
    }
}

/// `_set_range_flag`: the series labels' Value From Cells flag on.
fn range_flag(holder: &mut Owned, make: &Make) {
    let list_at = match holder.place("extLst") {
        Some(at) => at,
        None => holder.insert_ordered(Some(DLBLS_ORDER), make.c("extLst")),
    };
    let Some(list) = holder.at(list_at) else { return };
    let found = list
        .elements()
        .into_iter()
        .find(|(place, name)| {
            name == "ext"
                && list
                    .read(*place)
                    .is_some_and(|e| attr(&e, "uri") == Some(LABEL_FLAGS_EXT))
        })
        .map(|(place, _)| place);
    let ext_at = match found {
        Some(at) => at,
        None => {
            list.push(make.top15(make.c("ext").with_attr("uri", LABEL_FLAGS_EXT)));
            list.elements().last().map_or(0, |(at, _)| *at)
        }
    };
    let Some(ext) = list.at(ext_at) else { return };
    match ext.child("showDataLabelsRange") {
        Some(flag) => flag.set_attr("val", "1"),
        None => ext.push(make.c15("showDataLabelsRange").with_attr("val", "1")),
    }
}

/// `_label_text_style`: the `a:bodyPr` and `a:defRPr` the series' labels are drawn with.
fn text_style(holder: &Element, make: &Make) -> (Owned, Owned) {
    let text = child(holder, "txPr");
    let body = text.and_then(|t| child(t, "bodyPr"));
    let defaults = text.and_then(|t| {
        children(t, "p").find_map(|p| children(p, "pPr").find_map(|properties| child(properties, "defRPr")))
    });
    (
        body.map_or_else(|| make.a("bodyPr"), Owned::open),
        defaults.map_or_else(|| make.a("defRPr"), Owned::open),
    )
}

/// `_run`: a text run.
fn run(text: &str, lang: Option<&str>, make: &Make) -> Owned {
    let mut properties = make.a("rPr");
    if let Some(lang) = lang.filter(|l| !l.is_empty()) {
        properties.set_attr("lang", lang);
    }
    make.a("r").with(properties).with(make.a("t").with_text(text))
}

/// `_field`: a field of `kind` showing `text`.
fn field(kind: &str, text: &str, identifier: &str, lang: Option<&str>, make: &Make) -> Owned {
    let mut properties = make.a("rPr");
    if let Some(lang) = lang.filter(|l| !l.is_empty()) {
        properties.set_attr("lang", lang);
    }
    make.a("fld")
        .with_attr("id", identifier)
        .with_attr("type", kind)
        .with(properties)
        .with(make.a("pPr"))
        .with(make.a("t").with_text(text))
}

/// `_guid`: a GUID drawn from SHA-256 of `seed`.
fn guid(seed: &str, hash: Hash<'_>) -> String {
    let digest = hash(seed.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02X}")).collect();
    format!(
        "{{{}-{}-{}-{}-{}}}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// `_insert_label`: a `c:dLbl` in index order, before the series-wide settings.
fn insert_label(holder: &mut Owned, label: Owned, index: usize) {
    let places = holder.places("dLbl");
    let later = places.iter().copied().find(|&at| {
        holder
            .read(at)
            .and_then(|l| child(&l, "idx").and_then(|n| attr(n, "val")).map(str::to_owned))
            .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|v| v.parse::<u128>().ok())
            .is_some_and(|v| v > index as u128)
    });
    match (later, places.last()) {
        (Some(at), _) => holder.insert(at, label),
        (None, Some(&last)) => {
            let at = holder.after(last);
            holder.insert(at, label);
        }
        (None, None) => holder.insert(0, label),
    }
}
