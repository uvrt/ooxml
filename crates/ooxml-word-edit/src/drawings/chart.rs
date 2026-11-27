//! Owns: [`InsertChart`], a new chart inserted as docx-agent's `insert_chart` writes one (as
//! Word inserts a chart): in a new paragraph after a block (one continuing a heading's,
//! title's or subtitle's formatting takes `Normal`) or inline at a position; the chart part
//! (`word/charts/chart{n}.xml`, an `Override`) and its embedded workbook
//! (`word/embeddings/Microsoft_Excel_Worksheet{n}.xlsx`), each the lowest number free,
//! related chart to workbook, then story to chart; a `wp:inline` with Word's extent and
//! effect extent, a `docPr` numbered past every drawing the document has had,
//! `wp14:anchorId` and `wp14:editId` drawn from `{part}\0chart`, in a run of its own with
//! `w:noProof`. Tracked, the paragraph and the run are insertions.
//!
//! Must not: know what a chart holds -- its part and workbook come from a [`ChartParts`]
//! -- or write but through the sink.

use super::{emu, DrawingEdited, A};
use crate::controls::{rewrite, InParagraph, Rewrite};
use crate::doc::Key;
use crate::names::{W, WP, WP14};
use crate::paragraphs::insert_paragraph;
use crate::policy::RevisionPolicy;
use crate::ranges::renamed_id;
use crate::sink::{EditSink, TextWrite};
use crate::stamping::stamp_document;
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_base::Limits;
use ooxml_opc::name::rels_for;
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::resolve::refuse_inside_table;
use ooxml_word_model::docx::{find_path, style_name, Docx};
use ooxml_xml::Tree;
use std::sync::Arc;

/// DrawingML's charts.
const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
/// Relationships.
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// A story's relationship to its chart.
const REL_CHART: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";
/// A chart's relationship to its embedded workbook.
const REL_PACKAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/package";
/// A chart part's content type.
const CT_CHART: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
/// An embedded workbook's.
const CT_XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

/// What a new chart is made of, made by its caller (who knows charts).
pub trait ChartParts {
    /// The chart part's tree, for the part `part` (the seed of its ids) whose relationship
    /// to its workbook is `workbook_rel`, and the workbook's bytes.
    ///
    /// # Errors
    /// The chart's refusal: nothing is written.
    fn parts(&self, part: &str, workbook_rel: &str) -> Result<(Tree, Arc<[u8]>)>;
}

/// A new chart inserted.
#[derive(Debug, Clone)]
pub struct InsertChart<P> {
    /// A block the chart's paragraph follows, or a position inside a paragraph (`p:..@n`).
    pub at: ooxml_word_model::address::Address,
    /// Its parts.
    pub chart: P,
    /// Its size in points.
    pub size: (f64, f64),
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What [`InsertChart`] did: the drawing (its id, the paragraph made for it first in
/// `created`) and the chart's part.
#[derive(Debug, Clone)]
pub struct ChartInserted {
    /// The drawing.
    pub drawing: DrawingEdited,
    /// The chart's part.
    pub part: ooxml_opc::PartName,
}

/// Where a story keeps its charts (ooxml-edit's `_charts_folder`).
fn charts_folder(part: &str) -> String {
    let part = part.trim_matches('/');
    match part.split_once('/') {
        Some((top, _)) => format!("{top}/charts"),
        None => "charts".to_owned(),
    }
}

/// `template` with `{n}` the lowest number from 1 no part has (`unused_part_name`).
fn unused(package: &ooxml_opc::Package, template: &str) -> String {
    (1u32..)
        .map(|n| template.replace("{n}", &n.to_string()))
        .find(|name| !package.has_part(name))
        .unwrap_or_default()
}

/// The style a paragraph after `at` takes: `Normal` where `at` is a paragraph whose
/// style's name (the default paragraph style's, none declared) starts with heading, title
/// or subtitle.
fn continued_style(doc: &WordDoc, docx: &Docx, at: &str) -> Option<&'static str> {
    if !at.starts_with("p:") {
        return None;
    }
    let found = crate::set_text::find(doc, docx, at).ok()?;
    let declared = find_path(&found.element, &["pPr", "pStyle"]).and_then(|s| s.attribute(Some(W), "val"));
    let style = declared
        .and_then(|id| crate::styles::find_style(docx, id, Some("paragraph")))
        .or_else(|| {
            docx.styles()
                .iter()
                .find(|s| {
                    s.attribute(Some(W), "type") == Some("paragraph")
                        && s.attribute(Some(W), "default").is_some_and(ooxml_word_model::docx::on)
                })
                .cloned()
        })?;
    let name = style_name(&style)?.to_lowercase();
    ["heading", "title", "subtitle"]
        .iter()
        .any(|p| name.starts_with(p))
        .then_some("Normal")
}

impl<P: ChartParts> InsertChart<P> {
    /// The `w:drawing` Word writes for an inline chart (docx-agent's `_chart_drawing`).
    fn xml(&self, number: u64, rid: &str, (anchor_id, edit_id): (&str, &str)) -> String {
        let (cx, cy) = (emu(self.size.0), emu(self.size.1));
        format!(
            "<w:drawing xmlns:w=\"{W}\"><wp:inline xmlns:wp=\"{WP}\" xmlns:wp14=\"{WP14}\" distT=\"0\" distB=\"0\" \
             distL=\"0\" distR=\"0\" wp14:anchorId=\"{anchor_id}\" wp14:editId=\"{edit_id}\">\
             <wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"12700\" b=\"12700\"/>\
             <wp:docPr id=\"{number}\" name=\"Chart {number}\"/><wp:cNvGraphicFramePr/>\
             <a:graphic xmlns:a=\"{A}\"><a:graphicData uri=\"{C}\">\
             <c:chart xmlns:c=\"{C}\" xmlns:r=\"{R}\" r:id=\"{rid}\"/></a:graphicData></a:graphic>\
             </wp:inline></w:drawing>"
        )
    }

