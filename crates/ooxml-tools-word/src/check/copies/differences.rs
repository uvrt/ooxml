//! Owns: a copy's paragraphs' settings compared with their reference paragraphs'
//! (docx-agent's `copy_settings.compare_settings`): one difference per cause -- a list
//! level that differs told by `list_level` alone, a label by `list_format` alone, the page
//! settings apart -- grouped per block, setting and values; where a numbered item's number
//! is compared at all (`counted`); and the copies whose lists the caller states run on
//! ([`Run`], round 43: never inferred).
//!
//! Must not: write a fix (`super::fixes` does) or read the document's lists itself.

use super::fixes::{like_takes, settings_fix, Fixing, Wanted};
use super::settings::{region, Count, Paragraph, Reader, Settings, PAGE, RUNS_ON, SETTINGS};
use ooxml_word_model::numbering::ListKey;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// A previous copy's numbered item a run goes on from: its level, list, label and id.
#[derive(Debug, Clone)]
pub(super) struct RunItem {
    level: Option<i64>,
    key: Option<ListKey>,
    format: Option<String>,
    id: String,
}

/// The caller states that a copy's numbered lists run on through the copies of one sample:
/// `previous` is `None` for the run's first copy (it numbers as its sample shows), else the
/// copy before it's numbered items, in order.
#[derive(Debug, Clone, Default)]
pub(super) struct Run {
    pub(super) previous: Option<Vec<RunItem>>,
}

impl Run {
    /// The run as it stands after the copy whose region's paragraphs are `region_before`.
    pub(super) fn after(reader: &Reader<'_, '_>, region_before: &[Paragraph]) -> Run {
        let found = region(reader, region_before);
        let ids = &reader.body().ids;
        let mut items = Vec::new();
        for (path, _) in region_before {
            let (Some(values), Some(entry)) = (found.get(path), ids.paragraph_at(path)) else {
                continue;
            };
            if values.list_numbering.is_some() {
                items.push(RunItem {
                    level: values.list_level,
                    key: values.key,
                    format: values.list_format.clone(),
                    id: entry.id,
                });
            }
        }
        Run { previous: Some(items) }
    }

    /// The previous copy's item `own` goes on from: its last at `own`'s level and label,
    /// else its last at the level.
    fn target(&self, own: &Settings) -> Option<&RunItem> {
        let level: Vec<&RunItem> = self
            .previous
            .iter()
            .flatten()
            .filter(|item| item.level == own.list_level)
            .collect();
        let alike: Vec<&RunItem> = level
            .iter()
            .copied()
            .filter(|item| item.format == own.list_format)
            .collect();
        if alike.is_empty() {
            level.last().copied()
        } else {
            alike.last().copied()
        }
    }

    /// `(value, expected)` of `list_numbering` for a copy's item and its sample's; the item
    /// it goes on from kept in `joins`.
    pub(super) fn compared(
        &self,
        own: &Settings,
        sample: &Settings,
        identifier: &str,
        joins: &mut HashMap<String, String>,
    ) -> (Value, Value) {
        let plain = || (own.value("list_numbering"), sample.value("list_numbering"));
        let leads = sample.lead
            && sample.count == Some(Count::Own)
            && sample
                .list_numbering
                .as_deref()
                .is_some_and(|n| n.starts_with("starts at "));
        if self.previous.is_none() || !leads {
            return plain();
        }
        let Some(found) = self.target(own) else {
            return plain();
        };
        joins.insert(identifier.to_owned(), found.id.clone());
        let goes_on = own.count == Some(Count::RunsOn) && own.key == found.key;
        let value = if goes_on {
            Value::from(RUNS_ON)
        } else {
            own.value("list_numbering")
        };
        (value, Value::from(RUNS_ON))
    }
}

/// Whether where a copy's item stands in its count is compared with its sample's: only
/// where the sample's own count begins inside the sample and goes on there, and the copy's
/// item is not numbered by the outline.
fn counted(own: &Settings, sample: &Settings) -> bool {
    sample.count == Some(Count::Own) && own.count != Some(Count::Outline)
}

/// The paragraphs compared: each pair `(paragraph, reference)`, with the paragraph's
/// `(block, paragraph id)` and the reference's id.
pub(super) struct Compared<'p> {
    pub(super) pairs: &'p [(Paragraph, Paragraph)],
    pub(super) ids: &'p HashMap<Vec<u32>, (String, String)>,
    pub(super) reference_ids: &'p HashMap<Vec<u32>, String>,
    pub(super) mine: &'p HashMap<Vec<u32>, Settings>,
    pub(super) theirs: &'p HashMap<Vec<u32>, Settings>,
}

