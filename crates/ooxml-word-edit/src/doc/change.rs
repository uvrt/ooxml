//! Owns: what a transaction reads and a change writes ([`Key`]) and the primitives of a
//! change set ([`Change`]): tree patches per part, parts put or removed, ledger additions,
//! ids drawn where the change lands ([`Draw`]), the count of spaces the edit's language
//! spacing wrote; why one could not be applied ([`ModelError`]).
//!
//! Must not: apply a change (the model does, [`super::model`]) or decide one (commands do,
//! through the sink).

use crate::pool::IdsChange;
use ooxml_opc::{OpcError, Package, PartName};
use ooxml_word_model::ids::BlockId;
use ooxml_xml::{HeadPatch, Patch, PatchError, Tree};
use std::sync::Arc;

/// What a transaction reads and a change writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// A paragraph, table or row of a story part, by its durable id (`p:1A2B3C4D`).
    Block {
        /// The part.
        part: PartName,
        /// The block's id.
        id: BlockId,
    },
    /// A whole part (a part without durable ids, a part put or removed).
    Part(PartName),
    /// A part's root element's head: its namespace declarations and attributes.
    Root(PartName),
    /// An id sequence (a seed): drawing from it.
    Seq(Arc<str>),
    /// The annotation ids in use (revisions, bookmarks, comments): allocating one.
    Annotations,
    /// Who the people part lists.
    People,
    /// What a part's paragraphs show a reader of their style uses: each paragraph with text
    /// or in a list, in order, with its paragraph style, its list and whether its text is
    /// blank (the body style a copy maps to, the list a copied list continues). Written
    /// where a change alters one (`sink::style_use`), and with the part put or removed. It is
    /// derived ([`ooxml_store::DocModel::derived`]): a reader of it conflicts with a writer,
    /// two writers of it do not (their blocks decide).
    StyleUse(PartName),
}

impl Key {
    /// The key of block `id` of `part`.
    #[must_use]
    pub fn block(part: impl Into<PartName>, id: impl Into<BlockId>) -> Self {
        Key::Block {
            part: part.into(),
            id: id.into(),
        }
    }

    /// The key of the whole part `part`.
    #[must_use]
    pub fn part(part: impl Into<PartName>) -> Self {
        Key::Part(part.into())
    }
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
    /// The namespaces Word 2010's ids need declared on a part's root and listed in
    /// `mc:Ignorable` (docx-agent's `ensure_w14`: `w14`, `wp14` with `drawings`, each of
    /// `extra`), decided where the change is applied: only what the root lacks there and the
    /// tree uses is added (and repeats below then cleaned away). An append: two writers of it
    /// do not conflict, and no undo takes a declaration back.
    Declare {
        /// The part.
        part: Arc<str>,
        /// Whether `wp14` is wanted too.
        drawings: bool,
        /// Further namespaces wanted, by prefix and URI.
        extra: Arc<[(Arc<str>, Arc<str>)]>,
    },
    /// A part's root head as another editor's version has it, merged where the change lands
    /// (a co-authoring refresh): their namespace declarations added where the root there
    /// lacks them, their attributes by name (theirs over ours), `mc:Ignorable` the union of
    /// both lists. Never a child's place.
    RootHead {
        /// The part.
        part: Arc<str>,
        /// Their root's head.
        head: Arc<ooxml_xml::Head>,
    },
    /// A part as another editor's version of the document has it (a co-authoring refresh,
    /// [`crate::Arrival`]): `patches`, the changes from the version that editor's save was
    /// made from, applied in place (the blocks they do not touch keep their ids and what is
    /// kept of them); `None`, the part whole as `from` has it. Where the part it lands on is
    /// `on` (the very part the patches were worked out against), the result is the part as
    /// `from` holds it, so it saves as that editor wrote it; elsewhere it is written. A block
    /// left without a valid, unique id where it lands is stamped there, by the rule of a part
    /// another writer put ([`ooxml_word_model::stamp::received`]).
    Arrived {
        /// The part.
        part: Arc<str>,
        /// Its changes, or `None` for the part whole.
        patches: Option<Arc<[Patch]>>,
        /// The keys it writes.
        keys: Arc<[Key]>,
        /// The other editor's version.
        from: Package,
        /// The part the patches were worked out against, where it had one.
        on: Option<ooxml_opc::PartVersion>,
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
    /// A textId renewed: the next value of `seed`, unless the block's id is unwritten where
    /// the change is applied ([`super::WordDoc::para_id_written`]: `part` read with ids
    /// given at open and not yet written in the version the change's transaction begins on,
    /// and `para_id` not one the saved part holds) -- an unwritten id counts as fresh at its
    /// block's first write, so it keeps the value the element it replaces holds there (the
    /// attribute left as it is where that element has none). Decided wholly where applied,
    /// never where prepared: a first edit committed past another renews as a serial run
    /// would, and so does an edit prepared on a written part committed past an undo that
    /// took the part back to its ids as given at open.
    Renewed {
        /// The sequence.
        seed: Arc<str>,
        /// The part the block is in.
        part: Arc<str>,
        /// The block's paraId (8 hex digits), as given at open where the part is unwritten.
        para_id: Arc<str>,
    },
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
    /// The document is a strict Open XML document, which no change is written into
    /// (`WordDoc::strict`).
    #[error("a strict Open XML document is not changed here: save it as a Word Document (.docx) in Word first")]
    Strict,
}

impl Change {
    /// The part it changes, if it changes one.
    #[must_use]
    pub fn part(&self) -> Option<&str> {
        match self {
            Change::Splice { part, .. }
            | Change::Head { part, .. }
            | Change::Put { part, .. }
            | Change::Arrived { part, .. }
            | Change::Declare { part, .. }
            | Change::RootHead { part, .. }
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
