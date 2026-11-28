//! Owns: an address the Word model refused ([`AddressError`]) as the tool's refusal, coded
//! as docx-agent's tools code it (`not_found`, `invalid_arguments`, `refused`, `unit`),
//! with its way out: the ids that would do, and a call that works; and a form not supported
//! yet, refused with the call of a form that is ([`not_supported`]).
//!
//! Must not: word the refusal (the address resolver does) or resolve addresses.

use ooxml_tools::{Failure, Refusal, ToolError, TryCall, WayOut};
use ooxml_word_model::address::AddressError;

/// `error` refused at `field`, `call` the call that would work.
pub(crate) fn address_refusal(error: AddressError, field: &str, call: TryCall) -> ToolError {
    address_refused(error, call).at(field)
}

/// [`address_refusal`] at no field: where today's tool names none (an address its handler
/// resolves, not its arguments' validation).
pub(crate) fn address_refused(error: AddressError, call: TryCall) -> ToolError {
    let code = match error.code {
        "not_found" => Refusal::NotFound,
        "refused" => Refusal::Refused,
        "ambiguous" => Refusal::Ambiguous,
        "unit" => return ToolError::failure(Failure::Unit, error.message).with_way_out(WayOut::call(call)),
        // a column as accepted that is no one grid column (Word's Accept All moves cells)
        "not_supported" => {
            return ToolError::refusal(Refusal::Refused, error.message, WayOut::call(call)).with_detail(
                ooxml_tools::NOT_SUPPORTED,
                serde_json::Value::from(ooxml_word_model::docx::tables::MOVED_COLUMN),
            )
        }
        _ => Refusal::InvalidArguments,
    };
    let way_out = match WayOut::names(error.options) {
        Some(options) => options.and_call(call),
        None => WayOut::call(call),
    };
    ToolError::refusal(code, error.message, way_out)
}

/// A form not supported yet (`form`, its stable id), refused with the call of `tool` with
/// `arguments` as the way out, `fill` the arguments in it to give otherwise.
pub(crate) fn not_supported(form: &str, tool: &str, arguments: serde_json::Value, fill: &[&str]) -> ToolError {
    let call = TryCall {
        tool: tool.to_owned(),
        arguments: match arguments {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        },
        fill: fill.iter().map(|f| (*f).to_owned()).collect(),
    };
    ToolError::not_supported(form, WayOut::call(call))
}
