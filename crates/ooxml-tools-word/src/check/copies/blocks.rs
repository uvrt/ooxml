//! Owns: the body's blocks a copy or a reference names (docx-agent's `tools/scoped.py`
//! `_blocks`, `copies.blocks_of`): a block (`p:...`, `t:...`), a span `p:A..t:B`, a
//! heading's range `h:p:A`, a block-level control's blocks, or the top-level blocks another
//! address's paragraphs are in -- each refused as docx-agent refuses it, the body's
//! headings offered as the way in.
//!
//! Must not: compare blocks.

use super::compare::CopyBlock;
use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Refusal, ToolError, WayOut};
use ooxml_word_edit::{body_blocks, EditError, WordDoc};
use ooxml_word_model::address::resolve::offered;
use ooxml_word_model::docx::{at_path, Docx};
use std::sync::Arc;

/// Ways in offered at most.
const OFFERED: usize = 50;

/// The blocks `address` names in `docx` (`doc` as the session holds it), as `at` names the
/// argument (`copies[0].blocks`).
///
/// # Errors
/// `invalid_arguments` for an empty or malformed address; `not_found` for one naming no
/// block of the body, or no paragraph or table.
pub(super) fn blocks(
    doc: &WordDoc,
    docx: &Docx,
    address: &str,
    at: &str,
    name: &str,
) -> Result<Vec<CopyBlock>, ToolError> {
    let headings = || {
        let mut found = offered(docx);
        found.truncate(OFFERED);
        found
    };
    let address = address.trim();
    if address.is_empty() {
        return Err(refused(
            Refusal::InvalidArguments,
            format!("{at} is empty: a block (p:..., t:...) or a span p:A..t:B"),
            headings(),
            name,
        ));
    }
    let given = repr_str(address);
    let found = match body_blocks(doc, docx, &ooxml_word_model::address::Address::parse(address)) {
        Ok(found) => found,
        Err(EditError::Address(error)) => {
            let options = if error.options.is_empty() {
                headings()
            } else {
                error.options
            };
            return Err(refused(
                Refusal::InvalidArguments,
                format!("{at} {given}: {}", error.message),
                options,
                name,
            ));
        }
        Err(error) => {
            return Err(refused(
                Refusal::NotFound,
                format!(
                    "{at} {given} names no block of the body ({error}); give a block or a span p:A..t:B, or a \
                     heading's range h:p:A"
                ),
                headings(),
                name,
            ))
        }
    };
    let body = docx.body().map(|b| Arc::clone(b.root()));
    let out: Vec<CopyBlock> = found
        .into_iter()
        .filter_map(|(id, path)| {
            let element = Arc::clone(at_path(body.as_ref()?, &path)?);
            Some(CopyBlock { id, path, element })
        })
        .collect();
    if out.is_empty() {
        return Err(refused(
            Refusal::NotFound,
            format!("{at} {given} holds no paragraph or table"),
            headings(),
            name,
        ));
    }
    Ok(out)
}

/// A refusal at `copies`, `options` the ways in (none: reading the document `name` is).
fn refused(code: Refusal, message: String, options: Vec<String>, name: &str) -> ToolError {
    let way_out = WayOut::names(options)
        .unwrap_or_else(|| WayOut::call(crate::word_read::try_call(serde_json::json!({"doc": name}))));
    ToolError::refusal(code, message, way_out).at("copies")
}
