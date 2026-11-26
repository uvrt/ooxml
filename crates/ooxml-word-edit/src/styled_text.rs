//! Owns: [`SetStyledText`], one item of `word_set_text` as docx-agent's tool writes it
//! (`tools/text.py`: `_plan`, `_set_one`): text with inline emphasis read
//! ([`ooxml_markdown::styled`]); the paragraph style set first where asked (already so is a
//! note); the plain text written -- a paragraph's, in lines a paragraph each after it, or a
//! cell's (its paragraphs replaced, `_set_cell`); then each written paragraph's pieces
//! given their character styles ([`crate::style_spans`]) unless they look so already
//! (`_looks_as`, rule 3): `**` the strong role's style, `*` the emphasis role's (added as
//! Word writes them where the document lacks them, said in a note), a span its named style
//! -- all one batch, revision ids made unique once it ends.
//!
//! Must not: write but through the commands it composes, or know a tool's JSON.

use crate::names::W;
use crate::policy::RevisionPolicy;
use crate::set_text::{find, write_text};
use crate::sink::EditSink;
use crate::style_spans::{style_spans, Span};
use crate::text::write::check_text;
use crate::work::Work;
use crate::{EditError, Result, WordDoc};
use ooxml_markdown::styled::{parse, Piece, Styled};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::resolve::in_table;
use ooxml_word_model::docx::{style_kind, style_name, Docx};

/// The style insert_markdown's default style map writes `construct` in (`_role_styles`:
/// the strong and emphasis roles).
fn role_style(construct: &str) -> String {
    crate::markdown::WriteMap::default()
        .style_for(construct, 0)
        .and_then(|r| r.style.clone())
        .unwrap_or_default()
}

/// Write one paragraph's (or cell's) text with inline emphasis.
#[derive(Debug, Clone)]
pub struct SetStyledText {
    /// The paragraph, or a cell `t:X/c<row>,<col>`.
    pub target: ooxml_word_model::address::Address,
    /// The whole text: `\n` starts a paragraph; `**strong**`, `*emphasis*`,
    /// `[text]{Character style}`.
    pub text: String,
    /// The paragraph style the paragraphs written take.
    pub paragraph_style: Option<String>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// One edit of the item, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemEdit {
    /// The paragraph written, made or styled; `None` for a deletion.
    pub id: Option<String>,
    /// Whether it changed anything.
    pub changed: bool,
    /// Ids renamed, old to new.
    pub renamed: ooxml_word_model::ids::Renames,
    /// Paragraphs made.
    pub created: Vec<String>,
    /// Paragraphs removed.
    pub removed: Vec<String>,
}

/// A styled piece written: where, its text, its style and direct emphasis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledRun {
    /// `<paragraph>@<start>:<end>`.
    pub range: String,
    /// Its text.
    pub text: String,
    /// The character style's name.
    pub style: String,
    /// Direct bold beside the span's style.
    pub bold: bool,
    /// Direct italic beside it.
    pub italic: bool,
}

/// What the item did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledTextSet {
    /// Its edits, in order.
    pub edits: Vec<ItemEdit>,
    /// The styled pieces, in order.
    pub styled: Vec<StyledRun>,
    /// Notes: a paragraph already in the style, a style added.
    pub notes: Vec<String>,
    /// The paragraphs written, in order.
    pub written: Vec<String>,
}

/// The item read and checked before anything changes (`_SetPlan`).
struct Plan {
    lines: Vec<Styled>,
    cell: Option<(String, usize, usize)>,
    /// The strong and emphasis roles' style names.
    roles: (String, String),
    added: Vec<String>,
}

impl Plan {
    fn marked(&self) -> bool {
        self.lines.iter().any(Styled::marked)
    }

