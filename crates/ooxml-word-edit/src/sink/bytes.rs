//! Owns: a part put in place as bytes ([`EditSink::put_bytes`]): a part copied in from
//! another package (a picture, an embedded workbook) as its bytes are.
//!
//! Must not: parse the bytes, or put a part this edit has changed as a tree.

use super::EditSink;
use crate::doc::Change;
use crate::Result;
use std::sync::Arc;

impl EditSink<'_, '_> {
    /// Put `part` in place as `bytes` (a new part goes at the end of the archive).
    ///
    /// # Errors
    /// When the change cannot be applied.
    pub(crate) fn put_bytes(&mut self, part: &str, bytes: Arc<[u8]>) -> Result<()> {
        self.write(Change::Bytes {
            part: Arc::from(part),
            bytes,
        })
    }
}
