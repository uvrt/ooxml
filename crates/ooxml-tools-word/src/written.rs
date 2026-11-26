//! Owns: the values a host wrote into a call's arguments for references (`{{d:...}}`)
//! before the call ran ([`Written`]: ooxml-edit's `textfacts.Fact`, the host's `call.facts`),
//! and keeping each on the paragraph it went into after the call's edit (docx-agent's
//! `textfacts.keep_written`): found in the paragraphs the call wrote -- its reply's changed,
//! created and renamed blocks, its `data.paragraphs`, the targets it was given, tables and
//! cells as their paragraphs -- by the text around it in the argument (spaces and
//! Markdown's marks aside); a value found nowhere, or in two places alike, is not kept (a
//! figure is listed as typed rather than kept under a wrong place); `replace_text` keeps
//! its replacement at each place it wrote it. Also what a reply says of one value
//! ([`brief`], docx-agent's `field_text.brief`), and a value that is all of the text
//! written kept as the source of each paragraph it made ([`whole_keeps`],
//! `field_text.keep_in_paragraphs`).
//!
//! Must not: resolve a reference (the host's datasets do), or keep but through the edit
//! core's command ([`KeepText`]), in the transaction of the call's edit ([`run_keeping`]).

use crate::Editing;
use ooxml_base::pystr::is_digit;
use ooxml_store::{Command, Txn};
use ooxml_tools::{Done, Outcome, ToolResult};
use ooxml_word_edit::{EditError, KeepText, TextKeep, WordDoc};
use ooxml_word_model::address::resolve::{direct, in_table, scope_paragraphs};
use ooxml_word_model::address::Address;
use ooxml_word_model::docx::text::{paragraph_text, View};
use ooxml_word_model::docx::Docx;
use ooxml_word_model::ids::Entry;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::sync::Arc;

/// How far around a value its argument's text is compared with a paragraph's.
const CONTEXT: usize = 40;
/// A value written by reference longer than this (or over several lines) is not echoed in
/// a reply: `chars` says how long it was.
const ECHO_CHARS: usize = 200;
/// Characters Markdown writes that a paragraph's text does not have.
const MARKS: &str = "*_`|#>\\";

pub use ooxml_tools::written::Written;

/// What a reply says of `fact`: the reference, its text (a long or several-line one only
/// counted, `chars`), the blocks it made, its sources.
#[must_use]
pub(crate) fn brief(fact: &Written, blocks: Option<&[String]>) -> Value {
    let mut said = fact.brief.clone();
    let text = said.get("text").and_then(Value::as_str).unwrap_or_default().to_owned();
    let length = text.chars().count();
    if length > ECHO_CHARS || text.contains('\n') {
        said.remove("text");
        said.insert("chars".into(), Value::from(length));
    }
    if let Some(blocks) = blocks {
        said.insert(
            "blocks".into(),
            blocks.iter().map(|b| Value::from(b.as_str())).collect(),
        );
    }
    let mut out = Map::new();
    for key in ["from", "text", "chars", "blocks", "cell", "sources", "stale"] {
        if let Some(value) = said.remove(key) {
            out.insert(key.into(), value);
        }
    }
    Value::Object(out)
}

/// `fact` kept as the source of each paragraph of `blocks` with text in `docx`, its whole
/// text the part written.
#[must_use]
pub(crate) fn whole_keeps(after: &After, fact: &Written, blocks: &[String]) -> Vec<TextKeep> {
    let mut out = Vec::new();
    for block in blocks.iter().filter(|b| b.starts_with("p:")) {
        let Some(text) = text_of(after, block) else { continue };
        if text.is_empty() {
            continue;
        }
        let mut record = fact.record.clone();
        record.insert("text".into(), Value::from(text));
        record.insert("p".into(), Value::from(0));
        record.insert("at".into(), Value::from(0));
        out.push(TextKeep {
            paragraph: block.clone(),
            facts: vec![Value::Object(record)],
            labels: Vec::new(),
        });
    }
    out
}

