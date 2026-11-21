//! Owns: docx-agent's validation rules (`validate.py`, `check`) over a version's facts read
//! from scratch -- the oracle the kept rules ([`super::kept`]) are tested against, and the
//! rules both share:
//! each part's facts ([`PartFacts`]: its blocks' memoised facts and its root's own), folded
//! into the problems the whole-document rules find -- repeated paraIds, unbalanced
//! fields, unpaired bookmarks and comment ranges, references without notes or comments,
//! unknown relationship ids, missing lists and styles, repeated drawing numbers, revision
//! records without authors or with repeated ids, unpaired moves, comment parts that
//! disagree -- plus the package's own (content types, dangling relationships, orphans).
//!
//! Must not: read a block's XML (its facts are memoised by content), or report in another
//! form than docx-agent's (`[code] part: detail`, sorted).

use crate::names::{W, W15};
use crate::validate::facts::{python_int, ElementFacts, Local};
use crate::validate::Problem;
use ooxml_opc::{name::normalize, Package, CONTENT_TYPES_PART};
use ooxml_word_model::parts::{related, REL_COMMENTS, REL_COMMENTS_EXTENDED, REL_COMMENTS_IDS};
use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

const W16CID: &str = "http://schemas.microsoft.com/office/word/2016/wordml/cid";
const W16CEX: &str = "http://schemas.microsoft.com/office/word/2018/wordml/cex";
const REL_EXTENSIBLE: &str = "http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible";
const REL_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const REL_NUMBERING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";

/// One XML part's facts: its blocks' (each memoised by content) and its root's own.
#[derive(Debug)]
pub(crate) struct PartFacts {
    /// The blocks' facts, in order (a whole non-story part is one block).
    pub(crate) blocks: Vec<Arc<ElementFacts>>,
    /// The root's own facts (its head, its and its body's child order).
    pub(crate) rest: ElementFacts,
    /// The root (for the rules that read a small part whole).
    pub(crate) root: Arc<Element>,
    /// The main part's body problem, if any.
    pub(crate) body: Option<&'static str>,
}

impl PartFacts {
    fn all(&self) -> impl Iterator<Item = &ElementFacts> {
        self.blocks.iter().map(|b| &**b).chain(std::iter::once(&self.rest))
    }
}

/// The body problem of a main part's root: no body, a section break not last, two.
pub(crate) fn body_problem(root: &Element) -> Option<&'static str> {
    let Some(body) = root.find(Some(W), "body") else {
        return Some("no-body");
    };
    let elements: Vec<&Arc<Element>> = body.child_elements().collect();
    let sections: Vec<&&Arc<Element>> = elements.iter().filter(|e| e.name().is(Some(W), "sectPr")).collect();
    if let (Some(last), Some(section)) = (elements.last(), sections.last()) {
        if !Arc::ptr_eq(last, section) {
            return Some("body-section-not-last");
        }
    }
    (sections.len() > 1).then_some("body-has-two-sections")
}

/// What the rules read besides the parts' facts.
pub(crate) struct Inputs<'a> {
    pub(crate) package: &'a Package,
    pub(crate) main: &'a str,
    /// The main part, then the story parts, in relationship order.
    pub(crate) stories: &'a [String],
    /// Every XML part read, by name, in package order.
    pub(crate) parts: &'a [(String, Result<Arc<PartFacts>, String>)],
}

impl Inputs<'_> {
    fn facts(&self, part: &str) -> Option<&PartFacts> {
        self.parts
            .iter()
            .find(|(n, _)| n == part)
            .and_then(|(_, f)| f.as_ref().ok())
            .map(|f| &**f)
    }

    fn story_facts(&self) -> impl Iterator<Item = (&str, &PartFacts)> {
        self.stories
            .iter()
            .filter_map(|s| self.facts(s).map(|f| (s.as_str(), f)))
    }
}

pub(super) fn problem(code: &str, part: &str, detail: impl Into<String>) -> Problem {
    Problem {
        code: code.to_owned(),
        part: part.to_owned(),
        detail: detail.into(),
    }
}

fn locals<'f>(out: &mut Vec<Problem>, part: &str, found: impl Iterator<Item = &'f Local>) {
    out.extend(found.map(|(code, detail)| problem(code, part, detail.clone())));
}

