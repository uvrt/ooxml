//! Owns: docx-agent's problems of a version read from its [`Tally`] -- the same rules as the
//! from-scratch fold ([`super::rules`], the oracle), answered from the counts and sets the
//! tally keeps in place, the per-part aggregates of its trees, and the small parts read
//! whole (numbering, styles, comments, the package's own records).
//!
//! Must not: read a block's XML, or walk a part's blocks except for the rare rule that
//! needs their order (move ranges, in a part that has some).

use super::counts::Notes;
use super::rules::{
    bookmark_name_ok as name_ok, comment_parts_with, move_problems, num_ref_problems, numbering_tables,
    package_problems, problem, py_str, style_part_problems, style_table,
};
use super::tally::{Holder, Tally};
use super::Problem;
use ooxml_opc::Package;
use ooxml_word_model::parts::related;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const REL_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const REL_NUMBERING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";

/// `[^\W\d][\w]{0,39}`, whole (a bookmark name Word keeps).
pub(crate) fn bookmark_name_ok(name: &str) -> bool {
    name_ok(name)
}

/// Every problem of the version the tally reads, sorted.
pub(crate) fn problems(tally: &Tally, package: &Package) -> Vec<Problem> {
    let mut out = Vec::new();
    for (name, message) in &tally.unreadable {
        out.push(problem("not-well-formed", name, message.clone()));
    }
    out.extend(package_problems(package));
    word_parts(&mut out, tally);
    ranges(&mut out, tally);
    references(&mut out, tally, package);
    numbering(&mut out, tally, package);
    styles(&mut out, tally, package);
    tracking(&mut out, tally);
    let roots = |rel: &str| {
        related(package, &tally.main, rel)
            .ok()
            .flatten()
            .and_then(|p| tally.parts.get(p.as_str()).map(|t| Arc::clone(&t.root)))
    };
    let references: HashSet<Option<String>> = tally
        .counts
        .comment_references
        .iter()
        .map(|(id, _)| id.as_deref().map(str::to_owned))
        .collect();
    comment_parts_with(&mut out, &tally.main, roots, &references);
    if let Some(main) = tally.parts.get(&tally.main) {
        let sum = main.items.summary();
        let code = match main.holder {
            Holder::Body(_) if sum.sects > 0 && sum.last_is_sect == Some(false) => Some("body-section-not-last"),
            Holder::Body(_) if sum.sects > 1 => Some("body-has-two-sections"),
            Holder::Body(_) | Holder::Root => None,
            Holder::Whole => Some("no-body"),
        };
        if let Some(code) = code {
            out.push(problem(code, &tally.main, ""));
        }
    }
    out.sort();
    out
}

/// The rules each Word part is checked by alone.
fn word_parts(out: &mut Vec<Problem>, tally: &Tally) {
    for ((part, code, detail), count) in tally.counts.locals.iter() {
        for _ in 0..*count {
            out.push(problem(code, part, &**detail));
        }
    }
    for ((part, raw), ()) in tally.counts.paraids_repeated.iter() {
        out.push(problem("paraid-repeated", part, &**raw));
    }
    for (name, part) in &tally.parts {
        if !part.role.word {
            continue;
        }
        if let (Some(violation), Some(container)) = (part.items.summary().violation, &part.container) {
            out.push(problem("child-order", name, format!("{violation} in {container}")));
        }
        let sum = part.sum();
        if sum.field_low < 0 {
            out.push(problem("field-unbalanced", name, "end without begin"));
        } else if sum.field_net != 0 {
            out.push(problem("field-unbalanced", name, "begin without end"));
        }
        let root = &part.root;
        for prefix in root
            .attribute(Some(crate::names::MC), "Ignorable")
            .unwrap_or("")
            .split_whitespace()
            .filter(|p| !root.namespaces().iter().any(|d| d.prefix.as_deref() == Some(*p)))
        {
            out.push(problem("ignorable-prefix-undeclared", name, prefix));
        }
    }
}

