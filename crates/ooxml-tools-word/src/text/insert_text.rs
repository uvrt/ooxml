//! Owns: `word_insert_text` (docx-agent's `word_insert_text`): text put beside words found
//! once, or at a range's start or end, or at a position -- every item one
//! [`InsertBeside`] in one atomic batch (one history step; each item finds the text the ones
//! before left) -- the new ranges' ids in the reply, each named by its `ref`.
//!
//! Must not: write the document but through the command, or answer a call it does not
//! support yet, refused ([`Unported`]).

use super::{by_reference, Unported};
use crate::review::{edit_refusal, needed};
use crate::structure::common::{one_of, outcome, Outcome};
use crate::{written, Editing};
use ooxml_base::pyfmt::repr_str;
use ooxml_store::Batch;
use ooxml_tools::{ArgEnum, Refusal, ToolArgs, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{InsertBeside, Placed, Side, TextPlace};
use serde_json::{json, Map, Value};

/// Which side of the place.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Where {
    /// At its start.
    Before,
    /// At its end.
    After,
}

impl Where {
    fn name(self) -> &'static str {
        match self {
            Where::Before => "before",
            Where::After => "after",
        }
    }
}

/// One insertion.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
pub struct InsertItem {
    /// Plain text.
    #[arg(facts)]
    pub text: String,
    /// Text occurring once, to insert beside.
    pub find: Option<String>,
    /// Or a range, position (p:A@5) or paragraph.
    #[arg(refs)]
    pub target: Option<String>,
    /// Default after.
    #[arg(rename = "where")]
    pub side: Option<Where>,
    /// Or after:/before: a range, or a position.
    pub at: Option<String>,
    /// Ref name for $name.
    #[arg(rename = "ref")]
    pub ref_name: Option<String>,
}

/// `word_insert_text`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(
    name = "word_insert_text",
    description = include_str!("word_insert_text.md"),
    group = "word_text",
    mutates,
    places = "text",
    place_own("at", "target", "where"),
    place_find,
    place_items = "items"
)]
pub struct WordInsertText {
    /// Document id.
    pub doc: String,
    /// Text occurring once, to insert beside.
    pub find: Option<String>,
    /// Or a range, position (p:A@5) or paragraph.
    #[arg(refs)]
    pub target: Option<String>,
    /// Default after.
    #[arg(rename = "where")]
    pub side: Option<Where>,
    /// Or after:/before: a range, or a position.
    pub at: Option<String>,
    /// Plain text.
    #[arg(facts)]
    pub text: Option<String>,
    /// Ref name for $name.
    #[arg(rename = "ref")]
    pub ref_name: Option<String>,
    /// Block or span to search for find.
    #[arg(refs)]
    pub within: Option<String>,
    /// Or several insertions.
    pub items: Option<Vec<InsertItem>>,
}

/// `word_insert_text` on the document `editing` names. A failed reply is
/// `invalid_arguments` for a call giving both one insertion's fields and items, an item
/// without text or without exactly one of find, target and at, an `at` that is no side of a
/// range or position, a `ref` holding an address; the find's or the range's refusal;
/// `refused` (nothing changed) for what is not supported yet.
#[must_use]
pub fn word_insert_text(args: &WordInsertText, editing: &Editing<'_>) -> ToolResult<Value> {
    run(args, editing).unwrap_or_else(ToolResult::failed)
}

fn run(args: &WordInsertText, editing: &Editing<'_>) -> Result<ToolResult<Value>, ToolError> {
    crate::review::stopped(editing.cancel)?;
    let call = try_call(args);
    let single = [&args.find, &args.target, &args.at, &args.text];
    if args.items.is_some() && single.iter().any(|v| v.is_some()) {
        let way = WayOut::call(call.clone());
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            "give one insertion's fields or items, not both",
            way,
        )
        .at("items"));
    }
    if args.items.as_ref().is_some_and(Vec::is_empty) {
        return Ok(ToolResult::nothing_given("inserted", "items"));
    }
    let work: Vec<InsertItem> = match &args.items {
        Some(items) => items.clone(),
        None => vec![InsertItem {
            text: args.text.clone().unwrap_or_default(),
            find: args.find.clone(),
            target: args.target.clone(),
            side: args.side,
            at: args.at.clone(),
            ref_name: args.ref_name.clone(),
        }],
    };
    if let Some(unported) = unported(args, &work) {
        return Err(unported.refusal(try_call(args), &["text", "target", "at", "within"]));
    }
    let policy = editing.policy();
    let mut ops = Vec::with_capacity(work.len());
    let mut sides = Vec::with_capacity(work.len());
    for (k, item) in work.iter().enumerate() {
        let (side, place) = planned(args, item, k, &call)?;
        sides.push(side);
        ops.push(InsertBeside {
            place,
            side: match side {
                Where::Before => Side::Before,
                Where::After => Side::After,
            },
            text: item.text.clone(),
            policy: policy.clone(),
        });
    }
    let targets: Vec<&str> = work.iter().filter_map(|item| item.target.as_deref()).collect();
    let argument = |field: &str| argument(args, field);
    // the values written by reference kept in the same step
    let finish = |out: &Vec<Result<Placed, _>>, after: &written::After| {
        let placed: Vec<Placed> = out.iter().filter_map(|p| p.as_ref().ok().cloned()).collect();
        let result = reply(placed, &work, &sides);
        let keeps = written::reply_keeps(after, editing, &result, &targets, &argument);
        (result, keeps)
    };
    // docx-agent names no field when the document refuses a find or a range
    let (_, result) =
        written::run_keeping(editing, &Batch::atomic(ops), finish).map_err(|e| edit_refusal(e, "", call))?;
    Ok(result)
}