/// Every problem of the version, sorted (docx-agent's `check` without a target).
pub(crate) fn problems(inputs: &Inputs<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for (name, facts) in inputs.parts {
        if let Err(message) = facts {
            out.push(problem("not-well-formed", name, message.clone()));
        }
    }
    out.extend(package_problems(inputs.package));
    for (name, facts) in inputs.parts {
        let Ok(facts) = facts else { continue };
        if !(name.starts_with("word/") && name.ends_with(".xml")) {
            continue;
        }
        word_part(&mut out, name, facts);
    }
    ranges(&mut out, inputs);
    references(&mut out, inputs);
    numbering(&mut out, inputs);
    styles(&mut out, inputs);
    drawings(&mut out, inputs);
    tracking(&mut out, inputs);
    comment_parts(&mut out, inputs);
    if let Some(code) = inputs.facts(inputs.main).and_then(|f| f.body) {
        out.push(problem(code, inputs.main, ""));
    }
    out.sort();
    out
}

/// The rules each Word part is checked by alone.
fn word_part(out: &mut Vec<Problem>, name: &str, facts: &PartFacts) {
    locals(out, name, facts.all().flat_map(|f| f.order.iter()));
    locals(out, name, facts.all().flat_map(|f| f.paraid.iter()));
    let mut counts: HashMap<&str, usize> = HashMap::with_capacity(facts.blocks.len() * 2);
    for raw in facts.all().flat_map(|f| f.para_ids.iter()) {
        *counts.entry(raw).or_insert(0) += 1;
    }
    for (raw, count) in counts {
        if count > 1 {
            out.push(problem("paraid-repeated", name, raw));
        }
    }
    locals(out, name, facts.all().flat_map(|f| f.revision.iter()));
    let mut depth = 0i64;
    let mut unbalanced = None;
    for f in facts.blocks.iter().map(|b| &**b).chain(std::iter::once(&facts.rest)) {
        if depth + f.field_low < 0 {
            unbalanced = Some("end without begin");
            break;
        }
        depth += f.field_net;
    }
    match unbalanced {
        Some(detail) => out.push(problem("field-unbalanced", name, detail)),
        None if depth != 0 => out.push(problem("field-unbalanced", name, "begin without end")),
        None => {}
    }
    locals(out, name, facts.all().flat_map(|f| f.cells.iter()));
    locals(out, name, facts.all().flat_map(|f| f.grids.iter()));
    let listed = facts
        .root
        .attribute(Some(crate::names::MC), "Ignorable")
        .unwrap_or("")
        .split_whitespace()
        .filter(|p| !facts.root.namespaces().iter().any(|d| d.prefix.as_deref() == Some(*p)))
        .map(|p| problem("ignorable-prefix-undeclared", name, p))
        .collect::<Vec<_>>();
    out.extend(listed);
}

/// Bookmarks, comment ranges and references, note references.
fn ranges(out: &mut Vec<Problem>, inputs: &Inputs<'_>) {
    bookmarks(out, inputs);
    marks(out, inputs);
}

/// Bookmarks: starts and ends paired, names valid and not repeated (in any case).
fn bookmarks(out: &mut Vec<Problem>, inputs: &Inputs<'_>) {
    let main = inputs.main;
    let mut starts: HashMap<&str, i64> = HashMap::new();
    let mut ends: HashMap<&str, i64> = HashMap::new();
    let mut names: HashMap<&str, usize> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for (_, part) in inputs.story_facts() {
        for f in part.all() {
            for id in &f.bookmark_starts {
                *starts.entry(id).or_insert(0) += 1;
            }
            for id in &f.bookmark_ends {
                *ends.entry(id).or_insert(0) += 1;
            }
            for name in &f.bookmark_names {
                let count = names.entry(name).or_insert(0);
                if *count == 0 {
                    order.push(name);
                }
                *count += 1;
            }
        }
    }
    let marks: BTreeSet<&str> = starts.keys().chain(ends.keys()).copied().collect();
    for mark in marks {
        if starts.get(mark).copied().unwrap_or(0) != ends.get(mark).copied().unwrap_or(0) {
            out.push(problem("bookmark-unpaired", main, mark));
        }
    }
    let mut folded: HashMap<String, usize> = HashMap::new();
    for name in order {
        let count = names.get(name).copied().unwrap_or(0);
        *folded.entry(ooxml_word_model::docx::casefold(name)).or_insert(0) += count;
        if !bookmark_name_ok(name) {
            out.push(problem("bookmark-name-invalid", main, name));
        }
    }
    for (name, count) in folded {
        if count > 1 {
            out.push(problem("bookmark-name-repeated", main, name));
        }
    }
}