    fn plain(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.plain.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A piece's style and direct bold and italic: a span's style, else the strong role's,
    /// else the emphasis role's.
    fn look(&self, piece: &Piece) -> (Option<String>, bool, bool) {
        match &piece.style {
            Some(style) => (Some(style.clone()), piece.strong, piece.em),
            None if piece.strong => (Some(self.roles.0.clone()), false, piece.em),
            None if piece.em => (Some(self.roles.1.clone()), false, false),
            None => (None, false, false),
        }
    }

    fn spans(&self, line: &Styled) -> Vec<Span> {
        line.pieces
            .iter()
            .map(|p| {
                let (style, bold, italic) = self.look(p);
                Span {
                    start: p.start,
                    end: p.end,
                    style,
                    bold,
                    italic,
                }
            })
            .collect()
    }
}

impl Command<WordDoc> for SetStyledText {
    type Out = StyledTextSet;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        plan(doc, &*doc.docx()?, self).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<StyledTextSet> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let docx = sink.docx()?;
        let plan = plan(sink.doc(), &docx, self)?;
        let out = set_one(&mut sink, self, &plan)?;
        // the tool call's batch, once it ends: the written text spaced, text boxes whose
        // content changed synced, revision ids unique
        if out.edits.iter().any(|e| e.changed) {
            crate::after::after_edit(&mut sink)?;
        }
        Ok(out)
    }

    /// Its text, the paragraph it names and the styles it names; several lines into a cell,
    /// the whole document.
    fn read_refusal(&self, tx: &mut Txn<'_, WordDoc>) {
        if crate::paragraphs::is_lines(&self.text) {
            tx.read_everything();
            return;
        }
        crate::refusal::read_paragraph(tx, self.target.as_str());
        crate::refusal::read_styles(tx);
    }
}

/// `_plan`: the target, the lines read, the styles checked and the roles' styles.
fn plan(doc: &WordDoc, docx: &Docx, item: &SetStyledText) -> Result<Plan> {
    let split = crate::paragraphs::is_lines(&item.text);
    let cell = cell_of(docx, item.target.as_str())?.filter(|_| split);
    if cell.is_none() {
        find(doc, docx, item.target.as_str())?;
    }
    let lines: Vec<Styled> = crate::paragraphs::lines(&item.text).into_iter().map(parse).collect();
    for line in &lines {
        check_text(&line.plain)?;
    }
    styles_known(docx, item.paragraph_style.as_deref(), &lines)?;
    let mut plan = Plan {
        lines,
        cell,
        roles: (role_style("strong"), role_style("emphasis")),
        added: Vec::new(),
    };
    if plan.marked() {
        let used = |strong: bool| {
            plan.lines
                .iter()
                .flat_map(|l| &l.pieces)
                .any(|p| p.style.is_none() && if strong { p.strong } else { p.em })
        };
        let (strong, em) = (used(true), used(false));
        let mut role = |name: &str, used: bool| match docx.find_style(name, "character") {
            Some(found) => found
                .child_elements()
                .find(|c| c.name().is(Some(W), "name"))
                .and_then(|n| n.attribute(Some(W), "val"))
                .filter(|n| !n.is_empty())
                .map_or_else(|| name.to_owned(), str::to_owned),
            None => {
                if used {
                    plan.added.push(format!("style added: {name}, as Word writes it"));
                }
                name.to_owned()
            }
        };
        plan.roles = (role(&role_style("strong"), strong), role(&role_style("emphasis"), em));
        let named: Vec<String> = {
            let mut out: Vec<String> = Vec::new();
            for name in plan.lines.iter().flat_map(Styled::styles_named) {
                if !out.iter().any(|n| n == name) {
                    out.push(name.to_owned());
                }
            }
            out
        };
        for name in named {
            if docx.find_style(&name, "character").is_none() {
                plan.added.push(format!("style added: {name}, as Word writes it"));
            }
        }
    }
    Ok(plan)
}

