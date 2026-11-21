//! Text that was in the file as opened, taken out of the final view (docx-agent's
//! `tools/originals.py`, round 57, R122).
//!
//! The edits that remove whole blocks -- `word_edit_table`'s `delete_row` and
//! `delete_column` (and `sync` with `remove`), `word_delete` of a paragraph, a table or a
//! control -- say so when what they remove held text present when the document was opened:
//! in the summary and in `data.original_removed` ([`Watch::report`]): how many cells and
//! paragraphs, a short sample with their places, the `undo` that takes the call back and,
//! tracked, the `word_changes` reject that keeps them. Removing only what this session made
//! says nothing.
//!
//! Which paragraphs were the file's and which records this session wrote is
//! `ooxml_word_edit::originals`' (`Originals`), shared with `check`'s `accept_loses_original`.
//!
//! Owns: [`Watch`] (an edit's units before it, what of them left the final view after).
//!
//! Must not: change the document, or keep anything between calls.

mod watch;

pub(crate) use watch::{add, Watch};
