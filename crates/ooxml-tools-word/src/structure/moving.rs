//! Owns: `word_move` (docx-agent's `tools/structure.py`, `word_move`): its definition and
//! its move through the edit core's [`MoveBlocks`].
//!
//! Must not: change the document but through a command.

use super::common::{call, outcome, policy, refused, Outcome};
use crate::planned::run_planned;
use crate::review::Editing;
use ooxml_store::Txn;
use ooxml_tools::{Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{last_block_in, section_in, MoveBlocks, Moving, WordDoc};
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
/// Read on the version the edit's transaction `tx` is applied to ([`crate::planned`]): the
/// section `after_section` names, or the body's last block for `end`.
fn place(
    args: &WordMove,
    tx: &mut Txn<'_, WordDoc>,
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
        let section = section_in(tx, heading).map_err(edit_refusal)?;
        (section.last().cloned(), None)
    } else if let Some(after) = &args.after {
        if after == "end" {
            let last = last_block_in(tx).map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
            (last, None)
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

/// `word_move`'s plan, made in the edit's transaction ([`crate::planned`]): the section
/// that moves (its ids, said in the reply) and where it goes, read there.
pub(crate) fn move_plan<'a>(
    args: &'a WordMove,
    editing: &'a Editing<'a>,
    way: &'a TryCall,
) -> impl Fn(&mut Txn<'_, WordDoc>) -> Result<(MoveBlocks, Vec<String>), ToolError> + 'a {
    move |tx| {
        let edit_refusal = |e| crate::review::edit_refusal(e, "", way.clone());
        let (moving, ids) = match (&args.section_heading, &args.blocks) {
            (Some(heading), _) => {
                let ids = section_in(tx, heading).map_err(edit_refusal)?;
                (Moving::Ids(ids.clone()), ids)
            }
            (None, Some(blocks)) => (
                Moving::Blocks(ooxml_word_model::address::Address::parse(blocks)),
                vec![blocks.clone()],
            ),
            (None, None) => return Err(crate::review::needed("blocks", "", way.clone())),
        };
        let (after, before) = place(args, tx, way)?;
        let command = MoveBlocks {
            blocks: moving,
            after: after.map(ooxml_word_model::address::Address::from),
            before: before.map(ooxml_word_model::address::Address::from),
            policy: policy(editing),
        };
        Ok((command, ids))
    }
}

fn run(args: &WordMove, editing: &Editing<'_>, way: &TryCall) -> Result<ToolResult<Value>, ToolError> {
    Ok(match run_planned(editing, move_plan(args, editing, way))? {
        Ok(done) => {
            let (out, ids) = done.out;
            let edit = Outcome {
                id: out.id.clone(),
                changed: true,
                created: out.created.clone(),
                removed: Vec::new(),
                renamed: ooxml_word_model::ids::renames_text(&out.renamed),
                warnings: out.warnings.clone(),
            };
            let renamed = |id: &String| {
                out.renamed
                    .iter()
                    .find(|(old, _)| old == id)
                    .map_or_else(|| id.clone(), |(_, new)| new.to_string())
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
pub(crate) fn register(r: &mut impl ooxml_tools::Registrar<crate::Word>) {
    r.edit::<WordMove>(word_move);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::planned::held::{args, calling, compare, markdown, marked, paragraph, raced};
    use ooxml_word_edit::Editor;

    /// The fixture with a second heading's section at the end ("Two").
    fn two() -> Editor {
        let editor = marked();
        editor
            .run(
                &ooxml_store::Caller::anonymous(),
                &markdown("end", "# Two\n\nTwo's text."),
            )
            .unwrap();
        editor
    }

    fn raced_move(
        args: &WordMove,
        editor: &Editor,
        between: &impl ooxml_store::Command<WordDoc, Error = ooxml_word_edit::EditError>,
    ) -> (Option<ToolError>, u64) {
        let way = call("word_read", &[]);
        calling(editor, |editing| raced(editor, move_plan(args, editing, &way), between))
    }

    #[test]
    fn a_section_moved_is_read_where_the_move_commits() {
        // a paragraph put at the section's end meanwhile moves with it
        let editor = two();
        let (heading, first) = (paragraph(&editor, "Two"), paragraph(&editor, "Identifiers"));
        let args: WordMove = args(json!({"section_heading": heading, "before": first}));
        let compared = compare(
            two,
            |_| markdown("end", "Added to two."),
            |editor, between| raced_move(&args, editor, between),
            |editing| word_move(&args, editing),
        );
        compared.assert_same_edit();
        assert!(
            compared
                .serial
                .starts_with("\n[pStyle=Heading1]Two\nTwo's text.\nAdded to two.\n"),
            "{}",
            compared.serial
        );
    }

    #[test]
    fn after_a_section_is_its_last_block_where_the_move_commits() {
        // a paragraph put at the first section's end meanwhile: the block goes after it
        let editor = two();
        let (revenue, stamped) = (paragraph(&editor, "Revenue"), paragraph(&editor, "Stamped"));
        let first = paragraph(&editor, "Identifiers");
        let args: WordMove = args(json!({"blocks": revenue, "after_section": first}));
        let compared = compare(
            two,
            |_| markdown(&format!("after:{stamped}"), "Added to one."),
            |editor, between| raced_move(&args, editor, between),
            |editing| word_move(&args, editing),
        );
        compared.assert_same_edit();
        assert!(
            compared.serial.contains("Added to one.\nRevenue"),
            "{}",
            compared.serial
        );
    }

    #[test]
    fn after_the_end_is_the_last_block_where_the_move_commits() {
        let editor = marked();
        let args: WordMove = args(json!({"blocks": paragraph(&editor, "Revenue"), "after": "end"}));
        let compared = compare(
            marked,
            |_| markdown("end", "Appended meanwhile."),
            |editor, between| raced_move(&args, editor, between),
            |editing| word_move(&args, editing),
        );
        compared.assert_same_edit();
        assert!(
            compared.serial.contains("Appended meanwhile.\nRevenue"),
            "{}",
            compared.serial
        );
    }
}
