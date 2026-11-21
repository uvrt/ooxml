//! Owns: paragraphs' and table rows' keeps as Word applies them, read from the model
//! (docx-agent's `edit/table_format.py` `row_keeps`, `kept_together`, `kept_with_next`;
//! round 25 W2/W5): a paragraph's keeps with and without its direct formatting
//! ([`Keeps`]), its direct toggles ([`declared`]), per table row the paragraph Word
//! decides the row's keep with the next by ([`row_leads`]) and that keep ([`RowKeep`],
//! [`row_keeps`]), and whether a table's rows keep it together and with the next block
//! ([`TableKeeps`]). Shared by the editing crate (what keeping writes) and the page engine
//! (`check`'s layout facts).
//!
//! Must not: write, lay out, or read outside the story part it is given.

use crate::docx::cells::PartCells;
use crate::docx::formatting::{Cell, EffectiveParagraph, Formatting};
use crate::docx::{at_path, StoryPart};
use crate::ids::W;
use ooxml_xml::{Children, Element, Node};
use std::sync::Arc;

fn is_w(element: &Element, local: &str) -> bool {
    element.name().is(Some(W), local)
}

fn elements(parent: &Element) -> impl Iterator<Item = (u32, &Arc<Element>)> {
    parent.children().iter().enumerate().filter_map(|(k, n)| match n {
        Node::Element(e) => Some((u32::try_from(k).ok()?, e)),
        _ => None,
    })
}

fn child<'a>(parent: &'a Element, local: &str) -> Option<(u32, &'a Arc<Element>)> {
    elements(parent).find(|(_, e)| is_w(e, local))
}

/// A paragraph's direct toggle `w:{local}`: on, off, or not said.
#[must_use]
pub fn declared(paragraph: &Element, local: &str) -> Option<bool> {
    let node = paragraph.find(Some(W), "pPr")?.find(Some(W), local)?;
    Some(
        node.attribute(Some(W), "val")
            .is_none_or(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "off")),
    )
}

/// Per row of a table, the place of the paragraph Word decides the row's keep with the next
/// by (docx2svg's `lead_paragraph`: its first cell's first paragraph, a nested table's
/// first there); `place`, the table's.
#[must_use]
pub fn row_leads(table: &Element, place: &[u32]) -> Vec<Option<Vec<u32>>> {
    let mut out = Vec::new();
    for (k, row) in rows(table) {
        let at = [place, k.as_slice()].concat();
        let cell = contained(row, "tc").into_iter().next();
        out.push(cell.and_then(|(j, tc)| first_paragraph(tc, &[at.as_slice(), j.as_slice()].concat())));
    }
    out
}

/// `parent`'s `w:{local}` children, through content controls and custom XML, each with its
/// steps from `parent`.
fn contained<'a>(parent: &'a Element, local: &str) -> Vec<(Vec<u32>, &'a Arc<Element>)> {
    let mut out = Vec::new();
    for (k, node) in elements(parent) {
        if is_w(node, local) {
            out.push((vec![k], node));
        } else if is_w(node, "customXml") {
            out.extend(
                contained(node, local)
                    .into_iter()
                    .map(|(p, n)| ([&[k][..], &p].concat(), n)),
            );
        } else if is_w(node, "sdt") {
            if let Some((j, content)) = child(node, "sdtContent") {
                out.extend(
                    contained(content, local)
                        .into_iter()
                        .map(|(p, n)| ([&[k, j][..], &p].concat(), n)),
                );
            }
        }
    }
    out
}

/// A table's rows as the layout reads them: a row deleted as a tracked change is gone.
fn rows(table: &Element) -> Vec<(Vec<u32>, &Arc<Element>)> {
    let deleted = |row: &Element| {
        row.find(Some(W), "trPr")
            .is_some_and(|p| p.find(Some(W), "del").is_some())
    };
    contained(table, "tr")
        .into_iter()
        .filter(|(_, r)| !deleted(r))
        .collect()
}

fn first_paragraph(container: &Element, place: &[u32]) -> Option<Vec<u32>> {
    let (k, node) = elements(container).find(|(_, e)| is_w(e, "p") || is_w(e, "tbl") || is_w(e, "sdt"))?;
    let at = [place, &[k]].concat();
    if is_w(node, "p") {
        return Some(at);
    }
    if is_w(node, "sdt") {
        let (j, content) = child(node, "sdtContent")?;
        return first_paragraph(content, &[at.as_slice(), &[j]].concat());
    }
    let (r, row) = rows(node).into_iter().next()?;
    let row_at = [at.as_slice(), r.as_slice()].concat();
    let (c, cell) = contained(row, "tc").into_iter().next()?;
    first_paragraph(cell, &[row_at.as_slice(), c.as_slice()].concat())
}

