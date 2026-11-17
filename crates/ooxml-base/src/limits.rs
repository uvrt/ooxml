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

/// Limits on one metafile (EMF, EMF+ or WMF) and the bitmaps it carries. The defaults are
/// ooxml-common's metafile player's, whose output the Rust players reproduce: generous for
/// real files (the largest metafile in a normal deck has a few thousand records and tens of
/// thousands of points), and what a hostile file may ask for at most.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetafileLimits {
    /// Largest metafile, in bytes.
    pub max_input_bytes: usize,
    /// Largest EMF record, in bytes.
    pub max_record_bytes: usize,
    /// Most records played.
    pub max_records: usize,
    /// Most points over every shape of one metafile.
    pub max_points: u64,
    /// Highest object slot.
    pub max_objects: u32,
    /// Deepest saved-state stack (GDI's `SAVEDC`, GDI+'s saves and containers).
    pub max_save_depth: usize,
    /// Most characters of SVG one metafile may draw.
    pub max_output_chars: usize,
    /// Most decoded pixels over every bitmap of one metafile, and of one bitmap.
    pub max_pixels: u64,
    /// Most characters one text call draws.
    pub max_text_chars: usize,
    /// Largest EMF+ object, in bytes (a continued object's whole size).
    pub max_plus_object_bytes: usize,
    /// Most pixels shaded for path gradients over one metafile ...
    pub max_gradient_pixels: u64,
    /// ... and the most for one gradient before it is sampled more coarsely.
    pub max_gradient_image: u64,
    /// How deep metafiles drawn as EMF+ images may nest.
    pub max_nesting: u32,
}

impl Default for MetafileLimits {
    fn default() -> Self {
        MetafileLimits {
            max_input_bytes: 8 * 1024 * 1024,
            max_record_bytes: 4 * 1024 * 1024,
            max_records: 100_000,
            max_points: 500_000,
            max_objects: 65_536,
            max_save_depth: 1_024,
            max_output_chars: 32 * 1024 * 1024,
            max_pixels: 64_000_000,
            max_text_chars: 8_192,
            max_plus_object_bytes: 4 * 1024 * 1024,
            max_gradient_pixels: 1_000_000,
            max_gradient_image: 250_000,
            max_nesting: 2,
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
    /// Per metafile.
    pub metafile: MetafileLimits,
}
