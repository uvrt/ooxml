//! Owns: a table of contents' entries as Word writes them (docx-agent's `_rebuild_toc`,
//! `_field_head`, `_toc_text_run`, `_toc_page_runs`): each a paragraph in its level's
//! `toc N` style with a right dot-leader stop at the column's end less 10 twips (unless
//! the style has stops of its own), a `w:noProof` mark, the first opening with the field's
//! begin, instruction and separator; with `\h` a hyperlink to the heading's bookmark
//! holding its text, a tab and a `PAGEREF` field (`w:webHidden` with `\z`); with no
//! headings, Word's "No table of contents entries found.". A numbered heading's entry
//! (round 32, R12): the number in its own run, a tab run in the paragraph mark's
//! formatting, then the text, with a left stop after the number in the entry; a number
//! followed by a space or by nothing is the number, a space and the text in one run.
//!
//! Must not: decide what is listed or where a heading is (the rebuild does), or write
//! but into the copy it is given.

use crate::links::new_w;
use crate::names::XML;
use crate::text::write::{make_run, spell};
use crate::work::{Id, Work};

/// What Word writes in a table of contents that finds no headings.
pub(crate) const EMPTY_TOC: &str = "No table of contents entries found.";

/// One entry to write.
pub(crate) struct Entry<'a> {
    /// The `toc N` style's id.
    pub(crate) style: &'a str,
    /// The right stop's place, when the entry writes one.
    pub(crate) right_stop: Option<i64>,
    /// The heading's text.
    pub(crate) text: &'a str,
    /// Its bookmark.
    pub(crate) bookmark: &'a str,
    /// Its page's text (empty where the layout cannot say), `None` when `\n` omits it.
    pub(crate) page: Option<&'a str>,
    /// The heading's list number, where it draws one.
    pub(crate) number: Option<Number<'a>>,
}

/// A numbered heading's number in its entry.
#[derive(Clone, Copy)]
pub(crate) struct Number<'a> {
    /// The number as the heading draws it.
    pub(crate) label: &'a str,
    /// The left stop after it, where a tab follows it (`None`: a space follows it).
    pub(crate) stop: Option<i64>,
}

/// How the entries are written: the instruction, `\h`, `\z`, and the Hyperlink style.
pub(crate) struct Looks<'a> {
    pub(crate) instruction: &'a str,
    pub(crate) hyperlinks: bool,
    pub(crate) hidden: bool,
    pub(crate) link_style: Option<&'a str>,
}

/// A `w:instrText` with `text`, its spaces kept.
pub(crate) fn instruction(work: &mut Work, parent: Id, text: &str) -> Id {
    let node = new_w(work, parent, "instrText", &[]);
    let space = work.qname(XML, "space", "xml");
    work.set(node, space, "preserve");
    work.set_text(node, text);
    node
}

/// The field's begin, instruction and separator, each a run of its own.
pub(crate) fn head(work: &mut Work, parent: Id, text: &str) -> Vec<Id> {
    let mut out = Vec::new();
    for kind in ["begin", "instruction", "separate"] {
        let run = new_w(work, parent, "r", &[]);
        let child = if kind == "instruction" {
            instruction(work, run, text)
        } else {
            new_w(work, run, "fldChar", &[("fldCharType", kind)])
        };
        work.append(run, child);
        out.push(run);
    }
    out
}

fn no_proof(work: &mut Work, parent: Id, link_style: Option<&str>, hidden: bool) -> Id {
    let properties = new_w(work, parent, "rPr", &[]);
    if let Some(style) = link_style {
        let node = new_w(work, properties, "rStyle", &[("val", style)]);
        work.append(properties, node);
    }
    let proof = new_w(work, properties, "noProof", &[]);
    work.append(properties, proof);
    if hidden {
        let node = new_w(work, properties, "webHidden", &[]);
        work.append(properties, node);
    }
    properties
}

/// A run of `text` in the entry's formatting (the Hyperlink style, `w:noProof`).
fn text_run(work: &mut Work, parent: Id, text: &str, link_style: Option<&str>) -> Id {
    let run = new_w(work, parent, "r", &[]);
    let properties = no_proof(work, run, link_style, false);
    work.append(run, properties);
    for child in spell(work, text, None) {
        work.append(run, child);
    }
    run
}

