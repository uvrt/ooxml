//! Owns: the inputs a host registered for its session as a tool reads them ([`Blobs`]: a
//! blob by its handle -- name, type, bytes -- and the handles there are), and the lookup a
//! tool makes (docx-agent's `session.blob`: `not_found` listing the handles there are).
//!
//! Must not: keep blobs (the host keeps them, and checks their size and type when they are
//! registered), or decode them.

use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use std::sync::Arc;

/// An input the host registered under a handle (`b1`).
#[derive(Debug, Clone)]
pub struct Blob {
    /// Its handle.
    pub handle: String,
    /// The name it was registered with.
    pub name: String,
    /// Its media type, as the host checked it against the bytes.
    pub mime: String,
    /// Its bytes.
    pub data: Arc<[u8]>,
}

/// The session's blobs, as the host keeps them.
pub trait Blobs: Send + Sync {
    /// The blob `handle` names, if there is one.
    fn blob(&self, handle: &str) -> Option<Blob>;

    /// Every blob's handle and name, in the order they were registered.
    fn listed(&self) -> Vec<(String, String)>;
}

/// Blobs held in a list (a host's or a test's).
#[derive(Debug, Clone, Default)]
pub struct BlobList(pub Vec<Blob>);

impl Blobs for BlobList {
    fn blob(&self, handle: &str) -> Option<Blob> {
        self.0.iter().find(|b| b.handle == handle).cloned()
    }

    fn listed(&self) -> Vec<(String, String)> {
        self.0.iter().map(|b| (b.handle.clone(), b.name.clone())).collect()
    }
}

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