impl SetStyledText {
    /// Whether the styles the item names are known on `docx` (its paragraph style, its spans'
    /// character styles), as the item's plan checks them first: a tool finds the item a
    /// batch's style refusal is about by it.
    ///
    /// # Errors
    /// The style refusal [`SetStyledText`] would give.
    pub fn styles_known(&self, docx: &Docx) -> Result<()> {
        let lines: Vec<Styled> = crate::paragraphs::lines(&self.text).into_iter().map(parse).collect();
        styles_known(docx, self.paragraph_style.as_deref(), &lines)
    }
}

/// `_plan`'s style checks: the paragraph style (`known_style`), then each span's character
/// style in the order the text names them (`_character_style`, when the text is marked).
fn styles_known(docx: &Docx, paragraph_style: Option<&str>, lines: &[Styled]) -> Result<()> {
    if let Some(style) = paragraph_style {
        crate::builtin::resolve(docx, style, "paragraph")
            .map_err(|e| style_refused(docx, e, ("paragraph", "paragraph_style"), None))?;
    }
    if !lines.iter().any(Styled::marked) {
        return Ok(());
    }
    let mut seen: Vec<&str> = Vec::new();
    for name in lines.iter().flat_map(Styled::styles_named) {
        if seen.contains(&name) {
            continue;
        }
        seen.push(name);
        crate::builtin::resolve(docx, name, "character")
            .map_err(|e| style_refused(docx, e, ("character", "text"), Some(name)))?;
    }
    Ok(())
}

/// A style the item names refused as docx-agent's tool refuses it (`known_style`,
/// `_character_style`): not found, naming its argument (`field`: `paragraph_style`, or `text`
/// for a span's), the document's named styles of `kind` (the first 50) the options.
fn style_refused(docx: &Docx, error: EditError, (kind, field): (&str, &str), span: Option<&str>) -> EditError {
    let EditError::Refused(message) = error else {
        return error;
    };
    let message = match span {
        Some(name) => format!(
            "[...]{{{name}}}: {message}; a span names a character style of the template (or write \\[ for a literal bracket)"
        ),
        None => message,
    };
    let options = docx
        .styles()
        .iter()
        .filter(|s| style_kind(s) == kind)
        .filter_map(|s| style_name(s).filter(|n| !n.is_empty()).map(str::to_owned))
        .take(50)
        .collect();
    crate::styles::not_found_at(message, field, options)
}

/// The cell `target` names, as its table's id and grid place; `None` for another target.
fn cell_of(docx: &Docx, target: &str) -> Result<Option<(String, usize, usize)>> {
    let Ok(Some(found)) = in_table(docx, target) else {
        return Ok(None);
    };
    let Some(column) = found.column else {
        return Ok(None);
    };
    let table = found
        .part
        .ids
        .tables()
        .get(found.table)
        .ok_or_else(|| EditError::Refused(format!("{target} is not a cell")))?;
    Ok(Some((table.id.clone(), found.row, column)))
}

