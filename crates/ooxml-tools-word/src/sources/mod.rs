//! Owns: what Word's tools read of a call's sources beyond the values themselves (those --
//! [`SourceRows`], [`Cell`], [`label`], [`date_pattern`] -- are `ooxml-tools`' since phase 3
//! C1): table keys compared loosely ([`loose_key`], [`natural`]), rows placed in a table
//! ([`placed`]), and the refusals of a source as Word's tools word them: with the hint of a
//! drawing's id given for a dataset ([`refused_source`]), a value refused at its argument
//! ([`value_refusal`]), a source handed no rows ([`no_rows`]).
//!
//! Must not: resolve a source (the host's datasets and other documents do), or read the
//! document but to name a drawing in a refusal.

mod keys;
mod placed;

pub(crate) use keys::{loose_key, natural, Run};
pub(crate) use ooxml_tools::sources::{date_pattern, label, what, DatePattern};
pub use ooxml_tools::sources::{Cell, Clock, Day, SourceRefused, SourceRows, Sources};
pub(crate) use placed::{first_rows, fits, placed, thousands, Placed};

use ooxml_base::pyfmt::repr_str;
use ooxml_tools::{ErrorCode, Refusal, ToolError, TryCall, WayOut};

/// The refusal of a source the host refused, as the dispatcher words it: with the hint of a
/// drawing's id given for a dataset (the docx format's `source_hint`).
pub(crate) fn refused_source(
    refused: &SourceRefused,
    docx: Option<&ooxml_word_model::docx::Docx>,
    retry: &TryCall,
) -> ToolError {
    let given = &refused.given;
    let drawing_like = given.strip_prefix("d:").is_some_and(|rest| {
        let (number, member) = rest.split_once('#').map_or((rest, None), |(n, m)| (n, Some(m)));
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit());
        digits(number) && member.is_none_or(digits)
    });
    let mut message = refused.message.clone();
    if drawing_like {
        let here = docx.is_some_and(|d| {
            ooxml_word_model::docx::drawings::all_drawings(d)
                .iter()
                .any(|x| x.id == *given)
        });
        let what = if here {
            "is a drawing in this document"
        } else {
            "is a drawing's id (none here)"
        };
        message.push_str(&format!(
            "; {given} {what}, not a dataset: a source is a dataset d:<id>, its id starting with a letter, or a table \
             t:<name>"
        ));
    }
    ToolError::refusal(
        Refusal::NotFound,
        message,
        WayOut::names(refused.options.iter().cloned()).unwrap_or_else(|| WayOut::call(retry.clone())),
    )
    .at("from")
}

/// The host's refusal of one value a reference names ([`SourceRefused::field`]), as it
/// worded it, at its argument.
pub(crate) fn value_refusal(refused: &SourceRefused, retry: &TryCall) -> ToolError {
    let field = refused.field.clone().unwrap_or_else(|| "from".to_owned());
    match refused.code {
        ErrorCode::Refusal(code) => ToolError::refusal(
            code,
            refused.message.clone(),
            WayOut::names(refused.options.iter().cloned()).unwrap_or_else(|| WayOut::call(retry.clone())),
        ),
        ErrorCode::Failure(code) => ToolError::failure(code, refused.message.clone()),
    }
    .at(field)
}

/// The refusal of a source the host handed no rows for: a dataset as ooxml-edit's
/// session refuses an unknown one, anything else as a source naming nothing.
pub(crate) fn no_rows(given: &str, field: &str, retry: &TryCall) -> ToolError {
    let message = if given.starts_with("d:") {
        let id = given.trim_start_matches("d:");
        let id = id.split(['/', '[']).next().unwrap_or(id);
        format!("no dataset d:{id}")
    } else {
        format!("no rows for {}", repr_str(given))
    };
    ToolError::refusal(Refusal::NotFound, message, WayOut::call(retry.clone())).at(field)
}

#[cfg(test)]
mod tests;
