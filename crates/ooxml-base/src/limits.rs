//! Owns: the limits on untrusted input, as values (design section 11). Defaults are
//! libxml2's where the Python path relies on them (depth 256, text and attribute values of
//! 10,000,000 bytes, names of 50,000), plus caps libxml2 does not have.
//!
//! Must not: enforce anything itself -- the parser and the archive reader do, with these.

/// Limits on one XML part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XmlLimits {
    /// Deepest element nesting (libxml2 without `huge_tree`: 256).
    pub max_depth: usize,
    /// Longest text node, in bytes as decoded (libxml2: 10,000,000).
    pub max_text_len: usize,
    /// Longest attribute value, in bytes as decoded (libxml2: 10,000,000).
    pub max_attribute_len: usize,
    /// Longest element, attribute or processing-instruction name (libxml2: 50,000).
    pub max_name_len: usize,
    /// Most attributes (namespace declarations included) on one element.
    pub max_attributes: usize,
    /// Most nodes (elements, text, comments, processing instructions) in one part.
    pub max_nodes: usize,
}

impl Default for XmlLimits {
    fn default() -> Self {
        XmlLimits {
            max_depth: 256,
            max_text_len: 10_000_000,
            max_attribute_len: 10_000_000,
            max_name_len: 50_000,
            max_attributes: 4_096,
            max_nodes: 50_000_000,
        }
    }
}

/// Limits on one ZIP archive (zip bombs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipLimits {
    /// Most entries.
    pub max_entries: usize,
    /// Largest uncompressed size of one entry, in bytes.
    pub max_entry_size: u64,
    /// Largest uncompressed size of all entries together, in bytes. Nested packages count
    /// against the budget of the package they are in (their reader is given what is left).
    pub max_total_size: u64,
    /// Largest ratio of uncompressed to compressed size of one entry ...
    pub max_ratio: u64,
    /// ... checked only for entries larger than this, uncompressed.
    pub ratio_floor: u64,
}

impl Default for ZipLimits {
    fn default() -> Self {
        ZipLimits {
            max_entries: 100_000,
            max_entry_size: 1 << 30,
            max_total_size: 2 << 30,
            max_ratio: 1_000,
            ratio_floor: 1 << 20,
        }
    }
}

/// Every limit, as one value carried by a [`crate::Context`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Limits {
    /// Per XML part.
    pub xml: XmlLimits,
    /// Per archive.
    pub zip: ZipLimits,
}