/// `_set_one`: the item's style, text and emphasis.
fn set_one(sink: &mut EditSink<'_, '_>, item: &SetStyledText, plan: &Plan) -> Result<StyledTextSet> {
    let mut out = StyledTextSet {
        edits: Vec::new(),
        styled: Vec::new(),
        notes: Vec::new(),
        written: Vec::new(),
    };
    let text = plan.plain();
    let mut target = item.target.as_str().to_owned();
    if let Some(style) = &item.paragraph_style {
        target = paragraph_style(sink, item, plan, style, &mut out)?.unwrap_or(target);
    }
    if let Some((table, row, column)) = &plan.cell {
        let lines: Vec<&str> = text.split('\n').collect();
        if cell_texts(sink, item.target.as_str())? != lines {
            for e in crate::set_cells::set_cell(sink, table, (*row, *column), &text)? {
                out.edits.push(ItemEdit {
                    changed: true,
                    id: e.id,
                    renamed: e.renamed,
                    created: e.created,
                    removed: e.removed,
                });
            }
        }
        out.written = cell_paragraph_ids(sink, item.target.as_str())?;
        if out.edits.is_empty() {
            out.edits
                .push(edit(out.written.first().cloned(), false, Vec::new(), Vec::new()));
        }
    } else if plan.lines.len() > 1 {
        let made = crate::paragraphs::set_lines_in(sink, &target, &text)?;
        out.written = std::iter::once(made.id.to_string())
            .chain(made.created.iter().cloned())
            .collect();
        out.edits.push(edit(
            Some(made.id.to_string()),
            made.changed,
            made.renamed,
            made.created,
        ));
    } else {
        let made = write_text(sink, &target, &text)?;
        out.written = vec![made.id.to_string()];
        out.edits
            .push(edit(Some(made.id.to_string()), made.changed, made.renamed, Vec::new()));
    }
    if plan.marked() {
        for (paragraph, line) in out.written.clone().iter().zip(&plan.lines) {
            let spans = plan.spans(line);
            if looks_as(sink, paragraph, &spans)? {
                continue;
            }
            let made = style_spans(sink, paragraph, &spans)?;
            out.edits
                .push(edit(Some(made.id.to_string()), true, made.renamed, Vec::new()));
        }
        for (paragraph, line) in out.written.clone().iter().zip(&plan.lines) {
            let id = sink.doc().ids().follow(paragraph);
            out.styled.extend(styled_runs(&id, line, plan));
        }
        out.notes.extend(plan.added.iter().cloned());
    }
    Ok(out)
}

fn edit(id: Option<String>, changed: bool, renamed: ooxml_word_model::ids::Renames, created: Vec<String>) -> ItemEdit {
    ItemEdit {
        id,
        changed,
        renamed,
        created,
        removed: Vec::new(),
    }
}

/// `_paragraph_style`: the item's style on its paragraph (a cell in lines: on each of the
/// cell's paragraphs) unless it has it already (a note); the paragraph's id after it.
fn paragraph_style(
    sink: &mut EditSink<'_, '_>,
    item: &SetStyledText,
    plan: &Plan,
    style: &str,
    out: &mut StyledTextSet,
) -> Result<Option<String>> {
    let docx = sink.docx()?;
    let targets = match plan.cell {
        Some(_) => cell_paragraph_ids(sink, item.target.as_str())?,
        None => vec![find(sink.doc(), &docx, item.target.as_str())?.id],
    };
    let wanted = crate::builtin::resolve(&docx, style, "paragraph")?;
    let default = docx
        .default_style("paragraph")
        .and_then(|s| s.attribute(Some(W), "styleId"))
        .map(str::to_owned);
    let mut first = None;
    for paragraph in targets {
        let docx = sink.docx()?;
        let found = find(sink.doc(), &docx, &paragraph)?;
        let current = crate::paragraphs::style_of(&found.element)
            .map(str::to_owned)
            .or_else(|| default.clone());
        if current.as_deref() == Some(wanted.as_str()) {
            out.notes.push(format!("{} is already in {style}", found.id));
            first.get_or_insert(found.id);
            continue;
        }
        let made = crate::paragraph_style::set_style(sink, &paragraph, Some(style))?;
        first.get_or_insert(made.id.to_string());
        out.edits
            .push(edit(Some(made.id.to_string()), true, made.renamed, Vec::new()));
    }
    Ok(first.filter(|_| plan.cell.is_none()))
}

