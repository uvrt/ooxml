//! The persistent block index against a from-scratch reference (docx-agent's `PartIndex`
//! rules, written plainly here): made-up parts with missing, repeated (in any case),
//! invalid and valid paraIds, tables nested in cells, text boxes' fallbacks and blocks
//! outside the body, edited by random splices of the body's children -- every version's
//! index made from the previous one equals the reference on every name, place and query.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use ooxml_word_model::ids::{valid_long_hex, Entry, IdState, PartIds, MC, W, W14};
use ooxml_xml::{Attribute, Element, Node, QName};
use std::collections::HashMap;
use std::sync::Arc;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn el(ns: &str, prefix: &str, local: &str, attrs: Vec<Attribute>, children: Vec<Node>) -> Arc<Element> {
    Arc::new(Element::new(
        Arc::new(QName::new(Some(prefix), local, Some(ns))),
        Vec::new(),
        attrs,
        children,
    ))
}

fn para_id(rng: &mut Rng, pool: &[&str]) -> Vec<Attribute> {
    match rng.below(pool.len() + 2) {
        0 => Vec::new(),
        k => {
            let raw = pool.get(k - 1).copied().unwrap_or("7AB3C001");
            vec![Attribute {
                name: Arc::new(QName::new(Some("w14"), "paraId", Some(W14))),
                value: Arc::from(raw),
            }]
        }
    }
}

const POOL: &[&str] = &[
    "1A2B3C4D", "1a2b3c4d", "0BADF00D", "ZZ", "00000000", "7FFFFFFF", "80000000", "5E5E5E5E",
];

fn paragraph(rng: &mut Rng) -> Node {
    let mut children = Vec::new();
    if rng.below(5) == 0 {
        // a text box: its fallback's paragraphs are not blocks
        let choice = el(MC, "mc", "Choice", Vec::new(), vec![Node::Element(inner_p(rng))]);
        let fallback = el(MC, "mc", "Fallback", Vec::new(), vec![Node::Element(inner_p(rng))]);
        children.push(Node::Element(el(
            MC,
            "mc",
            "AlternateContent",
            Vec::new(),
            vec![Node::Element(choice), Node::Element(fallback)],
        )));
    }
    Node::Element(el(W, "w", "p", para_id(rng, POOL), children))
}

fn inner_p(rng: &mut Rng) -> Arc<Element> {
    el(W, "w", "p", para_id(rng, POOL), Vec::new())
}

fn table(rng: &mut Rng, depth: usize) -> Node {
    let rows = 1 + rng.below(3);
    let mut children = vec![Node::Element(el(W, "w", "tblPr", Vec::new(), Vec::new()))];
    for _ in 0..rows {
        let mut cells = Vec::new();
        for _ in 0..1 + rng.below(2) {
            let mut content = vec![paragraph(rng)];
            if depth < 2 && rng.below(4) == 0 {
                content.push(table(rng, depth + 1));
                content.push(paragraph(rng));
            }
            cells.push(Node::Element(el(W, "w", "tc", Vec::new(), content)));
        }
        children.push(Node::Element(el(W, "w", "tr", para_id(rng, POOL), cells)));
    }
    Node::Element(el(W, "w", "tbl", Vec::new(), children))
}

fn block(rng: &mut Rng) -> Node {
    match rng.below(6) {
        0 => table(rng, 0),
        1 => Node::Text(Arc::from("\n  ")),
        _ => paragraph(rng),
    }
}

fn document(rng: &mut Rng, blocks: usize, story: bool) -> Arc<Element> {
    let children: Vec<Node> = (0..blocks).map(|_| block(rng)).collect();
    if story {
        return el(W, "w", "hdr", Vec::new(), children);
    }
    let body = el(W, "w", "body", Vec::new(), children);
    let mut root = Vec::new();
    if rng.below(2) == 0 {
        root.push(Node::Element(el(
            W,
            "w",
            "background",
            Vec::new(),
            vec![paragraph(rng)],
        )));
    }
    root.push(Node::Element(body));
    if rng.below(3) == 0 {
        root.push(Node::Element(el(W, "w", "extra", Vec::new(), vec![paragraph(rng)])));
    }
    el(W, "w", "document", Vec::new(), root)
}

// -- the reference: docx-agent's rules, walked from scratch ------------------------------

/// A paragraph as named: id, place, state, ordinal, paraId.
type NamedP = (String, Vec<u32>, IdState, usize, Option<String>);
/// A table as named: id, place, rows (place, id).
type NamedT = (String, Vec<u32>, Vec<(Vec<u32>, String)>);
/// A block the walk found: place, kind, paraId, owning table.
type FoundB = (Vec<u32>, char, Option<String>, Option<Vec<u32>>);

#[derive(Debug, PartialEq)]
struct Named {
    paragraphs: Vec<NamedP>,
    tables: Vec<NamedT>,
}

