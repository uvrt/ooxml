//! Owns: a new comment written as docx-agent's `_write_comment` writes it -- `w:comment`
//! in the comments part (made when the document has none) with its paragraphs, its entries
//! in `commentsExtended` (not done; the comment it replies to), `commentsIds` (a new
//! durable id) and `commentsExtensible` (the UTC date) -- and every other comment given the
//! entries it lacks, as Word gives them on saving (`_complete_comment_parts`).
//!
//! Must not: write a comment part but through the edit's copies ([`Space`]); list the
//! author (the caller keeps the people part, after this).

use super::text::comment_paragraph;
use super::{ensure_part, Kind, W15, W16CEX, W16CID};
use crate::names::{W, W14};
use crate::review::space::Space;
use crate::review::xml::make;
use crate::sink::EditSink;
use crate::text::write::check_text;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::ids::IdPool;
use std::collections::{HashMap, HashSet};

/// Who writes a comment, and when (docx-agent's `Tracking.make`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Who {
    /// `w:author`.
    pub author: String,
    /// `w:date`, ISO 8601 in UTC.
    pub date: String,
    /// `w:initials`.
    pub initials: String,
}

impl Who {
    /// `author` (docx-agent's default author when none) at `date`, the initials made of
    /// the author's name.
    #[must_use]
    pub fn new(author: Option<&str>, date: &str) -> Who {
        let author = author.filter(|a| !a.is_empty()).unwrap_or(DEFAULT_AUTHOR).to_owned();
        let initials = initials(&author);
        Who {
            author,
            date: date.to_owned(),
            initials,
        }
    }
}

/// docx-agent's `DEFAULT_AUTHOR`.
const DEFAULT_AUTHOR: &str = "docx-agent";

/// docx-agent's `_initials`: the first letter of each word that opens with a letter or a
/// digit, upper case, at most nine; else the name's first character.
fn initials(author: &str) -> String {
    let made: String = author
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_uppercase()
        .chars()
        .take(9)
        .collect();
    if made.is_empty() {
        author
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default()
    } else {
        made
    }
}

/// The comment parts' copies one comment is written into.
pub(super) struct Parts {
    pub(super) comments: String,
}

/// Write comment `id` saying `text` by `who` into the comment parts (made where missing,
/// in docx-agent's order), replying to the comment whose last paragraph is `parent`; its
/// durable id.
///
/// # Errors
/// Text a comment cannot hold, or a part that is not XML.
pub(super) fn write_comment(
    sink: &mut EditSink<'_, '_>,
    space: &mut Space,
    main: &str,
    (id, text): (i64, &str),
    who: &Who,
    parent: Option<&str>,
) -> Result<(String, Parts)> {
    let comments = ensure_part(sink, main, Kind::Comments)?;
    let doc = sink.doc().clone();
    let work = space
        .work(&doc, &comments)?
        .ok_or_else(|| EditError::no_part(comments.clone()))?;
    let root = work.root();
    let id_text = id.to_string();
    let node = make(
        work,
        "comment",
        &[
            ("id", id_text.as_str()),
            ("author", who.author.as_str()),
            ("date", who.date.as_str()),
            ("initials", who.initials.as_str()),
        ],
    );
    work.append(root, node);
    let last_para = paragraphs(sink, space, &comments, node, text)?;
    let extended = ensure_part(sink, main, Kind::Extended)?;
    let mut entry = vec![("paraId", last_para.as_str())];
    if let Some(parent) = parent {
        entry.push(("paraIdParent", parent));
    }
    entry.push(("done", "0"));
    append(sink, space, &extended, (W15, "w15", "commentEx"), &entry)?;
    let durable = new_durable_id(sink, space, main)?;
    let ids = ensure_part(sink, main, Kind::Ids)?;
    append(
        sink,
        space,
        &ids,
        (W16CID, "w16cid", "commentId"),
        &[("paraId", &last_para), ("durableId", &durable)],
    )?;
    let extensible = ensure_part(sink, main, Kind::Extensible)?;
    append(
        sink,
        space,
        &extensible,
        (W16CEX, "w16cex", "commentExtensible"),
        &[("durableId", &durable), ("dateUtc", &who.date)],
    )?;
    Ok((durable, Parts { comments }))
}

