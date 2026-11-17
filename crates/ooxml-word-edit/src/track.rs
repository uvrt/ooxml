//! Owns: text edits in tracking mode, as docx-agent's `revisions/track.py` writes them
//! (measured on Word 16.106 there): deleted text is its runs inside `w:del`, `w:t` turned
//! `w:delText`; deleting one's own insertion removes it outright; inserted text is a run
//! of its own inside `w:ins` beside the text it follows, joining one's own insertion and
//! splitting another author's; a replacement is the deletion, then the insertion, of each
//! changed stretch at word boundaries ([`replace_span`]).
//!
//! Must not: decide whether an edit is tracked (the revision policy does), allocate
//! revision ids other than through its [`Stamp`], or read outside the paragraph.

use crate::inline::{check_span, isolate, position, prune, relationship_ids};
use crate::names::{R, W};
use crate::policy::Stamp;
use crate::text::diff::{by_words, coarse, Tag};
use crate::text::write::{make_run, run_properties_for};
use crate::text::{atoms, Atom, AtomKind, OBJECT};
use crate::work::{Id, Work};
use crate::EditError;

const DELETED: &[&str] = &["del", "moveFrom"];
const CONTAINERS: &[&str] = &["ins", "moveTo", "del", "moveFrom"];

fn is_container(work: &Work, node: Id) -> bool {
    work.is_any(node, W, CONTAINERS)
}

/// The innermost revision container `node` is in, below its paragraph.
fn revision_owner(work: &Work, node: Id) -> Option<Id> {
    let mut parent = work.parent(node);
    while let Some(p) = parent {
        if work.is(p, W, "p") {
            return None;
        }
        if is_container(work, p) {
            return Some(p);
        }
        parent = work.parent(p);
    }
    None
}

fn same_container(work: &Work, left: Id, right: Id) -> bool {
    work.name(left)
        .is_some_and(|l| work.name(right).is_some_and(|r| l == r))
        && work.get(left, Some(W), "author") == work.get(right, Some(W), "author")
        && work.get(left, Some(W), "date") == work.get(right, Some(W), "date")
}

/// Merge `container` into an adjacent revision container of the same kind, author and
/// date; the survivor.
fn join_neighbours(work: &mut Work, mut container: Id) -> Id {
    if let Some(previous) = work.previous(container) {
        if same_container(work, previous, container) && work.tail(previous).trim().is_empty() {
            for child in work.children(container).to_vec() {
                work.append(previous, child);
            }
            work.remove(container);
            container = previous;
        }
    }
    if let Some(following) = work.next(container) {
        if same_container(work, container, following) {
            for child in work.children(following).to_vec() {
                work.append(container, child);
            }
            work.remove(following);
        }
    }
    container
}

/// A deleted run's text spelled as deleted text.
fn spell_deleted(work: &mut Work, run: Id) {
    for child in work.children(run).to_vec() {
        let to = if work.is(child, W, "t") {
            "delText"
        } else if work.is(child, W, "instrText") {
            "delInstrText"
        } else {
            continue;
        };
        let name = work.qname(W, to, "w");
        work.rename(child, name);
    }
}

/// Track the deletion of `runs` (in document order); the relationship ids of what was
/// removed outright.
fn delete_runs(work: &mut Work, runs: &[Id], stamp: &mut Stamp) -> Vec<String> {
    let mut released = Vec::new();
    let mut current: Option<Id> = None;
    for &run in runs {
        if work.parent(run).is_none() {
            continue;
        }
        let owner = revision_owner(work, run);
        if owner.is_some_and(|o| work.is_any(o, W, DELETED)) {
            continue;
        }
        if owner.is_some_and(|o| work.is(o, W, "ins") && stamp.same(work, o)) {
            released.extend(relationship_ids(work, run));
            let mut parent = work.parent(run);
            work.remove(run);
            while let Some(p) = parent {
                if work.is(p, W, "p") || work.children(p).iter().any(|c| work.is_element(*c)) {
                    break;
                }
                if let Some(rid) = work.get(p, Some(R), "id").filter(|r| !r.is_empty()) {
                    released.push(rid.to_owned());
                }
                let grand = work.parent(p);
                work.remove(p);
                parent = grand;
            }
            current = None;
            continue;
        }
        spell_deleted(work, run);
        if let Some(c) = current.filter(|c| work.previous(run) == Some(*c)) {
            work.append(c, run);
            continue;
        }
        let made = stamp.make(work, "del");
        work.add_previous(run, made);
        work.append(made, run);
        join_neighbours(work, made);
        current = work.parent(made).is_some().then_some(made);
    }
    released
}

/// Track the deletion of the current view's characters `[start, end)`.
fn delete_span(
    work: &mut Work,
    paragraph: Id,
    start: usize,
    end: usize,
    stamp: &mut Stamp,
) -> Result<Vec<String>, EditError> {
    if start >= end {
        return Ok(Vec::new());
    }
    let runs = isolate(work, paragraph, start, end)?;
    Ok(delete_runs(work, &runs, stamp))
}

