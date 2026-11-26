//! Owns: the pages each paragraph of a laid-out version is drawn on, by its id, as a
//! check's facts place it (docx-agent's `_Facts._placed`): a body paragraph's from the
//! layout's lines, a header's, footer's or note's from the pages' text on every page asked
//! for that draws it.
//!
//! A header's, footer's or note's lines are read from the pages' text the facts paint once
//! ([`Facts::texts`]), only when such a paragraph is first asked for, and only on the
//! pages the check asks for (every page where it asks for none).
//!
//! Must not: lay out or paint but through the facts' pages.

use ooxml_word_pages::layout_facts::Facts;
use ooxml_word_pages::line_text::ItemPlace;
use std::cell::OnceCell;
use std::collections::{BTreeSet, HashMap};

/// The 1-based pages each paragraph of every story is drawn on (docx-agent's `placed`: a
/// header's paragraph is on every page that draws it).
pub(super) struct Placed<'f, 'a> {
    facts: &'f Facts<'a>,
    /// Each body paragraph's pages, by its path.
    body: HashMap<&'a str, BTreeSet<usize>>,
    /// Each header's, footer's and note's paragraph's pages among those asked for, by its
    /// id: read when one is first asked for.
    stories: OnceCell<HashMap<String, BTreeSet<usize>>>,
}

impl<'f, 'a> Placed<'f, 'a> {
    /// The pages of `facts`' version.
    pub(super) fn of(facts: &'f Facts<'a>) -> Self {
        let mut body: HashMap<&str, BTreeSet<usize>> = HashMap::new();
        for (index, page) in facts.version().facts().pages.iter().enumerate() {
            for line in &page.lines {
                body.entry(&*line.path).or_default().insert(index + 1);
            }
        }
        Placed {
            facts,
            body,
            stories: OnceCell::new(),
        }
    }

    /// The pages the paragraph `id` is drawn on, in order: a header's, footer's or note's
    /// only among those asked for.
    pub(super) fn pages(&self, id: &str) -> Vec<usize> {
        let found = match self.facts.version().path_of(id) {
            Some(path) => self.body.get(&*path),
            None => self.stories().get(id),
        };
        found.map(|pages| pages.iter().copied().collect()).unwrap_or_default()
    }

    /// Each header's, footer's and note's paragraph's pages among those asked for, read
    /// once.
    fn stories(&self) -> &HashMap<String, BTreeSet<usize>> {
        self.stories.get_or_init(|| {
            let facts = self.facts;
            let mut named: HashMap<(Option<String>, ItemPlace), Option<String>> = HashMap::new();
            let mut out: HashMap<String, BTreeSet<usize>> = HashMap::new();
            for page in (1..=facts.page_count()).filter(|&p| facts.shown([p])) {
                for line in facts.texts(page - 1).unwrap_or_default() {
                    if line.at.story == "body" {
                        continue;
                    }
                    let part = line.story.as_ref().map(|(_, part)| part.clone());
                    let id = named
                        .entry((part, line.place.clone()))
                        .or_insert_with(|| facts.version().line_paragraph_id(facts.docx(), line));
                    if let Some(id) = id {
                        out.entry(id.clone()).or_default().insert(page);
                    }
                }
            }
            out
        })
    }
}
