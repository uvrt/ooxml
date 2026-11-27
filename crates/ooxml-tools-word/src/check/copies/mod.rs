//! `check`'s `copies` (lane CP1; docx-agent's `tools/scoped.py` `compare_copies`, rounds 35,
//! 42 and 43): each copy compared with the reference it was made from, in this document or
//! another open one ([`compare`]), the comparison kept by what both sides hold ([`cache`]);
//! the reply within its budget (rule 4: what is left out is counted, and how to read it
//! said), its `fix` one `word_follow_sample` call per reference block and its `list_fix`
//! the `word_lists` calls of every copy's list differences, alike ones joined (rule 5). A
//! copy's `lists` `run_on` states that its numbered lists count on from the copy before it
//! of the same reference (`runs`).
//!
//! Must not: change a document.

mod blocks;
mod cache;
mod compare;
mod differences;
mod fixes;
mod report;
mod settings;

pub use cache::CopyCache;

use super::args::{Check, CheckCopy, CopyLists};
use crate::describe::cancelled;
use crate::style::compare::Side;
use crate::version::{Owned, Version};
use cache::{key, Context, Keyed};
use compare::{compare_copy, CopyBlock, Document, RunsOn};
use fixes::Fixing;
use ooxml_base::Cancel;
use ooxml_tools::{Failure, Refusal, ToolError, WayOut};
use ooxml_word_edit::WordDoc;
use ooxml_word_model::docx::numbering::Lists;
use ooxml_word_model::docx::Docx;
use report::Report;
use serde_json::{Map, Value};
use settings::Reader;
use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Copies in one check at most (a chapter's, or a reviewer's few chapters).
const MAX_COPIES: usize = 40;
/// What `unported` says of copies whose reference is a document the session holds only as
/// a package.
pub const CHECK_COPIES_PACKAGE: &str = "check: copies of a document held as a package";

/// One document a check of copies reads: its name, its reading and the document.
struct Doc<'v> {
    name: String,
    docx: Arc<Docx>,
    word: &'v WordDoc,
}

/// One copy resolved: its reference's document (by index), its blocks and the reference's.
struct Resolved<'a> {
    item: &'a CheckCopy,
    other: usize,
    copy: Vec<CopyBlock>,
    reference: Vec<CopyBlock>,
}

/// Where a copy stands in a stated run.
#[derive(Clone, Copy)]
enum Place {
    First,
    After(usize),
}

/// `copies`' report, or why the host answers the check.
///
/// # Errors
/// `limit` past [`MAX_COPIES`]; `not_found` or `invalid_arguments` for a copy or a
/// reference naming no blocks, or a document not open; `timeout` when cancelled.
pub(crate) fn copies(
    args: &Check,
    version: &Version<'_>,
    owned: Owned<'_>,
    cancel: &dyn Cancel,
) -> Result<Result<Value, &'static str>, ToolError> {
    let (docs, resolved) = match resolve(args, version, owned)? {
        Ok(found) => found,
        Err(why) => return Ok(Err(why)),
    };
    let (places, alone) = runs(&resolved);
    let contexts: Vec<Context> = docs.iter().map(|d| Context::of(&d.docx)).collect();
    let sides: Vec<Side<'_>> = docs.iter().map(|d| Side::new(&d.docx)).collect();
    let readers: Vec<OnceCell<Option<Reader<'_, '_>>>> = docs.iter().map(|_| OnceCell::new()).collect();
    let opened = Opened {
        docs: &docs,
        sides: &sides,
        readers: &readers,
        lists: OnceCell::new(),
    };
    let cache = owned.last_check.copies();
    let mut report = Report::default();
    for (number, r) in resolved.iter().enumerate() {
        cancel.check().map_err(cancelled)?;
        let place = places.get(&number).copied();
        let previous = match place {
            Some(Place::After(j)) => Some(at(&resolved, j)?),
            _ => None,
        };
        let before = match (place, previous) {
            (_, Some(previous)) => Ok(previous.copy.as_slice()),
            (Some(Place::First), _) => Err(true),
            _ => Err(false),
        };
        let keyed = key(
            &Keyed {
                doc: &at(&docs, 0)?.name,
                context: at(&contexts, 0)?,
                blocks: &r.copy,
            },
            &Keyed {
                doc: &at(&docs, r.other)?.name,
                context: at(&contexts, r.other)?,
                blocks: &r.reference,
            },
            before,
        );
        let (found, kept) = match cache.get(&keyed) {
            Some(found) => (found, true),
            None => {
                let found = Arc::new(opened.compare(r, before)?);
                cache.put(keyed, Arc::clone(&found));
                (found, false)
            }
        };
        let other = (r.other != 0).then(|| at(&docs, r.other)).transpose()?;
        let said = Said {
            other: other.map(|d| d.name.as_str()),
            runs_from: previous.map(|p| p.item.blocks.trim()),
            kept,
        };
        report.add(r, &said, &found);
    }
    Ok(Ok(report.finish(&args.doc, &alone)))
}