/// An entry's tab and `PAGEREF` field to `bookmark` showing `page`.
fn page_runs(work: &mut Work, parent: Id, bookmark: &str, page: &str, hidden: bool) -> Vec<Id> {
    let mut out = Vec::new();
    let reference = format!(" PAGEREF {bookmark} \\h ");
    for step in 0..7 {
        let run = new_w(work, parent, "r", &[]);
        let properties = no_proof(work, run, None, hidden);
        work.append(run, properties);
        let child = match step {
            0 => Some(new_w(work, run, "tab", &[])),
            1 => Some(new_w(work, run, "fldChar", &[("fldCharType", "begin")])),
            2 => Some(instruction(work, run, &reference)),
            4 => Some(new_w(work, run, "fldChar", &[("fldCharType", "separate")])),
            6 => Some(new_w(work, run, "fldChar", &[("fldCharType", "end")])),
            _ => None,
        };
        if let Some(child) = child {
            work.append(run, child);
        }
        if step == 5 {
            for child in spell(work, page, None) {
                work.append(run, child);
            }
        }
        out.push(run);
    }
    out
}

/// The new entries' paragraphs (detached nodes of `work`, made below `parent`'s scope).
pub(crate) fn paragraphs(work: &mut Work, parent: Id, looks: &Looks<'_>, entries: &[Entry<'_>]) -> Vec<Id> {
    let mut out = Vec::new();
    for (k, entry) in entries.iter().enumerate() {
        let paragraph = new_w(work, parent, "p", &[]);
        let properties = new_w(work, paragraph, "pPr", &[]);
        let style = new_w(work, properties, "pStyle", &[("val", entry.style)]);
        work.append(properties, style);
        let left = entry.number.as_ref().and_then(|n| n.stop);
        if left.is_some() || entry.right_stop.is_some() {
            let tabs = new_w(work, properties, "tabs", &[]);
            if let Some(stop) = left {
                let tab = new_w(work, tabs, "tab", &[("val", "left"), ("pos", &stop.to_string())]);
                work.append(tabs, tab);
            }
            if let Some(stop) = entry.right_stop {
                let tab = new_w(
                    work,
                    tabs,
                    "tab",
                    &[("val", "right"), ("leader", "dot"), ("pos", &stop.to_string())],
                );
                work.append(tabs, tab);
            }
            work.append(properties, tabs);
        }
        let mark = no_proof(work, properties, None, false);
        work.append(properties, mark);
        work.append(paragraph, properties);
        if k == 0 {
            for run in head(work, paragraph, looks.instruction) {
                work.append(paragraph, run);
            }
        }
        let container = if looks.hyperlinks {
            let link = new_w(
                work,
                paragraph,
                "hyperlink",
                &[("anchor", entry.bookmark), ("history", "1")],
            );
            work.append(paragraph, link);
            link
        } else {
            paragraph
        };
        let text = match &entry.number {
            Some(Number { label, stop: Some(_) }) => {
                let run = text_run(work, container, label, looks.link_style);
                work.append(container, run);
                let tab_run = new_w(work, container, "r", &[]);
                let properties = no_proof(work, tab_run, None, false);
                work.append(tab_run, properties);
                let tab = new_w(work, tab_run, "tab", &[]);
                work.append(tab_run, tab);
                work.append(container, tab_run);
                entry.text.to_owned()
            }
            Some(Number { label, stop: None }) => format!("{label} {}", entry.text),
            None => entry.text.to_owned(),
        };
        let run = text_run(work, container, &text, looks.link_style);
        work.append(container, run);
        if let Some(page) = entry.page {
            for run in page_runs(work, container, entry.bookmark, page, looks.hidden) {
                work.append(container, run);
            }
        }
        out.push(paragraph);
    }
    if entries.is_empty() {
        let paragraph = new_w(work, parent, "p", &[]);
        for run in head(work, paragraph, looks.instruction) {
            work.append(paragraph, run);
        }
        let run = make_run(work, EMPTY_TOC, None);
        work.append(paragraph, run);
        out.push(paragraph);
    }
    out
}