/// Split a revision container before its `index`-th child: the rest goes into a copy after
/// it, a revision with an id of its own.
fn split_container(work: &mut Work, container: Id, index: usize, stamp: &mut Stamp) -> Id {
    let rest = work.shallow_copy(container);
    let id = stamp.next_id();
    let name = work.qname(W, "id", "w");
    work.set(rest, name, &id);
    for child in work.children(container).get(index..).unwrap_or_default().to_vec() {
        work.append(rest, child);
    }
    work.add_next(container, rest);
    rest
}

/// Track the insertion of run-level `nodes` at `parent[index]`.
fn insert_nodes(work: &mut Work, mut parent: Id, mut index: usize, nodes: &[Id], stamp: &mut Stamp) {
    let owner_of = |work: &Work, p: Id| {
        if is_container(work, p) {
            Some(p)
        } else {
            revision_owner(work, p)
        }
    };
    let mut owner = owner_of(work, parent);
    if owner.is_some_and(|o| work.is(o, W, "ins") && stamp.same(work, o)) {
        for (k, node) in nodes.iter().enumerate() {
            work.insert(parent, index + k, *node);
        }
        return;
    }
    while let Some(o) = owner {
        if parent != o {
            break; // inside something (a hyperlink) the container owns: the insertion goes there
        }
        let length = work.children(parent).len();
        if 0 < index && index < length {
            let rest = split_container(work, o, index, stamp);
            parent = work.parent(rest).unwrap_or(parent);
            index = work.index(rest).unwrap_or(0);
        } else {
            let at = work.index(o).unwrap_or(0);
            parent = work.parent(o).unwrap_or(parent);
            index = at + usize::from(index >= length);
        }
        owner = owner_of(work, parent);
    }
    let container = stamp.make(work, "ins");
    work.insert(parent, index, container);
    for node in nodes {
        work.append(container, *node);
    }
    join_neighbours(work, container);
}

/// The run new text at `offset` takes its formatting from.
fn reference_run(items: &[Atom], offset: usize) -> Option<Id> {
    let before = offset.checked_sub(1).and_then(|k| items.get(k));
    let after = items.get(offset);
    let tests: [fn(&Atom) -> bool; 3] = [Atom::plain, |a| a.kind == AtomKind::Text, |_| true];
    for test in tests {
        for atom in [before, after].into_iter().flatten() {
            if atom.run.is_some() && test(atom) {
                return atom.run;
            }
        }
    }
    None
}

/// Track the insertion of `text` at the current view's `offset`, formatted as `reference`;
/// the new run.
fn insert_text(
    work: &mut Work,
    paragraph: Id,
    offset: usize,
    text: &str,
    stamp: &mut Stamp,
    reference: Option<Id>,
) -> Result<Id, EditError> {
    let reference = reference.or_else(|| reference_run(&atoms(work, paragraph), offset));
    let properties = run_properties_for(work, paragraph, reference);
    let run = make_run(work, text, properties);
    let (parent, index) = position(work, paragraph, offset)?;
    insert_nodes(work, parent, index, &[run], stamp);
    Ok(run)
}

/// A run's properties as formatting: without revision marks and change record, compared
/// as canonical XML compares them.
fn look(work: &Work, run: Id) -> String {
    let mut out = String::new();
    if let Some(properties) = work.find(run, W, "rPr") {
        for child in work.children(properties) {
            let skip = work.is_any(*child, W, &["ins", "del", "moveFrom", "moveTo", "rPrChange"]);
            if work.is_element(*child) && !skip {
                canonical(work, *child, &mut out);
                out.push_str(work.tail(*child));
            }
        }
    }
    out
}

/// An element in a form equal for equal canonical XML: names by namespace, attributes in
/// sorted order, text and tails, comments left out.
fn canonical(work: &Work, node: Id, out: &mut String) {
    let Some(name) = work.name(node) else { return };
    out.push_str(&format!("<{{{}}}{}", name.namespace().unwrap_or(""), name.local()));
    let mut attributes: Vec<(String, String, String)> = work
        .attributes(node)
        .iter()
        .map(|a| {
            let ns = a.name.namespace().unwrap_or("").to_owned();
            (ns, a.name.local().to_owned(), a.value.to_string())
        })
        .collect();
    attributes.sort();
    for (ns, local, value) in attributes {
        out.push_str(&format!(" {{{ns}}}{local}={value:?}"));
    }
    out.push('>');
    out.push_str(work.text(node));
    for child in work.children(node) {
        canonical(work, *child, out);
        out.push_str(work.tail(*child));
    }
    out.push_str("</>");
}

fn same_look(work: &Work, one: Option<Id>, other: Option<Id>) -> bool {
    match (one, other) {
        (Some(a), Some(b)) => a == b || look(work, a) == look(work, b),
        (None, None) => true,
        _ => false,
    }
}

