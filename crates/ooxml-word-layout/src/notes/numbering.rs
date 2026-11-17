//! Owns: notes numbered and endnotes placed (docx2svg's `notes.with_endnotes`,
//! `with_footnotes`, `footnote_numbers`, `footnote_references`): each reference's number
//! put in its run's text as a run of its own that draws a note's number
//! ([`Run::note_number`]), each note's own mark (`w:endnoteRef`, `w:footnoteRef`)
//! numbered alike, and the endnotes' paragraphs placed after the text -- after the
//! document's last paragraph, or under `w:pos` `sectEnd` after each section's -- each
//! group opened by the separator's paragraph ([`separator`]).
//!
//! Must not: measure or place anything (the flow and the paginator do), or read XML.

use super::NoteRole;
use ooxml_word_model::labels::format_number;
use ooxml_word_model::model::{Block, ParaProps, Paragraph, Run, Section, Table};
use ooxml_word_model::value::{text, Props, Text, Value};
use ooxml_word_model::{Notes, WordDocument};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

/// An endnote's number format where its section states none.
pub const DEFAULT_ENDNOTE_FORMAT: &str = "lowerRoman";
/// A footnote's.
pub const DEFAULT_FOOTNOTE_FORMAT: &str = "decimal";

/// A note's number in a `w:numFmt` (`notes.number_text`; `fallback` where none is
/// stated).
pub fn number_text(value: i64, format: Option<&str>, fallback: &str) -> String {
    let format = format.unwrap_or(fallback);
    if format == "chicago" {
        let symbol = ['*', '\u{2020}', '\u{2021}', '\u{a7}'];
        let k = usize::try_from((value - 1).rem_euclid(4)).unwrap_or(0);
        let count = usize::try_from((value - 1).div_euclid(4) + 1).unwrap_or(0);
        return symbol.get(k).copied().unwrap_or('*').to_string().repeat(count);
    }
    format_number(value, Some(format))
}

/// Which notes part a separator is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotesPart {
    /// `word/footnotes.xml`.
    Footnotes,
    /// `word/endnotes.xml`.
    Endnotes,
}

impl NotesPart {
    fn name(self) -> &'static str {
        match self {
            NotesPart::Footnotes => "footnote",
            NotesPart::Endnotes => "endnote",
        }
    }
}

/// Word's own separator paragraph (`default_separator`): the default style, no space
/// after, single spacing; its run holds only the separator's mark.
pub fn default_separator(kind: &str, part: NotesPart) -> Paragraph {
    let mut declared = Props::new();
    declared.insert(Cow::Borrowed("spacing.after"), Value::Int(0));
    declared.insert(Cow::Borrowed("spacing.line"), Value::Int(240));
    declared.insert(Cow::Borrowed("spacing.lineRule"), Value::Str(text("auto")));
    Paragraph {
        runs: Arc::from(vec![Run {
            text: text(""),
            breaks: Arc::from(vec![(0, text(kind))]),
            path: text("w:r[1]"),
            ..Run::default()
        }]),
        properties: Some(Arc::new(ParaProps {
            style_id: None,
            declared,
            mark: None,
        })),
        table_style_id: None,
        table_conditions: Arc::from(Vec::new()),
        path: text(&format!("w:{}s/{kind}", part.name())),
        para_id: None,
        joins: Arc::from(Vec::new()),
        mark_deleted: false,
        section: None,
        bookmarks: Arc::from(Vec::new()),
    }
}

/// A separator's blocks as Word draws them (`notes.separator`): the part's own when the
/// settings name it, else Word's ([`default_separator`]).
pub fn separator(document: &WordDocument, kind: &str, part: NotesPart) -> Vec<Block> {
    let (notes, named) = match part {
        NotesPart::Footnotes => (&document.footnotes, &document.context.settings.footnote_named),
        NotesPart::Endnotes => (&document.endnotes, &document.context.settings.endnote_named),
    };
    let id = notes.separator_ids.get(kind);
    match (notes.separators.get(kind), id) {
        (Some(blocks), Some(id)) if !blocks.is_empty() && named.contains(id) => blocks.to_vec(),
        _ => vec![Block::Paragraph(Arc::new(default_separator(kind, part)))],
    }
}

