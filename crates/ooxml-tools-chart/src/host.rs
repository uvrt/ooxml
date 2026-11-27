//! Owns: what the chart tools ask of the format whose document holds the chart -- the
//! graphic a frame's address names on the version a read pins ([`GraphicHost`]); an edit of
//! its parts planned and applied on the version the edit is applied to, a new graphic
//! inserted and finished in one step, and the language new text is written in
//! ([`GraphicWriter`]) -- and what they get back ([`Graphic`], [`Found`], [`Written`],
//! [`Inserted`]).
//!
//! Must not: know a format's frames, parts or store (each host does), or word a refusal the
//! chart tools word themselves.

use crate::add::ChartEdit;
use crate::Warnings;
use ooxml_chart::edit::{GraphicEdit, GraphicError};
use ooxml_opc::Package;
use ooxml_tools::{ToolError, ToolResult, TryCall};
use ooxml_xml::{Element, Tree};
use serde_json::Value;
use std::sync::Arc;

/// A chart graphic found on a version: the part holding its frame, its chart part, the
/// frame's address and the chart part's root, with the version's package (cheap to clone:
/// versions share parts) for the parts beside it (an embedded workbook).
#[derive(Debug, Clone)]
pub struct Graphic {
    /// The part holding the frame (a Word story, a slide, a sheet's drawing).
    pub holder: String,
    /// The chart part the frame names.
    pub chart: String,
    /// The frame's address, as the call names it (`d:3`, a group member's `d:3/5`).
    pub frame: String,
    /// The version's package.
    pub package: Package,
    /// The chart part's root.
    pub root: Arc<Element>,
}

/// Why a chart frame's graphic cannot be read: the tools refuse it as not supported yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoChart {
    /// The frame names no chart part the package has a relationship to.
    NoPart,
    /// The chart part is missing, or is not XML.
    NotXml,
}

impl NoChart {
    /// The form not supported, for `action` (`read`, `set_value`).
    #[must_use]
    pub fn said(self, action: &str) -> String {
        match self {
            NoChart::NoPart => format!("edit_chart {action}: a chart whose part is missing"),
            NoChart::NotXml => format!("edit_chart {action}: a chart part that is missing or not XML"),
        }
    }
}

/// A chart frame's graphic, or why it cannot be read.
pub type Found = Result<Graphic, NoChart>;

/// Where a read finds charts: the version the call pins.
///
/// Word's host finds the first drawing or group member with the address's `d:` id among a
/// document's drawings and its first `c:chart`'s target. In Phase 4 a deck's host finds a
/// slide's `p:graphicFrame` (the holder is the slide part) and a workbook's a drawing
/// part's `xdr:` anchor (the holder is the drawing part), each through its holder's
/// relationships to the chart part; nothing here changes.
pub trait GraphicHost {
    /// The chart graphic `address` names, or why it cannot be read.
    ///
    /// # Errors
    /// The host's refusal: `not_found` for an address no frame has, `refused` for a frame
    /// that is not a chart (each with the call that lists the frames), `timeout` when the
    /// call is cancelled.
    fn find(&self, address: &str) -> Result<Found, ToolError>;
}

/// What a [`GraphicWriter`] applied: its warnings (the host's first, then the edit's), and
/// the graphic as the version the edit made holds it.
#[derive(Debug, Clone)]
pub struct Written {
    /// What the result says.
    pub warnings: Warnings,
    /// The graphic on the version made.
    pub after: Found,
}

/// A new graphic's parts: the chart part's tree, for the part name the host picks (the seed
/// of its ids) and the relationship to its workbook, and the workbook's bytes.
pub trait NewGraphic {
    /// The parts, for the chart part `part` whose relationship to its workbook is
    /// `workbook_rel`.
    ///
    /// # Errors
    /// The graphic's refusal: nothing is written.
    fn parts(&self, part: &str, workbook_rel: &str) -> Result<(Tree, Arc<[u8]>), GraphicError>;
}

/// What finishes a new graphic in the step that inserts it: its edits in order, each planned
/// on the graphic as the edits before left it.
pub trait FinishGraphic {
    /// The edit after the first `done`, of the chart part `chart` in `package` whose frame is
    /// `frame`; `None` when there is none.
    fn next(&self, done: usize, package: &Package, chart: &str, frame: &str) -> Option<Result<ChartEdit, GraphicError>>;
}

/// What [`GraphicWriter::insert`] made: the new frame's address, the block made to hold it
/// where the host makes one (a document's paragraph), the chart part's root on the version
/// made, and the edit's result -- the ids made and changed, the host's warnings then each
/// finishing edit's -- for the tools to give its summary and data.
#[derive(Debug, Clone)]
pub struct Inserted {
    /// The new frame's address.
    pub frame: String,
    /// The block made to hold it.
    pub block: Option<String>,
    /// The chart part's root on the version made (`None`: the part is not there).
    pub root: Option<Arc<Element>>,
    /// The edit's result, its summary and data left to the tools.
    pub result: ToolResult<Value>,
}

/// Where a change of a chart goes: the call's editor.
///
/// Word's writer applies the edit as one command of the call's step (Word tracks no chart
/// data: tracked, it is applied untracked and says so), inserts a new chart inline in a
/// paragraph of its own after a block (or at a position in a paragraph), and the language
/// new chart text is written in is the document's default run language, else the call's
/// locale's. In Phase 4 a deck's writer and a workbook's apply the same [`GraphicEdit`]
/// through their own store commands, insert a slide's `p:graphicFrame` or a drawing
/// part's `xdr:` anchor (no block made), the language theirs (the presentation's or
/// workbook's default); nothing here changes.
pub trait GraphicWriter {
    /// `plan` made on the version the edit is applied to -- of the graphic `address` names
    /// there and the language new text is written in -- and applied as one edit; planned
    /// again where a commit meanwhile changed what it read.
    ///
    /// # Errors
    /// The host's refusal of the address (as [`GraphicHost::find`]'s), the plan's, or the
    /// edit's, with `read` as its way out.
    fn write<E: GraphicEdit<Out = Warnings>>(
        &self,
        address: &str,
        read: &TryCall,
        plan: &dyn Fn(&Found, &str) -> Result<E, ToolError>,
    ) -> Result<Written, ToolError>;

    /// A new graphic of `size` (points) placed where `at` says, its parts `graphic`'s, and
    /// `finish`'s edits applied to it, all in one edit.
    ///
    /// # Errors
    /// The host's refusal of the place, or the edit's (nothing is written), with `read` as
    /// its way out.
    fn insert(
        &self,
        at: &str,
        read: &TryCall,
        graphic: &dyn NewGraphic,
        size: (f64, f64),
        finish: &dyn FinishGraphic,
    ) -> Result<Inserted, ToolError>;

    /// The language new chart text is written in, on the version the call's editor holds.
    ///
    /// # Errors
    /// The host's, where the version cannot be read.
    fn language(&self) -> Result<String, ToolError>;
}
