//! Owns: `word_copy_from` (docx-agent's `tools/structure.py`, `word_copy_from`): its
//! definition and its copy through the edit core's [`CopyBlocks`] -- the source this
//! document (as it was before the call) or another open one, its blocks a range or a
//! heading's section, the reply naming the blocks made and each source id's copy.
//!
//! Refused as not supported yet: a place given by reference (the host resolves its refs
//! before it calls), and what the edit core does not port yet.
//!
//! Must not: change the document but through a command.

use super::common::{blocks_ref_as_place, call, is_place, name_blocks, outcome, policy, refused, Outcome};
use crate::review::Editing;
use ooxml_tools::pyfmt::repr_str;
use ooxml_tools::{ArgEnum, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{
    section_blocks, section_blocks_in, BookmarkPolicy, CopyBlocks, CopyRange, CopySource, ListPolicy, StylePolicy,
    UnmappedPolicy,
};
use ooxml_word_model::address::Place;
use serde_json::{json, Map, Value};

/// How many of the source ids and their copies' the reply lists.
const COPIED_SHOWN: usize = 60;

/// Which styles copied blocks keep.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyStyles {
    /// This document's styles of the same names.
    UseDestination,
    /// The source's, renamed where they differ.
    KeepSource,
    /// The source's merged into this document's.
    Merge,
}

/// What a source style this document lacks becomes.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unmapped {
    /// Imported.
    Import,
    /// The body style.
    Body,
}

/// A source style and this document's style it becomes.
// Today's schema describes these fields not at all: a doc comment would become a description.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct StyleMapping {
    pub source: String,
    pub destination: String,
}

/// `word_copy_from`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_copy_from",
    description = include_str!("word_copy_from.md"),
    group = "word_structure",
    mutates,
    documents("doc", "source_doc"),
    exactly_one("range", "section_heading"),
    places = "blocks",
    place_own("at", "range", "section_heading")
)]
pub struct WordCopyFrom {
    /// Document id.
    pub doc: String,
    /// Open document to copy from, e.g. d2.
    pub source_doc: String,
    /// end, `after:<id>`, `before:<id>` or `replace:<id>[..<id>]`.
    #[arg(description = "end, after:<id>, before:<id> or replace:<id>[..<id>].")]
    pub at: String,
    /// Source block or span p:A..t:B.
    pub range: Option<String>,
    /// Or a source heading: its whole section.
    pub section_heading: Option<String>,
    /// Default use_destination.
    pub styles: Option<CopyStyles>,
    /// Source style -> this document's style.
    pub style_map: Option<Vec<StyleMapping>>,
    /// Source styles this document lacks: import them (default) or use body.
    pub unmapped: Option<Unmapped>,
    /// Ref name for the new blocks ($name).
    #[arg(rename = "ref")]
    pub ref_name: Option<String>,
}

/// `word_copy_from` (docx-agent's `word_copy_from`): blocks of an open document copied
/// here -- one command.
#[must_use]
pub fn word_copy_from(args: &WordCopyFrom, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let way = call("word_read", &[("doc", json!(args.doc))]);
    match run(args, editing, &way) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

/// `what` refused as not supported yet, `way` (reading the document) the way out.
fn not_supported(what: &str, way: &TryCall) -> ToolError {
    ToolError::not_supported(what, WayOut::call(way.clone()))
}

fn run(args: &WordCopyFrom, editing: &Editing<'_>, way: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    if let Some(name) = args.ref_name.as_deref().filter(|r| is_place(r)) {
        return Err(blocks_ref_as_place(name, &args.at));
    }
    if args.at.contains('$') {
        return Err(not_supported("copy_from at a place by reference", way));
    }
    let source = if args.source_doc == args.doc {
        CopySource::Own
    } else {
        let other = editing
            .others
            .get(&args.source_doc)
            .ok_or_else(|| not_open(&args.source_doc, &args.doc))?;
        match other.word {
            Some(word) => CopySource::Other(Box::new(word.clone())),
            None => {
                return Err(not_supported(
                    "copy_from a source the session holds only as a package",
                    way,
                ))
            }
        }
    };
    let edit_refusal = |e| crate::review::edit_refusal(e, "", way.clone());
    let range = match (&args.section_heading, &args.range) {
        (Some(heading), _) => {
            let found = match &source {
                CopySource::Own => section_blocks(editing.editor.current().doc(), heading),
                CopySource::Other(other) => other.docx().and_then(|d| section_blocks_in(other.ids(), &d, heading)),
            };
            match found {
                Ok(ids) => CopyRange::Ids(ids),
                Err(error) => return Err(edit_refusal(error)),
            }
        }
        (None, Some(range)) => CopyRange::Blocks(range.clone()),
        (None, None) => return Err(crate::review::needed("range", "", way.clone())),
    };
    let command = CopyBlocks {
        source,
        range,
        at: Place::parse(&args.at),
        policy: policy(editing),
        styles: match args.styles {
            None | Some(CopyStyles::UseDestination) => StylePolicy::UseDestination,
            Some(CopyStyles::KeepSource) => StylePolicy::KeepSource,
            Some(CopyStyles::Merge) => StylePolicy::Merge,
        },
        style_map: args
            .style_map
            .iter()
            .flatten()
            .map(|m| (m.source.clone(), m.destination.clone()))
            .collect(),
        unmapped: match args.unmapped {
            Some(Unmapped::Body) => UnmappedPolicy::Body,
            None | Some(Unmapped::Import) => UnmappedPolicy::Import,
        },
        // the tool copies lists as Word's paste does: each a new list
        lists: ListPolicy::Separate,
        // and renames a bookmark whose name this document has
        bookmarks: BookmarkPolicy::Rename,
    };
    Ok(match editing.editor.run(editing.caller, &command) {
        Ok(done) => reply(args, done.out),
        Err(error) => refused(error, way.clone()),
    })
}

/// The reply, as docx-agent's `outcome` and `name_blocks` make it of the copy.
fn reply(args: &WordCopyFrom, out: ooxml_word_edit::BlocksCopied) -> ToolResult<Value> {
    let count = if out.blocks.is_empty() {
        out.copied.len()
    } else {
        out.blocks.len()
    };
    let copied: Map<String, Value> = out
        .copied
        .iter()
        .take(COPIED_SHOWN)
        .map(|(old, new)| (old.clone(), Value::from(new.as_str())))
        .collect();
    let edit = Outcome {
        id: out.id.clone(),
        changed: true,
        created: out.created.clone(),
        removed: out.removed.clone(),
        renamed: ooxml_word_model::ids::renames_text(&out.renamed),
        warnings: out.warnings.clone(),
    };
    let mut result = outcome(
        &[edit],
        format!("Copied {count} block(s) from {}", args.source_doc),
        Some(json!({"blocks": out.blocks, "copied": Value::Object(copied)})),
    );
    name_blocks(&mut result, args.ref_name.as_ref(), &out.blocks);
    result
}

/// The source document is not open.
fn not_open(id: &str, doc: &str) -> ToolError {
    let message = format!("no open document {}", repr_str(id));
    match WayOut::names([doc.to_owned()]) {
        Some(way_out) => ToolError::refusal(Refusal::NotFound, message, way_out).at("source_doc"),
        None => ToolError::internal(message, "word_copy_from:source_doc"),
    }
}

/// Registers `word_copy_from` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl ooxml_tools::Registrar<crate::Word>) {
    r.edit::<WordCopyFrom>(word_copy_from);
}
