//! Owns: where a find matches a version's text, as docx-agent's `_matches` finds it --
//! every story's paragraphs in order, each paragraph's current text alone or (across
//! paragraphs) a story's paragraphs joined by `\n`, matches left to right and not
//! overlapping, empty ones left out -- and the [`Pattern`] a find is made with: plain text
//! ([`Literal`]: case-sensitive or not, white space collapsed or not, as `re.escape`d text
//! matches) or one the caller supplies (a Python regular expression, matched by the tool
//! layer's translation: this crate holds no regex engine); and `HoldsMatch`, a find's read
//! by condition (the blocks whose text it matches), checked at commit.
//!
//! Must not: change anything, or answer a pattern differently from Python's `re`.

use crate::doc::{Key, WordDoc};
use crate::text::diff::python_space;
use crate::{EditError, Result};
use ooxml_store::ReadPredicate;
use ooxml_word_model::address::scope_paragraphs;
use ooxml_word_model::address::Address;
use ooxml_word_model::docx::text::{paragraph_text, View};
use ooxml_word_model::docx::{Docx, StoryPart};
use ooxml_word_model::ids::Entry;
use std::fmt::Debug;
use std::sync::Arc;

/// A find: where it matches a text, and what replaces each match.
pub trait Pattern: Debug + Send + Sync {
    /// Each match's start and end (in characters, left to right, not overlapping, none
    /// empty) and its replacement (a regular expression's template expanded).
    ///
    /// # Errors
    /// A message when the pattern cannot be matched as Python would.
    fn matches(&self, text: &str) -> std::result::Result<Vec<(usize, usize, String)>, String>;

    /// Whether `text` may hold a match: `false` only where it surely holds none (a quick
    /// test a find runs before [`Pattern::matches`]).
    fn may_match(&self, text: &str) -> bool {
        let _ = text;
        true
    }
}

/// Plain text, as `re.escape(text)` (or, with white space collapsed, its words joined by
/// `\s+`) matches it, replaced by `with`.
#[derive(Debug, Clone)]
pub struct Literal {
    words: Vec<Vec<char>>,
    case: bool,
    with: String,
    /// The first word as text, where case matters: every match holds it as it is.
    first: Option<String>,
}

impl Literal {
    /// The find of `text` (`case`: case matters; `collapse`: any run of white space
    /// matches any other), replaced by `with`.
    ///
    /// # Errors
    /// `nothing to find` for empty text (or, collapsed, text of white space only).
    pub fn new(text: &str, case: bool, collapse: bool, with: &str) -> Result<Literal> {
        let words: Vec<Vec<char>> = if collapse {
            split_python(text).into_iter().map(|w| w.chars().collect()).collect()
        } else {
            vec![text.chars().collect()]
        };
        if text.is_empty() || words.is_empty() || words.iter().all(Vec::is_empty) {
            return Err(EditError::Refused("nothing to find".to_owned()));
        }
        let first = words
            .first()
            .filter(|_| case)
            .map(|word| word.iter().collect::<String>());
        Ok(Literal {
            words,
            case,
            with: with.to_owned(),
            first,
        })
    }

    fn same(&self, a: char, b: char) -> bool {
        let lower = |c: char| {
            let mut l = c.to_lowercase();
            match (l.next(), l.next()) {
                (Some(one), None) => one,
                _ => c,
            }
        };
        a == b || (!self.case && lower(a) == lower(b))
    }

    /// The end of a match starting at `k`, if one does.
    fn match_at(&self, text: &[char], k: usize) -> Option<usize> {
        let mut at = k;
        for (n, word) in self.words.iter().enumerate() {
            if n > 0 {
                let spaces = text.get(at..)?.iter().take_while(|c| python_space(**c)).count();
                if spaces == 0 {
                    return None;
                }
                at += spaces;
            }
            let here = text.get(at..at + word.len())?;
            if !here.iter().zip(word).all(|(a, b)| self.same(*a, *b)) {
                return None;
            }
            at += word.len();
        }
        Some(at)
    }
}

