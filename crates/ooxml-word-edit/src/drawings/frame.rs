//! Owns: the edits of a drawing's frame, as docx-agent's `float_drawing`, `inline_drawing`,
//! `set_drawing` and `_picture_edit` (alternative text) make them: a frame floated as
//! Word's `convert to shape` (in front of the text, against the column and the paragraph,
//! a step above every other drawing, its ids kept), put in the line again (every distance
//! 0), set ([`super::settings`]). Word tracks none of them: tracked, they are applied
//! untracked, and the result says so.
//!
//! Must not: write but through the sink.

use super::settings::{alternate_content, apply, make_wp, DrawingValues};
use super::{all_heights, edit_frame, height_above, located, picture, DrawingEdited, SIDE_DISTANCE};
use crate::controls::Rewrite;
use crate::names::{WP, WP14};
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_xml::QName;
use std::sync::Arc;

const FLOAT_UNTRACKED: &str = "Word does not track floating a drawing: it is applied untracked";
const INLINE_UNTRACKED: &str = "Word does not track putting a drawing in the line: it is applied untracked";
const SET_UNTRACKED: &str = "Word does not track a drawing's place, wrapping, order or size: applied untracked";

fn tracked(policy: &RevisionPolicy) -> bool {
    matches!(policy, RevisionPolicy::Tracked(_))
}

/// The frame edits run untracked (Word records none of them).
fn untracked<'t, 's>(tx: &'t mut Txn<'s, WordDoc>) -> EditSink<'t, 's> {
    EditSink::new(tx, RevisionPolicy::Untracked)
}

/// An inline drawing floated (docx-agent's `float_drawing`), then `values` set; a drawing
/// that floats already stays where it is, `values` set as [`SetDrawing`] sets them.
#[derive(Debug, Clone)]
pub struct FloatDrawing {
    /// The drawing (`d:...`).
    pub drawing: String,
    /// The settings to give it.
    pub values: DrawingValues,
    /// Untracked or tracked (only the warning differs).
    pub policy: RevisionPolicy,
}

impl Command<WordDoc> for FloatDrawing {
    type Out = DrawingEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        located(&*doc.docx()?, &self.drawing).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<DrawingEdited> {
        let docx = tx.doc().docx()?;
        let drawing = located(&docx, &self.drawing)?;
        if !drawing.element.name().is(Some(WP), "inline") {
            let note = format!("{} floats already", self.drawing);
            if self.values.is_empty() {
                return Ok(DrawingEdited {
                    id: self.drawing.clone(),
                    changed: false,
                    warnings: vec![note],
                    ..DrawingEdited::default()
                });
            }
            let set = SetDrawing {
                drawing: self.drawing.clone(),
                alt_text: None,
                values: Some(self.values.clone()),
                policy: self.policy.clone(),
            };
            let mut out = set.prepare(tx)?;
            out.warnings
                .insert(0, format!("{note}: it stays where it is, with the settings given"));
            return Ok(out);
        }
        let mut sink = untracked(tx);
        let renamed = stamp_document(&mut sink, &docx)?;
        let heights = all_heights(&mut sink)?;
        let docx = sink.docx()?;
        let drawing = located(&docx, &self.drawing)?;
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        edit_frame(&mut sink, &drawing, Rewrite::default(), |_, work, frame| {
            let anchor = anchored(work, frame, height_above(&heights));
            if !self.values.is_empty() {
                apply(work, anchor, &self.values, &heights, orders);
            }
            Ok(())
        })?;
        sink.ensure_w14(&drawing.part, true, &[])?;
        crate::after::after_edit(&mut sink)?;
        Ok(DrawingEdited {
            id: self.drawing.clone(),
            changed: true,
            renamed,
            warnings: tracked(&self.policy)
                .then(|| FLOAT_UNTRACKED.to_owned())
                .into_iter()
                .collect(),
            ..DrawingEdited::default()
        })
    }
}

fn plain(name: &str) -> Arc<QName> {
    Arc::new(QName::new(None, name, None))
}