/// `run` with `texts` (mark index -> text) put in its text where those marks are, and the
/// marks removed (`keep`: left where their text starts) -- a run that also holds drawings,
/// whose marks keep their order with them (`_splice`).
fn splice(run: &Run, texts: &BTreeMap<usize, String>, keep: bool) -> Run {
    let chars: Vec<char> = run.text.chars().collect();
    let inserted: Vec<(usize, usize, &String)> = run
        .breaks
        .iter()
        .enumerate()
        .filter_map(|(k, (at, _))| texts.get(&k).map(|t| (k, *at, t)))
        .collect();
    let mut out = String::new();
    let mut last = 0;
    for (_, at, value) in &inserted {
        out.extend(chars.get(last..*at).unwrap_or_default());
        out.push_str(value);
        last = *at;
    }
    out.extend(chars.get(last..).unwrap_or_default());
    let moved: Vec<(usize, Text)> = run
        .breaks
        .iter()
        .enumerate()
        .filter(|(j, _)| keep || !texts.contains_key(j))
        .map(|(j, (at, kind))| {
            let shift: usize = inserted
                .iter()
                .filter(|(k, p, _)| *p < *at || (*p == *at && *k < j))
                .map(|(_, _, v)| v.chars().count())
                .sum();
            (at + shift, kind.clone())
        })
        .collect();
    Run {
        text: text(&out),
        breaks: Arc::from(moved),
        ..run.clone()
    }
}

/// `run` as runs, with `texts` (mark index -> text) put where those marks are, each a run
/// of its own that draws a note's number, and the marks removed (`keep`: at the start of
/// their number's run); every other mark stays with the text around it (`_split`).
fn split(run: &Run, texts: &BTreeMap<usize, String>, keep: bool) -> Vec<Run> {
    let chars: Vec<char> = run.text.chars().collect();
    let mut out = Vec::new();
    let mut last = 0;
    let mut pending: Vec<(usize, Text)> = Vec::new();
    let piece = |start: usize, stop: usize, pending: &[(usize, Text)], out: &mut Vec<Run>| {
        let marks: Vec<(usize, Text)> = pending.iter().map(|(at, kind)| (at - start, kind.clone())).collect();
        let part: String = chars.get(start..stop).unwrap_or_default().iter().collect();
        if !part.is_empty() || !marks.is_empty() {
            out.push(Run {
                text: text(&part),
                breaks: Arc::from(marks),
                ..run.clone()
            });
        }
    };
    for (k, (at, kind)) in run.breaks.iter().enumerate() {
        match texts.get(&k) {
            Some(value) => {
                piece(last, *at, &pending, &mut out);
                pending.clear();
                let marks = if keep { vec![(0, kind.clone())] } else { Vec::new() };
                out.push(Run {
                    text: text(value),
                    breaks: Arc::from(marks),
                    drawings: Arc::from(Vec::new()),
                    anchors: Arc::from(Vec::new()),
                    note_number: true,
                    ..run.clone()
                });
                last = *at;
            }
            None => pending.push((*at, kind.clone())),
        }
    }
    piece(last, chars.len(), &pending, &mut out);
    out
}

/// `paragraph` with every mark starting `kind` replaced by its number, `numbers(mark)`
/// (`keep`: the mark kept at the number's start); the same `Arc` where none is there
/// (`_number_runs`).
fn number_runs(
    paragraph: &Arc<Paragraph>,
    kind: &str,
    numbers: &mut dyn FnMut(&str) -> String,
    keep: bool,
) -> Arc<Paragraph> {
    if !paragraph
        .runs
        .iter()
        .any(|r| r.breaks.iter().any(|(_, m)| m.starts_with(kind)))
    {
        return Arc::clone(paragraph);
    }
    let mut runs = Vec::with_capacity(paragraph.runs.len() + 2);
    for run in paragraph.runs.iter() {
        let texts: BTreeMap<usize, String> = run
            .breaks
            .iter()
            .enumerate()
            .filter(|(_, (_, m))| m.starts_with(kind))
            .map(|(k, (_, m))| (k, numbers(m)))
            .collect();
        let drawn = run
            .breaks
            .iter()
            .any(|(_, m)| m.starts_with("drawing") || m.starts_with("anchor:"));
        if texts.is_empty() {
            runs.push(run.clone());
        } else if drawn {
            runs.push(splice(run, &texts, keep));
        } else {
            runs.extend(split(run, &texts, keep));
        }
    }
    Arc::new(Paragraph {
        runs: Arc::from(runs),
        ..(**paragraph).clone()
    })
}