/// Python's `str.split()`: the words between runs of white space.
fn split_python(text: &str) -> Vec<String> {
    text.split(python_space)
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

impl Pattern for Literal {
    fn may_match(&self, text: &str) -> bool {
        self.first.as_deref().is_none_or(|first| text.contains(first))
    }

    fn matches(&self, text: &str) -> std::result::Result<Vec<(usize, usize, String)>, String> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut k = 0;
        while k < chars.len() {
            match self.match_at(&chars, k) {
                Some(end) if end > k => {
                    out.push((k, end, self.with.clone()));
                    k = end;
                }
                _ => k += 1,
            }
        }
        Ok(out)
    }
}

/// A find within each paragraph, read by condition (design section 5): its outcome depends
/// on the blocks whose current text it matches, and the blocks it matched, which the edit
/// reads by key. A block written since its snapshot, or a story part written whole,
/// conflicts only when a paragraph of it matches on the head; nothing else does. (A find
/// across paragraphs reads the whole document instead: a match may be made by a paragraph
/// beside the one written.)
#[derive(Debug)]
pub(crate) struct HoldsMatch {
    pattern: Arc<dyn Pattern>,
}

impl HoldsMatch {
    /// The read of every paragraph `pattern` matches.
    pub(crate) fn new(pattern: Arc<dyn Pattern>) -> HoldsMatch {
        HoldsMatch { pattern }
    }

    /// Whether `text` holds a match (or the pattern cannot tell).
    fn holds(&self, text: &str) -> bool {
        self.pattern.may_match(text) && self.pattern.matches(text).map_or(true, |found| !found.is_empty())
    }

    /// Whether a paragraph of `story`, written whole, holds a match (reading the part is
    /// what the key costs).
    fn part_holds(&self, story: &StoryPart) -> bool {
        story.texts().iter().any(|text| self.holds(text))
    }

    /// Whether a paragraph of block `id` of `story` holds a match; a block the story does
    /// not hold holds none.
    fn block_holds(&self, story: &StoryPart, id: &str) -> bool {
        let ids = &story.ids;
        match ids.get(id) {
            Some(Entry::Paragraph(k)) => ids
                .paragraphs()
                .get(k)
                .is_some_and(|p| self.holds(&paragraph_text(&p.element, View::Current))),
            Some(Entry::Table(k)) => ids.tables().get(k).is_some_and(|table| {
                ids.paragraphs_within(&table.path)
                    .iter()
                    .any(|p| self.holds(&paragraph_text(&p.element, View::Current)))
            }),
            None => false,
        }
    }
}

impl ReadPredicate<WordDoc> for HoldsMatch {
    fn depends(&self, head: &WordDoc, key: &Key) -> bool {
        let Ok(docx) = head.docx() else { return true };
        match key {
            Key::Block { part, id } => docx.part(part).is_some_and(|story| self.block_holds(story, id)),
            Key::Part(part) => docx.part(part).is_some_and(|story| self.part_holds(story)),
            Key::Root(_) | Key::Seq(_) | Key::Annotations | Key::People | Key::StyleUse(_) => false,
        }
    }
}

/// A match: its first paragraph's id and offset, its last's, and its replacement.
pub(crate) type Found = (String, usize, String, usize, String);

/// Every match of `pattern` in `docx`'s stories, in document order (`across`: in each
/// story's paragraphs joined by `\n`).
pub(crate) fn matches(docx: &Docx, pattern: &dyn Pattern, across: bool) -> Result<Vec<Found>> {
    let refuse = |e: String| EditError::Refused(e);
    let mut out = Vec::new();
    for story in &docx.parts {
        // each paragraph's text as the version keeps it (read once per version)
        let texts = story.texts();
        let id_of = |k: usize| story.ids.paragraphs().get(k).map(|p| p.id);
        if !across {
            paragraph_matches(story, pattern, 0..texts.len(), &mut out)?;
            continue;
        }
        let mut starts = Vec::with_capacity(texts.len());
        let mut joined = String::new();
        let mut position = 0;
        for (k, text) in texts.iter().enumerate() {
            if k > 0 {
                joined.push('\n');
            }
            starts.push(position);
            position += text.chars().count() + 1;
            joined.push_str(text);
        }
        // the paragraph an offset is in; an end at a paragraph's start is offset 0 there
        let locate = |offset: usize| starts.partition_point(|s| *s <= offset).saturating_sub(1);
        for (a, b, with) in pattern.matches(&joined).map_err(refuse)? {
            let (p, q) = (locate(a), locate(b));
            let (Some(first), Some(last), Some(sp), Some(sq)) = (id_of(p), id_of(q), starts.get(p), starts.get(q))
            else {
                continue;
            };
            out.push((first, a - sp, last, b - sq, with));
        }
    }
    Ok(out)
}

