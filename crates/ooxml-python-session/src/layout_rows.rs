//! Owns: the rows `ooxml_core.LaidPages` hands Python (lane L4): exact lengths as
//! `(numerator, denominator)` pairs, a page's frame, a face's grade, and a page view's lines,
//! spans, rules, pictures and drawing slots as tuples and dicts.
//!
//! Must not: lay out, paint or lock an engine (the views are given).

use ooxml_word_layout::paint::drawings::{DrawingSlot, Layer};
use ooxml_word_layout::paint::names::PageNames;
use ooxml_word_layout::paint::{Drawn, PaintedLine, Rule, Span};
use ooxml_word_layout::px::Px;
use ooxml_word_model::docx::VersionNames;
use ooxml_word_pages::PageView;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

/// An exact length as `(numerator, denominator)`.
pub(crate) type Exact = (i128, i128);

pub(crate) fn exact(value: Px) -> Exact {
    let q = value.value();
    (*q.numer(), *q.denom())
}

/// What `LaidPages.coverage` answers.
pub(crate) type CoverageRows = (Vec<GradeRow>, Vec<(String, char, String)>, Vec<String>);

/// A face's grade row (see `LaidPages.coverage`).
pub(crate) type GradeRow = (
    String,
    &'static str,
    Option<String>,
    bool,
    Option<&'static str>,
    Option<&'static str>,
);

pub(crate) fn grade_row(family: &str, grade: &ooxml_fonts::Grade) -> GradeRow {
    use ooxml_fonts::{Grade, SubstitutionKind};
    let plain = |name| (family.to_owned(), name, None, false, None, None);
    match grade {
        Grade::Installed => plain("installed"),
        Grade::Embedded => plain("embedded"),
        Grade::Recorded => plain("recorded"),
        Grade::Missing => plain("missing"),
        Grade::Substituted(s) => (
            family.to_owned(),
            "substituted",
            Some(s.substitute.clone()),
            s.same_metrics,
            Some(match s.kind {
                SubstitutionKind::Named => "named",
                SubstitutionKind::Default => "default",
                SubstitutionKind::Generic => "generic",
            }),
            s.class.map(ooxml_fonts::FaceClass::name),
        ),
    }
}

/// A page's frame row (see `LaidPages.pages`).
pub(crate) type PageRow = (
    (i64, usize, bool, &'static str, bool),
    (i64, i64, Exact, Exact, (Exact, Exact, Exact, Exact)),
);

pub(crate) fn page_row(
    info: &ooxml_word_layout::paginate::PageInfo,
    frame: &ooxml_word_layout::geometry::PageFrame,
) -> PageRow {
    let q = |v: ooxml_base::Q| (*v.numer(), *v.denom());
    (
        (
            info.number,
            info.section,
            info.section_first,
            info.story.name(),
            info.blank,
        ),
        (
            frame.width_px,
            frame.height_px,
            q(frame.width_pt),
            q(frame.height_pt),
            (
                exact(frame.left),
                exact(frame.right),
                exact(frame.top),
                exact(frame.bottom),
            ),
        ),
    )
}

pub(crate) fn slot_dict<'py>(
    py: Python<'py>,
    slot: &DrawingSlot,
    names: &PageNames<'_>,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    let paths = names.slot_paths(slot);
    d.set_item("path", &paths.path)?;
    d.set_item("paragraph_path", &paths.paragraph)?;
    d.set_item("run", slot.run)?;
    d.set_item("index", slot.index)?;
    d.set_item("inline", slot.inline)?;
    d.set_item("x", exact(slot.x))?;
    d.set_item("y", exact(slot.y))?;
    d.set_item("width", exact(slot.width))?;
    d.set_item("height", exact(slot.height))?;
    d.set_item("behind", slot.layer == Layer::Behind)?;
    d.set_item("order", slot.order)?;
    d.set_item("story", slot.story.as_ref().map(|(k, p)| (*k, p.to_string())))?;
    Ok(d)
}

pub(crate) fn span_tuple<'py>(py: Python<'py>, s: &Span, names: &PageNames<'_>) -> PyResult<Bound<'py, PyTuple>> {
    let xs: Vec<Exact> = s.pens().map(exact).collect();
    let items: Vec<Bound<'py, PyAny>> = vec![
        names.written(&s.name).into_pyobject(py)?.into_any(),
        s.chars.iter().collect::<String>().into_pyobject(py)?.into_any(),
        xs.into_pyobject(py)?.into_any(),
        s.y.into_pyobject(py)?.into_any(),
        exact(s.end).into_pyobject(py)?.into_any(),
        s.face.as_ref().into_pyobject(py)?.into_any(),
        s.bold.into_pyobject(py)?.to_owned().into_any(),
        s.italic.into_pyobject(py)?.to_owned().into_any(),
        s.half_points.into_pyobject(py)?.into_any(),
        s.color.as_deref().into_pyobject(py)?.into_any(),
        s.underline.as_deref().into_pyobject(py)?.into_any(),
        s.strike.into_pyobject(py)?.to_owned().into_any(),
        s.double_strike.into_pyobject(py)?.to_owned().into_any(),
        s.highlight.into_pyobject(py)?.into_any(),
        s.shading.as_deref().into_pyobject(py)?.into_any(),
        s.kind.name().into_pyobject(py)?.into_any(),
        s.line_baseline.into_pyobject(py)?.into_any(),
        s.vertical_align.as_deref().into_pyobject(py)?.into_any(),
    ];
    PyTuple::new(py, items)
}