fn walk(e: &Element, path: &mut Vec<u32>, tables: &mut Vec<Vec<u32>>, out: &mut Vec<FoundB>) {
    for (k, node) in e.children().iter().enumerate() {
        let Node::Element(c) = node else { continue };
        if c.name().is(Some(MC), "Fallback") {
            continue;
        }
        path.push(k as u32);
        let kind = if c.name().is(Some(W), "p") {
            Some('p')
        } else if c.name().is(Some(W), "tr") {
            Some('r')
        } else if c.name().is(Some(W), "tbl") {
            Some('t')
        } else {
            None
        };
        if let Some(kind) = kind {
            let raw = (kind != 't')
                .then(|| c.attribute(Some(W14), "paraId").map(str::to_owned))
                .flatten();
            out.push((
                path.clone(),
                kind,
                raw,
                if kind == 'r' { tables.last().cloned() } else { None },
            ));
            if kind == 't' {
                tables.push(path.clone());
            }
        }
        walk(c, path, tables, out);
        if kind == Some('t') {
            tables.pop();
        }
        path.pop();
    }
}

fn reference(story: &str, root: &Element) -> Named {
    let mut found: Vec<FoundB> = Vec::new();
    walk(root, &mut Vec::new(), &mut Vec::new(), &mut found);
    let mut seen: HashMap<String, usize> = HashMap::new();
    let repeats: Vec<usize> = found
        .iter()
        .map(|(_, _, raw, _)| match raw {
            None => 0,
            Some(r) => {
                let c = seen.entry(r.to_uppercase()).or_insert(0);
                *c += 1;
                *c - 1
            }
        })
        .collect();
    let prefix = if story == "body" {
        String::new()
    } else {
        format!("{story}/")
    };
    let mut paragraphs = Vec::new();
    for ((path, kind, raw, _), repeat) in found.iter().zip(&repeats) {
        if *kind != 'p' {
            continue;
        }
        let index = paragraphs.len();
        let (id, state) = match raw {
            None => (format!("p@{story}/{index}"), IdState::Missing),
            Some(r) if *repeat > 0 => (format!("{prefix}p:{r}#{repeat}"), IdState::Repeat),
            Some(r) if valid_long_hex(r) => (format!("{prefix}p:{r}"), IdState::Ok),
            Some(r) => (format!("{prefix}p:{r}"), IdState::Invalid),
        };
        paragraphs.push((id, path.clone(), state, index, raw.clone()));
    }
    let tables = reference_tables(story, &prefix, &found, &repeats);
    Named { paragraphs, tables }
}

fn reference_tables(story: &str, prefix: &str, found: &[FoundB], repeats: &[usize]) -> Vec<NamedT> {
    let good = |raw: &Option<String>, repeat: usize| raw.clone().filter(|r| valid_long_hex(r) && repeat == 0);
    let mut tables = Vec::new();
    for (path, kind, _, _) in found {
        if *kind != 't' {
            continue;
        }
        let rows: Vec<(&Vec<u32>, &Option<String>, usize)> = found
            .iter()
            .zip(repeats)
            .filter(|((_, k, _, owner), _)| *k == 'r' && owner.as_ref() == Some(path))
            .map(|((p, _, raw, _), r)| (p, raw, *r))
            .collect();
        let number = tables.len();
        let id = match rows.first().and_then(|(_, raw, r)| good(raw, *r)) {
            Some(first) => format!("{prefix}t:{first}"),
            None => format!("t@{story}/{number}"),
        };
        let row_ids = rows
            .iter()
            .enumerate()
            .map(|(k, (p, raw, r))| {
                let rid = match good(raw, *r) {
                    Some(raw) => format!("{prefix}tr:{raw}"),
                    None => format!("{id}/r{k}"),
                };
                ((*p).clone(), rid)
            })
            .collect();
        tables.push((id, path.clone(), row_ids));
    }
    tables
}

fn named(ids: &PartIds) -> Named {
    Named {
        paragraphs: ids
            .paragraphs()
            .iter()
            .map(|p| (p.id, p.path.to_vec(), p.state, p.index, p.raw))
            .collect(),
        tables: ids
            .tables()
            .iter()
            .map(|t| {
                (
                    t.id,
                    t.path.to_vec(),
                    t.rows.iter().map(|(p, r)| (p.to_vec(), r.clone())).collect(),
                )
            })
            .collect(),
    }
}

