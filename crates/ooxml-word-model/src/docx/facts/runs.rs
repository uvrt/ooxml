//! Owns: [`Runs`], a value made of each element of one children list (a body's blocks, a
//! story root's children), held in runs of neighbouring places that versions share: a later
//! version of the list remakes only the runs its changed stretch touches
//! ([`Children::changed_span`]) and takes every other run as it is, by pointer, so neither
//! reading it nor letting the earlier one go visits each block of a long body. Places are not
//! stored: a node's place is its position, counted through the runs.
//!
//! Must not: make a value of an element it shares with the earlier reading (the value is
//! taken by pointer), or keep anything the list's nodes do not determine.

use ooxml_xml::{Children, Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// The nodes a run holds at most when it is made (an edit's run may be longer until a
/// later version remakes it).
const RUN: usize = 64;

/// One element and its value.
#[derive(Debug)]
struct Slot<T> {
    element: Arc<Element>,
    value: Arc<T>,
}

/// Neighbouring places of the list: per node, its element's slot (`None` for a node that is
/// not an element, or one left out).
#[derive(Debug)]
struct Run<T> {
    slots: Vec<Option<Slot<T>>>,
}

/// A value per element of a children list, in runs shared between versions.
#[derive(Debug)]
pub(super) struct Runs<T> {
    children: Children,
    runs: Vec<Arc<Run<T>>>,
}

impl<T> Clone for Runs<T> {
    fn clone(&self) -> Self {
        Runs {
            children: self.children.clone(),
            runs: self.runs.clone(),
        }
    }
}

impl<T> Runs<T> {
    /// The values of `children`'s elements that `keep` keeps, each made by `make`; `earlier`,
    /// a reading of another version of the list (any version: the stretch between them is
    /// remade), whose runs outside that stretch are taken as they are and whose elements
    /// within it are not made again.
    pub(super) fn read(
        children: &Children,
        earlier: Option<&Runs<T>>,
        keep: &impl Fn(&Element) -> bool,
        make: &impl Fn(&Arc<Element>) -> T,
    ) -> Runs<T> {
        let Some(earlier) = earlier.filter(|e| !e.runs.is_empty()) else {
            return Runs {
                children: children.clone(),
                runs: made(children, 0..children.len(), &HashMap::new(), keep, make),
            };
        };
        let Some((start, old_end, new_end)) = earlier.children.changed_span(children) else {
            return Runs {
                children: children.clone(),
                runs: earlier.runs.clone(),
            };
        };
        // the old places the change touches: its stretch, or the place it inserts at (the
        // last place when it appends)
        let old_len = earlier.children.len();
        let low = start.min(old_len.saturating_sub(1));
        let high = old_end.max(low + 1);
        let mut runs = Vec::with_capacity(earlier.runs.len() + 1);
        let (mut at, mut remade): (usize, Option<(usize, usize)>) = (0, None);
        let mut known: HashMap<*const Element, &Arc<T>> = HashMap::new();
        for run in &earlier.runs {
            let end = at + run.slots.len();
            if end <= low {
                runs.push(Arc::clone(run));
            } else if at < high {
                for slot in run.slots.iter().flatten() {
                    known.insert(Arc::as_ptr(&slot.element), &slot.value);
                }
                remade = Some((remade.map_or(at, |(from, _)| from), end));
            } else {
                if let Some((from, to)) = remade.take() {
                    runs.extend(made(children, from..shifted(to, old_end, new_end), &known, keep, make));
                }
                runs.push(Arc::clone(run));
            }
            at = end;
        }
        if let Some((from, _)) = remade {
            runs.extend(made(children, from..children.len(), &known, keep, make));
        }
        Runs {
            children: children.clone(),
            runs,
        }
    }

    /// Each kept element's place in the list, the element and its value, in order.
    pub(super) fn iter(&self) -> impl Iterator<Item = (u32, &Arc<Element>, &Arc<T>)> {
        self.runs
            .iter()
            .flat_map(|run| run.slots.iter())
            .enumerate()
            .filter_map(|(k, slot)| {
                slot.as_ref()
                    .map(|s| (u32::try_from(k).unwrap_or(u32::MAX), &s.element, &s.value))
            })
    }
}

/// An old place at or past the changed stretch's end, as a place of the new list.
fn shifted(old: usize, old_end: usize, new_end: usize) -> usize {
    (old + new_end).saturating_sub(old_end)
}

/// The runs of `children`'s places `range`, values taken from `known` by pointer where it
/// has them.
fn made<T>(
    children: &Children,
    range: std::ops::Range<usize>,
    known: &HashMap<*const Element, &Arc<T>>,
    keep: &impl Fn(&Element) -> bool,
    make: &impl Fn(&Arc<Element>) -> T,
) -> Vec<Arc<Run<T>>> {
    let mut runs = Vec::new();
    let mut slots = Vec::with_capacity(RUN);
    for k in range {
        let slot = match children.get(k) {
            Some(Node::Element(element)) if keep(element) => Some(Slot {
                element: Arc::clone(element),
                value: known
                    .get(&Arc::as_ptr(element))
                    .map_or_else(|| Arc::new(make(element)), |v| Arc::clone(v)),
            }),
            _ => None,
        };
        slots.push(slot);
        if slots.len() == RUN {
            runs.push(Arc::new(Run { slots: std::mem::take(&mut slots) }));
        }
    }
    if !slots.is_empty() {
        runs.push(Arc::new(Run { slots }));
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::{Runs, RUN};
    use ooxml_base::XmlLimits;
    use ooxml_xml::{Children, Element, Node};
    use std::cell::Cell;
    use std::sync::Arc;

    /// `n` fresh nodes: elements, every fifth a text node.
    fn nodes(n: usize) -> Vec<Node> {
        let xml = format!("<r>{}</r>", (0..n).map(|k| if k % 5 == 4 { "t" } else { "<a/>" }).collect::<String>());
        let tree = ooxml_xml::parse(xml.as_bytes(), &XmlLimits::default()).unwrap_or_else(|e| panic!("{e}"));
        tree.root().children().to_vec()
    }

    fn placed<T>(runs: &Runs<T>) -> Vec<(u32, *const Element, *const T)> {
        runs.iter().map(|(k, e, v)| (k, Arc::as_ptr(e), Arc::as_ptr(v))).collect()
    }

    #[test]
    fn a_reading_from_any_earlier_one_equals_a_fresh_reading_and_makes_only_new_values() {
        let made = Cell::new(0usize);
        let make = |_: &Arc<Element>| {
            made.set(made.get() + 1);
        };
        let keep = |_: &Element| true;
        let mut list = Children::from_vec(nodes(1000));
        let mut runs = Runs::read(&list, None, &keep, &make);
        let mut seed: u64 = 7;
        let mut next = |bound: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(seed >> 33).unwrap_or(0) % bound.max(1)
        };
        for round in 0..300 {
            let len = list.len();
            let (at, gone, new) = match round % 4 {
                0 => (next(len + 1), 0, 1 + next(3)),          // insert
                1 => (next(len), 1 + next(2), 0),              // delete
                2 => (next(len), 1, 1),                        // replace one
                _ => (len, 0, 1 + next(RUN * 2)),              // append
            };
            let gone = gone.min(len - at.min(len));
            let fresh = nodes(new);
            let elements = fresh.iter().filter(|n| matches!(n, Node::Element(_))).count();
            list = list.splice(at..at + gone, fresh).unwrap_or_else(|| panic!("a splice"));
            made.set(0);
            let next_runs = Runs::read(&list, Some(&runs), &keep, &make);
            assert_eq!(made.get(), elements, "round {round}: only the new elements are made");
            let again = Runs::read(&list, None, &keep, &|_: &Arc<Element>| ());
            let places = |r: &Runs<()>| r.iter().map(|(k, e, _)| (k, Arc::as_ptr(e))).collect::<Vec<_>>();
            assert_eq!(places(&next_runs), places(&again), "round {round}");
            runs = next_runs;
        }
        // the same list again: every run shared
        let same = Runs::read(&list, Some(&runs), &keep, &make);
        assert_eq!(placed(&same), placed(&runs));
        assert!(same.runs.iter().zip(&runs.runs).all(|(a, b)| Arc::ptr_eq(a, b)));
    }

    #[test]
    fn runs_away_from_an_edit_are_shared() {
        let list = Children::from_vec(nodes(RUN * 10));
        let runs = Runs::read(&list, None, &|_| true, &|_| ());
        let edited = list
            .splice(RUN * 5..RUN * 5 + 1, nodes(1))
            .unwrap_or_else(|| panic!("a splice"));
        let later = Runs::read(&edited, Some(&runs), &|_| true, &|_| ());
        let shared = later
            .runs
            .iter()
            .filter(|r| runs.runs.iter().any(|o| Arc::ptr_eq(o, r)))
            .count();
        assert_eq!(shared, 9);
    }
}