/// The text of the argument `field` names (`text`, `items[0].text`).
fn argument(args: &WordInsertText, field: &str) -> Option<String> {
    if field == "text" {
        return args.text.clone();
    }
    let k: usize = field.strip_prefix("items[")?.strip_suffix("].text")?.parse().ok()?;
    args.items.as_ref()?.get(k).map(|item| item.text.clone())
}

/// Item `k`'s side and place, its arguments checked as docx-agent checks them.
fn planned(
    args: &WordInsertText,
    item: &InsertItem,
    k: usize,
    call: &TryCall,
) -> Result<(Where, TextPlace), ToolError> {
    if args.items.is_none() && args.text.is_none() {
        return Err(needed("text", "", call.clone()));
    }
    let given = |v: &Option<String>| v.is_some();
    if !given(&item.find) && !given(&item.target) && !given(&item.at) {
        if let Some(place) = item.ref_name.as_deref().filter(|r| is_place(r)) {
            return Err(ref_as_place(args, item, place, args.items.as_ref().map(|_| k)));
        }
    }
    let key = one_of(
        &[
            ("find", given(&item.find)),
            ("target", given(&item.target)),
            ("at", given(&item.at)),
        ],
        call,
    )?;
    let side = item.side.unwrap_or(Where::After);
    Ok(match key.as_str() {
        // within is one insertion's: items' finds search every story, as docx-agent's do
        "find" => (
            side,
            TextPlace::Once {
                text: item.find.clone().unwrap_or_default(),
                within: args.items.is_none().then(|| args.within.clone()).flatten(),
            },
        ),
        "target" => (side, TextPlace::Range(item.target.clone().unwrap_or_default())),
        _ => {
            let (side, address) = text_at(item.at.as_deref().unwrap_or_default(), item.side, call)?;
            (side, TextPlace::Range(address))
        }
    })
}

/// The reply, as docx-agent's `outcome` makes it of the insertions, with the new ranges
/// (`data.ranges`) and the refs naming them.
fn reply(placed: Vec<Placed>, work: &[InsertItem], sides: &[Where]) -> ToolResult<Value> {
    let mut edits = Vec::with_capacity(placed.len());
    let mut made = Vec::with_capacity(placed.len());
    let mut refs = Vec::new();
    for ((placed, item), side) in placed.into_iter().zip(work).zip(sides) {
        let inserted = inserted_range(&placed.range, *side, &item.text, &placed.edited.renamed);
        if let Some(name) = item.ref_name.as_ref().filter(|n| !n.is_empty()) {
            refs.push((name.clone(), inserted.clone()));
        }
        made.push(Value::from(inserted));
        edits.push(Outcome {
            id: placed.edited.id,
            changed: placed.edited.count > 0,
            renamed: placed.edited.renamed,
            ..Outcome::default()
        });
    }
    let summary = format!("Inserted text at {} place(s)", edits.len());
    let mut result = outcome(&edits, summary, Some(json!({"ranges": made})));
    if let ooxml_tools::Outcome::Done(done) = &mut result.outcome {
        done.refs = refs;
    }
    result
}

/// What of the call is not ported: addresses by reference.
fn unported(args: &WordInsertText, work: &[InsertItem]) -> Option<Unported> {
    if args.within.as_deref().is_some_and(by_reference) {
        return Some(Unported::Reference);
    }
    let by_ref = |v: &Option<String>| v.as_deref().is_some_and(by_reference);
    work.iter()
        .any(|item| by_ref(&item.target) || by_ref(&item.at))
        .then_some(Unported::Reference)
}

/// docx-agent's `positions.is_place`: whether a value reads as an address, not a ref name.
fn is_place(value: &str) -> bool {
    value.contains([':', '@', '$']) || value.contains("..")
}