fn check(story: &str, ids: &PartIds, root: &Element, rng: &mut Rng) {
    let want = reference(story, root);
    assert_eq!(named(ids), want);
    assert_eq!(ids.paragraphs().len(), want.paragraphs.len());
    let mut first: HashMap<&str, Entry> = HashMap::new();
    for (k, (id, path, ..)) in want.paragraphs.iter().enumerate() {
        first.entry(id).or_insert(Entry::Paragraph(k));
        assert_eq!(ids.at(path), Some(Entry::Paragraph(k)));
        assert_eq!(ids.paragraphs().get(k).map(|p| p.id), Some(id.clone()));
    }
    for (k, (id, path, _)) in want.tables.iter().enumerate() {
        first.entry(id).or_insert(Entry::Table(k));
        assert_eq!(ids.at(path), Some(Entry::Table(k)));
        assert_eq!(ids.table_at(path).map(|t| t.id), Some(id.clone()));
    }
    for (id, entry) in &first {
        assert_eq!(ids.get(id), Some(*entry), "{id}");
    }
    assert_eq!(ids.get("p:00C0FFEE"), None);
    let all_ok = want.paragraphs.iter().all(|p| p.2 == IdState::Ok)
        && want.tables.iter().all(|t| t.2.iter().all(|(_, r)| r.contains("tr:")));
    assert_eq!(ids.fully_stamped(), all_ok);
    // queries by place, against a scan
    let paths: Vec<Vec<u32>> = want
        .paragraphs
        .iter()
        .map(|p| p.1.clone())
        .chain(want.tables.iter().map(|t| t.1.clone()))
        .collect();
    for _ in 0..8 {
        if paths.is_empty() {
            break;
        }
        let mut marker = paths[rng.below(paths.len())].clone();
        if rng.below(2) == 0 && marker.len() > 1 {
            marker.truncate(1 + rng.below(marker.len() - 1));
        }
        let after = want.paragraphs.iter().find(|p| p.1 > marker).map(|p| p.3);
        let before = want.paragraphs.iter().rev().find(|p| p.1 < marker).map(|p| p.3);
        assert_eq!(ids.paragraph_after(&marker).map(|p| p.index), after, "after {marker:?}");
        assert_eq!(
            ids.paragraph_before(&marker).map(|p| p.index),
            before,
            "before {marker:?}"
        );
        let within: Vec<usize> = want
            .paragraphs
            .iter()
            .filter(|p| p.1.starts_with(&marker))
            .map(|p| p.3)
            .collect();
        assert_eq!(
            ids.paragraphs_within(&marker)
                .iter()
                .map(|p| p.index)
                .collect::<Vec<_>>(),
            within
        );
        // the blocks below a stretch of the marker's parent's children
        if let Some((&k, parent)) = marker.split_last() {
            let (from, to) = (k.saturating_sub(rng.below(3) as u32), k + rng.below(3) as u32);
            let below =
                |p: &[u32]| p.len() > parent.len() && p.starts_with(parent) && (from..=to).contains(&p[parent.len()]);
            let (paragraphs, tables) = ids.within_stretch(parent, (from, to));
            let want_p: Vec<&String> = want.paragraphs.iter().filter(|p| below(&p.1)).map(|p| &p.0).collect();
            let want_t: Vec<&String> = want.tables.iter().filter(|t| below(&t.1)).map(|t| &t.0).collect();
            assert_eq!(
                paragraphs.iter().map(|p| &p.id).collect::<Vec<_>>(),
                want_p,
                "{parent:?} {from}..={to}"
            );
            assert_eq!(
                tables.iter().map(|t| &t.id).collect::<Vec<_>>(),
                want_t,
                "{parent:?} {from}..={to}"
            );
        }
    }
}

fn edited(rng: &mut Rng, root: &Arc<Element>, story: bool) -> Arc<Element> {
    let holder_at = if story {
        None
    } else {
        root.children()
            .iter()
            .position(|n| n.as_element().is_some_and(|e| e.name().is(Some(W), "body")))
    };
    let holder = match holder_at {
        Some(k) => Arc::clone(root.children().get(k).and_then(Node::as_element).unwrap()),
        None => Arc::clone(root),
    };
    let len = holder.children().len();
    let at = if len == 0 { 0 } else { rng.below(len + 1).min(len) };
    let children = match rng.below(5) {
        0 if len > 0 => holder.children().splice(at.min(len - 1)..at.min(len - 1) + 1, vec![]),
        1 => holder
            .children()
            .splice(at..at, (0..1 + rng.below(3)).map(|_| block(rng)).collect()),
        // many insertions at one place: labels run out and are given again
        2 => {
            let mut c = holder.children().clone();
            for _ in 0..30 {
                c = c.splice(at..at, vec![block(rng)]).unwrap();
            }
            Some(c)
        }
        _ if len > 0 => {
            let k = at.min(len - 1);
            holder.children().splice(k..k + 1, vec![block(rng)])
        }
        _ => holder.children().splice(0..0, vec![block(rng)]),
    }
    .unwrap();
    let holder = Arc::new(holder.with_children(children));
    match holder_at {
        None => holder,
        Some(k) => {
            let children = root.children().splice(k..k + 1, vec![Node::Element(holder)]).unwrap();
            Arc::new(root.with_children(children))
        }
    }
}

#[test]
fn persistent_index_equals_reference() {
    for seed in 1..=40u64 {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0x2545_F491_4F6C_DD1D));
        let story = seed % 4 == 0;
        let name = if story { "header1" } else { "body" };
        let blocks = 10 + rng.below(80);
        let mut root = document(&mut rng, blocks, story);
        let mut ids = PartIds::new(name, &root);
        check(name, &ids, &root, &mut rng);
        for _ in 0..12 {
            let next = edited(&mut rng, &root, story);
            ids = ids.after(&root, &next);
            root = next;
            check(name, &ids, &root, &mut rng);
            // the scratch index of the same version agrees too
            assert_eq!(named(&PartIds::new(name, &root)), named(&ids));
        }
    }
}