/// `block` with [`number_runs`] applied to every paragraph in it, cells included.
fn number_block(block: &Block, numbers: &mut dyn FnMut(&Arc<Paragraph>) -> Arc<Paragraph>) -> Block {
    match block {
        Block::Paragraph(p) => Block::Paragraph(numbers(p)),
        Block::Table(t) => {
            let rows: Vec<_> = t
                .rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| cell.iter().map(|b| number_block(b, numbers)).collect::<Arc<[Block]>>())
                        .collect::<Arc<[_]>>()
                })
                .collect();
            Block::Table(Arc::new(Table {
                rows: rows.into(),
                ..(**t).clone()
            }))
        }
    }
}

/// Each top-level block's section (`_section_numbers`).
fn section_numbers(document: &WordDocument) -> Vec<usize> {
    let last = document.sections.len().saturating_sub(1);
    let mut top_level = 0;
    document
        .body
        .iter()
        .map(|block| {
            let section = document
                .sections
                .iter()
                .position(|s| s.first_paragraph <= top_level && top_level < s.last_paragraph)
                .unwrap_or(last);
            if matches!(block, Block::Paragraph(_)) {
                top_level += 1;
            }
            section
        })
        .collect()
}

/// The note id a reference mark names (`footnoteReference:<id>[:custom]`), and whether
/// it is a custom mark.
fn reference_id(mark: &str) -> (Option<i64>, bool) {
    let mut parts = mark.split(':');
    let id = parts.nth(1).and_then(|p| p.parse::<i64>().ok());
    (id, parts.next() == Some("custom"))
}

/// A version with its endnotes numbered and placed in the flow, and each top-level
/// block's part in that: none for the file's own blocks.
#[derive(Debug, Clone)]
pub struct Placed {
    /// The version.
    pub document: WordDocument,
    /// Per top-level block, what of the notes it is (`None`: a block of the body).
    pub roles: Vec<Option<NoteRole>>,
}

impl Placed {
    /// A version with nothing placed.
    pub fn none(document: &WordDocument) -> Placed {
        Placed {
            roles: vec![None; document.body.len()],
            document: document.clone(),
        }
    }
}

/// An endnote referenced: its id and the number its reference draws (`""` for a custom
/// mark).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Referenced {
    id: Option<i64>,
    number: String,
}

/// The endnotes referenced in one section (or placed after one), in order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SectionNotes {
    section: usize,
    notes: Vec<Referenced>,
}

/// Push `note` to section `section`'s notes in `all`, made where there are none yet.
fn push_note(all: &mut Vec<SectionNotes>, section: usize, note: Referenced) {
    match all.iter_mut().find(|s| s.section == section) {
        Some(found) => found.notes.push(note),
        None => all.push(SectionNotes {
            section,
            notes: vec![note],
        }),
    }
}

/// The body with every endnote reference's number put in (each section's counter
/// starting at its `w:numStart` where it restarts, and in the first), and the notes each
/// section references.
fn number_endnotes(document: &WordDocument, sections: &[usize]) -> (Vec<Block>, Vec<SectionNotes>) {
    let mut counter = 1;
    let mut current: Option<usize> = None;
    let mut referenced: Vec<SectionNotes> = Vec::new();
    let mut body: Vec<Block> = Vec::with_capacity(document.body.len());
    for (block, &section) in document.body.iter().zip(sections) {
        let props = document.sections.get(section).map(|s| Arc::clone(&s.properties));
        let mut numbered = |p: &Arc<Paragraph>| -> Arc<Paragraph> {
            if current != Some(section) {
                let restarts = props.as_ref().and_then(|s| s.endnotes.restart.as_deref()) == Some("eachSect");
                if current.is_none() || restarts {
                    counter = props.as_ref().and_then(|s| s.endnotes.start).unwrap_or(1);
                }
                current = Some(section);
            }
            let format = props.as_ref().and_then(|s| s.endnotes.format.clone());
            let mut number = |mark: &str| -> String {
                let (id, custom) = reference_id(mark);
                if custom {
                    let number = String::new();
                    push_note(&mut referenced, section, Referenced { id, number });
                    return String::new();
                }
                let value = number_text(counter, format.as_deref(), DEFAULT_ENDNOTE_FORMAT);
                counter += 1;
                if id.is_some() {
                    let number = value.clone();
                    push_note(&mut referenced, section, Referenced { id, number });
                }
                value
            };
            number_runs(p, "endnoteReference:", &mut number, false)
        };
        body.push(number_block(block, &mut numbered));
    }
    (body, referenced)
}