/// `frame` (a `wp:inline`) replaced by the `wp:anchor` Word's `convert to shape` writes,
/// `height` its order; the anchor.
fn anchored(work: &mut Work, frame: Id, height: i64) -> Id {
    let side = SIDE_DISTANCE.to_string();
    let height = height.to_string();
    let anchor = make_wp(
        work,
        frame,
        "anchor",
        &[
            ("distT", "0"),
            ("distB", "0"),
            ("distL", &side),
            ("distR", &side),
            ("simplePos", "0"),
            ("relativeHeight", &height),
            ("behindDoc", "0"),
            ("locked", "0"),
            ("layoutInCell", "1"),
            ("allowOverlap", "1"),
        ],
    );
    copy_ids(work, frame, anchor);
    let simple = make_wp(work, frame, "simplePos", &[("x", "0"), ("y", "0")]);
    work.append(anchor, simple);
    for (tag, origin) in [("positionH", "column"), ("positionV", "paragraph")] {
        let position = make_wp(work, frame, tag, &[("relativeFrom", origin)]);
        let offset = make_wp(work, frame, "posOffset", &[]);
        work.set_text(offset, "0");
        work.append(position, offset);
        work.append(anchor, position);
    }
    for child in work.children(frame).to_vec() {
        if work.is(child, WP, "docPr") {
            if work.find(anchor, WP, "effectExtent").is_none() {
                let effect = make_wp(
                    work,
                    frame,
                    "effectExtent",
                    &[("l", "0"), ("t", "0"), ("r", "0"), ("b", "0")],
                );
                work.append(anchor, effect);
            }
            let none = make_wp(work, frame, "wrapNone", &[]);
            work.append(anchor, none);
        }
        let copy = work.deep_copy(child);
        work.append(anchor, copy);
    }
    work.add_previous(frame, anchor);
    work.remove(frame);
    anchor
}

/// The frame's `wp14:anchorId` and `wp14:editId`, kept on `to`, and the namespaces it
/// declares itself for its own names (a frame whose part's root does not declare `wp`:
/// its children keep their prefixes; docx-agent's lxml names it `ns0` there).
fn copy_ids(work: &mut Work, from: Id, to: Id) {
    for declared in work.declarations(from) {
        let own = matches!(&*declared.uri, WP | WP14);
        if let Some(prefix) = declared.prefix.as_deref().filter(|_| own) {
            work.declare(to, prefix, &declared.uri);
        }
    }
    for local in ["anchorId", "editId"] {
        let found = work
            .attributes(from)
            .iter()
            .find(|a| a.name.is(Some(WP14), local))
            .map(|a| (Arc::clone(&a.name), a.value.to_string()));
        if let Some((name, value)) = found {
            work.set(to, name, &value);
        }
    }
}

/// A floating drawing put in the line again (docx-agent's `inline_drawing`).
#[derive(Debug, Clone)]
pub struct InlineDrawing {
    /// The drawing (`d:...`).
    pub drawing: String,
    /// Untracked or tracked (only the warning differs).
    pub policy: RevisionPolicy,
}

impl Command<WordDoc> for InlineDrawing {
    type Out = DrawingEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        located(&*doc.docx()?, &self.drawing).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<DrawingEdited> {
        let docx = tx.doc().docx()?;
        let drawing = located(&docx, &self.drawing)?;
        if !drawing.element.name().is(Some(WP), "anchor") {
            return Ok(DrawingEdited {
                id: self.drawing.clone(),
                changed: false,
                warnings: vec![format!("{} is inline already", self.drawing)],
                ..DrawingEdited::default()
            });
        }
        let mut sink = untracked(tx);
        let renamed = stamp_document(&mut sink, &docx)?;
        let docx = sink.docx()?;
        let drawing = located(&docx, &self.drawing)?;
        let id = self.drawing.clone();
        edit_frame(&mut sink, &drawing, Rewrite::default(), |_, work, frame| {
            if alternate_content(work, frame).is_some() {
                return Err(EditError::Refused(format!(
                    "{id} is a shape or text box: Word puts only pictures in the line"
                )));
            }
            let inline = make_wp(
                work,
                frame,
                "inline",
                &[("distT", "0"), ("distB", "0"), ("distL", "0"), ("distR", "0")],
            );
            copy_ids(work, frame, inline);
            for tag in ["extent", "effectExtent", "docPr", "cNvGraphicFramePr"] {
                if let Some(node) = work.find(frame, WP, tag) {
                    let copy = work.deep_copy(node);
                    work.append(inline, copy);
                }
            }
            if let Some(graphic) = work.find(frame, super::A, "graphic") {
                let copy = work.deep_copy(graphic);
                work.append(inline, copy);
            }
            work.add_previous(frame, inline);
            work.remove(frame);
            Ok(())
        })?;
        crate::after::after_edit(&mut sink)?;
        Ok(DrawingEdited {
            id: self.drawing.clone(),
            changed: true,
            renamed,
            warnings: tracked(&self.policy)
                .then(|| INLINE_UNTRACKED.to_owned())
                .into_iter()
                .collect(),
            ..DrawingEdited::default()
        })
    }
}

