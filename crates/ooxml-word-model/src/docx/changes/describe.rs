//! Owns: a logical change described as docx-agent's `revisions/changes.py` describes one
//! (`_change`): its kind, author and date, its records, the text it inserted and deleted,
//! the paragraphs it touches, its story and the table structure edit it is.
//!
//! Must not: group records (the grouping does) or change the document.

use super::{is_mark, Group, Grouping};
use crate::docx::{Docx, StoryPart};
use crate::ids::W;

/// The kinds of logical change.
pub const CHANGE_KINDS: [&str; 7] = [
    "insertion",
    "deletion",
    "replacement",
    "move",
    "formatting",
    "table",
    "section",
];

/// One logical change: the records one person made in one go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalChange {
    /// `insertion`, `deletion`, `replacement`, `move`, `formatting`, `table` or `section`.
    pub kind: &'static str,
    /// Its author: the first record's that names one.
    pub author: Option<String>,
    /// Its date, with the author.
    pub date: Option<String>,
    /// Its records' ids, in document order (`rev:12`...).
    pub records: Vec<String>,
    /// The text inserted (a replacement's new text, a move's text); paragraphs end in `\n`.
    pub text: String,
    /// The text deleted.
    pub old_text: String,
    /// The paragraphs it touches, in order.
    pub paragraph_ids: Vec<String>,
    /// Its story.
    pub story: String,
    /// Its part.
    pub part: String,
    /// The table structure edit it is (`column deletion`, `column insertion`, `merge`,
    /// `split`, `row deletion`, `row insertion`).
    pub structure: Option<&'static str>,
}

impl LogicalChange {
    /// Its id: its first record's.
    #[must_use]
    pub fn id(&self) -> &str {
        self.records.first().map_or("", String::as_str)
    }
}

/// The document's revisions grouped into logical changes, in story and document order.
#[must_use]
pub fn changes(docx: &Docx) -> Vec<LogicalChange> {
    super::grouped(docx, describe)
}

const INSERTED: [&str; 5] = [
    "insertion",
    "paragraph-mark-insertion",
    "run-mark-insertion",
    "row-insertion",
    "cell-insertion",
];
const DELETED: [&str; 5] = [
    "deletion",
    "paragraph-mark-deletion",
    "run-mark-deletion",
    "row-deletion",
    "cell-deletion",
];

/// The kind of a group of records of `kinds`, the structure edit it is `structure`.
fn kind_of(kinds: &[String], structure: Option<&str>) -> &'static str {
    let any = |set: &[&str]| kinds.iter().any(|k| set.contains(&k.as_str()));
    if kinds.iter().any(|k| k.ends_with("move-from") || k.ends_with("move-to")) {
        "move"
    } else if matches!(structure, Some("merge" | "split")) {
        "table"
    } else if structure == Some("column deletion") {
        "deletion"
    } else if structure == Some("column insertion") {
        "insertion"
    } else if any(&INSERTED) && any(&DELETED) {
        "replacement"
    } else if any(&INSERTED) {
        "insertion"
    } else if any(&DELETED) {
        "deletion"
    } else if kinds
        .iter()
        .any(|k| k.starts_with("table-") || k.starts_with("row-") || k.starts_with("cell-"))
    {
        "table"
    } else if !kinds.is_empty() && kinds.iter().all(|k| k == "section-properties") {
        "section"
    } else {
        "formatting"
    }
}

fn describe(part: &StoryPart, grouping: &Grouping<'_>, group: &Group) -> LogicalChange {
    let arena = &grouping.arena;
    let at: Vec<usize> = group
        .members
        .iter()
        .filter_map(|k| grouping.at.get(*k).copied())
        .collect();
    let kinds: Vec<String> = at.iter().map(|a| grouping.kind(*a)).collect();
    let who = |a: usize| {
        (
            arena.attr(a, "author").map(str::to_owned),
            arena.attr(a, "date").map(str::to_owned),
        )
    };
    let (author, date) = at
        .iter()
        .find(|a| arena.attr(**a, "author").is_some())
        .or(at.first())
        .map_or((None, None), |a| who(*a));
    let mut paragraphs: Vec<usize> = Vec::new();
    for a in &at {
        if let Some(p) = arena.nearest(*a, "p") {
            if !paragraphs.contains(&p) {
                paragraphs.push(p);
            }
        }
    }
    let paragraph_ids = paragraphs
        .iter()
        .filter_map(|p| arena.paths.get(*p))
        .filter_map(|path| part.ids.paragraph_at(path))
        .map(|p| p.id)
        .collect();
    let content = |locals: &[&str]| -> Vec<usize> {
        at.iter()
            .copied()
            .filter(|a| {
                let e = arena.element(*a);
                e.name().namespace() == Some(W)
                    && locals.contains(&e.name().local())
                    && !is_mark(arena, *a)
                    && !arena.parent(*a).is_some_and(|p| arena.is(p, "rPr"))
            })
            .collect()
    };
    let structure = group.label;
    let kind = kind_of(&kinds, structure);
    let mut text = joined(grouping, &content(&["ins", "moveTo"]));
    let old_text = joined(grouping, &content(&["del"]));
    if kind == "move" && text.is_empty() {
        text = joined(grouping, &content(&["moveFrom"]));
    }
    LogicalChange {
        kind,
        author,
        date,
        records: group
            .members
            .iter()
            .filter_map(|k| grouping.held.get(*k).map(|r| r.id.clone()))
            .collect(),
        text,
        old_text,
        paragraph_ids,
        story: part.story.clone(),
        part: part.part.clone(),
        structure,
    }
}

/// The containers' text, a `\n` between paragraphs.
fn joined(grouping: &Grouping<'_>, containers: &[usize]) -> String {
    let mut out = String::new();
    let mut last: Option<usize> = None;
    for c in containers {
        let paragraph = grouping.arena.nearest(*c, "p");
        if last.is_some_and(|l| Some(l) != paragraph) {
            out.push('\n');
        }
        out.push_str(&grouping.texts(*c));
        last = paragraph;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::kind_of;

    #[test]
    fn a_deletion_and_an_insertion_are_a_replacement() {
        let kinds = vec!["deletion".to_owned(), "insertion".to_owned()];
        assert_eq!(kind_of(&kinds, None), "replacement");
        assert_eq!(kind_of(&["section-properties".to_owned()], None), "section");
        assert_eq!(kind_of(&["cell-deletion".to_owned()], Some("merge")), "table");
    }
}