/// The document with its endnotes numbered and placed (`with_endnotes`).
pub fn with_endnotes(document: &WordDocument) -> Placed {
    if !document.body.iter().any(|b| has_mark(b, "endnoteReference:")) {
        return Placed::none(document);
    }
    let sections = section_numbers(document);
    let (body, referenced) = number_endnotes(document, &sections);
    let docend = document.context.settings.endnote_position.as_deref() != Some("sectEnd");
    let last_section = document.sections.len().saturating_sub(1);
    let mut by_end: Vec<SectionNotes> = Vec::new();
    for found in referenced {
        let target = if docend { last_section } else { found.section };
        for note in found.notes {
            push_note(&mut by_end, target, note);
        }
    }
    let mut groups: Vec<(usize, Vec<(Block, NoteRole)>)> = by_end
        .into_iter()
        .map(|found| (found.section, endnote_group(document, &found.notes)))
        .collect();
    let mut out: Vec<Block> = Vec::with_capacity(body.len());
    let mut roles = Vec::with_capacity(body.len());
    for (k, (block, section)) in body.into_iter().zip(&sections).enumerate() {
        out.push(block);
        roles.push(None);
        let last_of_section = sections.get(k + 1).is_none_or(|next| next != section);
        if let Some(at) = groups
            .iter()
            .position(|(t, _)| t == section)
            .filter(|_| last_of_section)
        {
            let (_, group) = groups.remove(at);
            for (b, role) in group {
                out.push(b);
                roles.push(Some(role));
            }
        }
    }
    for (b, role) in groups.into_iter().flat_map(|(_, group)| group) {
        out.push(b);
        roles.push(Some(role));
    }
    let sections = renumbered(&document.sections, &out, &roles);
    let mut placed = document.clone();
    placed.body = Arc::from(out);
    placed.sections = sections;
    Placed {
        document: placed,
        roles,
    }
}

/// The separator's blocks, then each note's, its own mark numbered.
fn endnote_group(document: &WordDocument, notes: &[Referenced]) -> Vec<(Block, NoteRole)> {
    let mut group: Vec<(Block, NoteRole)> = separator(document, "separator", NotesPart::Endnotes)
        .into_iter()
        .map(|b| (b, NoteRole::Separator))
        .collect();
    for Referenced { id, number: value } in notes {
        let Some(blocks) = id.and_then(|id| document.endnotes.notes.get(&id)) else {
            continue;
        };
        for block in blocks.iter() {
            let block = match block {
                Block::Paragraph(p) => Block::Paragraph(number_runs(p, "endnoteRef", &mut |_| value.clone(), false)),
                other => other.clone(),
            };
            group.push((block, NoteRole::Endnote));
        }
    }
    group
}

/// The sections over a body with notes placed: each section's range of top-level
/// paragraphs grown by the notes placed in it (`_renumbered`).
fn renumbered(sections: &[Section], body: &[Block], roles: &[Option<NoteRole>]) -> Arc<[Section]> {
    let paragraphs = body.iter().filter(|b| matches!(b, Block::Paragraph(_))).count();
    let mut new_index: BTreeMap<usize, usize> = BTreeMap::new();
    let (mut top_level, mut original) = (0, 0);
    for (block, role) in body.iter().zip(roles) {
        if matches!(block, Block::Table(_)) {
            continue;
        }
        if role.is_none() {
            new_index.insert(original, top_level);
            original += 1;
        }
        top_level += 1;
    }
    let bounds: Vec<usize> = sections
        .iter()
        .map(|s| {
            if s.first_paragraph < original {
                new_index.get(&s.first_paragraph).copied().unwrap_or(paragraphs)
            } else {
                paragraphs
            }
        })
        .collect();
    sections
        .iter()
        .enumerate()
        .map(|(k, s)| Section {
            properties: Arc::clone(&s.properties),
            first_paragraph: bounds.get(k).copied().unwrap_or(paragraphs),
            last_paragraph: bounds.get(k + 1).copied().unwrap_or(paragraphs),
        })
        .collect()
}

