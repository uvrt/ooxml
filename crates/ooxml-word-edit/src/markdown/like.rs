//! Owns: the looks Markdown list items take, by level (docx-agent's `edit/like.py`
//! `LevelLooks`): a paragraph given for a level -- every list's items at that level, or one
//! kind's alone (a style map's `bullet2`) -- and, for a level below the given ones, the
//! nearest given level's list a level deeper per level, as Word demotes an item
//! ([`ParagraphLook::deeper`]) -- in a one-level list, the item moved as Word's list indent
//! moves it ([`ParagraphLook::indented`]); what a sample cannot give (no list, the other
//! kind, no such level) said once.
//!
//! Must not: write (the writer takes the look: [`super::writer`]), or read the samples (the
//! caller gives their looks, read on the version the edit is applied to).

use crate::format::list_indent::ListIndent;
use crate::format::ParagraphLook;
use ooxml_word_model::docx::numbering::Lists;
use ooxml_word_model::docx::Docx;
use std::collections::HashSet;

/// A Markdown list's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListKind {
    /// A bulleted list.
    Bullet,
    /// A numbered list.
    Number,
}

impl ListKind {
    /// The kind as results say it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ListKind::Bullet => "bullet",
            ListKind::Number => "number",
        }
    }
}

/// The look list items at a level take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelLook {
    /// The items of this kind alone (`None`: every list's).
    pub kind: Option<ListKind>,
    /// The Markdown level, 1 the top.
    pub level: i64,
    /// The sample's look.
    pub look: ParagraphLook,
}

/// What was said of a sample, once each (docx-agent's `_tell` keys).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Told {
    /// The sample is in no list.
    NoList(String),
    /// The sample's list is of the other kind.
    OtherKind(String),
    /// The sample's list has no level this deep.
    TooDeep(String, i64),
}

/// The looks of one insertion's list items, and what was said of them.
#[derive(Debug, Default)]
pub(crate) struct Levels {
    given: Vec<LevelLook>,
    told: HashSet<Told>,
    /// The position of the paragraph after the new items, whose first line and text are
    /// stops where an item moves in a one-level list; none where a table or nothing
    /// follows.
    next: Option<ListIndent>,
}

impl Levels {
    pub(crate) fn new(given: &[LevelLook]) -> Levels {
        Levels {
            given: given.to_vec(),
            told: HashSet::new(),
            next: None,
        }
    }

    /// The paragraph after the new items stands at `next`.
    pub(crate) fn followed_at(&mut self, next: Option<ListIndent>) {
        self.next = next;
    }

    /// Whether any level is given a look.
    pub(crate) fn any(&self) -> bool {
        !self.given.is_empty()
    }

    /// The look of `kind` items at Markdown `depth` (1 the top), and what is newly said of
    /// it: the level's own look (a kind's before every list's), else the nearest given
    /// level above's a level deeper per level; none above every given level.
    pub(crate) fn at(&mut self, docx: &Docx, kind: ListKind, depth: i64) -> (Option<ParagraphLook>, Vec<String>) {
        let mut said = Vec::new();
        for level in (1..=depth).rev() {
            let Some(look) = self.given_at(kind, level).cloned() else {
                continue;
            };
            let lists = Lists::read(docx);
            if level == depth {
                self.check(docx, &lists, &look, kind, depth, &mut said);
                return (Some(look), said);
            }
            let deeper = look
                .deeper(docx, &lists, depth - level)
                .or_else(|| look.indented(docx, &lists, depth - level, self.next));
            if deeper.is_none() {
                let sample = &look.sample;
                self.tell(
                    Told::TooDeep(sample.clone(), depth),
                    format!(
                        "like: level {depth} has no paragraph given, and {sample}'s list has no level {} below \
                         {sample}'s: written as without like",
                        depth - level
                    ),
                    &mut said,
                );
            }
            return (deeper, said);
        }
        (None, said)
    }

    fn given_at(&self, kind: ListKind, level: i64) -> Option<&ParagraphLook> {
        let find = |wanted: Option<ListKind>| {
            self.given
                .iter()
                .rev()
                .find(|g| g.kind == wanted && g.level == level)
                .map(|g| &g.look)
        };
        find(Some(kind)).or_else(|| find(None))
    }

    /// Say where the sample is in no list, or in one of the other kind.
    fn check(
        &mut self,
        docx: &Docx,
        lists: &Lists,
        look: &ParagraphLook,
        kind: ListKind,
        depth: i64,
        said: &mut Vec<String>,
    ) {
        let sample = &look.sample;
        match look.membership(docx, lists) {
            None => self.tell(
                Told::NoList(sample.clone()),
                format!("like: {sample} is in no list: the level {depth} items are written like it, with no marker"),
                said,
            ),
            Some(found) => {
                let theirs = if found.format.as_deref() == Some("bullet") {
                    ListKind::Bullet
                } else {
                    ListKind::Number
                };
                if theirs != kind {
                    self.tell(
                        Told::OtherKind(sample.clone()),
                        format!(
                            "like: the Markdown's level {depth} items are {}s and {sample} is a {} item: they \
                             follow {sample}",
                            kind.as_str(),
                            theirs.as_str()
                        ),
                        said,
                    );
                }
            }
        }
    }

    fn tell(&mut self, key: Told, text: String, said: &mut Vec<String>) {
        if self.told.insert(key) {
            said.push(text);
        }
    }
}