/// Comment ranges paired; comment and note references to what exists.
fn marks(out: &mut Vec<Problem>, inputs: &Inputs<'_>) {
    let main = inputs.main;
    let (mut c_starts, mut c_ends, mut c_refs) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    let (mut f_refs, mut e_refs) = (BTreeSet::new(), BTreeSet::new());
    for (_, part) in inputs.story_facts() {
        for f in part.all() {
            c_starts.extend(f.comment_starts.iter().cloned());
            c_ends.extend(f.comment_ends.iter().cloned());
            c_refs.extend(f.comment_refs.iter().cloned());
            f_refs.extend(f.footnote_refs.iter().cloned());
            e_refs.extend(f.endnote_refs.iter().cloned());
        }
    }
    for mark in c_starts.symmetric_difference(&c_ends) {
        out.push(problem("comment-range-unpaired", main, py_str(mark.as_deref())));
    }
    let notes_in = |local: &str, pick: fn(&ElementFacts) -> &Vec<Option<String>>| {
        let mut found = BTreeSet::new();
        for (_, part) in inputs.story_facts().skip(1) {
            if part.root.name().is(Some(W), local) {
                for f in part.all() {
                    found.extend(pick(f).iter().cloned());
                }
            }
        }
        found
    };
    let checks: [(&str, &Ids, Ids); 3] = [
        (
            "comment-reference-without-comment",
            &c_refs,
            notes_in("comments", |f| &f.comments),
        ),
        (
            "footnote-reference-without-note",
            &f_refs,
            notes_in("footnotes", |f| &f.footnotes),
        ),
        (
            "endnote-reference-without-note",
            &e_refs,
            notes_in("endnotes", |f| &f.endnotes),
        ),
    ];
    for (code, references, targets) in checks {
        for mark in references.difference(&targets) {
            out.push(problem(code, main, py_str(mark.as_deref())));
        }
    }
}

/// Ids of marks (`None`: a mark without one).
type Ids = BTreeSet<Option<String>>;

/// Python's `str()` of an optional string.
pub(super) fn py_str(value: Option<&str>) -> String {
    value.unwrap_or("None").to_owned()
}

/// `[^\W\d][\w]{0,39}`, whole: a letter or underscore, then up to 39 word characters.
pub(super) fn bookmark_name_ok(name: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return false };
    word(first) && !first.is_numeric() && name.chars().count() <= 40 && chars.all(word)
}

/// Every `r:` attribute names a relationship of its own part.
fn references(out: &mut Vec<Problem>, inputs: &Inputs<'_>) {
    for (name, facts) in inputs.parts {
        let Ok(facts) = facts else { continue };
        if !name.ends_with(".xml") || name == CONTENT_TYPES_PART {
            continue;
        }
        let values: Vec<&String> = facts.all().flat_map(|f| f.r_values.iter()).collect();
        if values.is_empty() {
            continue;
        }
        let known: HashSet<String> = inputs
            .package
            .relationships(name)
            .map(|r| r.iter().map(|x| x.id.clone()).collect())
            .unwrap_or_default();
        for value in values {
            if !value.is_empty() && !known.contains(value.as_str()) {
                out.push(problem("relationship-id-unknown", name, value.clone()));
            }
        }
    }
}

/// List references against the numbering part.
fn numbering(out: &mut Vec<Problem>, inputs: &Inputs<'_>) {
    let part = related(inputs.package, inputs.main, REL_NUMBERING).ok().flatten();
    let root = part
        .as_deref()
        .and_then(|p| inputs.facts(p))
        .map(|f| Arc::clone(&f.root));
    let nums = numbering_tables(out, part.as_deref(), root.as_ref());
    let styles = related(inputs.package, inputs.main, REL_STYLES).ok().flatten();
    let mut owners: Vec<(&str, &PartFacts)> = inputs.story_facts().collect();
    if let Some((name, facts)) = styles.as_deref().and_then(|s| inputs.facts(s).map(|f| (s, f))) {
        owners.push((name, facts));
    }
    let nums = num_ids(&nums);
    for (name, facts) in owners {
        for reference in facts.all().flat_map(|f| f.num_refs.iter()) {
            num_ref_problems(out, name, reference, &nums, 1);
        }
    }
}

