//! Owns: the lookup a tool makes of the inputs a host registered for its session
//! (docx-agent's `session.blob`: `not_found` listing the handles there are); the inputs
//! themselves are format-neutral ([`ooxml_tools::blobs`], re-exported here).
//!
//! Must not: keep blobs (the host keeps them, and checks their size and type when they are
//! registered), or decode them.

pub use ooxml_tools::blobs::{Blob, BlobList, Blobs};
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};

/// The blob `handle` names; `not_found` listing `"handle (name)"` of every blob there is.
pub(crate) fn blob(blobs: &dyn Blobs, handle: &str, call: &TryCall) -> Result<Blob, ToolError> {
    blobs.blob(handle).ok_or_else(|| {
        let listed = blobs.listed().into_iter().map(|(h, n)| format!("{h} ({n})"));
        let way_out = WayOut::names(listed).map_or_else(|| WayOut::call(call.clone()), |n| n.and_call(call.clone()));
        ToolError::refusal(
            Refusal::NotFound,
            format!("no blob {}", ooxml_base::pyfmt::repr_str(handle)),
            way_out,
        )
    })
}
