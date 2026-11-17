//! Owns: [`Counts`], the whole-document facts validation's rules count, as counted
//! multisets in persistent maps (design section 7, "whole-document facts are counted
//! multisets updated from change sets"): a block's facts are added when it appears and
//! taken out when it goes. Where a rule asks which values break it (a paraId repeated, a
//! bookmark unpaired, a reference without its note), the answer is kept as a set updated
//! when a count crosses the line, so a version's problems cost the problems, not the
//! document. And [`Role`], what a part's facts count towards.
//!
//! Must not: read XML (block facts are given), or decide a problem's wording (the kept
//! rules do).

use super::facts::{ElementFacts, NumRef};
use ooxml_derive::PersistentMap;
use ooxml_opc::CONTENT_TYPES_PART;
use std::sync::Arc;

/// What a part's facts count towards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Role {
    /// A `word/*.xml` part: its own rules (local problems, paraIds, fields).
    pub(crate) word: bool,
    /// The main part or a story part.
    pub(crate) story: bool,
    /// The main part.
    pub(crate) main: bool,
    /// The styles part (its list references count).
    pub(crate) styles: bool,
    /// A notes part (not the main one) whose root is `w:comments`, `w:footnotes` or
    /// `w:endnotes`: the notes it holds are reference targets.
    pub(crate) notes: Option<Notes>,
    /// An XML part other than the content types: its relationship ids are checked.
    pub(crate) references: bool,
}

/// A kind of note, and of reference to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Notes {
    Comments,
    Footnotes,
    Endnotes,
}

impl Role {
    /// The role of the part `name`, whose root is `root_local` (`w:` local name).
    pub(crate) fn of(name: &str, main: bool, story: bool, styles: bool, root_local: Option<&str>) -> Role {
        let notes = match root_local {
            Some("comments") if story && !main => Some(Notes::Comments),
            Some("footnotes") if story && !main => Some(Notes::Footnotes),
            Some("endnotes") if story && !main => Some(Notes::Endnotes),
            _ => None,
        };
        Role {
            word: name.starts_with("word/") && name.ends_with(".xml"),
            story,
            main,
            styles,
            notes,
            references: name.ends_with(".xml") && name != CONTENT_TYPES_PART,
        }
    }
}

type Part = Arc<str>;
type Text = Arc<str>;
type Id = Option<Arc<str>>;

