//! Owns: complex fields threaded through a part ([`Fields`]): a field's `begin`, `separate`
//! and `end` may lie in different runs and paragraphs, and its result (a table of contents)
//! may span paragraphs. Which fields layout computes (`PAGE`, `NUMPAGES`, `SECTIONPAGES`;
//! `PAGEREF`, `REF`, `SEQ` in the body's own text; `STYLEREF` in the body, headers and
//! footers) and which are drawn as Word cached them -- docx2svg's `_Fields`.
//!
//! Must not: compute a field's value (layout settles them: L3).

use crate::model::RunProps;
use crate::value::{text, Text};
use std::sync::Arc;

/// The fields layout computes.
pub const COMPUTED: &[&str] = &["PAGE", "NUMPAGES", "SECTIONPAGES", "PAGEREF", "REF", "SEQ", "STYLEREF"];
/// Computed in the body's own text only.
pub const CROSS_REFERENCES: &[&str] = &["PAGEREF", "REF", "SEQ"];

/// A field instruction's keyword, upper-cased (`PAGE` of ` page \* roman`).
pub fn keyword(instruction: &str) -> String {
    instruction
        .split_whitespace()
        .next()
        .map(str::to_uppercase)
        .unwrap_or_default()
}

/// Whether the instruction carries `\* MERGEFORMAT`.
pub fn merge_format(instruction: &str) -> bool {
    instruction.to_uppercase().replace(' ', "").contains("MERGEFORMAT")
}

/// Where a complex field is between its `begin` and `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    /// Its instruction (before `separate`).
    Instruction,
    /// Its result (after `separate`).
    Result,
}

/// A complex field between its `begin` and its `end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    /// Whether everything around it is drawn.
    pub visible: bool,
    /// The path of the run that began it.
    pub path: Text,
    /// Its instruction so far.
    pub instruction: String,
    /// Where it is.
    pub part: Part,
    /// Whether layout computes it.
    pub computed: bool,
    /// The first instruction run's properties and path, once met.
    pub instr: Option<(Option<Arc<RunProps>>, Text)>,
    /// The first cached result run's properties (empty ones where it has none), once met.
    pub result_properties: Option<Arc<RunProps>>,
    /// The cached result's text.
    pub result: String,
    /// The cached result, piece by piece.
    pub cached: Vec<(Text, Option<Arc<RunProps>>)>,
}

/// What a block's reading reads of one open field: what the memo of a block keys on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpenState {
    /// Where it is.
    pub part: Part,
    /// Whether it is computed.
    pub computed: bool,
    /// Its keyword.
    pub keyword: String,
}

/// The complex fields open at a point of a part, and what the part's fields leave.
#[derive(Debug, Clone, Default)]
pub struct Fields {
    /// The fields open, outermost first.
    pub stack: Vec<Open>,
    /// The keyword of every field whose cached result is drawn, in order met.
    pub cached: Vec<Text>,
    /// Whether `PAGEREF`, `REF` and `SEQ` are computed here (the body's own text).
    pub cross_references: bool,
    /// Whether `STYLEREF` is computed here (the body, a header, a footer).
    pub styleref: bool,
    /// The bookmarks of the paragraph being read.
    pub bookmarks: Option<Vec<crate::model::Bookmark>>,
}

impl Fields {
    /// Fields of a part: the body's computes cross references and `STYLEREF`, a header's
    /// or footer's `STYLEREF` only, a note's neither.
    pub fn new(cross_references: bool, styleref: bool) -> Fields {
        Fields {
            cross_references,
            styleref,
            ..Fields::default()
        }
    }

    /// Whether what is read now is drawn: inside no instruction and no computed result.
    pub fn visible(&self) -> bool {
        self.stack.iter().all(|f| f.part == Part::Result && !f.computed)
    }

    /// Whether this point is inside a `TOC` field's result.
    pub fn in_toc_result(&self) -> bool {
        self.stack
            .iter()
            .any(|f| f.part == Part::Result && keyword(&f.instruction) == "TOC")
    }

    /// Whether layout computes a field with this instruction here.
    pub fn computes(&self, instruction: &str) -> bool {
        let k = keyword(instruction);
        if k == "STYLEREF" {
            return self.styleref;
        }
        COMPUTED.contains(&k.as_str()) && (!CROSS_REFERENCES.contains(&k.as_str()) || self.cross_references)
    }

    /// What a block's reading reads of the open fields.
    pub fn open_state(&self) -> Vec<OpenState> {
        self.stack
            .iter()
            .map(|f| OpenState {
                part: f.part,
                computed: f.computed,
                keyword: keyword(&f.instruction),
            })
            .collect()
    }

    /// Note a field drawn as cached.
    pub fn note_cached(&mut self, instruction: &str) {
        self.cached.push(text(&keyword(instruction)));
    }
}