/// The keeps of each value of `editing.written` found in the paragraphs the call wrote
/// (`done`'s blocks and `targets`), each argument's text given by `argument`; `count` the
/// places `replace_text` wrote its replacement in.
#[must_use]
pub(crate) fn written_keeps(
    after: &After,
    editing: &Editing<'_>,
    done: &Done<Value>,
    targets: &[&str],
    argument: &dyn Fn(&str) -> Option<String>,
) -> Vec<TextKeep> {
    if editing.written.is_empty() {
        return Vec::new();
    }
    let count = done
        .data
        .as_ref()
        .and_then(|d| d.get("count"))
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
        .map_or(1, |n| usize::try_from(n).unwrap_or(1));
    keeps_in(after, editing, &touched(after, done, targets), count, argument)
}

/// The keeps of each value of `editing.written` found in the paragraphs `touched` (by id,
/// in order), each argument's text given by `argument` (`None`: a field whose values are not
/// looked for, such as a handler's own `from`); `count` the places `replace_text` wrote its
/// replacement in.
#[must_use]
pub(crate) fn keeps_in(
    after: &After,
    editing: &Editing<'_>,
    touched: &[String],
    count: usize,
    argument: &dyn Fn(&str) -> Option<String>,
) -> Vec<TextKeep> {
    let paragraphs: Vec<(String, Vec<char>)> = touched
        .iter()
        .filter_map(|id| Some((id.clone(), text_of(after, id)?.chars().collect())))
        .collect();
    let mut taken: HashSet<(String, usize)> = HashSet::new();
    let mut kept: Vec<(String, Vec<Value>)> = Vec::new();
    for fact in editing.written {
        let Some(given) = argument(&fact.field) else { continue };
        let copies = if fact.field == "replace" { count } else { 1 };
        for (id, at) in chosen(places(fact, &given, &paragraphs, &taken), copies) {
            taken.insert((id.clone(), at));
            let mut record = fact.record.clone();
            record.insert("p".into(), Value::from(0));
            record.insert("at".into(), Value::from(at));
            match kept.iter_mut().find(|(k, _)| *k == id) {
                Some((_, facts)) => facts.push(Value::Object(record)),
                None => kept.push((id, vec![Value::Object(record)])),
            }
        }
    }
    kept.into_iter()
        .map(|(paragraph, facts)| TextKeep {
            paragraph,
            facts,
            labels: Vec::new(),
        })
        .collect()
}

/// The keeps of each value of `editing.written` found where the call's reply (`result`)
/// says it wrote (`targets` the targets it was given; `argument` each argument's text by
/// its field): docx-agent's `keep_written`, as its wrapper runs it after a tool's handler.
#[must_use]
pub(crate) fn reply_keeps(
    after: &After,
    editing: &Editing<'_>,
    result: &ToolResult<Value>,
    targets: &[&str],
    argument: &dyn Fn(&str) -> Option<String>,
) -> Vec<TextKeep> {
    match &result.outcome {
        Outcome::Done(done) if !editing.written.is_empty() => written_keeps(after, editing, done, targets, argument),
        _ => Vec::new(),
    }
}

/// A place a value is found at: how much of the text around it matches the argument's,
/// whether all of it does, the paragraph and the character.
type Place = (usize, bool, String, usize);