/// The numbering part's `w:num`s (id and abstract id, the last of an id winning), its
/// own problems pushed (abstracts missing, nsids repeated).
pub(super) fn numbering_tables(
    out: &mut Vec<Problem>,
    part: Option<&str>,
    root: Option<&Arc<Element>>,
) -> Vec<(Option<String>, Option<String>)> {
    let mut nums: Vec<(Option<String>, Option<String>)> = Vec::new();
    let mut abstracts: HashSet<Option<String>> = HashSet::new();
    if let Some(root) = root {
        for e in root.child_elements() {
            if e.name().is(Some(W), "abstractNum") {
                abstracts.insert(e.attribute(Some(W), "abstractNumId").map(str::to_owned));
            } else if e.name().is(Some(W), "num") {
                let id = e.attribute(Some(W), "numId").map(str::to_owned);
                let abstract_id = e
                    .find(Some(W), "abstractNumId")
                    .and_then(|r| r.attribute(Some(W), "val"))
                    .map(str::to_owned);
                match nums.iter_mut().find(|(n, _)| *n == id) {
                    Some(found) => found.1 = abstract_id,
                    None => nums.push((id, abstract_id)),
                }
            }
        }
    }
    let numbering_part = part.unwrap_or("None");
    for (num, abstract_id) in &nums {
        if !abstracts.contains(abstract_id) {
            let detail = format!("{} -> {}", py_str(num.as_deref()), py_str(abstract_id.as_deref()));
            out.push(problem("numbering-abstract-missing", numbering_part, detail));
        }
    }
    if let Some(root) = root {
        let mut nsids: Vec<(String, usize)> = Vec::new();
        let mut stack = vec![Arc::clone(root)];
        let mut found = Vec::new();
        while let Some(e) = stack.pop() {
            if e.name().is(Some(W), "nsid") {
                found.push(e.attribute(Some(W), "val").unwrap_or("").to_uppercase());
            }
            let children: Vec<Arc<Element>> = e.child_elements().cloned().collect();
            stack.extend(children.into_iter().rev());
        }
        for nsid in found {
            match nsids.iter_mut().find(|(n, _)| *n == nsid) {
                Some(f) => f.1 += 1,
                None => nsids.push((nsid, 1)),
            }
        }
        for (nsid, count) in nsids {
            if count > 1 {
                out.push(problem("numbering-nsid-repeated", numbering_part, nsid));
            }
        }
    }
    nums
}

/// The `w:num` ids of `nums` ([`numbering_tables`]), to look references up in.
pub(super) fn num_ids(nums: &[(Option<String>, Option<String>)]) -> HashSet<&str> {
    nums.iter().filter_map(|(n, _)| n.as_deref()).collect()
}

/// The problems of one list reference (`count` of them) of the part `name`; `nums`, the
/// numbering part's `w:num` ids ([`num_ids`]).
pub(super) fn num_ref_problems(
    out: &mut Vec<Problem>,
    name: &str,
    (num, level): &crate::validate::facts::NumRef,
    nums: &HashSet<&str>,
    count: usize,
) {
    for _ in 0..count {
        if let Some(value) = num {
            let missing = value.as_deref().is_some_and(|v| v != "0" && !nums.contains(v));
            if missing {
                out.push(problem("numbering-num-missing", name, py_str(value.as_deref())));
            }
        }
        if let Some(level) = level {
            let ok = level
                .as_deref()
                .and_then(python_int)
                .is_some_and(|v| (0..=8).contains(&v));
            if !ok {
                out.push(problem("numbering-level-out-of-range", name, py_str(level.as_deref())));
            }
        }
    }
}

/// Style references against the styles part; the styles' own references and cycles.
fn styles(out: &mut Vec<Problem>, inputs: &Inputs<'_>) {
    let Some(part) = related(inputs.package, inputs.main, REL_STYLES).ok().flatten() else {
        return;
    };
    let Some(facts) = inputs.facts(&part) else { return };
    let by_id = style_table(&facts.root);
    let known = |v: Option<&str>| by_id.iter().any(|(k, _)| k.as_deref() == v);
    for (name, story) in inputs.story_facts() {
        for tag in ["pStyle", "rStyle", "tblStyle"] {
            for (t, value) in story.all().flat_map(|f| f.style_refs.iter()) {
                if *t == tag && !known(value.as_deref()) {
                    out.push(problem(
                        "style-missing",
                        name,
                        format!("{tag} {}", py_str(value.as_deref())),
                    ));
                }
            }
        }
    }
    style_part_problems(out, &part, &by_id);
}