/// One group of differences: a block's setting with the same values, its paragraphs, the
/// settings it was first seen with (the sample paragraph's and the copy paragraph's), per
/// paragraph the sample paragraph of the same document it may look like, and whether a
/// restart would break a stated run.
struct Group<'s> {
    block: String,
    name: &'static str,
    value: Value,
    expected: Value,
    paragraphs: Vec<String>,
    sample: &'s Settings,
    own: &'s Settings,
    likes: Vec<(String, Option<String>)>,
    held: bool,
}

/// The differences of `c`'s pairs, each `{"block", "property", "paragraphs", "value",
/// "reference", "fix"}` grouped per block, property and values, in the settings' order;
/// `run` where the caller states the copy's lists run on.
pub(super) fn compare_settings(
    c: &Compared<'_>,
    fixing: &Fixing<'_, '_>,
    run: Option<&Run>,
) -> Vec<Map<String, Value>> {
    let mut groups: Vec<Group<'_>> = Vec::new();
    let mut joins: HashMap<String, String> = HashMap::new();
    for (paragraph, reference) in c.pairs {
        let Some((block, identifier)) = c.ids.get(&paragraph.0) else {
            continue;
        };
        let (Some(a), Some(b)) = (c.mine.get(&paragraph.0), c.theirs.get(&reference.0)) else {
            continue;
        };
        let names: Vec<&'static str> = if a.list_level != b.list_level {
            std::iter::once("list_level").chain(PAGE).collect()
        } else if a.list_format != b.list_format {
            std::iter::once("list_format").chain(PAGE).collect()
        } else {
            SETTINGS.to_vec()
        };
        let sample = c.reference_ids.get(&reference.0).cloned().unwrap_or_default();
        for name in names {
            let (mut value, mut expected) = (a.value(name), b.value(name));
            if name == "list_numbering" {
                if !counted(a, b) {
                    continue;
                }
                if let Some(run) = run {
                    (value, expected) = run.compared(a, b, identifier, &mut joins);
                }
            }
            if value == expected {
                continue;
            }
            // Round 42: the same document's sample paragraph to look like, where taking its
            // look changes nothing else this comparison reads.
            let like = (fixing.same && !sample.is_empty() && like_takes(name, &paragraph.1, &reference.1, a, b))
                .then(|| sample.clone());
            let at = groups
                .iter()
                .position(|g| g.block == *block && g.name == name && g.value == value && g.expected == expected);
            let k = match at {
                Some(k) => k,
                None => {
                    groups.push(Group {
                        block: block.clone(),
                        name,
                        value,
                        expected,
                        paragraphs: Vec::new(),
                        sample: b,
                        own: a,
                        likes: Vec::new(),
                        held: false,
                    });
                    groups.len() - 1
                }
            };
            let Some(group) = groups.get_mut(k) else { continue };
            group.paragraphs.push(identifier.clone());
            // a restart inside a running list moves the run
            group.held |= run.is_some() && name == "list_numbering" && !b.lead;
            match group.likes.iter_mut().find(|(p, _)| p == identifier) {
                Some(entry) => entry.1 = like,
                None => group.likes.push((identifier.clone(), like)),
            }
        }
    }
    let mut out: Vec<Map<String, Value>> = groups.iter().map(|g| difference(g, fixing, &joins)).collect();
    let order = |d: &Map<String, Value>| {
        let name = d.get("property").and_then(Value::as_str).unwrap_or("");
        SETTINGS.iter().position(|s| *s == name).unwrap_or(SETTINGS.len())
    };
    out.sort_by_key(order);
    out
}

/// A group's difference as reported, with its fix.
fn difference(g: &Group<'_>, fixing: &Fixing<'_, '_>, joins: &HashMap<String, String>) -> Map<String, Value> {
    let wanted = Wanted {
        name: g.name,
        value: &g.value,
        reference: &g.expected,
        paragraphs: &g.paragraphs,
        sample_settings: g.sample,
        own_settings: g.own,
        likes: &g.likes,
        joins,
        restart: !g.held,
    };
    let mut out = Map::new();
    out.insert("block".into(), Value::from(g.block.as_str()));
    out.insert("property".into(), Value::from(g.name));
    out.insert("paragraphs".into(), Value::from(g.paragraphs.clone()));
    out.insert("value".into(), g.value.clone());
    out.insert("reference".into(), g.expected.clone());
    out.insert("fix".into(), settings_fix(fixing, &wanted));
    out
}