/// `_text_at`: `at` as `(side, range)` -- `after:` or `before:` a range, or a bare range or
/// position (after it unless `side` says before).
fn text_at(at: &str, side: Option<Where>, call: &TryCall) -> Result<(Where, String), ToolError> {
    let named = [("after:", Where::After), ("before:", Where::Before)]
        .into_iter()
        .find_map(|(prefix, w)| at.strip_prefix(prefix).map(|rest| (w, rest)));
    if at == "end" || at.starts_with("replace:") {
        let message = format!(
            "at {}: text goes beside a range: after:<range>, before:<range>, or a position p:<id>@<offset>",
            repr_str(at)
        );
        return Err(ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(call.clone())).at("at"));
    }
    let Some((named, rest)) = named else {
        return Ok((side.unwrap_or(Where::After), at.to_owned()));
    };
    if let Some(side) = side.filter(|s| *s != named) {
        let message = format!("at says {} and where says {}: give one", named.name(), side.name());
        let options = WayOut::names([named.name(), side.name()]).unwrap_or_else(|| WayOut::call(call.clone()));
        return Err(ToolError::refusal(Refusal::InvalidArguments, message, options).at("where"));
    }
    Ok((named, rest.to_owned()))
}

/// `_ref_as_place`: a `ref` holding an address where the call gives no place.
fn ref_as_place(args: &WordInsertText, item: &InsertItem, place: &str, index: Option<usize>) -> ToolError {
    let message = format!(
        "ref {} is an address: ref names what this call makes, for $name later; the place is at: write at: {}",
        repr_str(place),
        repr_str(place)
    );
    let field = index.map_or_else(|| "ref".to_owned(), |k| format!("items[{k}].ref"));
    let mut meant = item_arguments(item);
    meant.remove("ref");
    meant.insert("at".into(), Value::from(place));
    if index.is_none() {
        meant.insert("doc".into(), Value::from(args.doc.as_str()));
    }
    // docx-agent's details: the item as meant, or the call that places it (its redo)
    let detail = if index.is_some() {
        ("item", Value::Object(meant.clone()))
    } else {
        let call = serde_json::json!({"tool": "word_insert_text", "arguments": meant.clone()});
        ("call", call)
    };
    let way = WayOut::names(["at"]).map_or_else(
        || WayOut::call(try_call(args)),
        |w| {
            w.and_call(TryCall {
                tool: "word_insert_text".to_owned(),
                arguments: meant,
                fill: Vec::new(),
            })
        },
    );
    ToolError::refusal(Refusal::InvalidArguments, message, way)
        .at(field)
        .with_detail(detail.0, detail.1)
}

/// `inserted_range`: the id of text just inserted before or after the range `place` (its
/// id before the insertion): where the range started or ended, `text`'s length on.
fn inserted_range(place: &str, side: Where, text: &str, renamed: &[(String, String)]) -> String {
    let (head, tail) = place.split_once("..").unwrap_or((place, ""));
    let anchor = if side == Where::Before || tail.is_empty() {
        head
    } else {
        tail
    };
    let (paragraph, span) = anchor.rsplit_once('@').unwrap_or(("", anchor));
    let offsets: Vec<&str> = span.split(':').collect();
    let pick = if side == Where::Before {
        offsets.first()
    } else {
        offsets.last()
    };
    let at: usize = pick.and_then(|o| o.parse().ok()).unwrap_or(0);
    let paragraph = renamed
        .iter()
        .find(|(old, _)| old == paragraph)
        .map_or(paragraph, |(_, new)| new.as_str());
    format!("{paragraph}@{at}:{}", at + text.chars().count())
}

/// One item's arguments as given.
fn item_arguments(item: &InsertItem) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in [
        ("find", &item.find),
        ("target", &item.target),
        ("at", &item.at),
        ("ref", &item.ref_name),
    ] {
        if let Some(v) = value {
            out.insert(key.into(), Value::from(v.as_str()));
        }
    }
    if let Some(side) = item.side {
        out.insert("where".into(), Value::from(side.name()));
    }
    out.insert("text".into(), Value::from(item.text.as_str()));
    out
}

/// `word_insert_text` called with `args`.
fn try_call(args: &WordInsertText) -> TryCall {
    let mut arguments = Map::new();
    arguments.insert("doc".into(), Value::from(args.doc.as_str()));
    for (key, value) in [
        ("find", &args.find),
        ("target", &args.target),
        ("at", &args.at),
        ("text", &args.text),
        ("ref", &args.ref_name),
        ("within", &args.within),
    ] {
        if let Some(v) = value {
            arguments.insert(key.into(), Value::from(v.as_str()));
        }
    }
    if let Some(side) = args.side {
        arguments.insert("where".into(), Value::from(side.name()));
    }
    if let Some(items) = &args.items {
        let items = items.iter().map(|i| Value::Object(item_arguments(i))).collect();
        arguments.insert("items".into(), Value::Array(items));
    }
    TryCall {
        tool: "word_insert_text".to_owned(),
        arguments,
        fill: Vec::new(),
    }
}

/// Registers `word_insert_text` (the crate's [`crate::register`]).
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.edit::<WordInsertText>(word_insert_text, crate::Hosting::PLAIN.keeps_written());
}
