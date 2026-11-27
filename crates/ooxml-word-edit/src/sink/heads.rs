//! Owns: what the sink's writes compute from a tree without writing: the draws that number
//! a write's revision records again where its change is applied ([`annotation_draws`]). A
//! root's head with the namespaces an edit needs declared is word-model's
//! ([`ooxml_word_model::stamp::ensured_head`]).
//!
//! Must not: write (the sink does).

use super::Numbered;
use crate::doc::{Draw, DrawFrom};
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// The draws that number again, where the change is applied, the revision records a
/// write made in `element` (`w:id` among the stand-ins it gave), each by its offset from
/// the edit's first id; paths from `element`.
pub(crate) fn annotation_draws(element: &Element, numbered: Numbered, node: u32) -> Vec<Draw> {
    let mut out = Vec::new();
    let mut stack: Vec<(&Element, Vec<u32>)> = vec![(element, Vec::new())];
    while let Some((e, path)) = stack.pop() {
        if e.name().namespace() == Some(crate::names::W) && crate::revision_ids::is_record(e) {
            let value = e
                .attribute(Some(crate::names::W), "id")
                .and_then(|v| v.trim().parse::<i64>().ok())
                .filter(|v| (numbered.from..numbered.to).contains(v));
            if let Some(value) = value {
                if let (Ok(offset), Some(attribute)) = (
                    u32::try_from(value - numbered.base),
                    e.attributes().iter().find(|a| a.name.is(Some(crate::names::W), "id")),
                ) {
                    out.push(Draw {
                        from: DrawFrom::Annotation(offset),
                        node,
                        at: path.clone(),
                        attribute: Arc::clone(&attribute.name),
                    });
                }
            }
        }
        for (k, node) in e.children().iter().enumerate() {
            if let Node::Element(child) = node {
                let mut below = path.clone();
                below.push(u32::try_from(k).unwrap_or(u32::MAX));
                stack.push((child, below));
            }
        }
    }
    out
}
