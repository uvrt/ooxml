//! Owns: the inputs a host registered for its session as a tool reads them ([`Blob`]: its
//! handle, name, media type and bytes; [`Blobs`]: a blob by its handle and the handles there
//! are), format-neutral: the session's store stands behind [`Blobs`], and a tool of any
//! format reads it.
//!
//! Must not: keep or check blobs (the session's store does, when they are registered), or
//! decode them.

use serde_json::{Map, Value};
use std::sync::Arc;

/// An input the host registered under a handle (`b1`).
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl Blob {
    /// What the session's `describe` says of it: never its bytes.
    #[must_use]
    pub fn describe(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("handle".into(), Value::from(self.handle.as_str()));
        out.insert("name".into(), Value::from(self.name.as_str()));
        out.insert("mime".into(), Value::from(self.mime.as_str()));
        out.insert("size".into(), Value::from(self.data.len()));
        out
    }
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
