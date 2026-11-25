//! Owns: [`BlockId`], a block's durable id as written (`p:1A2B3C4D`, `t:00000001`,
//! `tr:0A0B0C0D`), and [`Renames`], the ids an edit renamed, old to new.
//!
//! Must not: parse an address (the address module does) or check an id's form: an id is
//! carried as the index or an edit wrote it.

use std::sync::Arc;

/// A paragraph's, table's or row's durable id, exactly as written.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BlockId(Arc<str>);

/// The ids an edit renamed, old to new, in the order made.
pub type Renames = Vec<(BlockId, BlockId)>;

impl BlockId {
    /// The id `id`.
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }

    /// The id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The id, shared.
    #[must_use]
    pub fn as_arc(&self) -> &Arc<str> {
        &self.0
    }
}

impl std::ops::Deref for BlockId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for BlockId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for BlockId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Debug for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&*self.0, f)
    }
}

impl serde::Serialize for BlockId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl From<&str> for BlockId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl From<String> for BlockId {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl From<&String> for BlockId {
    fn from(id: &String) -> Self {
        Self::new(id.as_str())
    }
}

impl From<Arc<str>> for BlockId {
    fn from(id: Arc<str>) -> Self {
        Self(id)
    }
}

impl From<BlockId> for String {
    fn from(id: BlockId) -> Self {
        id.0.to_string()
    }
}

impl PartialEq<str> for BlockId {
    fn eq(&self, other: &str) -> bool {
        *self.0 == *other
    }
}

impl PartialEq<&str> for BlockId {
    fn eq(&self, other: &&str) -> bool {
        *self.0 == **other
    }
}

impl PartialEq<String> for BlockId {
    fn eq(&self, other: &String) -> bool {
        *self.0 == **other
    }
}

impl PartialEq<BlockId> for str {
    fn eq(&self, other: &BlockId) -> bool {
        *self == *other.0
    }
}

impl PartialEq<BlockId> for &str {
    fn eq(&self, other: &BlockId) -> bool {
        **self == *other.0
    }
}

impl PartialEq<BlockId> for String {
    fn eq(&self, other: &BlockId) -> bool {
        **self == *other.0
    }
}

/// `renames` as strings, for a reply.
#[must_use]
pub fn renames_text(renames: &[(BlockId, BlockId)]) -> Vec<(String, String)> {
    renames
        .iter()
        .map(|(old, new)| (old.to_string(), new.to_string()))
        .collect()
}
