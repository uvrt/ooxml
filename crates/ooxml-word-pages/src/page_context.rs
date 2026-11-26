//! Owns: what a page's summary depends on beyond its laid page and the flow items it
//! draws ([`page_contexts`]), digested per page: its `PageInfo` (its number, section,
//! header kind, whether it is blank), the page count (`NUMPAGES`) and its section's
//! (`SECTIONPAGES`), its section's properties (its size, margins, borders, columns and
//! the headers and footers it shows), the texts its header's and footer's `STYLEREF`s
//! show on it, and the line numbers drawn beside its lines. Two pages with the same laid
//! page, the same items and the same context paint alike; the body's field values are in
//! its items already (a paragraph whose field shows another value is a new one).
//!
//! A page's context is also digested in two parts ([`PageContext`]): what its body is
//! painted in (its `PageInfo`, its section, its line numbers) and what only its stories
//! show (the counts and the `STYLEREF` texts): a page whose stories show another page
//! count has its body painted as before.
//!
//! Must not: paint, or digest what is not the page's own (one page more changes only the
//! pages whose counts it changes, not every page's).

use ooxml_base::{Digest, DigestKey, Digester};
use ooxml_word_layout::fields::page_counts;
use ooxml_word_layout::paginate::PageInfo;
use ooxml_word_layout::paint::LineNumber;
use ooxml_word_layout::sections::StoryKind;
use ooxml_word_layout::DocumentLayout;
use ooxml_word_model::Model;
use std::collections::BTreeMap;

/// A page's context, digested: whole, and in its body's and its stories' parts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PageContext {
    /// Everything (what a page's digests depend on).
    pub(crate) whole: Digest,
    /// What its body is painted in: its `PageInfo`, section and line numbers.
    pub(crate) body: Digest,
    /// What only its stories show: the counts and its `STYLEREF` texts.
    pub(crate) stories: Digest,
}

/// Each page's context (0-based, blank pages counted), digested under `key`.
pub(crate) fn page_contexts(
    layout: &DocumentLayout,
    model: &mut Model,
    locale: &ooxml_base::Locale,
    key: DigestKey,
) -> Vec<PageContext> {
    let sections: Vec<Digest> = layout
        .document
        .sections
        .iter()
        .map(|s| Digest::of(key, format!("{:?}", s.properties).as_bytes()))
        .collect();
    // The `STYLEREF` texts each page's stories show, by page, in their stories' order.
    let mut references: BTreeMap<usize, Vec<(&str, String)>> = BTreeMap::new();
    for ((page, id), texts) in layout.story_texts(locale) {
        references.entry(*page).or_default().push((&**id, format!("{texts:?}")));
    }
    for texts in references.values_mut() {
        texts.sort_unstable();
    }
    let numbers = layout.line_numbers(model);
    layout
        .pagination
        .info
        .iter()
        .enumerate()
        .map(|(page, info)| {
            let section = sections.get(info.section).copied().unwrap_or_default();
            let referenced = references.get(&page).map(Vec::as_slice).unwrap_or_default();
            let parts = Parts {
                info,
                counts: page_counts(layout, info.section),
                section,
                referenced,
                numbered: numbers.on(page),
            };
            parts.context(key)
        })
        .collect()
}

/// What one page's context is made of, each fed to the digests field by field.
struct Parts<'a> {
    info: &'a PageInfo,
    counts: (Option<i64>, Option<i64>),
    section: Digest,
    referenced: &'a [(&'a str, String)],
    numbered: &'a [LineNumber],
}

impl Parts<'_> {
    fn context(&self, key: DigestKey) -> PageContext {
        let mut whole = Digester::new(key);
        info_fields(&mut whole, self.info);
        counts_fields(&mut whole, self.counts);
        whole.update(&self.section.0);
        referenced_fields(&mut whole, self.referenced);
        numbered_fields(&mut whole, self.numbered);
        let mut body = Digester::new(key);
        info_fields(&mut body, self.info);
        body.update(&self.section.0);
        numbered_fields(&mut body, self.numbered);
        let mut stories = Digester::new(key);
        counts_fields(&mut stories, self.counts);
        referenced_fields(&mut stories, self.referenced);
        PageContext {
            whole: whole.finish(),
            body: body.finish(),
            stories: stories.finish(),
        }
    }
}

