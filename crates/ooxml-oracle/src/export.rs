//! Owns: the Word model of a document as JSON, field by field (the form docx2svg's model was
//! once compared in): the source model (body, sections, stories, notes, styles, numbering,
//! settings, theme), every body paragraph's and table's resolved formats, each run's format
//! per character slot, and the body's list labels -- what the model's open and reopen
//! benchmark compares (`bench`); and the unlimited budget and key the benchmarks open with.
//!
//! Must not: be used by any library; it is a test instrument.

use ooxml_base::{DigestKey, Limits};
use ooxml_derive::CacheBudget;
use ooxml_opc::Package;
use ooxml_word_model::model::{
    Anchor, AnchorPosition, Block, BodyValue, Graphic, InlineObject, Paragraph, Run, RunProps, SectionProps, Table,
};
use ooxml_word_model::resolve::{Edge, ResolvedTable, Slot};
use ooxml_word_model::value::{Props, Value};
use ooxml_word_model::{Model, WordDocument};
use serde_json::{json, Map, Value as J};
use std::sync::Arc;

/// A budget without a limit, for a one-off export or benchmark.
pub struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _bytes: usize) -> bool {
        true
    }
    fn release(&self, _bytes: usize) {}
    fn used(&self) -> usize {
        0
    }
}

/// The key the tasks digest content with.
pub const KEY: DigestKey = DigestKey::new(0x0123_4567_89ab_cdef, 0x0fed_cba9_8765_4321);

/// Open `bytes` with a fresh model.
pub fn open(bytes: Vec<u8>) -> Result<(Model, WordDocument), String> {
    let package = Package::open(Arc::from(bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let mut model = Model::new(Arc::new(Unlimited), KEY);
    let document = model.open(&package).map_err(|e| e.to_string())?;
    Ok((model, document))
}

fn value(v: &Value) -> J {
    match v {
        Value::Bool(b) => json!(b),
        Value::Int(i) => json!(i),
        Value::Str(s) => json!(&**s),
        Value::Font(f) => json!({"face": f.face.as_deref(), "theme": f.theme.as_deref()}),
        Value::RunBorder(b) => json!([b.style.as_deref(), b.size, b.space]),
        Value::Border(b) => json!([b.style.as_deref(), b.size, b.space, b.color.as_deref()]),
        Value::Shading(s) => json!([s.kind.as_deref(), s.color.as_deref(), s.fill.as_deref()]),
        Value::Tabs(t) => J::Array(
            t.iter()
                .map(|s| json!([s.position, &*s.alignment, s.leader.as_deref()]))
                .collect(),
        ),
        Value::Attrs(a) => J::Object(a.iter().map(|(k, v)| (k.to_string(), json!(&**v))).collect()),
        Value::Width(w, k) | Value::Height(w, k) => json!([w, &**k]),
        Value::TextEffect(effect) => json!({"drawingml": ooxml_xml::view::local(&effect.element)}),
    }
}

/// Declared or resolved properties.
pub fn props(p: &Props) -> J {
    J::Object(p.iter().map(|(k, v)| (k.to_string(), value(v))).collect())
}

fn run_props(p: Option<&Arc<RunProps>>) -> J {
    p.map_or(
        J::Null,
        |r| json!({"style_id": r.style_id.as_deref(), "declared": props(&r.declared)}),
    )
}

fn position(p: &AnchorPosition) -> J {
    json!({"relative": p.relative.as_deref(), "offset": p.offset, "align": p.align.as_deref()})
}

fn graphic(g: &Graphic) -> J {
    let mut out = json!({
        "kind": &*g.kind, "offset": [g.offset.0, g.offset.1], "extent": [g.extent.0, g.extent.1],
        "rotation": g.rotation, "flip_h": g.flip_h, "flip_v": g.flip_v, "relationship": g.relationship.as_deref(),
        "text": blocks(&g.text), "children": J::Array(g.children.iter().map(|c| graphic(c)).collect()),
        "child_offset": [g.child_offset.0, g.child_offset.1], "child_extent": [g.child_extent.0, g.child_extent.1],
        "unsupported": J::Array(g.unsupported.iter().map(|u| json!(&**u)).collect()),
        "uri": g.uri.as_deref(), "path": &*g.path,
    });
    if &*g.kind != "textpath" {
        let body: Map<String, J> = g
            .body
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    match v {
                        BodyValue::Int(i) => json!(i),
                        BodyValue::Str(s) => json!(&**s),
                    },
                )
            })
            .collect();
        if let Some(o) = out.as_object_mut() {
            o.insert("body".into(), J::Object(body));
        }
    }
    out
}