/// Every comment given the entries it lacks in the modern parts, and every comment part
/// written (`_complete_comment_parts`, then each part marked dirty).
///
/// # Errors
/// When a part is not XML.
pub(crate) fn complete(sink: &mut EditSink<'_, '_>, space: &mut Space, main: &str, comments: &str) -> Result<()> {
    let extended = ensure_part(sink, main, Kind::Extended)?;
    let ids = ensure_part(sink, main, Kind::Ids)?;
    let extensible = ensure_part(sink, main, Kind::Extensible)?;
    let doc = sink.doc().clone();
    for part in [comments, extended.as_str(), ids.as_str(), extensible.as_str()] {
        space.work(&doc, part)?;
    }
    let have_ex: HashSet<String> = entries_of(space, &extended, W15, "commentEx").into_iter().collect();
    let mut have_ids = durable_ids(space, &ids);
    let mut have_cex: HashSet<String> = match space.get(&extensible) {
        Some(work) => work
            .children(work.root())
            .iter()
            .map(|e| work.get(*e, Some(W16CEX), "durableId").unwrap_or("").to_uppercase())
            .collect(),
        None => HashSet::new(),
    };
    let listed: Vec<(String, Option<String>)> = {
        let work = space
            .get(comments)
            .ok_or_else(|| EditError::no_part(comments.to_owned()))?;
        work.children(work.root())
            .iter()
            .filter(|c| work.is(**c, W, "comment"))
            .map(|c| (last_para(work, *c), work.get(*c, Some(W), "date").map(str::to_owned)))
            .collect()
    };
    for (para, date) in listed {
        if para.is_empty() {
            continue;
        }
        let key = para.to_uppercase();
        if !have_ex.contains(&key) {
            append(
                sink,
                space,
                &extended,
                (W15, "w15", "commentEx"),
                &[("paraId", &para), ("done", "0")],
            )?;
        }
        // a commentId without a durableId is none: a new one is made
        let durable = match have_ids.get(&key).cloned().flatten() {
            Some(found) => found,
            None => {
                let made = new_durable_id(sink, space, main)?;
                append(
                    sink,
                    space,
                    &ids,
                    (W16CID, "w16cid", "commentId"),
                    &[("paraId", &para), ("durableId", &made)],
                )?;
                have_ids.insert(key, Some(made.clone()));
                made
            }
        };
        if let Some(date) = date.filter(|d| !d.is_empty()) {
            if !have_cex.contains(&durable.to_uppercase()) {
                append(
                    sink,
                    space,
                    &extensible,
                    (W16CEX, "w16cex", "commentExtensible"),
                    &[("durableId", &durable), ("dateUtc", &date)],
                )?;
                have_cex.insert(durable.to_uppercase());
            }
        }
    }
    for part in [comments, extended.as_str(), ids.as_str(), extensible.as_str()] {
        space.mark_dirty(part);
    }
    Ok(())
}

/// Each `commentsIds` entry's durable id, by its upper-case `paraId`.
fn durable_ids(space: &Space, ids: &str) -> HashMap<String, Option<String>> {
    let mut out = HashMap::new();
    if let Some(work) = space.get(ids) {
        for e in work.children(work.root()) {
            if work.is(*e, W16CID, "commentId") {
                let para = work.get(*e, Some(W16CID), "paraId").unwrap_or("").to_uppercase();
                out.insert(para, work.get(*e, Some(W16CID), "durableId").map(str::to_owned));
            }
        }
    }
    out
}