/// The styles part's `w:style`s by id (the last of an id winning, in the place of the
/// first).
pub(super) fn style_table(root: &Element) -> Vec<(Option<String>, Arc<Element>)> {
    let mut by_id: Vec<(Option<String>, Arc<Element>)> = Vec::new();
    let mut at: HashMap<Option<String>, usize> = HashMap::new();
    for e in root.child_elements().filter(|e| e.name().is(Some(W), "style")) {
        let id = e.attribute(Some(W), "styleId").map(str::to_owned);
        match at.get(&id).and_then(|k| by_id.get_mut(*k)) {
            Some(found) => found.1 = Arc::clone(e),
            None => {
                at.insert(id.clone(), by_id.len());
                by_id.push((id, Arc::clone(e)));
            }
        }
    }
    by_id
}

/// The styles' own references and cycles.
pub(super) fn style_part_problems(out: &mut Vec<Problem>, part: &str, by_id: &[(Option<String>, Arc<Element>)]) {
    let index: HashMap<Option<&str>, &Arc<Element>> = by_id.iter().map(|(k, e)| (k.as_deref(), e)).collect();
    let known = |v: Option<&str>| index.contains_key(&v);
    let get = |id: Option<&str>| index.get(&id).copied();
    for (style_id, node) in by_id {
        for tag in ["basedOn", "next", "link"] {
            if let Some(reference) = node.find(Some(W), tag) {
                let value = reference.attribute(Some(W), "val");
                if !known(value) {
                    let detail = format!("{} {tag} {}", py_str(style_id.as_deref()), py_str(value));
                    out.push(problem("style-missing", part, detail));
                }
            }
        }
        let mut seen: Vec<Option<String>> = Vec::new();
        let mut current = style_id.clone();
        while let Some(found) = get(current.as_deref()).filter(|_| !seen.contains(&current)) {
            seen.push(current.clone());
            current = found
                .find(Some(W), "basedOn")
                .and_then(|r| r.attribute(Some(W), "val"))
                .map(str::to_owned);
        }
        if seen.contains(&current) {
            out.push(problem("style-based-on-cycle", part, py_str(style_id.as_deref())));
        }
    }
}

/// Drawing numbers repeated across stories.
fn drawings(out: &mut Vec<Problem>, inputs: &Inputs<'_>) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (_, part) in inputs.story_facts() {
        for id in part.all().flat_map(|f| f.docpr_ids.iter()) {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    for (id, count) in counts {
        if count > 1 {
            out.push(problem("docpr-id-repeated", inputs.main, id));
        }
    }
}

/// Revision records: an author each, ids unique across stories; moves paired.
fn tracking(out: &mut Vec<Problem>, inputs: &Inputs<'_>) {
    let mut ids: HashMap<&str, usize> = HashMap::new();
    for (name, part) in inputs.story_facts() {
        for f in part.all() {
            for (kind, author, id) in &f.records {
                if !author {
                    out.push(problem("revision-without-author", name, *kind));
                }
                if let Some(id) = id {
                    *ids.entry(id).or_insert(0) += 1;
                }
            }
        }
        move_problems(out, name, part.all());
    }
    for (id, count) in ids {
        if count > 1 {
            out.push(problem("revision-id-repeated", inputs.main, id));
        }
    }
}

/// A story part's move ranges and moves paired, from its facts in order.
pub(super) fn move_problems<'f>(out: &mut Vec<Problem>, name: &str, facts: impl Iterator<Item = &'f ElementFacts>) {
    {
        let mut starts: [BTreeMap<&str, &str>; 2] = [BTreeMap::new(), BTreeMap::new()];
        let mut ends: [BTreeSet<&str>; 2] = [BTreeSet::new(), BTreeSet::new()];
        for f in facts {
            for mark in &f.moves {
                let side = usize::from(!mark.from);
                if mark.start {
                    if let Some(slot) = starts.get_mut(side) {
                        slot.insert(&mark.id, &mark.name);
                    }
                } else if let Some(slot) = ends.get_mut(side) {
                    slot.insert(&mark.id);
                }
            }
        }
        for (side, label) in [(0usize, "from"), (1, "to")] {
            let (Some(s), Some(e)) = (starts.get(side), ends.get(side)) else {
                continue;
            };
            let started: BTreeSet<&str> = s.keys().copied().collect();
            for mark in started.symmetric_difference(e) {
                out.push(problem("move-range-unpaired", name, format!("{label} {mark}")));
            }
        }
        let names = |side: usize| -> BTreeSet<&str> {
            starts
                .get(side)
                .map(|s| s.values().copied().collect())
                .unwrap_or_default()
        };
        for name_ in names(0).symmetric_difference(&names(1)) {
            out.push(problem("move-unpaired", name, *name_));
        }
    }
}

