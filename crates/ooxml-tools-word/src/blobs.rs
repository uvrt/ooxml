//! Owns: the inputs a host registered for its session as a tool reads them ([`Blobs`]: a
//! blob by its handle -- name, type, bytes -- and the handles there are), and the lookups a
//! tool makes (docx-agent's `session.blob`: `not_found` listing the handles there are; a
//! Markdown picture's source, `images_from`: an image blob by its handle or name).
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

/// The bytes of the image blob a Markdown picture's `src` names, by its handle or name
/// (docx-agent's `images_from`): `invalid_arguments` for a blob that is no image,
/// `not_found` listing the image blobs there are.
pub(crate) fn image(blobs: &dyn Blobs, src: &str, call: &TryCall) -> Result<Arc<[u8]>, ToolError> {
    let listed = blobs.listed();
    let found = listed
        .iter()
        .find(|(handle, name)| src == handle || src == name)
        .and_then(|(handle, _)| blobs.blob(handle));
    if let Some(found) = found {
        if !found.mime.starts_with("image/") {
            let message = format!("{src} is a {} blob, not an image", found.mime);
            return Err(ToolError::refusal(
                Refusal::InvalidArguments,
                message,
                WayOut::call(call.clone()),
            ));
        }
        return Ok(found.data);
    }
    let images = listed.into_iter().filter_map(|(handle, name)| {
        let blob = blobs.blob(&handle)?;
        blob.mime.starts_with("image/").then(|| format!("{handle} ({name})"))
    });
    let way_out = WayOut::names(images).map_or_else(|| WayOut::call(call.clone()), |n| n.and_call(call.clone()));
    let message = format!(
        "no image blob {}: use a blob handle as the image source",
        ooxml_base::pyfmt::repr_str(src)
    );
    Err(ToolError::refusal(Refusal::NotFound, message, way_out))
}