/// Item `k` of `items` (`internal` where there is none: a bug).
fn at<T>(items: &[T], k: usize) -> Result<&T, ToolError> {
    items
        .get(k)
        .ok_or_else(|| ToolError::internal("a copy's document read out of range", "check:copies"))
}

/// Every copy's documents and blocks, as named (the copy's document first), or why the
/// host answers the check.
#[allow(clippy::type_complexity)]
fn resolve<'a, 'v>(
    args: &'a Check,
    version: &Version<'v>,
    owned: Owned<'v>,
) -> Result<Result<(Vec<Doc<'v>>, Vec<Resolved<'a>>), &'static str>, ToolError> {
    let items: &[CheckCopy] = args.copies.as_deref().unwrap_or_default();
    if items.len() > MAX_COPIES {
        return Err(ToolError::failure(
            Failure::Limit,
            format!(
                "{} copies: at most {MAX_COPIES} per check; split them over two checks (one per chapter)",
                items.len()
            ),
        )
        .with_limit(MAX_COPIES, items.len())
        .at("copies"));
    }
    let mine = Doc {
        name: args.doc.clone(),
        docx: version.docx()?,
        word: owned.doc,
    };
    let mut docs = vec![mine];
    let mut resolved = Vec::new();
    for (number, item) in items.iter().enumerate() {
        let name = item
            .reference
            .doc
            .as_deref()
            .filter(|d| !d.is_empty())
            .unwrap_or(&args.doc);
        let other = match docs.iter().position(|d| d.name == name) {
            Some(k) => k,
            None => {
                let Some(found) = version.others.get(name) else {
                    return Err(not_open(number, name, &args.doc));
                };
                let Some(word) = found.word else {
                    return Ok(Err(CHECK_COPIES_PACKAGE));
                };
                docs.push(Doc {
                    name: name.to_owned(),
                    docx: found.docx()?,
                    word,
                });
                docs.len() - 1
            }
        };
        let own = at(&docs, 0)?;
        let copy = blocks::blocks(
            own.word,
            &own.docx,
            &item.blocks,
            &format!("copies[{number}].blocks"),
            &args.doc,
        )?;
        let theirs = at(&docs, other)?;
        let reference = blocks::blocks(
            theirs.word,
            &theirs.docx,
            &item.reference.block,
            &format!("copies[{number}].reference.block"),
            name,
        )?;
        resolved.push(Resolved {
            item,
            other,
            copy,
            reference,
        });
    }
    Ok(Ok((docs, resolved)))
}

/// The documents a check of copies compares, as each comparison reads them: the copy's
/// document first; each document's body read for its lists only when a comparison needs it.
struct Opened<'o, 'd> {
    docs: &'o [Doc<'d>],
    sides: &'o [Side<'o>],
    readers: &'o [OnceCell<Option<Reader<'o, 'o>>>],
    lists: OnceCell<Lists>,
}

impl<'o> Opened<'o, '_> {
    /// The document `k` as a comparison reads it.
    fn document(&self, k: usize) -> Result<Document<'o, 'o>, ToolError> {
        let side = at(self.sides, k)?;
        let doc = at(self.docs, k)?;
        let reader = at(self.readers, k)?
            .get_or_init(|| doc.docx.body().map(|body| Reader::new(side, body)))
            .as_ref()
            .ok_or_else(|| ToolError::internal("a document without a body", "check:copies"))?;
        Ok(Document { side, reader })
    }

    /// `r`'s comparison, `before` the blocks of the copy before it in a stated run
    /// (`Err(first)`: none, the run's first copy or no run).
    fn compare(&self, r: &Resolved<'_>, before: Result<&[CopyBlock], bool>) -> Result<Map<String, Value>, ToolError> {
        let (mine, theirs) = (self.document(0)?, self.document(r.other)?);
        let own = at(self.docs, 0)?;
        let fixing = Fixing {
            side: mine.side,
            docx: &own.docx,
            body: mine.reader.body(),
            lists: self.lists.get_or_init(|| Lists::read(&own.docx)),
            same: r.other == 0,
        };
        let runs_on = match before {
            Ok(blocks) => RunsOn::After(blocks),
            Err(true) => RunsOn::First,
            Err(false) => RunsOn::No,
        };
        compare_copy((&mine, &theirs), &fixing, (&r.copy, &r.reference), &runs_on)
    }
}

/// What a copy's entry says besides its comparison: its reference's document where it is
/// another, the copy before it in a run, and whether the comparison was kept.
struct Said<'s> {
    other: Option<&'s str>,
    runs_from: Option<&'s str>,
    kept: bool,
}