/// The ids of the paragraphs of the cell `target` names, in order (its table's id
/// followed through renames, as the stamping may have renamed it).
fn cell_paragraph_ids(sink: &mut EditSink<'_, '_>, target: &str) -> Result<Vec<String>> {
    let docx = sink.docx()?;
    let target = match target.rsplit_once('/') {
        Some((table, cell)) => format!("{}/{cell}", sink.doc().ids().follow(table)),
        None => target.to_owned(),
    };
    let target = target.as_str();
    let found = in_table(&docx, target)
        .map_err(EditError::Address)?
        .ok_or_else(|| EditError::Refused(format!("{target} is not a cell")))?;
    let (table, column) = (found.part.ids.tables().get(found.table), found.column);
    let (Some(table), Some(column)) = (table, column) else {
        return Err(EditError::Refused(format!("{target} is not a cell")));
    };
    let Some((row_path, _)) = table.rows.get(found.row) else {
        return Err(EditError::Refused(format!("{target} is not a cell")));
    };
    let Some(row) = ooxml_word_model::docx::at_path(found.part.root(), row_path) else {
        return Err(EditError::Refused(format!("{target} is not a cell")));
    };
    let Some(cell) = ooxml_word_model::address::resolve::cell_at(row, column) else {
        return Err(EditError::Refused(format!("{target} is not a cell")));
    };
    let mut path = row_path.to_vec();
    path.push(cell);
    Ok(found
        .part
        .ids
        .paragraphs_within(&path)
        .into_iter()
        .map(|p| p.id)
        .collect())
}

/// The texts of the cell's paragraphs.
fn cell_texts(sink: &mut EditSink<'_, '_>, target: &str) -> Result<Vec<String>> {
    let docx = sink.docx()?;
    let mut out = Vec::new();
    for id in cell_paragraph_ids(sink, target)? {
        let found = find(sink.doc(), &docx, &id)?;
        let work = Work::of(&found.element, Vec::new());
        out.push(crate::text::paragraph_text(&work, work.root()));
    }
    Ok(out)
}

/// `_looks_as`: whether the paragraph's characters have the spans' styles and emphasis
/// already (a repeat writes nothing).
fn looks_as(sink: &mut EditSink<'_, '_>, paragraph: &str, spans: &[Span]) -> Result<bool> {
    let docx = sink.docx()?;
    let found = find(sink.doc(), &docx, paragraph)?;
    sink.read(crate::set_text::block_key(&found.part, &found.id));
    let work = Work::of(&found.element, Vec::new());
    let atoms = crate::text::atoms(&work, work.root());
    for span in spans {
        let wanted = match &span.style {
            Some(name) => match docx.find_style(name, "character") {
                Some(style) => style.attribute(Some(W), "styleId").map(str::to_owned),
                None => return Ok(false),
            },
            None => None,
        };
        for atom in atoms.get(span.start..span.end).unwrap_or_default() {
            let Some(run) = atom.run.filter(|_| atom.kind == crate::text::AtomKind::Text) else {
                continue;
            };
            let properties = work.find(run, W, "rPr");
            let have = properties
                .and_then(|p| work.find(p, W, "rStyle"))
                .and_then(|s| work.get(s, Some(W), "val"))
                .map(str::to_owned);
            if wanted.is_none() && have.is_some() && linked(&work, run) {
                continue;
            }
            if have != wanted || on(&work, properties, "b") != span.bold || on(&work, properties, "i") != span.italic {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn on(work: &Work, properties: Option<crate::work::Id>, local: &str) -> bool {
    properties
        .and_then(|p| work.find(p, W, local))
        .is_some_and(|n| !matches!(work.get(n, Some(W), "val"), Some("0" | "false" | "off")))
}

fn linked(work: &Work, run: crate::work::Id) -> bool {
    let mut parent = work.parent(run);
    while let Some(p) = parent.filter(|p| !work.is(*p, W, "p")) {
        if work.is(p, W, "hyperlink") {
            return true;
        }
        parent = work.parent(p);
    }
    false
}

/// `_styled_runs`: the styled pieces of a written paragraph.
fn styled_runs(paragraph: &str, line: &Styled, plan: &Plan) -> Vec<StyledRun> {
    line.pieces
        .iter()
        .filter_map(|piece| {
            let (style, bold, italic) = plan.look(piece);
            Some(StyledRun {
                range: format!("{paragraph}@{}:{}", piece.start, piece.end),
                text: piece.text.clone(),
                style: style?,
                bold,
                italic,
            })
        })
        .collect()
}