/// The upper-case `paraId`s of the `local` entries of `part` (none: no copy).
fn entries_of(space: &Space, part: &str, ns: &str, local: &str) -> Vec<String> {
    let Some(work) = space.get(part) else { return Vec::new() };
    work.children(work.root())
        .iter()
        .filter(|e| work.is(**e, ns, local))
        .map(|e| work.get(*e, Some(ns), "paraId").unwrap_or("").to_uppercase())
        .collect()
}

/// `text`'s paragraphs written into the comment `node` (`_comment_paragraphs`); the last
/// one's paraId.
fn paragraphs(sink: &mut EditSink<'_, '_>, space: &mut Space, part: &str, node: Id, text: &str) -> Result<String> {
    check_text(&text.replace('\n', " "))?;
    let style = crate::builtin::ensure(sink, "annotation text", "paragraph")?;
    let reference = crate::builtin::ensure(sink, "annotation reference", "character")?;
    let seed = format!("{part}\0comment");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut last = String::new();
    for (k, line) in lines.iter().enumerate() {
        let drawn = sink.draw(&seed, 2)?;
        let (Some(para), Some(text_id)) = (drawn.first(), drawn.get(1)) else {
            return Err(EditError::no_element(part.to_owned()));
        };
        let work = space.get_mut(part).ok_or_else(|| EditError::no_part(part.to_owned()))?;
        let paragraph = comment_paragraph(work, node, (para, text_id), (&style, &reference), (k == 0, line));
        work.append(node, paragraph);
        last.clone_from(para);
    }
    Ok(last)
}

/// An entry `local` with `attributes` (in `ns`, prefixed as the root has it, else
/// `prefix`) appended to `part`'s root (`etree.SubElement`).
pub(crate) fn append(
    sink: &mut EditSink<'_, '_>,
    space: &mut Space,
    part: &str,
    (ns, prefix, local): (&str, &str, &str),
    attributes: &[(&str, &str)],
) -> Result<()> {
    let doc = sink.doc().clone();
    let work = space
        .work(&doc, part)?
        .ok_or_else(|| EditError::no_part(part.to_owned()))?;
    let root = work.root();
    let element = work.new_element(work.element_name(root, ns, local, prefix));
    for (name, value) in attributes {
        let name = work.qname(ns, name, prefix);
        work.set(element, name, value);
    }
    work.append(root, element);
    Ok(())
}

/// A durable id no comment has (`_new_durable_id`): the next of the sequence seeded by
/// how many the ids part holds.
fn new_durable_id(sink: &mut EditSink<'_, '_>, space: &mut Space, main: &str) -> Result<String> {
    let mut used: HashSet<u32> = HashSet::new();
    let mut count = 0_usize;
    let part = super::part_of(sink.doc().package(), main, super::REL_IDS);
    if let Some(part) = part {
        let doc = sink.doc().clone();
        if let Some(work) = space.work(&doc, &part)? {
            let mut seen: HashSet<u64> = HashSet::new();
            for e in work.children(work.root()) {
                let value = work.get(*e, Some(W16CID), "durableId").unwrap_or("").trim();
                if let Ok(v) = u64::from_str_radix(value, 16) {
                    if seen.insert(v) {
                        count += 1;
                    }
                    if let Ok(v) = u32::try_from(v) {
                        used.insert(v);
                    }
                }
            }
        }
    }
    let seed = format!("durable\0{count}");
    let mut pool = IdPool::of(used);
    pool.generate(&seed, 1)
        .into_iter()
        .next()
        .ok_or_else(|| EditError::Refused("no durable id left".to_owned()))
}

/// The `w14:paraId` of the comment's last paragraph (any depth), as written.
pub(crate) fn last_para(work: &Work, comment: Id) -> String {
    work.iter(comment, W, &["p"])
        .last()
        .and_then(|p| work.get(*p, Some(W14), "paraId"))
        .unwrap_or("")
        .to_owned()
}