/// The copies stated to run on (`lists` `run_on`) as runs -- those of one reference
/// (document and blocks), in the order they stand in the document: per copy's number its
/// place in its run; and the copies alone in their run.
fn runs(resolved: &[Resolved<'_>]) -> (BTreeMap<usize, Place>, Vec<usize>) {
    // a run's reference: its document and blocks
    type Reference<'r> = (usize, Vec<&'r str>);
    let mut members: Vec<(Reference<'_>, Vec<usize>)> = Vec::new();
    for (number, r) in resolved.iter().enumerate() {
        if r.item.lists != Some(CopyLists::RunOn) {
            continue;
        }
        let key = (r.other, r.reference.iter().map(|b| b.id.as_str()).collect::<Vec<_>>());
        match members.iter_mut().find(|(k, _)| *k == key) {
            Some((_, numbers)) => numbers.push(number),
            None => members.push((key, vec![number])),
        }
    }
    let mut places = BTreeMap::new();
    let mut alone = Vec::new();
    for (_, mut numbers) in members {
        numbers.sort_by(|a, b| {
            let place = |n: usize| {
                resolved
                    .get(n)
                    .and_then(|r| r.copy.first())
                    .map(|b| b.path.clone())
                    .unwrap_or_default()
            };
            place(*a).cmp(&place(*b))
        });
        if numbers.len() == 1 {
            alone.extend(numbers.first().copied());
        }
        for (k, number) in numbers.iter().enumerate() {
            let place = match k.checked_sub(1).and_then(|j| numbers.get(j)) {
                Some(previous) => Place::After(*previous),
                None => Place::First,
            };
            places.insert(*number, place);
        }
    }
    alone.sort_unstable();
    (places, alone)
}

/// A reference document the call names but was not handed.
fn not_open(number: usize, name: &str, doc: &str) -> ToolError {
    let message = format!(
        "copies[{number}].reference.doc {} is not open here; open it first (open_document) or name one of these",
        ooxml_base::pyfmt::repr_str(name)
    );
    match WayOut::names([doc.to_owned()]) {
        Some(way_out) => ToolError::refusal(Refusal::NotFound, message, way_out).at("copies"),
        None => ToolError::internal(message, "check:copies"),
    }
}

#[cfg(test)]
mod tests {
    use super::cache::CopyCache;
    use super::report::{merged, Report, LIST_CALLS};
    use super::{Resolved, Said};
    use crate::{CheckCopy, CopyReference};
    use ooxml_base::Digest;
    use serde_json::{json, Map, Value};
    use std::sync::Arc;

    #[test]
    fn the_kept_comparisons_are_bounded_the_least_recently_used_dropped() {
        let cache = CopyCache::default();
        let key = |k: u8| Digest([k; 16]);
        for k in 0..=128u8 {
            cache.put(key(k), Arc::new(Map::new()));
            if k == 1 {
                assert!(cache.get(&key(0)).is_some()); // used again: kept past the next
            }
        }
        assert!(cache.get(&key(0)).is_some() && cache.get(&key(1)).is_none());
    }

    #[test]
    fn past_the_reply_s_budget_differences_are_counted_and_the_way_to_read_them_said() {
        let item = CheckCopy {
            blocks: "p:A..p:B".into(),
            reference: CopyReference {
                doc: None,
                block: "p:C..p:D".into(),
            },
            lists: None,
        };
        let resolved = Resolved {
            item: &item,
            other: 0,
            copy: Vec::new(),
            reference: Vec::new(),
        };
        let differences: Vec<Value> = (0..30)
            .map(|k| json!({"block": "p:A", "property": "bold", "value": k, "kind": "format"}))
            .collect();
        let mut found = Map::new();
        found.insert("follows".into(), json!(false));
        found.insert("differences".into(), Value::Array(differences));
        let mut report = Report::default();
        let said = Said {
            other: None,
            runs_from: None,
            kept: false,
        };
        for _ in 0..6 {
            report.add(&resolved, &said, &found);
        }
        let reply = report.finish("d1", &[]);
        let items = reply["items"].as_array().unwrap();
        let shown: usize = items.iter().map(|i| i["differences"].as_array().unwrap().len()).sum();
        let more: u64 = items.iter().filter_map(|i| i["more"].as_u64()).sum();
        assert_eq!((shown, more), (100, 80));
        assert!(reply["left_out"].as_str().unwrap().contains("word_compare_blocks"));
    }

    #[test]
    fn alike_calls_that_change_each_target_are_joined_the_others_kept_apart() {
        let calls = [
            json!({"targets": ["p:A"], "action": "level", "level": 1}),
            json!({"targets": ["p:B"], "action": "restart", "start": 1}),
            json!({"targets": ["p:C"], "action": "level", "level": 1}),
            json!({"targets": ["p:D"], "action": "restart", "start": 1}),
            json!({"targets": ["p:A"], "action": "level", "level": 1}),
        ];
        assert_eq!(
            merged(&calls),
            vec![
                json!({"targets": ["p:A", "p:C"], "action": "level", "level": 1}),
                json!({"targets": ["p:B"], "action": "restart", "start": 1}),
                json!({"targets": ["p:D"], "action": "restart", "start": 1}),
            ]
        );
    }

    #[test]
    fn the_list_fix_keeps_to_its_budget() {
        let mut report = Report::default();
        report.list_calls = (0..LIST_CALLS + 2)
            .map(|k| json!({"targets": [format!("p:{k}")], "action": "restart", "start": 1}))
            .collect();
        let found = report.finish("d1", &[]);
        assert_eq!(found["list_fix"]["calls"].as_array().map(Vec::len), Some(LIST_CALLS));
        assert_eq!(found["list_fix"]["more"], json!(2));
    }
}