/// Paragraphs' keeps as Word applies them, with and without their direct formatting, in
/// one story part.
pub struct Keeps<'a> {
    formatting: &'a Formatting,
    cells: PartCells<'a>,
}

impl<'a> Keeps<'a> {
    /// The keeps of `story`'s paragraphs under `formatting`.
    #[must_use]
    pub fn new(formatting: &'a Formatting, story: &'a StoryPart) -> Keeps<'a> {
        Keeps {
            formatting,
            cells: PartCells::new(formatting, story.root()),
        }
    }

    /// The cell of the outermost table holding the paragraph at `place`.
    fn cell(&mut self, place: &[u32]) -> Option<Cell> {
        self.cells.cell(place)
    }

    /// The paragraph's formatting as Word applies it; without `direct`, what its styles
    /// (and its list) give it.
    pub fn effective(&mut self, paragraph: &Element, place: &[u32], direct: bool) -> EffectiveParagraph {
        let cell = self.cell(place);
        if direct {
            return self.formatting.paragraph(paragraph, cell.as_ref());
        }
        let styled = styled_only(paragraph);
        self.formatting.paragraph(&styled, cell.as_ref())
    }
}

/// A copy of `paragraph` whose properties are only its style and its list.
fn styled_only(paragraph: &Element) -> Element {
    let mut first = true;
    let children: Children = paragraph
        .children()
        .iter()
        .map(|node| match node {
            Node::Element(e) if first && is_w(e, "pPr") => {
                first = false;
                let kept: Children = e
                    .children()
                    .iter()
                    .filter(|c| c.as_element().is_some_and(|c| is_w(c, "pStyle") || is_w(c, "numPr")))
                    .cloned()
                    .collect();
                Node::Element(Arc::new(e.with_children(kept)))
            }
            other => other.clone(),
        })
        .collect();
    paragraph.with_children(children)
}

/// One row's keep with the next, as Word decides it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowKeep {
    /// The paragraph it is decided by, its steps below the table; `None`: a row with none.
    pub lead: Option<Vec<u32>>,
    /// Whether it keeps with the next.
    pub keeps: bool,
    /// Whether its styles alone would.
    pub styled: bool,
    /// Its direct `w:keepNext`: on, off, or none.
    pub declared: Option<bool>,
}

/// Whether a table's rows keep it together (every row but the last keeping with the next,
/// two rows at least) and with the next block (its last row keeping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TableKeeps {
    /// docx-agent's `kept_together`.
    pub together: bool,
    /// docx-agent's `kept_with_next`.
    pub with_next: bool,
}

impl TableKeeps {
    /// What `rows` (a table's, [`row_keeps`]) keep.
    #[must_use]
    pub fn of(rows: &[RowKeep]) -> TableKeeps {
        let leads = rows.iter().all(|r| r.lead.is_some());
        let but_last = rows.get(..rows.len().saturating_sub(1)).unwrap_or_default();
        TableKeeps {
            together: rows.len() >= 2 && leads && but_last.iter().all(|r| r.keeps),
            with_next: rows.last().is_some_and(|r| r.lead.is_some() && r.keeps),
        }
    }
}

/// Per row of the table at `path` in `story`, its keep under `formatting` (docx-agent's
/// `row_keeps`); `None` where `path` holds nothing.
#[must_use]
pub fn row_keeps(formatting: &Formatting, story: &StoryPart, path: &[u32]) -> Option<Vec<RowKeep>> {
    let table = at_path(story.root(), path)?;
    let mut look = Keeps::new(formatting, story);
    let mut out = Vec::new();
    for lead in row_leads(table, path) {
        let paragraph = lead.as_ref().and_then(|l| Some((l, at_path(story.root(), l)?)));
        let Some((lead, paragraph)) = paragraph else {
            out.push(RowKeep {
                lead: None,
                keeps: false,
                styled: false,
                declared: None,
            });
            continue;
        };
        out.push(RowKeep {
            lead: lead.get(path.len()..).map(<[u32]>::to_vec),
            keeps: look.effective(paragraph, lead, true).keep_with_next,
            styled: look.effective(paragraph, lead, false).keep_with_next,
            declared: declared(paragraph, "keepNext"),
        });
    }
    Some(out)
}
