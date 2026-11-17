//! Owns: an address the Word model refused ([`AddressError`]) as the tool's refusal, coded
//! as docx-agent's tools code it (`not_found`, `invalid_arguments`, `refused`, `unit`),
//! with its way out: the ids that would do, and a call that works.
//!
//! Must not: word the refusal (the address resolver does) or resolve addresses.

use ooxml_tools::{Failure, Refusal, ToolError, TryCall, WayOut};
use ooxml_word_model::address::AddressError;

/// `error` refused at `field`, `call` the call that would work.
pub(crate) fn address_refusal(error: AddressError, field: &str, call: TryCall) -> ToolError {
    let code = match error.code {
        "not_found" => Refusal::NotFound,
        "refused" => Refusal::Refused,
        "unit" => {
            return ToolError::failure(Failure::Unit, error.message)
                .with_way_out(WayOut::call(call))
                .at(field)
        }
        _ => Refusal::InvalidArguments,
    };
    let way_out = match WayOut::names(error.options) {
        Some(options) => options.and_call(call),
        None => WayOut::call(call),
    };
    ToolError::refusal(code, error.message, way_out).at(field)
}