fn anchor(a: &Anchor) -> J {
    json!({
        "extent": [a.extent.0, a.extent.1], "effect": a.effect, "h": position(&a.h), "v": position(&a.v),
        "simple": a.simple.map(|(x, y)| json!([x, y])), "wrap": a.wrap.as_deref(), "wrap_text": a.wrap_text.as_deref(),
        "distance": a.distance, "behind": a.behind, "relative_height": a.relative_height,
        "allow_overlap": a.allow_overlap, "layout_in_cell": a.layout_in_cell, "locked": a.locked,
        "graphic": a.graphic.as_deref().map(graphic), "identifier": a.identifier.as_deref(),
        "polygon": a.polygon.as_ref().map(|p| J::Array(p.iter().map(|(x, y)| json!([x, y])).collect())),
        "vml": a.vml,
    })
}

fn run_json(r: &Run) -> J {
    json!({
        "text": &*r.text, "properties": run_props(r.properties.as_ref()),
        "breaks": J::Array(r.breaks.iter().map(|(i, m)| json!([i, &**m])).collect()),
        "path": &*r.path,
        "drawings": J::Array(r.drawings.iter().map(|d| match d {
            InlineObject::None => J::Null,
            InlineObject::Picture(rid) => json!({"picture": &**rid}),
            InlineObject::Graphic(g) => graphic(g),
        }).collect()),
        "anchors": J::Array(r.anchors.iter().map(anchor).collect()),
        "field": r.field.as_ref().map(|f| json!({
            "instruction": &*f.instruction, "keyword": &*f.keyword,
            "cached": J::Array(f.cached.iter().map(|(t, p)| json!([&**t, run_props(p.as_ref())])).collect()),
        })),
        "toc_link": r.toc_link,
    })
}

/// A section's properties.
pub fn section_props(s: &SectionProps) -> J {
    let m = &s.margins;
    json!({
        "page_size": [s.page_size.width, s.page_size.height, s.page_size.orientation.as_deref()],
        "margins": [m.top, m.right, m.bottom, m.left, m.header, m.footer, m.gutter],
        "start_type": s.start_type.as_deref(), "column_count": s.column_count, "column_space": s.column_space,
        "columns": J::Array(s.columns.iter().map(|(w, sp)| json!([w, sp])).collect()),
        "column_separator": s.column_separator,
        "headers": J::Array(s.headers.iter().map(|(t, r)| json!([&**t, &**r])).collect()),
        "footers": J::Array(s.footers.iter().map(|(t, r)| json!([&**t, &**r])).collect()),
        "title_page": s.title_page, "page_number_start": s.page_number_start,
        "page_number_format": s.page_number_format.as_deref(), "vertical_alignment": s.vertical_alignment.as_deref(),
        "endnote_format": s.endnotes.format.as_deref(), "endnote_start": s.endnotes.start,
        "endnote_restart": s.endnotes.restart.as_deref(), "footnote_format": s.footnotes.format.as_deref(),
        "footnote_start": s.footnotes.start, "footnote_restart": s.footnotes.restart.as_deref(),
        "footnote_position": s.footnotes.position.as_deref(),
        "line_numbering": s.line_numbering.as_ref().map(|l| json!([l.count_by, l.start, l.distance, &*l.restart])),
    })
}

fn paragraph(p: &Paragraph) -> J {
    json!({
        "type": "p", "path": &*p.path, "para_id": p.para_id.as_deref(),
        "runs": J::Array(p.runs.iter().map(run_json).collect()),
        "properties": p.properties.as_ref().map(|x| json!({
            "style_id": x.style_id.as_deref(), "declared": props(&x.declared), "mark": run_props(x.mark.as_ref()),
        })),
        "table_style_id": p.table_style_id.as_deref(),
        "table_conditions": J::Array(p.table_conditions.iter().map(|c| json!(&**c)).collect()),
        "joins": &*p.joins, "mark_deleted": p.mark_deleted,
        "section": p.section.as_deref().map(section_props),
        "bookmarks": J::Array(p.bookmarks.iter().map(|b| json!([if b.start { "start" } else { "end" }, b.id.as_deref(), b.name.as_deref(), b.run])).collect()),
    })
}

fn table(t: &Table) -> J {
    let l = &t.look;
    json!({
        "type": "tbl", "style_id": t.style_id.as_deref(), "path": &*t.path, "grid": &*t.grid,
        "properties": props(&t.properties),
        "row_properties": J::Array(t.row_properties.iter().map(props).collect()),
        "cell_properties": J::Array(t.cell_properties.iter().map(|r| J::Array(r.iter().map(props).collect())).collect()),
        "look": {"firstRow": l.first_row, "lastRow": l.last_row, "firstColumn": l.first_column,
                 "lastColumn": l.last_column, "noHBand": l.no_h_band, "noVBand": l.no_v_band},
        "conditions": J::Array(t.conditions.iter().map(|r| J::Array(r.iter().map(|c| json!(c.iter().map(|x| &**x).collect::<Vec<_>>())).collect())).collect()),
        "cell_paths": J::Array(t.cell_paths.iter().map(|r| json!(r.iter().map(|x| &**x).collect::<Vec<_>>())).collect()),
        "joined": t.joined,
        "rows": J::Array(t.rows.iter().map(|r| J::Array(r.iter().map(|c| blocks(c)).collect())).collect()),
    })
}