/// Every field of a `PageInfo`, fixed width (destructured: a field added to it must be
/// fed here too).
fn info_fields(d: &mut Digester, info: &PageInfo) {
    let PageInfo {
        number,
        section,
        section_first,
        story,
        blank,
    } = *info;
    d.update(&number.to_le_bytes());
    d.update(&(section as u64).to_le_bytes());
    let story = match story {
        StoryKind::Default => 0u8,
        StoryKind::First => 1,
        StoryKind::Even => 2,
    };
    d.update(&[u8::from(section_first), story, u8::from(blank)]);
}

fn counts_fields(d: &mut Digester, (pages, section_pages): (Option<i64>, Option<i64>)) {
    for count in [pages, section_pages] {
        match count {
            Some(n) => {
                d.update(&[1]);
                d.update(&n.to_le_bytes());
            }
            None => d.update(&[0]),
        }
    }
}

fn referenced_fields(d: &mut Digester, referenced: &[(&str, String)]) {
    d.update(&(referenced.len() as u64).to_le_bytes());
    for (id, texts) in referenced {
        d.field(id.as_bytes());
        d.field(texts.as_bytes());
    }
}

fn numbered_fields(d: &mut Digester, numbered: &[LineNumber]) {
    d.update(&(numbered.len() as u64).to_le_bytes());
    for n in numbered {
        // its key names the number as its path does (a name, never a memo key: ADR-008)
        let LineNumber {
            path,
            key: _,
            line,
            baseline,
            count,
            section,
        } = n;
        d.field(path.as_bytes());
        d.update(&(*line as u64).to_le_bytes());
        d.update(&baseline.to_le_bytes());
        d.update(&count.to_le_bytes());
        d.update(&(*section as u64).to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;

    const KEY: DigestKey = DigestKey::new(1, 2);

    fn digest(info: &PageInfo) -> PageContext {
        Parts {
            info,
            counts: (Some(3), Some(2)),
            section: Digest::default(),
            referenced: &[],
            numbered: &[],
        }
        .context(KEY)
    }

    #[test]
    fn page_infos_differing_in_any_field_digest_apart() {
        let base = PageInfo {
            number: 4,
            section: 1,
            section_first: false,
            story: StoryKind::Default,
            blank: false,
        };
        assert_eq!(digest(&base), digest(&PageInfo { ..base }));
        let others = [
            PageInfo { number: 5, ..base },
            PageInfo { section: 2, ..base },
            PageInfo {
                section_first: true,
                ..base
            },
            PageInfo {
                story: StoryKind::First,
                ..base
            },
            PageInfo {
                story: StoryKind::Even,
                ..base
            },
            PageInfo { blank: true, ..base },
        ];
        let mut seen = vec![digest(&base)];
        for other in &others {
            let made = digest(other);
            for was in &seen {
                assert_ne!(made.whole, was.whole, "{other:?}");
                assert_ne!(made.body, was.body, "{other:?}");
                // A page's stories do not read its PageInfo.
                assert_eq!(made.stories, was.stories, "{other:?}");
            }
            seen.push(made);
        }
    }

    #[test]
    fn counts_reach_the_stories_only() {
        let info = PageInfo {
            number: 1,
            section: 0,
            section_first: true,
            story: StoryKind::Default,
            blank: false,
        };
        let a = digest(&info);
        let b = Parts {
            info: &info,
            counts: (Some(4), Some(2)),
            section: Digest::default(),
            referenced: &[],
            numbered: &[],
        }
        .context(KEY);
        assert_ne!(a.whole, b.whole);
        assert_ne!(a.stories, b.stories);
        assert_eq!(a.body, b.body);
        let none = Parts {
            info: &info,
            counts: (None, Some(2)),
            section: Digest::default(),
            referenced: &[],
            numbered: &[],
        }
        .context(KEY);
        assert_ne!(none.stories, a.stories);
    }
}