/// The counted facts of a version.
#[derive(Debug, Clone, Default)]
pub(crate) struct Counts {
    /// Problems a block holds alone (word parts), counted.
    pub(crate) locals: PersistentMap<(Part, &'static str, Text), usize>,
    /// Live paraIds (upper-cased) per word part, and those repeated.
    paraids: PersistentMap<(Part, Text), usize>,
    pub(crate) paraids_repeated: PersistentMap<(Part, Text), ()>,
    /// Bookmark starts and ends by id, and the ids whose counts differ.
    bookmarks: PersistentMap<Text, (usize, usize)>,
    pub(crate) bookmarks_unpaired: PersistentMap<Text, ()>,
    /// Bookmark names, those invalid, and names repeated in any case.
    bookmark_names: PersistentMap<Text, usize>,
    pub(crate) bookmark_names_invalid: PersistentMap<Text, ()>,
    bookmark_folded: PersistentMap<Text, usize>,
    pub(crate) bookmark_folded_repeated: PersistentMap<Text, ()>,
    /// Comment range starts and ends, and the ids with one and not the other.
    comment_ranges: PersistentMap<Id, (usize, usize)>,
    pub(crate) comment_ranges_unpaired: PersistentMap<Id, ()>,
    /// References and the notes they name, and references without one.
    references: PersistentMap<(Notes, Id), usize>,
    targets: PersistentMap<(Notes, Id), usize>,
    pub(crate) references_missing: PersistentMap<(Notes, Id), ()>,
    /// Relationship-namespace attribute values per part.
    pub(crate) r_values: PersistentMap<(Part, Text), usize>,
    /// List references per story part (and the styles part).
    pub(crate) num_refs: PersistentMap<(Part, NumRef), usize>,
    /// Style references per story part.
    pub(crate) style_refs: PersistentMap<(Part, &'static str, Id), usize>,
    /// Drawing numbers across stories, and those repeated.
    docpr: PersistentMap<Text, usize>,
    pub(crate) docpr_repeated: PersistentMap<Text, ()>,
    /// Revision records without an author, per story part and kind.
    pub(crate) without_author: PersistentMap<(Part, &'static str), usize>,
    /// Revision ids across stories, and those repeated.
    record_ids: PersistentMap<Text, usize>,
    pub(crate) record_ids_repeated: PersistentMap<Text, ()>,
    /// Move range marks per story part.
    pub(crate) moves: PersistentMap<Part, usize>,
    /// Authors of revisions and comments across stories.
    pub(crate) authors: PersistentMap<Text, usize>,
    /// Comment references across stories (by id, for the comments part's rule).
    pub(crate) comment_references: PersistentMap<Id, usize>,
}

/// Add `sign` (1 or -1) times to `key`'s count; the count before and after.
fn bump<K: std::hash::Hash + Eq + Clone>(map: &mut PersistentMap<K, usize>, key: K, sign: i64) -> (usize, usize) {
    let before = map.get(&key).copied().unwrap_or(0);
    let after = if sign >= 0 {
        before + 1
    } else {
        before.saturating_sub(1)
    };
    if after == 0 {
        map.remove(&key);
    } else {
        map.insert(key, after);
    }
    (before, after)
}

/// Keep `key` in `set` exactly when `inside`.
fn mark<K: std::hash::Hash + Eq + Clone>(set: &mut PersistentMap<K, ()>, key: K, inside: bool) {
    if inside {
        if !set.contains_key(&key) {
            set.insert(key, ());
        }
    } else {
        set.remove(&key);
    }
}

fn text(value: &str) -> Text {
    Arc::from(value)
}

fn id(value: Option<&String>) -> Id {
    value.map(|v| Arc::from(v.as_str()))
}

impl Counts {
    /// Count `facts` (a block of the part `part`, of role `role`) in, or out (`sign` -1).
    pub(crate) fn apply(&mut self, part: &Part, role: &Role, facts: &ElementFacts, sign: i64) {
        if role.word {
            self.word(part, facts, sign);
        }
        if role.references {
            for value in &facts.r_values {
                bump(&mut self.r_values, (Arc::clone(part), text(value)), sign);
            }
        }
        if role.story || role.styles {
            for num in &facts.num_refs {
                bump(&mut self.num_refs, (Arc::clone(part), num.clone()), sign);
            }
        }
        if role.story {
            self.story(part, facts, sign);
        }
        if let Some(kind) = role.notes {
            let ids = match kind {
                Notes::Comments => &facts.comments,
                Notes::Footnotes => &facts.footnotes,
                Notes::Endnotes => &facts.endnotes,
            };
            for value in ids {
                let key = (kind, id(value.as_ref()));
                let (_, now) = bump(&mut self.targets, key.clone(), sign);
                let referenced = self.references.get(&key).is_some_and(|n| *n > 0);
                mark(&mut self.references_missing, key, referenced && now == 0);
            }
        }
    }

    fn word(&mut self, part: &Part, facts: &ElementFacts, sign: i64) {
        for (code, detail) in facts
            .order
            .iter()
            .chain(&facts.paraid)
            .chain(&facts.revision)
            .chain(&facts.cells)
            .chain(&facts.grids)
        {
            bump(&mut self.locals, (Arc::clone(part), *code, text(detail)), sign);
        }
        for raw in &facts.para_ids {
            let key = (Arc::clone(part), text(raw));
            let (_, now) = bump(&mut self.paraids, key.clone(), sign);
            mark(&mut self.paraids_repeated, key, now > 1);
        }
    }

    fn story(&mut self, part: &Part, facts: &ElementFacts, sign: i64) {
        self.bookmarks(facts, sign);
        for (starts, value) in facts
            .comment_starts
            .iter()
            .map(|v| (true, v))
            .chain(facts.comment_ends.iter().map(|v| (false, v)))
        {
            let key = id(value.as_ref());
            let (a, b) = self.comment_ranges.get(&key).copied().unwrap_or((0, 0));
            let step = |n: usize| if sign >= 0 { n + 1 } else { n.saturating_sub(1) };
            let (a, b) = if starts { (step(a), b) } else { (a, step(b)) };
            if a == 0 && b == 0 {
                self.comment_ranges.remove(&key);
            } else {
                self.comment_ranges.insert(key.clone(), (a, b));
            }
            mark(&mut self.comment_ranges_unpaired, key, (a > 0) != (b > 0));
        }
        for (kind, values) in [
            (Notes::Comments, &facts.comment_refs),
            (Notes::Footnotes, &facts.footnote_refs),
            (Notes::Endnotes, &facts.endnote_refs),
        ] {
            for value in values {
                let key = (kind, id(value.as_ref()));
                let (_, now) = bump(&mut self.references, key.clone(), sign);
                let targeted = self.targets.get(&key).is_some_and(|n| *n > 0);
                mark(&mut self.references_missing, key, now > 0 && !targeted);
                if kind == Notes::Comments {
                    bump(&mut self.comment_references, id(value.as_ref()), sign);
                }
            }
        }
        for (tag, value) in &facts.style_refs {
            bump(&mut self.style_refs, (Arc::clone(part), *tag, id(value.as_ref())), sign);
        }
        for value in &facts.docpr_ids {
            let key = text(value);
            let (_, now) = bump(&mut self.docpr, key.clone(), sign);
            mark(&mut self.docpr_repeated, key, now > 1);
        }
        for (kind, author, value) in &facts.records {
            if !author {
                bump(&mut self.without_author, (Arc::clone(part), *kind), sign);
            }
            if let Some(value) = value {
                let key = text(value);
                let (_, now) = bump(&mut self.record_ids, key.clone(), sign);
                mark(&mut self.record_ids_repeated, key, now > 1);
            }
        }
        for _ in &facts.moves {
            bump(&mut self.moves, Arc::clone(part), sign);
        }
        for author in &facts.authors {
            bump(&mut self.authors, text(author), sign);
        }
    }

    fn bookmarks(&mut self, facts: &ElementFacts, sign: i64) {
        for (starts, value) in facts
            .bookmark_starts
            .iter()
            .map(|v| (true, v))
            .chain(facts.bookmark_ends.iter().map(|v| (false, v)))
        {
            let key = text(value);
            let (a, b) = self.bookmarks.get(&key).copied().unwrap_or((0, 0));
            let step = |n: usize| if sign >= 0 { n + 1 } else { n.saturating_sub(1) };
            let (a, b) = if starts { (step(a), b) } else { (a, step(b)) };
            if a == 0 && b == 0 {
                self.bookmarks.remove(&key);
            } else {
                self.bookmarks.insert(key.clone(), (a, b));
            }
            mark(&mut self.bookmarks_unpaired, key, a != b);
        }
        for name in &facts.bookmark_names {
            let key = text(name);
            let (_, now) = bump(&mut self.bookmark_names, key.clone(), sign);
            mark(&mut self.bookmark_names_invalid, key, now > 0 && !super::kept::bookmark_name_ok(name));
            let folded = text(&ooxml_word_model::docx::casefold(name));
            let (_, now) = bump(&mut self.bookmark_folded, folded.clone(), sign);
            mark(&mut self.bookmark_folded_repeated, folded, now > 1);
        }
    }

    /// Whether `author` made a revision or a comment that is still there.
    pub(crate) fn author_in_use(&self, author: &str) -> bool {
        self.authors.get(author).is_some_and(|n| *n > 0)
    }
}
