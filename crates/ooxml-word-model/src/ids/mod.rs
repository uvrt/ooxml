//! Stable addressing (docs/architecture/README.md, section 4 "Ids"; phase P1): what every
//! paragraph, table and row is called, the id ledger, and the `docx-ids.1.` state kept
//! beside a document's bytes -- docx-agent's rules (`docx_agent/edit/ids.py`), so ids are
//! the same whichever side reads a document, before and after it is saved and opened again.
//!
//! Owns: the part index ([`PartIds`]), the ledger and the value sequences ([`Ledger`],
//! [`IdPool`]), the state string ([`export_state`], [`load_state`]), and the story names
//! ids are prefixed with ([`story_name`]).
//!
//! Must not: stamp or write a part (edits do, phase P2), resolve an address (the address
//! module does), or hold global state: sequences are cached per pool.

mod index;
mod ledger;
mod sha256;
pub use sha256::sha256;
mod state;

pub use index::{Entry, IdState, ParagraphId, PartIds, Path, TableId};
pub use ledger::{block_and_drawing_ids, drawing_numbers, sequence_value, used_long_hex, IdPool, Ledger, REACH};
pub use state::{
    export_state, footprint, load_state, python_json, record_of, IdStateError, LoadedState, Versions, STATE_PREFIX,
};

/// WordprocessingML's main namespace.
pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
/// Word 2010's namespace (`w14:paraId`, `w14:textId`).
pub const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
/// Markup compatibility (`mc:Fallback`).
pub const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
/// WordprocessingML drawings (`wp:inline`, `wp:anchor`, `wp:docPr`).
pub const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
/// Word 2010's drawing namespace (`wp14:anchorId`, `wp14:editId`).
pub const WP14: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing";

/// The limit Word's paraIds and textIds stay below (`ST_LongHexNumber` as Word accepts it).
pub const PARA_ID_LIMIT: u32 = 0x8000_0000;

/// A value written as one to eight hex digits (`[0-9A-Fa-f]{1,8}`), or `None`.
#[must_use]
pub fn hex_value(raw: &str) -> Option<u32> {
    if raw.is_empty() || raw.len() > 8 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(raw, 16).ok()
}

/// Whether `raw` is a paraId Word keeps: one to eight hex digits, above 0, below
/// [`PARA_ID_LIMIT`].
#[must_use]
pub fn valid_long_hex(raw: &str) -> bool {
    hex_value(raw).is_some_and(|v| v > 0 && v < PARA_ID_LIMIT)
}

/// The name a part's story goes by in ids: `body` for the main part, else the part's stem
/// (`header2`, `footnotes`, `comments`).
#[must_use]
pub fn story_name(main_part: &str, part: &str) -> String {
    if part == main_part {
        return "body".to_owned();
    }
    let base = part.rsplit('/').next().unwrap_or(part);
    match base.rfind('.') {
        Some(dot) if dot > 0 => base.get(..dot).unwrap_or(base).to_owned(),
        _ => base.to_owned(),
    }
}
