//! Owns: [`BlockId`], a block's id as written (`p:1A2B3C4D`, `t:00000001`, `tr:0A0B0C0D`,
//! `header1/p:1A2B3C4D`; a table without rows `t@body/2`), read into its story
//! ([`StoryName`]), kind ([`BlockKind`]) and name ([`BlockName`]); and [`Renames`], the ids
//! an edit renamed, old to new.
//!
//! Every paragraph and row has a paraId of its own from the document's open (stamp on open):
//! the names by place the index gave before (`p@body/3`, `p:1A#1`, `t:1A/r2`) are no block
//! ids (the address resolver refuses them with guidance).
//!
//! An id keeps its text exactly as the index or an edit wrote it: [`BlockId`]'s `Display`
//! gives that text back, and two ids are equal (and hash alike) when their texts are, so a
//! lower-case or short paraId stays the id it was written as (phase 3 design, decision 1).
//!
//! Must not: resolve an id (the part's index does) or decide what an address means (the
//! address module does).

use super::{hex_value, valid_long_hex, PARA_ID_LIMIT};
use std::num::NonZeroU32;
use std::sync::{Arc, OnceLock};

/// A paraId Word keeps: above 0, below [`PARA_ID_LIMIT`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ParaId(NonZeroU32);

impl ParaId {
    /// `value` as a paraId, where it is one.
    #[must_use]
    pub fn new(value: u32) -> Option<ParaId> {
        NonZeroU32::new(value).filter(|v| v.get() < PARA_ID_LIMIT).map(ParaId)
    }

    /// The paraId written `raw` (one to eight hex digits, in any case), where it is one.
    #[must_use]
    pub fn parse(raw: &str) -> Option<ParaId> {
        if !valid_long_hex(raw) {
            return None;
        }
        hex_value(raw).and_then(ParaId::new)
    }

    /// Its value.
    #[must_use]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl std::fmt::Display for ParaId {
    /// Eight upper-case hex digits, as Word writes it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:08X}", self.0.get())
    }
}

/// The story an id is in: `body`, or a story part's stem (`header2`, `footnotes`,
/// `comments`).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoryName(Arc<str>);

impl StoryName {
    /// The main part's story.
    #[must_use]
    pub fn body() -> StoryName {
        static BODY: OnceLock<Arc<str>> = OnceLock::new();
        StoryName(Arc::clone(BODY.get_or_init(|| Arc::from("body"))))
    }

    /// The story `name` (`body` shares one allocation).
    #[must_use]
    pub fn new(name: &str) -> StoryName {
        if name == "body" {
            return StoryName::body();
        }
        StoryName(Arc::from(name))
    }

    /// Its name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether it is the main part's story.
    #[must_use]
    pub fn is_body(&self) -> bool {
        &*self.0 == "body"
    }

    /// Whether `name` can name a story in an id: not empty, and none of the characters an
    /// id's form is read by (`/ : @ #`).
    fn valid(name: &str) -> bool {
        !name.is_empty() && !name.contains(['/', ':', '@', '#'])
    }
}

impl std::fmt::Display for StoryName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Debug for StoryName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&*self.0, f)
    }
}

/// What a block is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum BlockKind {
    /// A paragraph (`p:`).
    Paragraph,
    /// A table (`t:`, `t@` without rows).
    Table,
    /// A table row (`tr:`).
    Row,
}

/// How a block is named within its story.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BlockName {
    /// By a valid paraId (`p:1A2B3C4D`; a table by its first row's, `t:`; a row by its own,
    /// `tr:`): durable, kept across edits and saves.
    Durable(ParaId),
    /// A table without rows, by its ordinal among the story's tables (`t@body/2`): no row
    /// gives it a paraId, and Word drops such a table on save (decision 7).
    Rowless(u32),
}

impl BlockName {
    /// Whether the name stays the block's across edits (a paraId of its own).
    #[must_use]
    pub fn durable(self) -> bool {
        matches!(self, BlockName::Durable(_))
    }
}

/// What an id is read into.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Form {
    story: StoryName,
    kind: BlockKind,
    name: BlockName,
}

/// Text that is not a block id ([`BlockId::parse`]).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a block id: {0:?}")]
pub struct NotABlockId(pub String);

/// A paragraph's, table's or row's id, exactly as written, with its story, kind and name.
///
/// Equal, ordered and hashed by its text. An id made from text that is not a block id
/// ([`BlockId::new`], the `From` conversions) keeps that text and has no kind, story or
/// name: only [`BlockId::parse`] refuses it.
#[derive(Clone, Default)]
pub struct BlockId {
    text: Arc<str>,
    form: Option<Form>,
}