    /// The chart part and its workbook added and related to `part` (ooxml-edit's
    /// `add_chart`, in its order); the chart part's name and the story's relationship id.
    fn add_parts(&self, sink: &mut EditSink<'_, '_>, part: &str) -> Result<(String, String)> {
        let package = sink.doc().package().clone();
        let folder = charts_folder(part);
        let chart = unused(&package, &format!("{folder}/chart{{n}}.xml"));
        let above = folder.rsplit_once('/').map_or("", |(top, _)| top);
        let embeddings = if above.is_empty() {
            "embeddings".to_owned()
        } else {
            format!("{above}/embeddings")
        };
        let book = unused(&package, &format!("{embeddings}/Microsoft_Excel_Worksheet{{n}}.xlsx"));
        // the names chosen stay free only while no other edit takes them
        sink.read(Key::part(chart.as_str()));
        sink.read(Key::part(book.as_str()));
        let (related, workbook_rel) = crate::new_part::add_internal(&package, &chart, &book, REL_PACKAGE)?;
        let (tree, bytes) = self.chart.parts(&chart, &workbook_rel)?;
        sink.put_bytes(&book, bytes)?;
        if let Some(made) = crate::new_part::declare_type(sink.doc().package(), &book, CT_XLSX)? {
            put_from(sink, &made, ooxml_opc::CONTENT_TYPES_PART)?;
        }
        sink.put(&chart, tree)?;
        if let Some(made) = crate::new_part::declare_type_override(sink.doc().package(), &chart, CT_CHART)? {
            put_from(sink, &made, ooxml_opc::CONTENT_TYPES_PART)?;
        }
        if let Some(made) = related {
            put_from(sink, &made, &rels_for(&chart))?;
        }
        let (made, rid) = crate::new_part::add_internal(sink.doc().package(), part, &chart, REL_CHART)?;
        if let Some(made) = made {
            put_from(sink, &made, &rels_for(part))?;
        }
        Ok((chart, rid))
    }

    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<ChartInserted> {
        check_size(self.size)?;
        let mut created = Vec::new();
        let mut renamed = Vec::new();
        let at = if self.at.as_str().contains('@') {
            self.at.clone()
        } else {
            let docx = sink.docx()?;
            no_row_or_cell(&docx, self.at.as_str())?;
            let style = continued_style(sink.doc(), &docx, self.at.as_str());
            let inserted = insert_paragraph(sink, self.at.as_str(), (true, "", style))?;
            created.push(inserted.id.to_string());
            renamed.extend(inserted.renamed);
            ooxml_word_model::address::Address::from(format!("{}@0", inserted.id))
        };
        let docx = sink.docx()?;
        let place = crate::fields::place(sink.doc(), &docx, &at, true)?;
        let stamped = stamp_document(sink, &docx)?;
        let docx = sink.docx()?;
        let id = renamed_id(&stamped, &place.id);
        renamed.extend(stamped);
        let found = crate::set_text::find(sink.doc(), &docx, &id)?;
        let part = found.part.clone();
        let (chart, rid) = self.add_parts(sink, &part)?;
        let number = sink.draw_drawings(1)?.first().copied().unwrap_or(1);
        let ids = sink.draw(&format!("{part}\0chart"), 2)?;
        let drawn = (
            ids.first().map_or("", String::as_str),
            ids.get(1).map_or("", String::as_str),
        );
        let parsed = ooxml_xml::parse(self.xml(number, &rid, drawn).as_bytes(), &Limits::default().xml)
            .map_err(|e| EditError::own_xml(e))?;
        let tree = sink
            .doc()
            .tree(&part)?
            .ok_or_else(|| EditError::no_part(part.clone()))?;
        let mut scope = scope_above(tree.root(), &found.path);
        scope.extend_from_slice(found.element.namespaces());
        let drawing = ooxml_word_model::stamp::namespaces::strip_redundant(parsed.root(), &mut scope);
        let at = InParagraph {
            part: part.clone(),
            path: found.path.clone(),
            id: found.id.clone(),
        };
        let how = Rewrite {
            renew: true,
            ensure_w14: false,
        };
        rewrite(sink, &at, how, |sink, work| {
            let drawing = work
                .adopt(&drawing)
                .ok_or_else(|| EditError::no_element(part.clone()))?;
            let run = crate::inline::new_run(work, work.root(), place.offset, &[drawing])?;
            no_proof(work, run);
            let wrote = sink.track(work, |work, stamp, _| {
                if let (Some(parent), Some(index)) = (work.parent(run), work.index(run)) {
                    work.remove(run);
                    crate::track::insert_nodes(work, parent, index, &[run], stamp);
                }
                Vec::new()
            })?;
            Ok(((), TextWrite { changed: true, ..wrote }))
        })?;
        sink.ensure_w14(&part, true, &[])?;
        let drawing = format!("d:{number}");
        created.push(drawing.clone());
        Ok(ChartInserted {
            drawing: DrawingEdited {
                id: drawing,
                changed: true,
                created,
                renamed,
                warnings: Vec::new(),
            },
            part: chart.into(),
        })
    }
}

/// `w:noProof` last in the run's properties, unless they hold one (made first when it has
/// none), as docx-agent appends it.
fn no_proof(work: &mut Work, run: Id) {
    let properties = match work.find(run, W, "rPr") {
        Some(p) => p,
        None => {
            let p = work.new_element(work.element_name(run, W, "rPr", "w"));
            work.insert(run, 0, p);
            p
        }
    };
    if work.find(properties, W, "noProof").is_none() {
        let node = work.new_element(work.element_name(properties, W, "noProof", "w"));
        work.append(properties, node);
    }
}

/// Put `made`'s tree of `name` into the edit.
fn put_from(sink: &mut EditSink<'_, '_>, made: &ooxml_opc::Package, name: &str) -> Result<()> {
    match made.tree(name)? {
        Some(tree) => sink.put(name, tree),
        None => Ok(()),
    }
}

/// A row or cell is no block a chart's paragraph goes after: refused with its table and its
/// paragraphs (docx-agent's `insert_paragraph(after=)` resolves block ids only).
fn no_row_or_cell(docx: &Docx, at: &str) -> Result<()> {
    match refuse_inside_table(docx, at, "a paragraph or table") {
        Some(error) => Err(EditError::Address(error)),
        None => Ok(()),
    }
}

/// docx-agent's `_chart_size` bounds: 1-1584 pt each way.
fn check_size((width, height): (f64, f64)) -> Result<()> {
    if [width, height].iter().all(|v| (1.0..=1584.0).contains(v)) {
        Ok(())
    } else {
        Err(EditError::Refused(
            "a chart's width and height are 1-1584 pt".to_owned(),
        ))
    }
}

impl<P: ChartParts> InsertChart<P> {
    /// The edit in `tx`, as [`Command::prepare`] makes it, for a caller that goes on
    /// editing the new chart in the same step.
    ///
    /// # Errors
    /// As [`Command::prepare`].
    pub fn insert(&self, tx: &mut Txn<'_, WordDoc>) -> Result<ChartInserted> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl<P: ChartParts> Command<WordDoc> for InsertChart<P> {
    type Out = ChartInserted;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        check_size(self.size)?;
        if self.at.as_str().contains('@') {
            let docx = doc.docx()?;
            crate::fields::place(doc, &docx, &self.at, true).map(|_| ())
        } else {
            let docx = doc.docx()?;
            no_row_or_cell(&docx, self.at.as_str())?;
            crate::paragraphs::anchor(doc, &docx, self.at.as_str()).map(|_| ())
        }
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<ChartInserted> {
        self.insert(tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charts_kept_beside_the_story() {
        assert_eq!(charts_folder("word/document.xml"), "word/charts");
        assert_eq!(charts_folder("/word/header1.xml"), "word/charts");
        assert_eq!(charts_folder("document.xml"), "charts");
    }

    #[test]
    fn sizes_bounded() {
        assert!(check_size((432.0, 252.0)).is_ok());
        assert!(check_size((0.5, 10.0)).is_err());
        assert!(check_size((1600.0, 10.0)).is_err());
    }
}
