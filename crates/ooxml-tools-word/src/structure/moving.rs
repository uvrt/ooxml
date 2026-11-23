//! Owns: `word_move` (docx-agent's `tools/structure.py`, `word_move`): its definition and
//! its move through the edit core's [`MoveBlocks`].
//!
//! Must not: change the document but through a command.

use super::common::{call, outcome, policy, refused, Outcome};
use crate::review::Editing;
use ooxml_tools::{Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{section_blocks, MoveBlocks, Moving};
use serde_json::{json, Value};

/// `word_move`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_move",
    description = include_str!("word_move.md"),
    group = "word_structure",
    mutates,
    exactly_one("section_heading", "blocks"),
    places = "sides",
    place_own("section_heading", "blocks", "after", "before", "after_section", "target", "where")
)]
pub struct WordMove {
    /// Document id.
    pub doc: String,
    /// Heading whose section moves.
    #[arg(refs)]
    pub section_heading: Option<String>,
    /// Or a block or span p:A..t:B.
    #[arg(refs)]
    pub blocks: Option<String>,
    /// Move after this block.
    #[arg(refs)]
    pub after: Option<String>,
    /// Move before this block.
    #[arg(refs)]
    pub before: Option<String>,
    /// Move after this heading's section.
    #[arg(refs)]
    pub after_section: Option<String>,
}

/// Where the blocks go (docx-agent's `_place`): exactly one of `after`, `before`,
/// `after_section` (its heading's section's last block); `after` may say its side
/// (`before:X`) or be `end`.
fn place(
    args: &WordMove,
    doc: &ooxml_word_edit::WordDoc,
    way: &TryCall,
) -> Result<(Option<String>, Option<String>), ToolError> {
    let edit_refusal = |e| crate::review::edit_refusal(e, "", way.clone());
    let given: Vec<&str> = [
        ("after", &args.after),
        ("before", &args.before),
        ("after_section", &args.after_section),
    ]
    .into_iter()
    .filter(|(_, v)| v.is_some())
    .map(|(n, _)| n)
    .collect();
    if given.len() != 1 {
        let mut message = "give exactly one of after, before".to_owned();
        if args.after_section.is_some() {
            message.push_str(", after_section");
        }
        let way_out = WayOut::names(["after", "before", "after_section"]).unwrap_or_else(|| WayOut::call(way.clone()));
        return Err(ToolError::refusal(Refusal::InvalidArguments, message, way_out));
    }
    Ok(if let Some(heading) = &args.after_section {
        let section = section_blocks(doc, heading).map_err(edit_refusal)?;
        (section.last().cloned(), None)
    } else if let Some(after) = &args.after {
        if after == "end" {
            let docx = doc
                .docx()
                .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
            (super::sections::body_blocks(&docx).last().cloned(), None)
        } else if let Some(anchor) = after.strip_prefix("before:") {
            (None, Some(anchor.to_owned()))
        } else {
            (Some(after.strip_prefix("after:").unwrap_or(after).to_owned()), None)
        }
    } else {
        (None, args.before.clone())
    })
}

/// `word_move` (docx-agent's `word_move`): a heading's section, or a span of blocks, moved
/// beside a block -- one command.
#[must_use]
pub fn word_move(args: &WordMove, editing: &Editing<'_>) -> ToolResult<Value> {
    if let Err(error) = crate::review::stopped(editing.cancel) {
        return ToolResult::failed(error);
    }
    let way = call("word_read", &[("doc", json!(args.doc))]);
    match run(args, editing, &way) {
        Ok(result) => result,
        Err(error) => ToolResult::failed(error),
    }
}

fn run(args: &WordMove, editing: &Editing<'_>, way: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    let head = editing.editor.store().snapshot();
    let edit_refusal = |e| crate::review::edit_refusal(e, "", way.clone());
    let (moving, ids) = match (&args.section_heading, &args.blocks) {
        (Some(heading), _) => {
            let ids = section_blocks(head.doc(), heading).map_err(edit_refusal)?;
            (Moving::Ids(ids.clone()), ids)
        }
        (None, Some(blocks)) => (Moving::Blocks(blocks.clone()), vec![blocks.clone()]),
        (None, None) => return Err(crate::review::needed("blocks", "", way.clone())),
    };
    let (after, before) = place(args, head.doc(), way)?;
    let command = MoveBlocks {
        blocks: moving,
        after,
        before,
        policy: policy(editing),
    };
    Ok(match editing.editor.run(editing.caller, &command) {
        Ok(done) => {
            let out = done.out;
            let edit = Outcome {
                id: out.id.clone(),
                changed: true,
                created: out.created.clone(),
                removed: Vec::new(),
                renamed: out.renamed.clone(),
                warnings: out.warnings.clone(),
            };
            let renamed = |id: &String| {
                out.renamed
                    .iter()
                    .find(|(old, _)| old == id)
                    .map_or_else(|| id.clone(), |(_, new)| new.clone())
            };
            let mut result = outcome(
                &[edit],
                format!("Moved {} block(s)", ids.len()),
                Some(json!({"moved": ids})),
            );
            // docx-agent adds each block's id after the move to `changed` as it is, a copy
            // a tracked move made too
            if let ooxml_tools::Outcome::Done(done) = &mut result.outcome {
                for id in ids.iter().map(renamed) {
                    if !done.changed.contains(&id) {
                        done.changed.push(id);
                    }
                }
            }
            result
        }
        Err(error) => refused(error, way.clone()),
    })
}

/// Registers `word_move` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordMove>(word_move, crate::Hosting::PLAIN);
}