/// The ids an edit renamed, old to new, in the order made.
pub type Renames = Vec<(BlockId, BlockId)>;

impl BlockId {
    /// The id written `id`, read where it is a block id (kept as text where it is not).
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        let text: Arc<str> = id.into();
        let form = read(&text);
        Self { text, form }
    }

    /// The block id written `text`, or why it is not one. `parse(x).to_string() == x`.
    ///
    /// # Errors
    /// [`NotABlockId`] where `text` is not a form the index names blocks by.
    pub fn parse(text: &str) -> Result<BlockId, NotABlockId> {
        match read(text) {
            Some(form) => Ok(BlockId {
                text: Arc::from(text),
                form: Some(form),
            }),
            None => Err(NotABlockId(text.to_owned())),
        }
    }

    /// The id of the block named `name` of kind `kind` in `story`, written as the index
    /// writes it, a paraId as eight upper-case hex digits; `None` for a name that does not
    /// fit the kind.
    #[must_use]
    pub fn of(story: &StoryName, kind: BlockKind, name: BlockName) -> Option<BlockId> {
        let prefix = if story.is_body() {
            String::new()
        } else {
            format!("{story}/")
        };
        let text = match (kind, name) {
            (BlockKind::Paragraph, BlockName::Durable(id)) => format!("{prefix}p:{id}"),
            (BlockKind::Table, BlockName::Durable(id)) => format!("{prefix}t:{id}"),
            (BlockKind::Table, BlockName::Rowless(n)) => format!("t@{story}/{n}"),
            (BlockKind::Row, BlockName::Durable(id)) => format!("{prefix}tr:{id}"),
            _ => return None,
        };
        Some(BlockId::new(text))
    }

    /// The id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The id, shared.
    #[must_use]
    pub fn as_arc(&self) -> &Arc<str> {
        &self.text
    }

    /// What the id names: a paragraph, a table or a row; `None` for text that is not a block
    /// id.
    #[must_use]
    pub fn kind(&self) -> Option<BlockKind> {
        self.form.as_ref().map(|f| f.kind)
    }

    /// The story the id is in (`body` where it names none); `None` for text that is not a
    /// block id.
    #[must_use]
    pub fn story(&self) -> Option<&StoryName> {
        self.form.as_ref().map(|f| &f.story)
    }

    /// How the id names its block within its story; `None` for text that is not a block id.
    #[must_use]
    pub fn name(&self) -> Option<BlockName> {
        self.form.as_ref().map(|f| f.name)
    }

    /// The paraId a durable id names its block by (a table's: its first row's); `None` for a
    /// rowless table's, or text that is not a block id.
    #[must_use]
    pub fn para_id(&self) -> Option<ParaId> {
        match self.name()? {
            BlockName::Durable(id) => Some(id),
            BlockName::Rowless(_) => None,
        }
    }
}

/// `text` read as a block id.
fn read(text: &str) -> Option<Form> {
    if let Some(rest) = text.strip_prefix("t@") {
        let (story, n) = rest.split_once('/')?;
        return Some(Form {
            story: story_of(story)?,
            kind: BlockKind::Table,
            name: BlockName::Rowless(number(n)?),
        });
    }
    let (head, value) = text.split_once(':')?;
    let (story, tag) = match head.split_once('/') {
        Some((story, tag)) if story != "body" => (story_of(story)?, tag),
        Some(_) => return None,
        None => (StoryName::body(), head),
    };
    let kind = match tag {
        "p" => BlockKind::Paragraph,
        "t" => BlockKind::Table,
        "tr" => BlockKind::Row,
        _ => return None,
    };
    Some(Form {
        story,
        kind,
        name: BlockName::Durable(ParaId::parse(value)?),
    })
}

fn story_of(name: &str) -> Option<StoryName> {
    StoryName::valid(name).then(|| StoryName::new(name))
}

/// A number written as the index writes one: digits, no sign, no leading zero.
fn number(text: &str) -> Option<u32> {
    let plain = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) && (text == "0" || !text.starts_with('0'));
    plain.then(|| text.parse().ok()).flatten()
}

impl PartialEq for BlockId {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for BlockId {}

impl PartialOrd for BlockId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BlockId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.text.cmp(&other.text)
    }
}

impl std::hash::Hash for BlockId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // as `str` hashes: `Borrow<str>` lookups find it
        self.text.hash(state);
    }
}

impl std::ops::Deref for BlockId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

impl AsRef<str> for BlockId {
    fn as_ref(&self) -> &str {
        &self.text
    }
}

impl std::borrow::Borrow<str> for BlockId {
    fn borrow(&self) -> &str {
        &self.text
    }
}

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::fmt::Debug for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&*self.text, f)
    }
}