/// The places `fact` is found at in `paragraphs` (not one `taken` already, nor one with a
/// digit right beside it), best first; none where its text is not where it says in
/// `given`, the argument's text.
fn places(
    fact: &Written,
    given: &str,
    paragraphs: &[(String, Vec<char>)],
    taken: &HashSet<(String, usize)>,
) -> Vec<Place> {
    let given: Vec<char> = given.chars().collect();
    let own: Vec<char> = fact.text.chars().collect();
    if own.is_empty() || given.get(fact.start..fact.end) != Some(&own[..]) {
        return Vec::new();
    }
    let before = given
        .get(fact.start.saturating_sub(CONTEXT)..fact.start)
        .unwrap_or_default();
    let before = before.rsplit(|c| *c == '\n').next().unwrap_or_default();
    let after = given
        .get(fact.end..(fact.end + CONTEXT).min(given.len()))
        .unwrap_or_default();
    let after = after.split(|c| *c == '\n').next().unwrap_or_default();
    let left: Vec<char> = squeezed(before).into_iter().rev().collect();
    let right = squeezed(after);
    let mut found: Vec<Place> = Vec::new();
    for (id, text) in paragraphs {
        for at in occurrences(text, &own) {
            let end = at + own.len();
            let digit_before = at > 0 && text.get(at - 1).is_some_and(|c| is_digit(*c));
            let digit_after = text.get(end).is_some_and(|c| is_digit(*c));
            if digit_before || digit_after || taken.contains(&(id.clone(), at)) {
                continue;
            }
            let around: Vec<char> = squeezed(text.get(at.saturating_sub(CONTEXT)..at).unwrap_or_default())
                .into_iter()
                .rev()
                .collect();
            let a = common(&around, &left);
            let b = common(
                &squeezed(text.get(end..(end + CONTEXT).min(text.len())).unwrap_or_default()),
                &right,
            );
            found.push((a + b, a == left.len() && b == right.len(), id.clone(), at));
        }
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found
}

/// The places kept of `found` (best first): the ones the text around matches whole -- up
/// to `copies`, and only where they are no more than that or copies are asked for -- else
/// the one best place, where no other matches as well.
fn chosen(found: Vec<Place>, copies: usize) -> Vec<(String, usize)> {
    let whole: Vec<&Place> = found.iter().filter(|f| f.1).collect();
    let picked: Vec<&Place> = if !whole.is_empty() {
        if whole.len() <= copies || copies > 1 {
            whole.into_iter().take(copies).collect()
        } else {
            Vec::new()
        }
    } else if found.len() == 1 || found.first().map(|f| f.0) > found.get(1).map(|f| f.0) {
        found.iter().take(1).collect()
    } else {
        Vec::new()
    };
    picked.into_iter().map(|(_, _, id, at)| (id.clone(), *at)).collect()
}

/// `command` run through the call's editor with the values written by reference kept where
/// they went, in the same transaction -- one commit, one history step, all or nothing:
/// `finish` makes the call's reply of the command's out and the keeps of the version it
/// made (none: nothing more is written). The command's out and the reply.
///
/// # Errors
/// The edit's refusal (the command's, or a keep's).
pub(crate) fn run_keeping<C, F>(
    editing: &Editing<'_>,
    command: &C,
    finish: F,
) -> Result<(C::Out, ToolResult<Value>), EditError>
where
    C: Command<WordDoc, Error = EditError>,
    F: Fn(&C::Out, &After) -> (ToolResult<Value>, Vec<TextKeep>),
{
    editing
        .editor
        .run(editing.caller, &keeping(command, finish))
        .map(|done| done.out)
}

/// `command`, then the keeps `finish` makes of its out and the version it made, as one
/// command (for a caller that runs it inside another, [`run_keeping`] otherwise): its out
/// is the command's and what `finish` said beside the keeps.
pub(crate) fn keeping<C, F, R>(command: &C, finish: F) -> Keeping<'_, C, F>
where
    C: Command<WordDoc, Error = EditError>,
    F: Fn(&C::Out, &After) -> (R, Vec<TextKeep>),
{
    Keeping { command, finish }
}

/// A command, then the keeps its out names, as one transaction ([`keeping`]).
pub(crate) struct Keeping<'a, C, F> {
    command: &'a C,
    finish: F,
}

impl<C, F, R> Command<WordDoc> for Keeping<'_, C, F>
where
    C: Command<WordDoc, Error = EditError>,
    F: Fn(&C::Out, &After) -> (R, Vec<TextKeep>),
{
    type Out = (C::Out, R);
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        self.command.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Self::Out, EditError> {
        let out = self.command.prepare(tx)?;
        let after = After::of(tx.doc())?;
        let (said, keeps) = (self.finish)(&out, &after);
        if !keeps.is_empty() {
            let keep = KeepText { keeps };
            keep.validate(tx.doc())?;
            keep.prepare(tx)?;
        }
        Ok((out, said))
    }
}