/// Whether any paragraph of a block, cells included, holds a mark starting `prefix`.
fn has_mark(block: &Block, prefix: &str) -> bool {
    match block {
        Block::Paragraph(p) => p
            .runs
            .iter()
            .any(|r| r.breaks.iter().any(|(_, m)| m.starts_with(prefix))),
        Block::Table(t) => t
            .rows
            .iter()
            .any(|row| row.iter().any(|cell| cell.iter().any(|b| has_mark(b, prefix)))),
    }
}

/// Every mark of a block's paragraphs, cells included.
fn block_marks(block: &Block) -> Vec<Text> {
    let mut out = Vec::new();
    walk_paragraphs(block, &mut |p| {
        for run in p.runs.iter() {
            out.extend(run.breaks.iter().map(|(_, m)| m.clone()));
        }
    });
    out
}

fn walk_paragraphs(block: &Block, each: &mut dyn FnMut(&Paragraph)) {
    match block {
        Block::Paragraph(p) => each(p),
        Block::Table(t) => {
            for row in t.rows.iter() {
                for cell in row.iter() {
                    for inner in cell.iter() {
                        walk_paragraphs(inner, each);
                    }
                }
            }
        }
    }
}

/// One footnote reference of the body: its note, section, whether it is a custom mark,
/// whether it is in a table (`footnote_references`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FootnoteReference {
    /// The note's id.
    pub note: i64,
    /// The section of its top-level block.
    pub section: usize,
    /// A custom mark (`w:customMarkFollows`): it draws no number.
    pub custom: bool,
    /// In a table cell.
    pub in_table: bool,
}

/// Every footnote reference of the body in order, table cells included.
pub fn footnote_references(document: &WordDocument) -> Vec<FootnoteReference> {
    let sections = section_numbers(document);
    let mut out = Vec::new();
    for (block, &section) in document.body.iter().zip(&sections) {
        let in_table = matches!(block, Block::Table(_));
        for mark in block_marks(block) {
            if let Some(rest) = mark.strip_prefix("footnoteReference:") {
                let (id, custom) = reference_id(&format!(":{rest}"));
                if let Some(note) = id {
                    out.push(FootnoteReference {
                        note,
                        section,
                        custom,
                        in_table,
                    });
                }
            }
        }
    }
    out
}

/// Note id -> the page (0-based) its reference's line is on: what footnotes restarting on
/// each page are numbered by.
pub type FootnotePages = BTreeMap<i64, usize>;

/// Each footnote reference's number, [`footnote_references`]' order (`""` for a custom
/// mark): the section's `w:footnotePr` (`footnote_numbers`).
pub fn footnote_numbers(document: &WordDocument, pages: Option<&FootnotePages>) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 0;
    let mut by_section: BTreeMap<usize, i64> = BTreeMap::new();
    let mut by_page: BTreeMap<(usize, Option<usize>), i64> = BTreeMap::new();
    for reference in footnote_references(document) {
        if reference.custom {
            out.push(String::new());
            continue;
        }
        index += 1;
        let in_section = by_section.entry(reference.section).or_insert(0);
        *in_section += 1;
        let numbering = document
            .sections
            .get(reference.section)
            .map(|s| &s.properties.footnotes);
        let restart = numbering.and_then(|n| n.restart.as_deref()).unwrap_or("continuous");
        let start = numbering.and_then(|n| n.start).unwrap_or(1);
        let place = match restart {
            "eachSect" => *in_section,
            "eachPage" => {
                let page = pages.and_then(|p| p.get(&reference.note)).copied();
                let count = by_page.entry((reference.section, page)).or_insert(0);
                *count += 1;
                *count
            }
            _ => index,
        };
        let format = numbering.and_then(|n| n.format.as_deref());
        out.push(number_text(start - 1 + place, format, DEFAULT_FOOTNOTE_FORMAT));
    }
    out
}