/// Bookmarks, comment ranges, note and comment references.
fn ranges(out: &mut Vec<Problem>, tally: &Tally) {
    let main = &*tally.main;
    let counts = &tally.counts;
    for (mark, ()) in counts.bookmarks_unpaired.iter() {
        out.push(problem("bookmark-unpaired", main, &**mark));
    }
    for (name, ()) in counts.bookmark_names_invalid.iter() {
        out.push(problem("bookmark-name-invalid", main, &**name));
    }
    for (name, ()) in counts.bookmark_folded_repeated.iter() {
        out.push(problem("bookmark-name-repeated", main, &**name));
    }
    for (mark, ()) in counts.comment_ranges_unpaired.iter() {
        out.push(problem("comment-range-unpaired", main, py_str(mark.as_deref())));
    }
    for ((kind, mark), ()) in counts.references_missing.iter() {
        let code = match kind {
            Notes::Comments => "comment-reference-without-comment",
            Notes::Footnotes => "footnote-reference-without-note",
            Notes::Endnotes => "endnote-reference-without-note",
        };
        out.push(problem(code, main, py_str(mark.as_deref())));
    }
}

/// Every relationship-namespace attribute names a relationship of its own part.
fn references(out: &mut Vec<Problem>, tally: &Tally, package: &Package) {
    let mut known: HashMap<&str, HashSet<String>> = HashMap::new();
    for ((part, value), count) in tally.counts.r_values.iter() {
        if value.is_empty() {
            continue;
        }
        let ids = known.entry(part).or_insert_with(|| {
            package
                .relationships(part)
                .map(|r| r.iter().map(|x| x.id.clone()).collect())
                .unwrap_or_default()
        });
        if !ids.contains(&**value) {
            for _ in 0..*count {
                out.push(problem("relationship-id-unknown", part, &**value));
            }
        }
    }
}

/// List references against the numbering part.
fn numbering(out: &mut Vec<Problem>, tally: &Tally, package: &Package) {
    let part = related(package, &tally.main, REL_NUMBERING).ok().flatten();
    let root = part
        .as_deref()
        .and_then(|p| tally.parts.get(p))
        .map(|t| Arc::clone(&t.root));
    let nums = numbering_tables(out, part.as_deref(), root.as_ref());
    for ((name, reference), count) in tally.counts.num_refs.iter() {
        num_ref_problems(out, name, reference, &nums, *count);
    }
}

/// Style references against the styles part; the styles' own references and cycles.
fn styles(out: &mut Vec<Problem>, tally: &Tally, package: &Package) {
    let Some(part) = related(package, &tally.main, REL_STYLES).ok().flatten() else {
        return;
    };
    let Some(styles) = tally.parts.get(part.as_str()) else {
        return;
    };
    let by_id = style_table(&styles.root);
    let known: HashSet<Option<&str>> = by_id.iter().map(|(k, _)| k.as_deref()).collect();
    for ((name, tag, value), count) in tally.counts.style_refs.iter() {
        if known.contains(&value.as_deref()) {
            continue;
        }
        for _ in 0..*count {
            out.push(problem(
                "style-missing",
                name,
                format!("{tag} {}", py_str(value.as_deref())),
            ));
        }
    }
    style_part_problems(out, &part, &by_id);
}

/// Revision records: an author each, ids unique across stories; moves paired; drawing
/// numbers unique.
fn tracking(out: &mut Vec<Problem>, tally: &Tally) {
    let counts = &tally.counts;
    for ((part, kind), count) in counts.without_author.iter() {
        for _ in 0..*count {
            out.push(problem("revision-without-author", part, *kind));
        }
    }
    for (id, ()) in counts.record_ids_repeated.iter() {
        out.push(problem("revision-id-repeated", &tally.main, &**id));
    }
    for (id, ()) in counts.docpr_repeated.iter() {
        out.push(problem("docpr-id-repeated", &tally.main, &**id));
    }
    for (name, count) in counts.moves.iter() {
        if *count == 0 {
            continue;
        }
        if let Some(part) = tally.parts.get(name) {
            move_problems(out, name, part.all());
        }
    }
}

/// The largest annotation id in use in any story, `-1` when none is.
pub(crate) fn annotation_ceiling(tally: &Tally) -> i64 {
    tally
        .stories
        .iter()
        .filter_map(|s| tally.parts.get(s))
        .filter_map(|p| p.sum().annotation_max)
        .max()
        .unwrap_or(-1)
}

/// Whether the part `part` has a live drawing without its anchorId and editId.
pub(crate) fn unstamped_drawings(tally: &Tally, part: &str) -> bool {
    tally.parts.get(part).is_some_and(|p| p.sum().unstamped > 0)
}