/// A drawing's settings (docx-agent's `set_drawing`; `move_drawing` and `resize_drawing`
/// through it) and a picture's alternative text (`Picture.alt_text`, set first).
#[derive(Debug, Clone)]
pub struct SetDrawing {
    /// The drawing (`d:...`).
    pub drawing: String,
    /// A picture's new alternative text (empty: none).
    pub alt_text: Option<String>,
    /// The settings (`None`: only the alternative text is set; empty: none, the
    /// fallback still brought in line, as docx-agent's `resize_drawing` without a size).
    pub values: Option<DrawingValues>,
    /// Untracked or tracked (only the warning differs).
    pub policy: RevisionPolicy,
}

impl SetDrawing {
    fn check(&self, docx: &ooxml_word_model::docx::Docx) -> Result<()> {
        if self.alt_text.is_some() {
            picture(docx, &self.drawing)?;
        }
        let Some(values) = &self.values else {
            return Ok(());
        };
        let drawing = located(docx, &self.drawing)?;
        if drawing.element.name().is(Some(WP), "inline") && values.places() {
            return Err(EditError::Refused(format!(
                "{} is inline: float it first (float_drawing)",
                self.drawing
            )));
        }
        Ok(())
    }
}

impl Command<WordDoc> for SetDrawing {
    type Out = DrawingEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.check(&*doc.docx()?)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<DrawingEdited> {
        let docx = tx.doc().docx()?;
        self.check(&docx)?;
        let mut sink = untracked(tx);
        let renamed = stamp_document(&mut sink, &docx)?;
        let heights = if self.values.as_ref().is_some_and(|v| v.z_order.is_some()) {
            all_heights(&mut sink)?
        } else {
            Vec::new()
        };
        let docx = sink.docx()?;
        let drawing = located(&docx, &self.drawing)?;
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        edit_frame(&mut sink, &drawing, Rewrite::default(), |_, work, frame| {
            if let Some(text) = &self.alt_text {
                alt_text(work, frame, text);
            }
            if let Some(values) = &self.values {
                apply(work, frame, values, &heights, orders);
            }
            Ok(())
        })?;
        crate::after::after_edit(&mut sink)?;
        let set = self.values.is_some();
        Ok(DrawingEdited {
            id: self.drawing.clone(),
            changed: true,
            renamed,
            warnings: (set && tracked(&self.policy))
                .then(|| SET_UNTRACKED.to_owned())
                .into_iter()
                .collect(),
            ..DrawingEdited::default()
        })
    }
}

/// A picture's `wp:docPr` and `pic:cNvPr` described by `text` (none when empty).
fn alt_text(work: &mut Work, frame: Id, text: &str) {
    let mut nodes: Vec<Id> = work.find(frame, WP, "docPr").into_iter().collect();
    let properties = work
        .descendants(frame)
        .into_iter()
        .find(|n| work.is(*n, "http://schemas.openxmlformats.org/drawingml/2006/picture", "cNvPr"));
    nodes.extend(properties);
    for node in nodes {
        if text.is_empty() {
            work.remove_attr(node, None, "descr");
        } else {
            work.set(node, plain("descr"), text);
        }
    }
}