/// A story's blocks.
pub fn blocks(items: &[Block]) -> J {
    J::Array(
        items
            .iter()
            .map(|b| match b {
                Block::Paragraph(p) => paragraph(p),
                Block::Table(t) => table(t),
            })
            .collect(),
    )
}

fn edge(e: &Edge) -> J {
    let border =
        e.0.as_ref()
            .map(|b| json!([b.style.as_deref(), b.size, b.space, b.color.as_deref()]));
    json!([border, if e.1 { "right" } else { "left" }])
}

fn resolved_table(t: &ResolvedTable, table: &Table) -> J {
    let path = &*table.path;
    let cell_path = |r: usize, c: usize| {
        table.cell_paths.get(r).and_then(|row| row.get(c)).map_or_else(
            || format!("{path}/w:tr[{}]/w:tc[{}]", r + 1, c + 1),
            ToString::to_string,
        )
    };
    json!({
        "path": path, "properties": props(&t.properties), "indent_stated": t.indent_stated, "grid": t.grid,
        "mismatched": J::Array(t.mismatched.iter().map(|(r, n)| json!([r, n])).collect()),
        "rows": J::Array(t.rows.iter().map(|r| json!({
            "index": r.index, "properties": props(&r.properties), "grid_before": r.grid_before, "grid_after": r.grid_after,
            "cells": J::Array(r.cells.iter().map(|c| json!({
                "row": c.row, "index": c.index, "column": c.column, "span": c.span, "properties": props(&c.properties),
                "merge": c.merge.as_deref(), "path": cell_path(c.row, c.index), "margins": c.margins,
                "borders": J::Array(c.borders.iter().map(|b| b.as_ref().map_or(J::Null, |b| json!([b.style.as_deref(), b.size, b.space, b.color.as_deref()]))).collect()),
                "own": c.own, "left_edge": edge(&c.left_edge), "right_edge": edge(&c.right_edge),
            })).collect()),
        })).collect()),
    })
}

fn formats(document: &WordDocument, resolved: &Props) -> J {
    let slots: &[Slot] = if ooxml_word_model::resolve::complex_script(resolved) {
        &[Slot::ComplexScript]
    } else {
        &[Slot::Ascii, Slot::HAnsi, Slot::EastAsia]
    };
    J::Object(
        slots
            .iter()
            .map(|s| {
                let f = document.character_format(resolved, *s);
                (
                    s.name().to_owned(),
                    json!({
                        "face": &*f.face, "half_points": f.half_points, "bold": f.bold, "italic": f.italic,
                        "vertical_align": f.vertical_align.as_deref(), "position": f.position,
                        "border": f.border.map(|(w, s)| json!([w, s])),
                    }),
                )
            })
            .collect(),
    )
}

fn walk(model: &mut Model, document: &WordDocument, block: &Block, paragraphs: &mut Vec<J>, tables: &mut Vec<J>) {
    match block {
        Block::Paragraph(p) => {
            let r = model.resolve_paragraph(document, p);
            paragraphs.push(json!({
                "path": &*p.path, "paragraph": props(&r.paragraph), "mark": props(&r.mark),
                "runs": J::Array(r.runs.iter().map(|x| props(x)).collect()),
                "formats": J::Array(r.runs.iter().map(|x| formats(document, x)).collect()),
                "mark_formats": formats(document, &r.mark),
            }));
        }
        Block::Table(t) => {
            tables.push(resolved_table(&model.resolve_table(document, t), t));
            for cell in t.rows.iter().flat_map(|r| r.iter()) {
                cell.iter().for_each(|b| walk(model, document, b, paragraphs, tables));
            }
        }
    }
}

/// The resolved part of the export: paragraphs and tables of the body in document order.
pub fn resolved(model: &mut Model, document: &WordDocument) -> (J, J) {
    let (mut paragraphs, mut tables) = (Vec::new(), Vec::new());
    for block in document.body.iter() {
        walk(model, document, block, &mut paragraphs, &mut tables);
    }
    (J::Array(paragraphs), J::Array(tables))
}

/// The whole export of a document.
pub fn document_json(model: &mut Model, d: &WordDocument) -> J {
    let parts = crate::export_parts::parts(d);
    let (paragraphs, tables) = resolved(model, d);
    let mut out = match parts {
        J::Object(o) => o,
        _ => Map::new(),
    };
    out.insert("body".into(), blocks(&d.body));
    out.insert("labels".into(), json!(d.body_labels()));
    let stories: Map<String, J> = d
        .stories
        .iter()
        .map(|(k, v)| (k.to_string(), json!(model.story_labels(d, v))))
        .collect();
    out.insert("story_labels".into(), J::Object(stories));
    let boxes: Map<String, J> = model
        .text_box_labels(d)
        .into_iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect();
    out.insert("text_box_labels".into(), J::Object(boxes));
    out.insert("resolved_paragraphs".into(), paragraphs);
    out.insert("resolved_tables".into(), tables);
    J::Object(out)
}