/// The text replacing `[a, b)`, in pieces each with the run whose formatting it takes, as
/// the untracked edit would give each character.
fn pieces(work: &Work, items: &[Atom], a: usize, b: usize, text: &[char]) -> Vec<(String, Option<Id>)> {
    let old: Vec<char> = items.get(a..b).unwrap_or_default().iter().map(|x| x.ch).collect();
    let run_at = |k: usize| items.get(k).and_then(|x| x.run);
    let mut sources: Vec<Option<Id>> = Vec::new();
    for op in coarse(&old, text) {
        let count = op.j2 - op.j1;
        if op.tag == Tag::Equal {
            sources.extend((0..count).map(|k| run_at(a + op.i1 + k)));
            continue;
        }
        let k = a + op.i1;
        let source = if op.i2 > op.i1 && items.get(k).is_some_and(Atom::plain) {
            run_at(k)
        } else if k > 0 && run_at(k - 1).is_some() {
            run_at(k - 1)
        } else {
            run_at(a)
        };
        sources.extend(std::iter::repeat_n(source, count));
    }
    let mut out: Vec<(String, Option<Id>)> = Vec::new();
    for (ch, source) in text.iter().zip(sources) {
        match out.last_mut() {
            Some(last) if same_look(work, last.1, source) => last.0.push(*ch),
            _ => out.push((ch.to_string(), source)),
        }
    }
    out
}

/// The insertion of a replacement moved after the deletions it replaces.
fn after_deletion(work: &mut Work, run: Id, made: &[Id]) {
    let Some(container) = work.parent(run).filter(|c| work.is(*c, W, "ins")) else {
        return;
    };
    if made.is_empty() {
        return;
    }
    let mut following = work.next(container);
    let mut last = None;
    while let Some(f) = following.filter(|f| work.is(*f, W, "del") && made.contains(f)) {
        last = Some(f);
        following = work.next(f);
    }
    if let Some(last) = last {
        work.add_next(last, container);
    }
}

/// Tracked replacement of the current view's characters `[start, end)` by `new`, each
/// changed stretch at word boundaries deleted then inserted; the relationship ids of what
/// was removed outright.
pub(crate) fn replace_span(
    work: &mut Work,
    paragraph: Id,
    start: usize,
    end: usize,
    new: &str,
    stamp: &mut Stamp,
) -> Result<Vec<String>, EditError> {
    let items = atoms(work, paragraph);
    check_span(work, &items, start, end, true)?;
    if new.contains(OBJECT) {
        return Err(EditError::Span("new text cannot create an object (U+FFFC)".to_owned()));
    }
    let old: Vec<char> = items.get(start..end).unwrap_or_default().iter().map(|a| a.ch).collect();
    let new: Vec<char> = new.chars().collect();
    if old == new {
        return Ok(Vec::new());
    }
    let stretches: Vec<(usize, usize, Vec<char>)> = if old.is_empty() {
        vec![(start, end, new.clone())]
    } else {
        by_words(&old, &new)
            .into_iter()
            .filter(|op| op.tag != Tag::Equal)
            .map(|op| {
                (
                    start + op.i1,
                    start + op.i2,
                    new.get(op.j1..op.j2).unwrap_or_default().to_vec(),
                )
            })
            .collect()
    };
    let mut released = Vec::new();
    for (a, b, text) in stretches.into_iter().rev() {
        let items = atoms(work, paragraph);
        let mut reference = None;
        if b > a {
            reference = match items.get(a) {
                Some(first) if first.kind == AtomKind::Text => first.run,
                _ => reference_run(&items, a),
            };
        }
        if old.is_empty()
            && a > 0
            && items.get(a).is_some_and(Atom::plain)
            && !items.get(a - 1).is_some_and(Atom::plain)
        {
            reference = items.get(a).and_then(|x| x.run);
        }
        let pieces = if b > a {
            pieces(work, &items, a, b, &text)
        } else {
            Vec::new()
        };
        let before = work.iter(paragraph, W, &["del"]);
        released.extend(delete_span(work, paragraph, a, b, stamp)?);
        if text.is_empty() {
            continue;
        }
        if reference.is_none_or(|r| work.parent(r).is_none()) {
            reference = reference_run(&atoms(work, paragraph), a);
        }
        let run = if pieces.len() > 1 {
            let mut runs = Vec::new();
            for (piece, source) in &pieces {
                let properties = run_properties_for(work, paragraph, *source);
                runs.push(make_run(work, piece, properties));
            }
            let (parent, index) = position(work, paragraph, a)?;
            insert_nodes(work, parent, index, &runs, stamp);
            runs.first().copied().unwrap_or(paragraph)
        } else {
            let text: String = text.iter().collect();
            insert_text(work, paragraph, a, &text, stamp, reference)?
        };
        let made: Vec<Id> = work
            .iter(paragraph, W, &["del"])
            .into_iter()
            .filter(|d| !before.contains(d))
            .collect();
        after_deletion(work, run, &made);
    }
    released.extend(prune(work, paragraph));
    Ok(released)
}
