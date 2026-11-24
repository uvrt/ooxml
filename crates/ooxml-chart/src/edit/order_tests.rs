//! The generated orders place a child where the hand-kept prefixes they replace did: an axis'
//! title went by the first seven steps of every axis' sequence (`axId` .. `title`), a series'
//! name by `idx`, `order`, `tx`. Siblings an order does not name rank last, so a prefix and the
//! whole sequence rank every sibling after the child alike; this checks it on generated
//! sibling lists, in and out of order, with names no sequence holds.
#![allow(clippy::unwrap_used)]

use super::axes::axis_order;
use super::owned::{Order, Owned};
use ooxml_schema::orders::local as schema;
use ooxml_xml::{Element, Node, QName};
use std::sync::Arc;

/// The hand-kept prefix an axis title was placed by.
const AXIS_HEAD: Order = &[
    &["axId"],
    &["scaling"],
    &["delete"],
    &["axPos"],
    &["majorGridlines"],
    &["minorGridlines"],
    &["title"],
];
/// The hand-kept prefix a series' name was placed by.
const SERIES_HEAD: Order = &[&["idx"], &["order"], &["tx"]];

/// A small deterministic generator (no dependency for a test).
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

/// Sibling lists: names of `order` (and two it lacks), some kept in order, some shuffled.
fn siblings(order: Order, random: &mut Lcg) -> Vec<Vec<&'static str>> {
    let mut pool: Vec<&'static str> = order.iter().flat_map(|step| step.iter().copied()).collect();
    pool.extend(["AlternateContent", "unknownExt"]);
    let mut out = Vec::new();
    for round in 0..400 {
        let mut list: Vec<&'static str> = pool.iter().copied().filter(|_| random.below(3) == 0).collect();
        if round % 2 == 1 {
            for i in (1..list.len()).rev() {
                list.swap(i, random.below(i + 1));
            }
        }
        out.push(list);
    }
    out
}

fn element(names: &[&str]) -> Element {
    let name = |local: &str| Arc::new(QName::new(Some("c"), local, Some("urn:c")));
    let children = names
        .iter()
        .map(|n| Node::Element(Arc::new(Element::new(name(n), Vec::new(), Vec::new(), Vec::new()))))
        .collect();
    Element::new(name("parent"), Vec::new(), Vec::new(), children)
}

/// Where `name` goes among `names`, by `order`, through both insertion paths.
fn places(order: Order, names: &[&str], name: &str) -> (usize, Vec<String>) {
    let parent = element(names);
    let mut owned = Owned::open(&parent);
    let child = Owned::named(Some("c"), name, Some("urn:c"));
    let at = owned.insert_ordered(Some(order), child.clone());
    let inserted = ooxml_xml::view::insert_ordered(&parent, order, child.close());
    let locals = inserted.child_elements().map(|e| e.name().local().to_owned()).collect();
    (at, locals)
}

fn same_places(head: Order, full: Order, seed: u64) {
    let mut random = Lcg(seed);
    for names in siblings(full, &mut random) {
        for name in head.iter().flat_map(|step| step.iter()) {
            assert_eq!(
                places(head, &names, name),
                places(full, &names, name),
                "{name} among {names:?}"
            );
        }
    }
}

#[test]
fn an_axis_title_goes_where_the_seven_step_head_put_it() {
    for (k, kind) in ["catAx", "valAx", "dateAx", "serAx"].into_iter().enumerate() {
        let full = axis_order(kind);
        assert!(full.len() > AXIS_HEAD.len() && full.starts_with(AXIS_HEAD), "{kind}");
        same_places(AXIS_HEAD, full, 17 + k as u64);
    }
}

#[test]
fn a_series_name_goes_where_the_three_step_head_put_it() {
    assert!(schema::SER.starts_with(SERIES_HEAD));
    same_places(SERIES_HEAD, schema::SER, 5);
}