/// A version the keeps read: its paragraphs, and the renames it keeps.
pub(crate) struct After {
    doc: WordDoc,
    docx: Arc<Docx>,
}

impl After {
    /// The version `doc` (a transaction's working one, as the call's edit left it).
    ///
    /// # Errors
    /// When a story part is not XML.
    fn of(doc: &WordDoc) -> Result<Self, EditError> {
        Ok(Self {
            docx: doc.docx()?,
            doc: doc.clone(),
        })
    }

    /// The id `name` leads to through the version's renames.
    fn follow(&self, name: &str) -> String {
        self.doc.ids().follow(name)
    }

    /// The version.
    pub(crate) fn doc(&self) -> &WordDoc {
        &self.doc
    }

    /// The version's parts, read.
    pub(crate) fn docx(&self) -> &Docx {
        &self.docx
    }
}

/// The paragraphs the call wrote, by id, in order (docx-agent's `_touched`).
fn touched(after: &After, done: &Done<Value>, targets: &[&str]) -> Vec<String> {
    let mut named: Vec<String> = done.changed.iter().chain(&done.created).cloned().collect();
    named.extend(done.renamed.iter().map(|(_, new)| new.clone()));
    if let Some(Value::Array(listed)) = done.data.as_ref().and_then(|d| d.get("paragraphs")) {
        named.extend(listed.iter().filter_map(Value::as_str).map(str::to_owned));
    }
    named.extend(targets.iter().map(|t| (*t).to_owned()));
    let mut seen = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for name in named {
        if !seen.insert(name.clone()) {
            continue;
        }
        let name = if name.contains('@') && !name.contains("..") {
            name.split('@').next().unwrap_or_default().to_owned()
        } else {
            name
        };
        out.extend(paragraphs_of(after, &name));
    }
    let mut unique = HashSet::new();
    out.retain(|id| unique.insert(id.clone()));
    out
}

/// The paragraphs `name` holds as docx-agent's `document.get` reads it: a paragraph, a
/// cell's paragraphs, a table's; nothing for any other address.
pub(crate) fn paragraphs_of(after: &After, name: &str) -> Vec<String> {
    let (docx, followed) = (&*after.docx, after.follow(name));
    let ids = |address: &str| {
        scope_paragraphs(docx, &Address::parse(address)).map_or_else(
            |_| Vec::new(),
            |(part, found)| {
                found
                    .into_iter()
                    .filter_map(|k| part.ids.paragraphs().get(k).map(|p| p.id))
                    .collect()
            },
        )
    };
    match direct(docx, &followed) {
        Some((part, Entry::Paragraph(k))) => part.ids.paragraphs().get(k).map(|p| p.id).into_iter().collect(),
        Some((_, Entry::Table(_))) => ids(&followed),
        _ => match in_table(docx, &followed) {
            Ok(Some(found)) if found.column.is_some() => ids(&followed),
            _ => Vec::new(),
        },
    }
}

/// The text of the paragraph `id` names (a rename followed), as `find_text` reads it.
fn text_of(after: &After, id: &str) -> Option<String> {
    let followed = after.follow(id);
    let (part, Entry::Paragraph(k)) = direct(&after.docx, &followed)? else {
        return None;
    };
    let paragraph = part.ids.paragraphs().get(k)?;
    Some(paragraph_text(&paragraph.element, View::Current))
}

/// Each place `own` starts in `text` (overlapping ones too, as `str.find` from the next
/// character finds them).
fn occurrences(text: &[char], own: &[char]) -> Vec<usize> {
    if own.is_empty() || own.len() > text.len() {
        return Vec::new();
    }
    (0..=text.len() - own.len())
        .filter(|&at| text.get(at..at + own.len()) == Some(own))
        .collect()
}

/// `text` without spaces of any kind and Markdown's marks: what is compared.
fn squeezed(text: &[char]) -> Vec<char> {
    text.iter()
        .copied()
        .filter(|c| !py_space(*c) && !MARKS.contains(*c))
        .collect()
}

/// How many characters `a` and `b` start with alike.
fn common(a: &[char], b: &[char]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

pub(crate) use ooxml_tools::written::py_space;