/// `comments.xml` and the modern parts beside it agree; every comment is referenced.
fn comment_parts(out: &mut Vec<Problem>, inputs: &Inputs<'_>) {
    let main = inputs.main;
    let root_of = |rel: &str| {
        related(inputs.package, main, rel)
            .ok()
            .flatten()
            .and_then(|p| inputs.facts(&p).map(|f| Arc::clone(&f.root)))
    };
    let references: HashSet<Option<String>> = inputs
        .story_facts()
        .flat_map(|(_, p)| p.all().flat_map(|f| f.comment_refs.iter().cloned()).collect::<Vec<_>>())
        .collect();
    comment_parts_with(out, main, root_of, &references);
}

/// [`comment_parts`] given the parts' roots by relationship type and the comment
/// references in the stories.
pub(super) fn comment_parts_with(
    out: &mut Vec<Problem>,
    main: &str,
    root_of: impl Fn(&str) -> Option<Arc<Element>>,
    references: &HashSet<Option<String>>,
) {
    let Some(comments) = root_of(REL_COMMENTS) else { return };
    let mut last_ids = Vec::new();
    for comment in comments.child_elements().filter(|c| c.name().is(Some(W), "comment")) {
        let id = comment.attribute(Some(W), "id").map(str::to_owned);
        if !references.contains(&id) {
            out.push(problem("comment-without-reference", main, id.unwrap_or_default()));
        }
        last_ids.push(last_paragraph_id(comment));
    }
    if let Some(extended) = root_of(REL_COMMENTS_EXTENDED) {
        extended_entries(out, main, &extended, &last_ids);
    }
    let mut durable: Vec<(String, usize)> = Vec::new();
    let ids_root = root_of(REL_COMMENTS_IDS);
    if let Some(ids_root) = &ids_root {
        let nodes: Vec<&Arc<Element>> = ids_root
            .child_elements()
            .filter(|e| e.name().is(Some(W16CID), "commentId"))
            .collect();
        let known: HashSet<String> = nodes
            .iter()
            .map(|n| n.attribute(Some(W16CID), "paraId").unwrap_or("").to_uppercase())
            .collect();
        for n in &nodes {
            let d = n.attribute(Some(W16CID), "durableId").unwrap_or("").to_uppercase();
            match durable.iter_mut().find(|(k, _)| *k == d) {
                Some(found) => found.1 += 1,
                None => durable.push((d, 1)),
            }
        }
        for para in last_ids.iter().filter(|p| !p.is_empty()) {
            if !known.contains(para) {
                out.push(problem("comment-without-durable-id", main, para.clone()));
            }
        }
    }
    for (raw, count) in &durable {
        if *count > 1 {
            out.push(problem("comment-durable-id-repeated", main, raw.clone()));
        }
    }
    if let (Some(extensible), Some(_)) = (root_of(REL_EXTENSIBLE), &ids_root) {
        for n in extensible
            .child_elements()
            .filter(|e| e.name().is(Some(W16CEX), "commentExtensible"))
        {
            let raw = n.attribute(Some(W16CEX), "durableId").unwrap_or("").to_uppercase();
            if !durable.iter().any(|(k, _)| *k == raw) {
                out.push(problem("comment-extensible-unknown", main, raw));
            }
        }
    }
}