/// Whether any section's footnotes restart on each page.
pub fn footnotes_by_page(document: &WordDocument) -> bool {
    document
        .sections
        .iter()
        .any(|s| s.properties.footnotes.restart.as_deref() == Some("eachPage"))
}

/// The document with every footnote reference's number put in as a run of its own that
/// still holds the reference's mark (for the paginator), and every note's own mark
/// numbered (`with_footnotes`).
pub fn with_footnotes(document: &WordDocument, pages: Option<&FootnotePages>) -> WordDocument {
    if !document.body.iter().any(|b| has_mark(b, "footnoteReference:")) {
        return document.clone();
    }
    let mut numbers = footnote_numbers(document, pages).into_iter();
    let mut by_note: BTreeMap<i64, String> = BTreeMap::new();
    let mut numbered = |p: &Arc<Paragraph>| -> Arc<Paragraph> {
        let mut number = |mark: &str| -> String {
            let value = numbers.next().unwrap_or_default();
            if let Some(id) = mark.split(':').nth(1).and_then(|s| s.parse::<i64>().ok()) {
                by_note.entry(id).or_insert_with(|| value.clone());
            }
            value
        };
        number_runs(p, "footnoteReference:", &mut number, true)
    };
    let body: Vec<Block> = document.body.iter().map(|b| number_block(b, &mut numbered)).collect();
    let notes = document
        .footnotes
        .notes
        .iter()
        .map(|(id, blocks)| {
            let value = by_note.get(id).cloned().unwrap_or_default();
            let numbered: Vec<Block> = blocks
                .iter()
                .map(|b| match b {
                    Block::Paragraph(p) => {
                        Block::Paragraph(number_runs(p, "footnoteRef", &mut |_| value.clone(), false))
                    }
                    other => other.clone(),
                })
                .collect();
            (*id, Arc::from(numbered))
        })
        .collect();
    let mut out = document.clone();
    out.body = Arc::from(body);
    out.footnotes = Notes {
        notes,
        ..document.footnotes.clone()
    };
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(number_text(3, None, DEFAULT_ENDNOTE_FORMAT), "iii");
        assert_eq!(number_text(5, Some("chicago"), DEFAULT_ENDNOTE_FORMAT), "**");
        assert_eq!(number_text(27, Some("lowerLetter"), DEFAULT_FOOTNOTE_FORMAT), "aa");
        assert_eq!(number_text(4, None, DEFAULT_FOOTNOTE_FORMAT), "4");
    }

    fn run(text_: &str, marks: &[(usize, &str)]) -> Run {
        Run {
            text: text(text_),
            breaks: Arc::from(marks.iter().map(|(a, m)| (*a, text(m))).collect::<Vec<_>>()),
            ..Run::default()
        }
    }

    #[test]
    fn split_keeps_marks_with_their_text() {
        let r = run("abcd", &[(1, "tab"), (2, "footnoteReference:3"), (3, "tab")]);
        let texts = BTreeMap::from([(1, "7".to_owned())]);
        let out = split(&r, &texts, true);
        type Shape = (String, Vec<(usize, String)>, bool);
        let shape: Vec<Shape> = out
            .iter()
            .map(|r| {
                (
                    r.text.to_string(),
                    r.breaks.iter().map(|(a, m)| (*a, m.to_string())).collect(),
                    r.note_number,
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                ("ab".into(), vec![(1, "tab".into())], false),
                ("7".into(), vec![(0, "footnoteReference:3".into())], true),
                ("cd".into(), vec![(1, "tab".into())], false),
            ]
        );
    }

    #[test]
    fn splice_shifts_later_marks() {
        let r = run("ab", &[(1, "endnoteReference:2"), (1, "drawing:1:1:0:0:0:0")]);
        let texts = BTreeMap::from([(0, "ii".to_owned())]);
        let out = splice(&r, &texts, false);
        assert_eq!(&*out.text, "aiib");
        assert_eq!(out.breaks.first().map(|(a, _)| *a), Some(3));
    }
}
