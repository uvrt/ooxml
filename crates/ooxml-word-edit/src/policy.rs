//! Owns: [`RevisionPolicy`], whether an edit writes its primitives as they are or as
//! tracked changes (design section 4, "Tracked changes are a strategy, not branches"), the
//! [`Tracking`] they are written by, and [`Stamp`], one edit's revision records: their ids
//! -- one above the largest annotation id in use in any story, as docx-agent allocates
//! them -- and the attributes Word 16.106 writes (`revisions/stamp.py`).
//!
//! Must not: read the clock (a tracking's date is given), or decide what an edit changes.

use crate::names::{W, W16DU};
use crate::work::{Id, Work};

/// Who makes tracked changes, and when (ISO 8601, minutes, UTC with a `Z`, as Word writes a
/// revision's date).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracking {
    /// The author every record carries.
    pub author: String,
    /// The date every record carries, `w:date` and `w16du:dateUtc` alike.
    pub date: String,
}

/// How an edit's primitives are written.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RevisionPolicy {
    /// As they are.
    #[default]
    Untracked,
    /// As tracked changes by this author at this date.
    Tracked(Tracking),
}

/// One edit's revision records: the ids it gives and the attributes it writes.
#[derive(Debug)]
pub(crate) struct Stamp {
    tracking: Tracking,
    next: i64,
    /// Whether a record carries `w16du:dateUtc` (its part's root then declares it).
    pub(crate) dated: bool,
}

impl Stamp {
    /// Records by `tracking`, numbered from `next` (one above the largest annotation id).
    pub(crate) fn new(tracking: Tracking, next: i64) -> Stamp {
        Stamp {
            tracking,
            next,
            dated: false,
        }
    }

    /// The next revision id.
    pub(crate) fn next_id(&mut self) -> String {
        let value = self.next;
        self.next += 1;
        value.to_string()
    }

    /// The id the next record would get (the ids given are those below it).
    pub(crate) fn next(&self) -> i64 {
        self.next
    }

    /// A revision record (`w:ins`, `w:del`), detached: its id, author, date and UTC date
    /// (a `w:tblGridChange` its id only).
    pub(crate) fn make(&mut self, work: &mut Work, local: &str) -> Id {
        let element = work.new_element(work.qname(W, local, "w"));
        let id = self.next_id();
        if local == "tblGridChange" {
            // a grid's record carries its id only (as Word writes it)
            let name = work.qname(W, "id", "w");
            work.set(element, name, &id);
            return element;
        }
        for (name, value) in [
            ("id", id.as_str()),
            ("author", &self.tracking.author),
            ("date", &self.tracking.date),
        ] {
            let name = work.qname(W, name, "w");
            work.set(element, name, value);
        }
        let utc = work.qname(W16DU, "dateUtc", "w16du");
        work.set(element, utc, &self.tracking.date);
        self.dated = true;
        element
    }

    /// A move's range marker (`w:moveFromRangeStart`, `w:moveToRangeStart`), detached: its
    /// id, author, date and the move's `name`; no UTC date (as Word writes it).
    pub(crate) fn make_named(&mut self, work: &mut Work, local: &str, name: &str) -> Id {
        let element = work.new_element(work.qname(W, local, "w"));
        let id = self.next_id();
        for (attribute, value) in [
            ("id", id.as_str()),
            ("author", &self.tracking.author),
            ("date", &self.tracking.date),
            ("name", name),
        ] {
            let attribute = work.qname(W, attribute, "w");
            work.set(element, attribute, value);
        }
        element
    }

    /// The author every record carries.
    pub(crate) fn author(&self) -> &str {
        &self.tracking.author
    }

    /// Whether a revision record is this edit's author's.
    pub(crate) fn same(&self, work: &Work, element: Id) -> bool {
        work.get(element, Some(W), "author") == Some(self.tracking.author.as_str())
    }
}