pub(crate) fn line_tuple<'py>(
    py: Python<'py>,
    l: &PaintedLine,
    names: &PageNames<'_>,
) -> PyResult<Bound<'py, PyTuple>> {
    let spans = l
        .spans
        .iter()
        .map(|s| span_tuple(py, s, names))
        .collect::<PyResult<Vec<_>>>()?;
    let (story, part) = match &l.story {
        Some((kind, part)) => (Some(*kind), Some(part.to_string())),
        None => (None, None),
    };
    let items: Vec<Bound<'py, PyAny>> = vec![
        names.written(&l.name).into_pyobject(py)?.into_any(),
        l.number.into_pyobject(py)?.into_any(),
        l.baseline.into_pyobject(py)?.into_any(),
        exact(l.top).into_pyobject(py)?.into_any(),
        exact(l.pitch).into_pyobject(py)?.into_any(),
        l.paragraph_id.as_deref().into_pyobject(py)?.into_any(),
        l.band.into_pyobject(py)?.into_any(),
        story.into_pyobject(py)?.into_any(),
        part.into_pyobject(py)?.into_any(),
        l.column.into_pyobject(py)?.into_any(),
        spans.into_pyobject(py)?.into_any(),
    ];
    PyTuple::new(py, items)
}

pub(crate) fn rule_tuple<'a>(r: &'a Rule, names: &PageNames<'_>) -> RuleRow<'a> {
    (
        r.kind,
        names.written(&r.name),
        exact(r.x),
        exact(r.y),
        exact(r.width),
        exact(r.height),
        r.color.as_deref(),
        r.style.as_deref(),
    )
}

/// A rule's row (see `LaidPages.page`).
pub(crate) type RuleRow<'a> = (
    &'static str,
    String,
    Exact,
    Exact,
    Exact,
    Exact,
    Option<&'a str>,
    Option<&'a str>,
);

pub(crate) type CropRow = Option<[Exact; 4]>;

pub(crate) fn picture_tuple<'a>(
    p: &'a Drawn,
    names: &PageNames<'_>,
) -> (
    String,
    Option<&'a str>,
    Exact,
    Exact,
    Exact,
    Exact,
    Option<&'a str>,
    CropRow,
) {
    let crop = p.crop.map(|c| c.map(|q| (*q.numer(), *q.denom())));
    (
        names.written(&p.name),
        p.relationship.as_deref(),
        exact(p.x),
        exact(p.y),
        exact(p.width),
        exact(p.height),
        p.part.as_deref(),
        crop,
    )
}

/// Page `view` as a dict, what it draws named through the version's `names`.
pub(crate) fn view_dict<'py>(
    py: Python<'py>,
    view: &PageView,
    names: &VersionNames,
    with_lines: bool,
) -> PyResult<Bound<'py, PyDict>> {
    let names = PageNames::new(names, view.lines.iter().chain(view.text_box_lines.iter().flatten()));
    let names = &names;
    let d = PyDict::new(py);
    d.set_item("number", view.page + 1)?;
    d.set_item("info", page_row(&view.info, &view.frame))?;
    let lines = view
        .lines
        .iter()
        .filter(|_| with_lines)
        .map(|l| line_tuple(py, l, names))
        .collect::<PyResult<Vec<_>>>()?;
    d.set_item("lines", lines)?;
    d.set_item(
        "rules",
        view.rules.iter().map(|r| rule_tuple(r, names)).collect::<Vec<_>>(),
    )?;
    let pictures: Vec<_> = view.pictures.iter().map(|p| picture_tuple(p, names)).collect();
    d.set_item("pictures", pictures)?;
    let slots = view
        .drawings
        .iter()
        .zip(&view.drawing_faces)
        .zip(&view.text_box_lines)
        .map(|((s, faces), text)| {
            let d = slot_dict(py, s, names)?;
            d.set_item("faces", faces.clone())?;
            let lines = text
                .iter()
                .filter(|_| with_lines)
                .map(|l| line_tuple(py, l, names))
                .collect::<PyResult<Vec<_>>>()?;
            d.set_item("lines", lines)?;
            Ok(d)
        })
        .collect::<PyResult<Vec<_>>>()?;
    d.set_item("drawings", slots)?;
    d.set_item(
        "blocks",
        view.blocks.iter().map(ToString::to_string).collect::<Vec<_>>(),
    )?;
    d.set_item("unpainted", &view.unpainted)?;
    Ok(d)
}
