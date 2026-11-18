//! Word's edits as transactions (docs/architecture/README.md, sections 4, 5 and 7; phase
//! P2, lane P2a; layer 3).
//!
//! Owns:
//! - [`WordDoc`], a Word document as the store keeps it: its package version, its ids
//!   ([`Ids`]: the pool, the sequences' cursors, the renames) and its reading, with the
//!   [`Key`]s transactions read and write and the [`Change`]s of a change set (tree
//!   patches per part rebased past concurrent ones, parts put, ledger additions);
//! - [`EditSink`], the one seam every Word edit writes through, and the [`RevisionPolicy`]
//!   behind it that writes a text edit untracked or as tracked changes, as docx-agent
//!   writes them;
//! - stamping blocks with durable ids before the first edit, and new text ids;
//! - the commands: [`SetText`] (in lines: new paragraphs after it); [`InsertParagraph`]; [`SetParagraphStyle`] (built-in styles added as Word writes them); [`DeleteBlock`]; [`SetCells`]; [`InsertText`], [`DeleteText`] and [`ReplaceText`] (text
//!   within a paragraph, docx-agent's `_replace_ranges`);
//! - validation per change ([`validate`]: facts memoised by content, docx-agent's rules)
//!   and what a change touched ([`touched`]);
//! - [`Editor`], which runs a command through the store and reports its result with
//!   `touched` and the validation delta.
//!
//! Must not: lay out, render, know a tool's JSON, hold global state, or depend on an
//! interpreter lock: every value is `Send + Sync`, caches belong to a document, and the
//! store orders concurrent edits.
#![forbid(unsafe_code)]

mod across;
mod block_checks;
mod builtin;
mod delete;
mod delete_write;
mod doc;
mod editor;
mod error;
pub mod find;
mod inline;
mod list_style;
mod marks;
mod names;
mod namespaces;
mod paragraph_style;
mod paragraphs;
mod people;
mod policy;
mod pool;
mod ranges;
mod ranges_across;
mod release;
mod revision_ids;
mod set_cells;
mod set_text;
mod sink;
mod stamping;
mod text;
mod textbox;
pub mod touched;
mod track;
pub mod validate;
mod work;

pub use delete::{BlockDeleted, DeleteBlock};
pub use doc::{Change, Draw, DrawFrom, Key, ModelError, Shared, WordDoc};
pub use editor::{EditTiming, Edited, Editor};
pub use error::{EditError, Result};
pub use paragraph_style::{SetParagraphStyle, StyleSet};
pub use paragraphs::{InsertParagraph, ParagraphInserted};
pub use policy::{RevisionPolicy, Tracking};
pub use pool::{Ids, IdsChange, Sequences};
pub use ranges::{DeleteText, InsertText, RangesEdited, ReplaceText};
pub use set_cells::{CellEdit, CellsSet, SetCells};
pub use set_text::{SetText, TextEdited};
pub use sink::{EditSink, Keep, Numbered, TextWrite};
