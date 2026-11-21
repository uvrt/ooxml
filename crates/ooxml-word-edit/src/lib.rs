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
//!   [`DeleteBlocks`] (`word_delete`'s blocks, the outline checked first);
//!   [`SetCells`]; [`StyleSpans`] (character styles and emphasis over a paragraph's text);
//!   [`SetStyledText`] (`word_set_text`'s item: text with inline emphasis);
//!   [`InsertMarkdown`] (Markdown written as the document's own blocks); [`CopyBlocks`]
//!   (blocks copied from another open document, or from this one);
//!   [`InsertText`], [`DeleteText`] and [`ReplaceText`] (text within a
//!   paragraph, docx-agent's `_replace_ranges`); [`InsertBeside`] and [`DeletePlaced`] (text
//!   at a range or at the one place a text occurs, `word_insert_text`'s and `word_delete`'s); [`Review`] (accepting and rejecting tracked
//!   changes, `word_changes`); [`SetWordTracking`] (Word's own Track Changes switch); [`UpgradeToModern`] (Word's
//!   Convert to compatibility mode 15);
//!   [`CommentEdits`] (resolving, reopening and deleting comments); [`FormatTable`] (a
//!   table's, row's, column's and cell's formatting); [`FollowSample`] (copies made to follow
//!   a revised sample); [`InsertControl`], [`FillControl`] and [`RemoveControl`] (content
//!   controls, a bound one's custom XML node written with its fill);
//! - how a copy's rows and paragraphs correspond to its sample's ([`pairing`]);
//! - validation per change ([`validate`]: facts memoised by content, docx-agent's rules)
//!   and what a change touched ([`touched`]);
//! - which paragraphs were the file's when the session opened it, and which revision
//!   records this session wrote ([`originals`]);
//! - [`Editor`], which runs a command through the store and reports its result with
//!   `touched` and the validation delta.
//!
//! Must not: lay out, render, know a tool's JSON, hold global state, or depend on an
//! interpreter lock: every value is `Send + Sync`, caches belong to a document, and the
//! store orders concurrent edits.
#![forbid(unsafe_code)]

mod across;
mod address_keys;
mod after;
mod beside;
mod block_checks;
mod blocks;
mod builtin;
mod comments;
mod content_types;
mod control_delete;
mod controls;
mod copying;
mod delete;
mod delete_blocks;
mod delete_write;
mod doc;
mod drawings;
mod drawn;
mod editor;
mod error;
pub mod facts;
mod fields;
pub mod find;
mod follow;
mod format;
mod ids_delta;
mod imported;
mod inline;
mod links;
mod list_style;
mod lists;
mod markdown;
mod marks;
mod merge;
mod moving;
mod moving_tracked;
mod names;
mod namespaces;
mod new_part;
mod notes;
pub mod originals;
pub mod pairing;
mod paragraph_style;
mod paragraphs;
mod people;
mod placing;
mod policy;
mod pool;
mod props;
mod provenance;
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
mod styles;
mod switch;
mod table_delete;
mod table_delete_write;
mod tables;
mod template;
mod text;
mod text_range;
mod textbox;
pub mod touched;
mod track;
pub mod validate;
mod work;