/// `commentsExtended`: an entry for each comment's last paragraph, parents that exist.
pub(super) fn extended_entries(out: &mut Vec<Problem>, main: &str, extended: &Element, last_ids: &[String]) {
    // a dict by paraId: the last entry of a key is the one kept
    let mut entries: HashMap<String, Arc<Element>> = HashMap::new();
    for n in extended
        .child_elements()
        .filter(|e| e.name().is(Some(W15), "commentEx"))
    {
        entries.insert(
            n.attribute(Some(W15), "paraId").unwrap_or("").to_uppercase(),
            Arc::clone(n),
        );
    }
    for para in last_ids.iter().filter(|p| !p.is_empty()) {
        if !entries.contains_key(para) {
            out.push(problem("comment-without-extended-entry", main, para.clone()));
        }
    }
    for node in entries.values() {
        let parent = node.attribute(Some(W15), "paraIdParent").unwrap_or("").to_uppercase();
        if !parent.is_empty() && !entries.contains_key(&parent) {
            out.push(problem("comment-parent-unknown", main, parent));
        }
    }
}

/// The paraId (upper-cased) of a comment's last paragraph, `""` when it has none.
pub(super) fn last_paragraph_id(comment: &Element) -> String {
    let mut last = None;
    let mut stack: Vec<&Element> = vec![comment];
    let mut order = Vec::new();
    while let Some(e) = stack.pop() {
        order.push(e);
        let children: Vec<&Element> = e.child_elements().map(|c| &**c).collect();
        stack.extend(children.into_iter().rev());
    }
    for e in order.into_iter().skip(1) {
        if e.name().is(Some(W), "p") {
            last = Some(e);
        }
    }
    last.map(|p| {
        p.attribute(Some(crate::names::W14), "paraId")
            .unwrap_or("")
            .to_uppercase()
    })
    .unwrap_or_default()
}

/// The package's own problems: parts without a content type, overrides without a part,
/// dangling relationships, parts nothing reaches.
pub(crate) fn package_problems(package: &Package) -> Vec<Problem> {
    let mut out = Vec::new();
    let names = package.part_names();
    let parts: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    let types = package.content_types().unwrap_or_default();
    for name in &parts {
        if *name != CONTENT_TYPES_PART && types.of(name).is_none() {
            out.push(problem("no-content-type", name, ""));
        }
    }
    for (target, _) in &types.overrides {
        if !target.is_empty() && !parts.contains(target.as_str()) {
            out.push(problem("override-without-part", CONTENT_TYPES_PART, target.clone()));
        }
    }
    let owners = std::iter::once("").chain(parts.iter().copied().filter(|p| !p.ends_with(".rels")));
    for owner in owners {
        let Ok(rels) = package.relationships(owner) else {
            continue;
        };
        for rel in rels.iter() {
            let dangling = !rel.external && rel.target_part.as_deref().is_none_or(|t| !parts.contains(t));
            if dangling {
                let kind = rel.rel_type.rsplit('/').next().unwrap_or("");
                let target = rel.target_part.clone().unwrap_or_else(|| "None".to_owned());
                out.push(problem(
                    "dangling-relationship",
                    if owner.is_empty() { "/" } else { owner },
                    format!("{kind} -> {target}"),
                ));
            }
        }
    }
    for orphan in unreachable(package, &parts) {
        out.push(problem("orphaned-part", &orphan, ""));
    }
    out
}

/// Parts no chain of relationships from the package root reaches (a relationships part
/// counts as reached with its owner; the content types part always is).
fn unreachable(package: &Package, parts: &BTreeSet<&str>) -> Vec<String> {
    let mut reached: HashSet<String> = HashSet::from([String::new()]);
    let mut pending = vec![String::new()];
    while let Some(source) = pending.pop() {
        let Ok(rels) = package.relationships(&source) else {
            continue;
        };
        for rel in rels.iter() {
            if let Some(target) = &rel.target_part {
                if !reached.contains(target) && parts.contains(target.as_str()) {
                    reached.insert(target.clone());
                    pending.push(target.clone());
                }
            }
        }
    }
    parts
        .iter()
        .filter(|p| **p != CONTENT_TYPES_PART && !reached.contains(**p))
        .filter(|p| owner_of_rels(p).is_none_or(|owner| !reached.contains(&owner)))
        .map(|p| (*p).to_owned())
        .collect()
}

/// The part a relationships part belongs to (`None` for another part).
fn owner_of_rels(path: &str) -> Option<String> {
    let (directory, name) = path.rsplit_once('/').unwrap_or(("", path));
    let file = name.strip_suffix(".rels")?;
    let parent = directory.strip_suffix("_rels")?;
    let parent = parent.strip_suffix('/').unwrap_or(parent);
    Some(normalize(&if parent.is_empty() {
        file.to_owned()
    } else {
        format!("{parent}/{file}")
    }))
}
