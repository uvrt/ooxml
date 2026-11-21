//! Owns: `check`'s refusal of a `range` (docx-agent's `scope._paragraphs_of`, `_refused`):
//! a range that names nothing, nothing of the body or no paragraph, worded with its place
//! among the ranges and the ways in -- the body's headings as ranges (the resolver's
//! options where it gives some), else the call that reads the document.
//!
//! Must not: resolve an address (the Word model does) or scope the facts
//! (`ooxml_word_pages::layout_facts::Scope` does).

use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{Failure, Refusal, ToolError, WayOut};
use ooxml_word_model::address::resolve::offered;
use ooxml_word_model::docx::Docx;
use ooxml_word_pages::layout_facts::ScopeError;
use serde_json::json;

/// What every refused range is told to give instead (`_refused`).
const WAYS_IN: &str =
    "; give a block, a span p:A..t:B, a cell, a section s:2 or a heading's range h:p:A (word_read's range)";

/// The options a refusal lists at most.
const OPTIONS: usize = 50;

/// `error`, a range of `doc` (read as `docx`) refused, as docx-agent's `check` refuses it.
pub(super) fn range_refused(error: ScopeError, docx: &Docx, doc: &str) -> ToolError {
    let (code, message, options) = match worded(error) {
        Ok(worded) => worded,
        // docx-agent's `ValueError` is not caught there: said as the tools say one.
        Err(unit) => {
            return ToolError::failure(Failure::Unit, unit)
                .with_way_out(WayOut::call(read_call(doc)))
                .at("range")
        }
    };
    let options = if options.is_empty() { offered(docx) } else { options };
    let way_out = match WayOut::names(options.into_iter().take(OPTIONS)) {
        Some(names) => names,
        None => WayOut::call(read_call(doc)),
    };
    ToolError::refusal(code, message, way_out).at("range")
}

/// The refusal's code, words and the resolver's options (none: the body's headings are
/// offered); `Err` with its words for a range in the wrong unit.
fn worded(error: ScopeError) -> Result<(Refusal, String, Vec<String>), String> {
    let (code, message, options) = match error {
        ScopeError::Address { number, address, error } => {
            let given = repr_str(&address);
            match error.code {
                // docx-agent's `KeyError`: the id names nothing.
                "not_found" => (
                    Refusal::NotFound,
                    format!(
                        "range[{number}] {given} names nothing in the document ({})",
                        error.message
                    ),
                    Vec::new(),
                ),
                "unit" => return Err(error.message),
                // `AddressError`, with its options.
                "invalid_arguments" => (
                    Refusal::InvalidArguments,
                    format!("range[{number}] {given}: {}", error.message),
                    error.options,
                ),
                // `EditError`.
                _ => (
                    Refusal::InvalidArguments,
                    format!("range[{number}] {given}: {}", error.message),
                    Vec::new(),
                ),
            }
        }
        ScopeError::NotBody { number, address, story } => (
            Refusal::InvalidArguments,
            format!(
                "range[{number}] {} is in {story}, not the body: a range names the body's blocks",
                repr_str(&address)
            ),
            Vec::new(),
        ),
        ScopeError::Empty { number, address } => (
            Refusal::NotFound,
            format!("range[{number}] {} holds no paragraph", repr_str(&address)),
            Vec::new(),
        ),
    };
    Ok((code, format!("{message}{WAYS_IN}"), options))
}

/// The call that reads `doc`, for its blocks' ids.
fn read_call(doc: &str) -> ooxml_tools::TryCall {
    crate::word_read::try_call(json!({"doc": doc}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_word_model::address::AddressError;

    #[test]
    fn a_range_naming_nothing_is_refused_as_docx_agent_words_it() {
        let error = ScopeError::Address {
            number: 0,
            address: "p:0BADF00D".into(),
            error: AddressError::not_found("no paragraph or table 'p:0BADF00D'".into(), vec!["p:1".into()]),
        };
        let (code, message, options) = worded(error).expect("a refusal");
        assert_eq!(code, Refusal::NotFound);
        assert_eq!(
            message,
            "range[0] 'p:0BADF00D' names nothing in the document (no paragraph or table 'p:0BADF00D'); give a \
             block, a span p:A..t:B, a cell, a section s:2 or a heading's range h:p:A (word_read's range)"
        );
        assert!(
            options.is_empty(),
            "the headings are offered, not the resolver's paragraphs"
        );
        let empty = ScopeError::Empty {
            number: 1,
            address: "s:1".into(),
        };
        let (code, message, _) = worded(empty).expect("a refusal");
        assert_eq!(code, Refusal::NotFound);
        assert!(message.starts_with("range[1] 's:1' holds no paragraph; give a block"));
    }

    #[test]
    fn a_range_outside_the_body_names_its_story() {
        let error = ScopeError::NotBody {
            number: 2,
            address: "p:1".into(),
            story: "header1".into(),
        };
        let (code, message, _) = worded(error).expect("a refusal");
        assert_eq!(code, Refusal::InvalidArguments);
        assert!(message.starts_with("range[2] 'p:1' is in header1, not the body: a range names the body's blocks; "));
    }
}