pub use beside::{DeletePlaced, InsertBeside, Placed, Side, TextPlace};
pub use comments::{
    AddComments, CommentEdit, CommentEdited, CommentEdits, CommentMade, CommentWrite, EditCommentText, Who,
};
pub use controls::{ControlEdited, ControlKind, FillControl, Filling, InsertControl, RemoveControl};
pub use copying::{
    BlocksCopied, BookmarkPolicy, CopyBlocks, CopyRange, CopySource, ListPolicy, StylePolicy, UnmappedPolicy,
};
pub use delete::{BlockDeleted, DeleteBlock};
pub use delete_blocks::{blocks_named, BlocksDeleted, DeleteBlocks};
pub use doc::{Change, Draw, DrawFrom, Key, ModelError, Shared, WordDoc};
pub use drawings::{
    image_format, picture_size, written_size, DrawingEdited, DrawingValues, FloatDrawing, InlineDrawing, InsertPicture,
    InsertShape, SetDrawing, ZOrder,
};
pub use drawn::Drawn;
pub use editor::{EditTiming, Edited, Editor};
pub use error::{EditError, Result};
pub use fields::{
    field_facts, fields, format_date, CharAdvance, Day, EditLayout, FieldEdited, FieldFacts, FieldInfo, FieldLayout,
    FieldPlace, FieldRun, InsertCaption, InsertCrossReference, InsertField, InsertToc, LaidFields, LayOutError,
    PageReflow, PageShows, SetStoryNumbered, StoryNumber, StoryNumbered, Unknown, UpdateFields,
};
pub use follow::{CopyFollowed, FollowSample, Followed};
pub use format::{
    check as check_formatting, differs, keep_writes, kept_blocks, kept_package, known_style, occurrences, range_text,
    theme_color_name, FormatEdit, FormatPlan, FormatStep, Formatted, Like, LikeDone, Occurrence, ParagraphLook,
    Property, Setting, Value, KEEPS_BLOCKS, NOT_TAKEN, PARTS,
};
pub use ids_delta::{ids_delta, IdsDelta};
pub use links::{hyperlink_span, AddBookmark, AddHyperlink, LinkEdited, RemoveBookmark, RenameBookmark};
pub use lists::{
    numbered_by_style, paragraphs_of, unnumbered_styles_like, LevelFormat, ListAction, ListEdit, ListStep, ListsEdited,
};
pub use markdown::{markdown_shape, InsertMarkdown, Made, MarkdownInserted, Options as MarkdownOptions, WriteMap};
pub use moving::{section_blocks, section_blocks_in, BlocksMoved, MoveBlocks, Moving};
pub use notes::{DeleteNote, EditNote, InsertNote, MoveNote, NoteEdited, NoteKind};
pub use paragraph_style::{SetParagraphStyle, StyleSet};
pub use paragraphs::{InsertParagraph, ParagraphInserted};
pub use policy::{RevisionPolicy, Tracking};
pub use pool::{EarlierIds, Ids, IdsChange, Sequences};
pub use provenance::{KeepText, TextKeep};
pub use ranges::{DeleteText, InsertText, RangesEdited, ReplaceText};
pub use review::{record_marks, Mark, RemovedBlocks, Review, Reviewed, RowGone, Selection, TableGone, Typed};
pub use sections::{
    InsertSectionBreak, LinkToPrevious, RemoveSectionBreak, SectionEdited, SectionValues, SetSection, SetStoryText,
    StoryEdited, UnlinkFromPrevious,
};
pub use set_cells::{CellAxis, CellEdit, CellWrite, CellsSet, SetCells};
pub use set_text::{SetText, TextEdited};
pub use sink::{EditSink, Keep, Numbered, TextWrite};
pub use style_spans::{Span, SpansStyled, StyleSpans};
pub use styled_text::{ItemEdit, SetStyledText, StyledRun, StyledTextSet};
pub use styles::{
    content_uses, declared, find_style, languages, looks_needed, read_paragraph, read_regions, read_run, read_shading,
    read_tabs, style_link, table_default, usage, word_tag, Declared, Languages, ListLevel, ReadStop, Region,
    RegionBorders, RegionValues, StyleAction, StyleEdit, StyleFormatting, StyleLink, StyleRefusal, StyleRefusalKind,
    StyleStep, TabStop, Usage, BORDER_SIDES, BORDER_STYLES, LOOK_OF, REGIONS, TAB_ALIGNMENTS, TAB_LEADERS,
    WORD_TAG_SLOTS,
};
pub use switch::{tracks_changes, word_tracks_changes, SetWordTracking, WordTracking};
pub use tables::{
    body_lines, body_width, loose_paragraph_after, pending_structure, row_cells, row_shown, rows_unlike, table_keeps,
    table_keeps_at, BodyLines, BodyWidth, CellUnlike, CellValues, EditTable, FormatCall, FormatTable, Look, LookField,
    RowUnlike, TableChange, TableChanged, TableEdit, TableFormat, TableFormatted, TableKeeps, TableValues, TableWidth,
    TextLook, ROW_WINDOW,
};
pub use template::{
    compatibility_mode, Reflowed, UpgradeReflowed, UpgradeToModern, Upgraded, DEFAULT_COMPATIBILITY_MODE, MODERN_MODE,
};
pub use text::diff::ratio as text_ratio;
pub use text_range::anchor;
