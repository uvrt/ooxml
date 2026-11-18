//! Word's tools (docs/architecture/README.md, section 3 `ooxml-tools-*`, section 9; layer 4).
//!
//! Owns: each Word tool's definition (a `#[derive(ToolArgs)]` struct with its description
//! as a Markdown file beside it) and its handler: a reading tool's, a function of the
//! typed arguments and a version of the document; an editing tool's, of the typed
//! arguments and the document's editor ([`Editing`]).
//!
//! Must not: hold sessions, documents by id, locks or dispatch (`ooxml-session` does, and
//! hands a handler the version it reads), know providers, or keep state between calls.
//! Every handler is a pure function of its inputs and checks the [`ooxml_base::Cancel`]
//! port in its loops.
#![forbid(unsafe_code)]

mod describe;
mod find_text;
mod paging;
mod placements;
mod refusal;
mod review;
mod structure;
mod style;
mod text;
mod version;
mod word_inspect;
mod word_read;

pub use describe::{describe, Describe, DESCRIBED, NOT_DESCRIBED};
pub use find_text::{find_text, FindStories, FindText};
pub use placements::Placer;
pub use review::{
    word_changes, word_comments, word_set_tracking, ChangeAction, ChangeDetail, ChangeKind, CommentAction, CommentItem,
    Editing, Tracked, WordChanges, WordComments, WordSetTracking, COMMENT_WRITES,
};
pub use structure::{
    word_headers_footers, word_links, word_move, word_notes, word_sections, CopyStyles, FieldAction, HeaderAction,
    LinkAction, Margins, NoteAction, NoteKind, Orientation, PageNumber, ReferenceKind, SectionAction, SectionStart,
    StoryType, StyleMapping, Unmapped, Which, WordCopyFrom, WordFields, WordHeadersFooters, WordLinks, WordMove,
    WordNotes, WordSections,
};
pub use style::{word_lists, Follow, ListKind, ListsAction, WordLists};
pub use text::{replace_text, word_set_text, Expect, ReplaceStories, ReplaceText, SetTextItem, Unported, WordSetText};
pub use version::Version;
pub use word_inspect::{word_inspect, WordInspect};
pub use word_read::{word_read, Story, View, WordRead};

/// What `unported` says of a reply holding a chart's or SmartArt diagram's summary or JSON,
/// which docx-agent reads from the chart model (phase P3).
pub const GRAPHICS: &str = "chart or SmartArt summary";

/// What `unported` says of a final view where a cell's end is one Accept All takes away.
pub const CELL_ENDS: &str = "Accept All's cell ends";

/// The document kind these tools serve.
pub const KIND: &str = "docx";
