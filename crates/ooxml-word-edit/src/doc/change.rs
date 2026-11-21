//! Owns: what a transaction reads and a change writes ([`Key`]) and the primitives of a
//! change set ([`Change`]): tree patches per part, parts put or removed, ledger additions,
//! ids drawn where the change lands ([`Draw`]), the count of spaces the edit's language
//! spacing wrote; why one could not be applied ([`ModelError`]).
//!
//! Must not: apply a change (the model does, [`super::model`]) or decide one (commands do,
//! through the sink).

use crate::pool::IdsChange;
use ooxml_opc::{OpcError, Package};
use ooxml_xml::{HeadPatch, Patch, PatchError, Tree};
use std::sync::Arc;

/// What a transaction reads and a change writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// A paragraph, table or row of a story part, by its durable id (`p:1A2B3C4D`).
    Block {
        /// The part.
        part: Arc<str>,
        /// The block's id.
        id: Arc<str>,
    },
    /// A whole part (a part without durable ids, a part put or removed).
    Part(Arc<str>),
    /// A part's root element's head: its namespace declarations and attributes.
    Root(Arc<str>),
    /// An id sequence (a seed): drawing from it.
    Seq(Arc<str>),
    /// The annotation ids in use (revisions, bookmarks, comments): allocating one.
    Annotations,
    /// Who the people part lists.
    People,
}

/// One primitive of a change set.
#[derive(Debug, Clone)]
pub enum Change {
    /// A splice of an element's children in a part's tree.
    Splice {
        /// The part.
        part: Arc<str>,
        /// The patch.
        patch: Patch,
        /// The keys it writes.
        keys: Arc<[Key]>,
        /// Ids drawn from the ledger where the change is applied, written into the
        /// inserted element (the ledger merged at commit: a draw is not a conflict).
        draws: Arc<[Draw]>,
        /// The keys of the blocks it makes under stand-ins ([`DocModel::created`]): kept
        /// apart from `keys`, resolved at commit.
        created: Arc<[Key]>,
    },
    /// An element's head changed in place in a part's tree.
    Head {
        /// The part.
        part: Arc<str>,
        /// The patch.
        patch: HeadPatch,
        /// The keys it writes.
        keys: Arc<[Key]>,
    },
    /// A part put in place (a new part, or one replaced whole).
    Put {
        /// The part.
        part: Arc<str>,
        /// Its tree.
        tree: Tree,
    },
    /// A part put in place as bytes, kept as given (media, or any part a bridged tool wrote
    /// that is not XML).
    Bytes {
        /// The part.
        part: Arc<str>,
        /// Its bytes.
        bytes: Arc<[u8]>,
    },
    /// A part removed (a relationships part left empty, a part nothing uses any more).
    Remove {
        /// The part.
        part: Arc<str>,
    },
    /// Parts as another version of the package has them, or absent there (what undo puts
    /// back).
    Restore {
        /// The parts.
        parts: Arc<[Arc<str>]>,
        /// The version to take them from.
        from: Package,
    },
    /// The people part kept, where the change is applied (docx-agent's `Stamp.finish`):
    /// `add` listed (the part made when there is none), then `tidy` dropped when nothing
    /// of theirs is left in use (the part with it once nobody is listed). Applied at
    /// commit, so two writers listing themselves do not conflict.
    People {
        /// The author to list.
        add: Option<Arc<str>>,
        /// The author to drop if none of their revisions or comments is left.
        tidy: Option<Arc<str>>,
        /// The keys it writes (the parts it may change).
        keys: Arc<[Key]>,
    },
    /// Ids added to the ledger (never taken back: an id given is never given again).
    Ids(IdsChange),
    /// Values drawn from `seed`'s sequence where the change is applied, one for each of
    /// `stand_ins` (the values the version the edit was prepared on gave): each recorded as
    /// what its stand-in was drawn as ([`crate::Drawn`]), for the splices after it
    /// ([`DrawFrom::Drawn`]) and the keys and replies of the commit. Merged at commit: two
    /// transactions drawing from one seed do not conflict.
    Draw {
        /// The seed.
        seed: Arc<str>,
        /// The values drawn where the edit was prepared, upper-case hex.
        stand_ins: Arc<[Arc<str>]>,
    },
    /// Spaces the edit's language spacing wrote or made U+00A0 (docx-agent's
    /// `_space_written`, when the edit ends): nothing in the package changes; the version
    /// counts them for the transaction's reply ([`super::WordDoc::spaced`]).
    Spaced(usize),
    /// Where the edit's revision ids start: one above the largest annotation id in use in
    /// the version it is applied to (docx-agent's `Stamp`, made as the edit begins
    /// recording). The draws after it ([`DrawFrom::Annotation`]) count from there.
    Number,
}

/// Where a drawn value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawFrom {
    /// The next value of a seed's sequence the ledger lacks (paraIds, textIds).
    Seed(Arc<str>),
    /// An annotation id: the edit's first ([`Change::Number`]) plus this offset (its
    /// revision records, numbered in the order it made them).
    Annotation(u32),
    /// What a stand-in was drawn as by a [`Change::Draw`] before it in the transaction.
    Drawn(Arc<str>),
}

/// An id drawn when a splice is applied, into an attribute of the element it inserts: the
/// value is the one the version it is applied to gives next, so a commit rebased past
/// others draws what a serial run would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draw {
    /// Where the value comes from.
    pub from: DrawFrom,
    /// Which of the splice's inserted nodes it goes into (0 for a splice of one).
    pub node: u32,
    /// The element's place below that node (empty: the node itself).
    pub at: Vec<u32>,
    /// The attribute the value goes into.
    pub attribute: ooxml_xml::Name,
}

/// Why a change could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    /// The package refused it.
    #[error(transparent)]
    Package(#[from] OpcError),
    /// A patch did not apply or could not be rebased.
    #[error(transparent)]
    Patch(#[from] PatchError),
    /// A tree change names a part the package does not have.
    #[error("no part {0}")]
    NoPart(String),
    /// The version could not be read as a Word document.
    #[error("{0}")]
    Unreadable(String),
}

impl Change {
    /// The part it changes, if it changes one.
    #[must_use]
    pub fn part(&self) -> Option<&str> {
        match self {
            Change::Splice { part, .. }
            | Change::Head { part, .. }
            | Change::Put { part, .. }
            | Change::Bytes { part, .. }
            | Change::Remove { part } => Some(part),
            Change::Restore { .. }
            | Change::Ids(_)
            | Change::Draw { .. }
            | Change::Number
            | Change::People { .. }
            | Change::Spaced(_) => None,
        }
    }
}