/// Every match of `pattern` in the paragraphs of `story` at `places` (each paragraph's
/// text alone), in order, added to `out`.
fn paragraph_matches(
    story: &StoryPart,
    pattern: &dyn Pattern,
    places: impl IntoIterator<Item = usize>,
    out: &mut Vec<Found>,
) -> Result<()> {
    let texts = story.texts();
    for k in places {
        let Some(text) = texts.get(k).filter(|t| pattern.may_match(t)) else {
            continue;
        };
        let found = pattern.matches(text).map_err(EditError::Refused)?;
        let Some(id) = story.ids.paragraphs().get(k).map(|p| p.id) else {
            continue;
        };
        for (a, b, with) in found {
            out.push((id.clone(), a, id.clone(), b, with));
        }
    }
    Ok(())
}

/// Every match of `pattern` in the paragraphs `within` holds (docx-agent's `_scope`: the
/// whole paragraphs of a story, block, span, cell, section, bookmark or range), each
/// paragraph's text alone, in document order; only those paragraphs are read.
///
/// # Errors
/// `within`'s refusal, or the pattern's.
pub(crate) fn matches_within(docx: &Docx, pattern: &dyn Pattern, within: &str) -> Result<Vec<Found>> {
    let (story, places) = scope_paragraphs(docx, &Address::parse(within)).map_err(EditError::Address)?;
    let mut out = Vec::new();
    paragraph_matches(story, pattern, places, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    //! The paragraph texts a find reads (the version's kept texts, `StoryPart::texts`) equal
    //! the edit core's own walk of each paragraph, on every docx-agent fixture, as opened and
    //! after edits (where the texts are taken from the version before's).
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use crate::text::paragraph_text;
    use crate::work::Work;
    use crate::{DeleteBlock, Editor, InsertParagraph, RevisionPolicy, SetText, Tracking, WordDoc};
    use ooxml_base::Limits;
    use ooxml_derive::CacheBudget;
    use ooxml_opc::Package;
    use ooxml_store::{Caller, Clock, StoreConfig};
    use ooxml_word_model::docx::text::{paragraph_text as model_text, View};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    struct Monotonic(Instant);

    impl Clock for Monotonic {
        fn monotonic(&self) -> Duration {
            self.0.elapsed()
        }
    }

    struct Unlimited;

    impl CacheBudget for Unlimited {
        fn reserve(&self, _bytes: usize) -> bool {
            true
        }
        fn release(&self, _bytes: usize) {}
        fn used(&self) -> usize {
            0
        }
    }

    fn fixtures(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                fixtures(&path, out);
            } else if path.extension().is_some_and(|e| e == "docx") {
                out.push(path);
            }
        }
    }

    fn check(doc: &WordDoc, what: &str) {
        let docx = doc.docx().unwrap();
        for part in &docx.parts {
            let texts = part.texts();
            assert_eq!(texts.len(), part.ids.paragraphs().len(), "{what}: {}", part.part);
            for (k, paragraph) in part.ids.paragraphs().iter().enumerate() {
                let work = Work::of(&paragraph.element, Vec::new());
                let kept = texts.get(k).unwrap();
                assert_eq!(kept, paragraph_text(&work, work.root()), "{what}: {}", paragraph.id);
                assert_eq!(
                    kept,
                    model_text(&paragraph.element, View::Current),
                    "{what}: {}",
                    paragraph.id
                );
            }
        }
    }

    #[test]
    fn kept_texts_equal_each_paragraphs_walk_on_every_fixture() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/ooxml-oracle/fixtures");
        let mut files = Vec::new();
        fixtures(&root, &mut files);
        files.sort();
        assert!(files.len() > 10);
        for file in files {
            let bytes: Arc<[u8]> = Arc::from(std::fs::read(&file).unwrap());
            let package = Package::open(bytes, &Limits::default()).unwrap();
            let Ok(editor) = Editor::open(
                package,
                StoreConfig::default(),
                Arc::new(Monotonic(Instant::now())),
                Arc::new(Unlimited),
            ) else {
                continue;
            };
            let opened = editor.store().snapshot();
            check(opened.doc(), &format!("{}: as opened", file.display()));
            let docx = opened.doc().docx().unwrap();
            let targets: Vec<String> = docx
                .parts
                .iter()
                .filter_map(|p| p.ids.paragraphs().iter().next().map(|q| q.id))
                .collect();
            for (k, target) in targets.iter().enumerate() {
                let policy = if k % 2 == 0 {
                    RevisionPolicy::Tracked(Tracking {
                        author: "Writer A".to_owned(),
                        date: "2026-11-17T10:00:00Z".to_owned(),
                    })
                } else {
                    RevisionPolicy::Untracked
                };
                let command = SetText {
                    target: ooxml_word_model::address::Address::from(target.clone()),
                    text: format!("Edited {k}: a sentence"),
                    policy,
                };
                let Ok(done) = editor.run(&Caller::anonymous(), &command) else {
                    continue;
                };
                check(
                    done.version.doc(),
                    &format!("{}: after editing {target}", file.display()),
                );
            }
        }
    }

    /// Versions made by inserting, deleting and rewriting paragraphs, their texts read out of
    /// order (each from whichever version read last, newer or older), equal each paragraph's
    /// walk: chunks are shared only where every paragraph of them is held in order.
    #[test]
    fn kept_texts_read_out_of_order_equal_each_paragraphs_walk() {
        let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/ooxml-oracle/fixtures/samplelib/sample-long.docx");
        let bytes: Arc<[u8]> = Arc::from(std::fs::read(&file).unwrap());
        let package = Package::open(bytes, &Limits::default()).unwrap();
        let editor = Editor::open(
            package,
            StoreConfig::default(),
            Arc::new(Monotonic(Instant::now())),
            Arc::new(Unlimited),
        )
        .unwrap();
        let ids: Vec<String> = {
            let opened = editor.store().snapshot();
            let docx = opened.doc().docx().unwrap();
            docx.body().unwrap().ids.paragraphs().iter().map(|p| p.id).collect()
        };
        assert!(ids.len() > 110, "{}", ids.len());
        let caller = Caller::anonymous();
        let mut versions = vec![editor.store().snapshot()];
        for (k, n) in [3usize, 70, 100, 12].into_iter().enumerate() {
            let target = ids[n].clone();
            let done = match k % 3 {
                0 => editor
                    .run(
                        &caller,
                        &InsertParagraph {
                            anchor: target,
                            after: true,
                            text: format!("Inserted {k}"),
                            style: None,
                            policy: RevisionPolicy::Untracked,
                        },
                    )
                    .map(|d| d.version),
                1 => editor
                    .run(
                        &caller,
                        &DeleteBlock {
                            target: ooxml_word_model::address::Address::from(target),
                            policy: RevisionPolicy::Untracked,
                        },
                    )
                    .map(|d| d.version),
                _ => editor
                    .run(
                        &caller,
                        &SetText {
                            target: ooxml_word_model::address::Address::from(target),
                            text: format!("Rewritten {k}"),
                            policy: RevisionPolicy::Untracked,
                        },
                    )
                    .map(|d| d.version),
            };
            versions.push(done.unwrap());
        }
        for k in [4usize, 1, 3, 0, 2, 4] {
            check(versions[k].doc(), &format!("version {k}"));
        }
    }
}
