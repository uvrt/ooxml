//! Owns: the lines between text columns (`w:cols/@w:sep`, docx2svg's
//! `_column_separators`): one between each two columns of a region where the second holds
//! text, centred on the space between them exactly, 0.75 pt wide, black, from the region's
//! top to the foot of its longest column, its edges on whole pixels.
//!
//! A separator is anchored on the block that holds its section's `w:sectPr` -- Word's own
//! identity for a section, which stays when text is inserted at the section's start
//! ([`section_holders`]).
//!
//! Must not: place columns (the paginator has) or draw anything else.

use super::Rule;
use crate::engine::DocumentLayout;
use crate::paginate::{LaidPage, PageColumn};
use crate::px::Px;
use ooxml_base::Q;
use ooxml_word_model::ids::{Anchor, BlockKey, Sub};
use ooxml_word_model::model::Block;
use std::sync::Arc;

/// The page's columns grouped by region: consecutive columns of one section from one top.
fn regions(columns: &[PageColumn]) -> Vec<Vec<&PageColumn>> {
    let mut out: Vec<Vec<&PageColumn>> = Vec::new();
    for column in columns {
        let joins = out.last().is_some_and(|region| {
            region.first().is_some_and(|first| {
                first.section == column.section
                    && first.region_top == column.region_top
                    && region.last().is_some_and(|last| column.column > last.column)
            })
        });
        match out.last_mut() {
            Some(region) if joins => region.push(column),
            _ => out.push(vec![column]),
        }
    }
    out
}

/// The lines between the columns of page `laid`, in order.
pub(super) fn column_separators(layout: &DocumentLayout, laid: &LaidPage) -> Vec<Rule> {
    let mut out = Vec::new();
    // 0.75 pt in px.
    let half = Px::points(Q::new(3, 8));
    for region in regions(&laid.columns) {
        let Some(first) = region.first() else { continue };
        let Some(section) = layout.sections.get(first.section) else {
            continue;
        };
        if !first.multi || !section.props.column_separator {
            continue;
        }
        let filled: Vec<&&PageColumn> = region.iter().filter(|c| c.filled).collect();
        let Some(bottom) = filled.iter().map(|c| c.y).max() else {
            continue;
        };
        let holder = layout
            .section_holders()
            .get(first.section)
            .copied()
            .flatten()
            .zip(u32::try_from(first.section).ok());
        let top = first.region_top.round_half_up();
        let bottom = bottom.round_half_up();
        let boxes = section.column_boxes();
        let m = &section.props.margins;
        for column in filled.iter().filter(|c| c.column > 0) {
            let Some(b) = column.column.checked_sub(1).and_then(|k| boxes.get(k)) else {
                continue;
            };
            // The middle of the space, in half twips, as Word holds a length.
            let halves = i128::from(m.left + m.gutter + b.offset + b.width) * 2 + i128::from(b.space);
            let centre = Px::points(Q::new(halves, 40)).quantise();
            let (x0, x1) = ((centre - half).round_half_up(), (centre + half).round_half_up());
            out.push(Rule {
                kind: "column-separator",
                path: format!("w:sectPr[{}]/w:cols", first.section + 1),
                anchor: holder.map(|(block, section)| Anchor {
                    block,
                    sub: Sub::Cols(section),
                }),
                x: Px::int(x0),
                y: Px::int(top),
                width: Px::int(x1 - x0),
                height: Px::int(bottom - top),
                color: Some("000000".to_owned()),
                style: None,
            });
        }
    }
    out
}

/// Per section of `layout`, the key of the block holding its `w:sectPr`: the paragraph
/// whose mark ends it; for the last section, whose `w:sectPr` is the body's own, the body's
/// last block (the endnotes placed in the flow left out). `None` where that block has no
/// key.
pub(crate) fn section_holders(layout: &DocumentLayout) -> Vec<Option<BlockKey>> {
    let sections = &layout.sections.list;
    let mut out = vec![None; sections.len()];
    let mut next = 0;
    let mut last = None;
    for (k, block) in layout.document.body.iter().enumerate() {
        if layout.roles.get(k).is_some_and(Option::is_some) {
            continue;
        }
        let key = match block {
            Block::Paragraph(p) => {
                let ends = p.section.as_ref().is_some_and(|own| {
                    sections
                        .get(next)
                        .is_some_and(|s| Arc::ptr_eq(own, &s.props) || **own == *s.props)
                });
                if ends {
                    if let Some(holder) = out.get_mut(next) {
                        *holder = p.key;
                    }
                    next += 1;
                }
                p.key
            }
            Block::Table(t) => t.key,
        };
        last = Some(key);
    }
    // the sections left: the body's own `w:sectPr` (or the one made where none is written)
    for holder in out.iter_mut().skip(next) {
        *holder = last.flatten();
    }
    out
}
