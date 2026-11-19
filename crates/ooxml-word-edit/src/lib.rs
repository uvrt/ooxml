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
//! - the commands: [`SetText`] (in lines: new paragraphs after it); [`InsertParagraph`];
//!   [`SetParagraphStyle`] (built-in styles added as Word writes them); [`DeleteBlock`];
//!   [`SetCells`]; [`StyleSpans`] (character styles and emphasis over a paragraph's text);
//!   [`SetStyledText`] (`word_set_text`'s item: text with inline emphasis);
//!   [`InsertMarkdown`] (Markdown written as the document's own blocks);
//!   [`InsertText`], [`DeleteText`] and [`ReplaceText`] (text within a
//!   paragraph, docx-agent's `_replace_ranges`); [`Review`] (accepting and rejecting tracked
//!   changes, `word_changes`); [`SetWordTracking`] (Word's own Track Changes switch);
//!   [`CommentEdits`] (resolving, reopening and deleting comments); [`FormatTable`] (a
//!   table's, row's, column's and cell's formatting);
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
mod after;
mod block_checks;
mod blocks;
mod builtin;
mod comments;
mod delete;
mod delete_write;
mod doc;
mod editor;
mod error;
pub mod find;
mod format;
mod inline;
mod links;
mod list_style;
mod lists;
mod markdown;
mod marks;
mod merge;
mod moving;
mod names;
mod namespaces;
mod new_part;
mod notes;
mod paragraph_style;
mod paragraphs;
mod people;
mod policy;
mod pool;
mod props;
mod ranges;
mod ranges_across;
mod release;
pub mod review;
mod revision_ids;
mod sections;
mod set_cells;
mod set_text;
mod sink;
mod spacing;
mod stamping;
mod style_spans;
mod styled_text;
mod switch;
mod table_delete;
mod table_delete_write;
mod tables;
mod text;
mod text_range;
mod textbox;
pub mod touched;
mod track;
pub mod validate;
mod work;

pub use comments::{CommentEdit, CommentEdited, CommentEdits, EditCommentText};
pub use delete::{BlockDeleted, DeleteBlock};
pub use doc::{Change, Draw, DrawFrom, Key, ModelError, Shared, WordDoc};
pub use editor::{EditTiming, Edited, Editor};
pub use error::{EditError, Result};
pub use format::{
    check as check_formatting, differs, known_style, occurrences, range_text, theme_color_name, FormatEdit, FormatPlan,
    FormatStep, Formatted, Like, LikeDone, Occurrence, ParagraphLook, Property, Setting, Value, KEEPS_BLOCKS,
    NOT_TAKEN, PARTS,
};
pub use links::{hyperlink_span, AddBookmark, AddHyperlink, LinkEdited, RemoveBookmark, RenameBookmark};
pub use lists::{
    numbered_by_style, paragraphs_of, unnumbered_styles_like, LevelFormat, ListAction, ListEdit, ListStep, ListsEdited,
};
pub use markdown::{InsertMarkdown, MarkdownInserted, Options as MarkdownOptions, WriteMap};
pub use moving::{section_blocks, BlocksMoved, MoveBlocks, Moving};
pub use notes::{DeleteNote, EditNote, InsertNote, MoveNote, NoteEdited, NoteKind};
pub use paragraph_style::{SetParagraphStyle, StyleSet};
pub use paragraphs::{InsertParagraph, ParagraphInserted};
pub use policy::{RevisionPolicy, Tracking};
pub use pool::{Ids, IdsChange, Sequences};
pub use ranges::{DeleteText, InsertText, RangesEdited, ReplaceText};
pub use review::{record_marks, Mark, RemovedBlocks, Review, Reviewed, RowGone, Selection, TableGone, Typed};
pub use sections::{
    InsertSectionBreak, LinkToPrevious, RemoveSectionBreak, SectionEdited, SectionValues, SetSection, SetStoryText,
    StoryEdited, UnlinkFromPrevious,
};
pub use set_cells::{CellEdit, CellsSet, SetCells};
pub use set_text::{SetText, TextEdited};
pub use sink::{EditSink, Keep, Numbered, TextWrite};
pub use style_spans::{Span, SpansStyled, StyleSpans};
pub use styled_text::{ItemEdit, SetStyledText, StyledRun, StyledTextSet};
pub use switch::{tracks_changes, word_tracks_changes, SetWordTracking, WordTracking};
pub use tables::{
    pending_structure, CellValues, EditTable, FormatTable, Look, TableChange, TableChanged, TableEdit, TableFormat,
    TableFormatted, TableValues, TableWidth,
};
pub use text_range::anchor;