impl serde::Serialize for BlockId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl From<&str> for BlockId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl From<String> for BlockId {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl From<&String> for BlockId {
    fn from(id: &String) -> Self {
        Self::new(id.as_str())
    }
}

impl From<Arc<str>> for BlockId {
    fn from(id: Arc<str>) -> Self {
        Self::new(id)
    }
}

impl From<BlockId> for String {
    fn from(id: BlockId) -> Self {
        id.text.to_string()
    }
}

impl PartialEq<str> for BlockId {
    fn eq(&self, other: &str) -> bool {
        *self.text == *other
    }
}

impl PartialEq<&str> for BlockId {
    fn eq(&self, other: &&str) -> bool {
        *self.text == **other
    }
}

impl PartialEq<String> for BlockId {
    fn eq(&self, other: &String) -> bool {
        *self.text == **other
    }
}

impl PartialEq<BlockId> for str {
    fn eq(&self, other: &BlockId) -> bool {
        *self == *other.text
    }
}

impl PartialEq<BlockId> for &str {
    fn eq(&self, other: &BlockId) -> bool {
        **self == *other.text
    }
}

impl PartialEq<BlockId> for String {
    fn eq(&self, other: &BlockId) -> bool {
        **self == *other.text
    }
}

/// `renames` as strings, for a reply.
#[must_use]
pub fn renames_text(renames: &[(BlockId, BlockId)]) -> Vec<(String, String)> {
    renames
        .iter()
        .map(|(old, new)| (old.to_string(), new.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn every_form_reads_into_its_parts_and_writes_back_as_written() {
        let id = |t: &str| BlockId::parse(t).unwrap();
        let p = ParaId::new(0x1A2B_3C4D).unwrap();
        let short = ParaId::new(0x1A).unwrap();
        let cases: Vec<(&str, &str, BlockKind, BlockName)> = vec![
            ("p:1A2B3C4D", "body", BlockKind::Paragraph, BlockName::Durable(p)),
            ("p:1a2b3c4d", "body", BlockKind::Paragraph, BlockName::Durable(p)),
            ("p:1A", "body", BlockKind::Paragraph, BlockName::Durable(short)),
            (
                "header2/p:1A2B3C4D",
                "header2",
                BlockKind::Paragraph,
                BlockName::Durable(p),
            ),
            ("t:1A2B3C4D", "body", BlockKind::Table, BlockName::Durable(p)),
            ("t@body/0", "body", BlockKind::Table, BlockName::Rowless(0)),
            ("t@header1/3", "header1", BlockKind::Table, BlockName::Rowless(3)),
            ("tr:1A2B3C4D", "body", BlockKind::Row, BlockName::Durable(p)),
        ];
        for (text, story, kind, name) in &cases {
            let parsed = id(text);
            assert_eq!(parsed.to_string(), *text);
            assert_eq!(parsed.story().map(StoryName::as_str), Some(*story), "{text}");
            assert_eq!(parsed.kind(), Some(*kind), "{text}");
            assert_eq!(parsed.name(), Some(*name), "{text}");
            assert_eq!(parsed, BlockId::new(*text), "{text}");
        }
        assert_eq!(id("t:1A2B3C4D").para_id(), Some(p));
        assert_eq!(id("t@body/0").para_id(), None);
        // the parts written back: the index's spelling (eight upper-case digits)
        let lower = id("header2/p:1a2b3c4d");
        let made = BlockId::of(lower.story().unwrap(), BlockKind::Paragraph, lower.name().unwrap()).unwrap();
        assert_eq!(made.as_str(), "header2/p:1A2B3C4D");
        assert_ne!(made, lower, "equal by text");
    }

    #[test]
    fn text_that_is_no_block_id_is_refused_and_kept_as_text_by_new() {
        for text in [
            "",
            "bm:x",
            "p",
            "P:1A2B3C4D",
            "body/p:1A2B3C4D",
            "x/y/p:1A2B3C4D",
            "tr:ZZ",
            "tr:1A#1",
            "t:ZZ",
            "p:ZZ",
            "p:80000000",
            "p:",
            // names by place, which the index no longer gives
            "p@body/3",
            "p@footnotes/12",
            "p:1A2B3C4D#2",
            "t:1A2B3C4D/r2",
            "t@body/1/r1",
            "t@body",
            "t@body/01",
            "t@/1",
            "q:1A2B3C4D",
        ] {
            assert!(BlockId::parse(text).is_err(), "{text}");
            let kept = BlockId::new(text);
            assert_eq!(kept.as_str(), text);
            assert_eq!(kept.kind(), None, "{text}");
        }
    }
}
