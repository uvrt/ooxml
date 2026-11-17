//! Owns: one painting pass's state (ooxml-common's `drawingml.context.RenderContext`): the
//! resources the drawing references (gradients, patterns, markers, clips, filters), the
//! counter their ids are minted from, the enclosing groups' scale and turn, and what the
//! pass was asked for (the rules, the measure, the font mapping, hyperlinks, editor marks).
//!
//! Must not: outlive one drawing, or be shared between threads while painting (each pass
//! owns its own; nothing here is global).

use crate::measure::{TableMeasurer, TextMeasurer};
use crate::rules::DrawingRules;
use ooxml_fonts::FontMapping;
use ooxml_scene::markup::Element;
use std::collections::BTreeSet;

/// One painting pass.
pub struct Painter<'m> {
    /// The text measure.
    pub measurer: &'m dyn TextMeasurer,
    /// Which faces are drawn with which.
    pub font_mapping: FontMapping,
    /// The last-resort East Asian face.
    pub jpan_fallback_font: Option<String>,
    /// The symbol faces ([`ooxml_fonts::symbols::symbol_face`] names) the drawing has no
    /// copy of: their text is drawn as the Unicode it stands for, in faces that have it,
    /// at the face's own advances.
    pub mapped_symbol_faces: BTreeSet<String>,
    /// The drawing rules.
    pub rules: DrawingRules,
    /// Pixels an inch of the user space fills, outlines and effects are drawn in: 96
    /// (PowerPoint's and a chart's), or 300 where docx2svg draws on Word's device grid.
    pub dpi: f64,
    /// Draw a hyperlink as an `a` element around what it covers.
    pub hyperlinks: bool,
    /// Mark what an editor addresses (`data-pptx-*` attributes) and leave positions out.
    pub edit_marks: bool,
    /// The resources referenced so far, in order.
    pub defs: Vec<Element>,
    /// The product of every enclosing uniform group's scale.
    pub group_scale: (f64, f64),
    /// Whether an enclosing group turns its children.
    pub turned: bool,
    next_id: u32,
    id_scope: String,
}

impl<'m> Painter<'m> {
    /// A pass under `rules`, measuring with `measurer`.
    pub fn new(rules: DrawingRules, measurer: &'m dyn TextMeasurer) -> Painter<'m> {
        Painter {
            measurer,
            font_mapping: FontMapping::office_default(),
            jpan_fallback_font: None,
            mapped_symbol_faces: BTreeSet::new(),
            rules,
            dpi: crate::units::DPI,
            hyperlinks: true,
            edit_marks: false,
            defs: Vec::new(),
            group_scale: (1.0, 1.0),
            turned: false,
            next_id: 0,
            id_scope: String::new(),
        }
    }

    /// A new id: `{prefix}-{scope}{n}`.
    pub fn new_id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}-{}{}", self.id_scope, self.next_id)
    }

    /// Continue the numbering of a document whose resources have taken `count` ids already
    /// (a page that holds what other painters drew before this pass).
    pub fn resume_ids(&mut self, count: u32) {
        self.next_id = count;
    }

    /// The ids minted so far (where a later pass on the same page resumes).
    pub fn ids(&self) -> u32 {
        self.next_id
    }

    /// Mint the ids that follow as `{prefix}-{scope}{n}`, counting from 1 again.
    pub fn scope_ids(&mut self, scope: &str) {
        self.id_scope = scope.to_owned();
        self.next_id = 0;
    }

    /// Declare a resource.
    pub fn add_def(&mut self, def: Element) {
        self.defs.push(def);
    }
}

/// The measure a pass under `rules` uses when its caller brings none.
pub fn table_measurer(rules: &DrawingRules) -> TableMeasurer {
    TableMeasurer { kerning: rules.kerning }
}
