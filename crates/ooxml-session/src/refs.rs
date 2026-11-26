//! Refs as a session service (design phase 3, section 5; ooxml-edit's `session.py` refs,
//! `ref_states`, `ref_owners` and the dispatcher's ref resolution, ported exactly).
//!
//! Owns: a document's refs ([`RefTable`]): its names and the address each stands for, as
//! they are now and as each state number of the document had them (so undo and redo bring
//! back the refs of the state they return to: [`RefTable::restore`]; a scoped undo takes back
//! the names its step defined and a scoped redo brings them back: [`RefTable::scoped`]), and
//! who defined each, by name and address ([`RefTable::owner`]); and one call's (or batch's)
//! use of them ([`CallRefs`]): the refs it defined, kept only when it succeeds; the refs its
//! failed ops would have made; `$name` and `$name/rest` resolved on the paths a tool
//! declares ([`CallRefs::resolve_declared`]), by a handler ([`CallRefs::resolve_ref`]) and at
//! a span's ends ([`CallRefs::resolve_span`]), with the camelCase fold and its note, the
//! misspelt suggestion, and "was to be made by ops[N], which failed"; a name kept as given
//! or folded ([`CallRefs::ref_name`], [`CallRefs::define`]); and the refusal to point another
//! caller's ref elsewhere ([`CallRefs::own_first`]).
//!
//! A table is kept per document by the host session, beside its store, and outlives its
//! document's close. What a call defines is applied as an effect once its change is
//! published ([`RefTable::keep`]); nothing here is written by a handler.
//!
//! Must not: read or change a document, re-point a ref (refs stay the address strings they
//! were given), or name a caller's context in a message (it is the application's).

use ooxml_base::pyfmt::repr_str;
use ooxml_tools::refs::{
    fold_ref, folded_use, folded_warning, is_ref_name, misspelt_ref, ref_names, ref_rule, ref_use,
};
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// A caller as a ref's owner: its key (two callers are the same when their keys are).
pub type RefOwner = Arc<str>;

/// Refs by name, in the order they were first defined (a Python `dict`'s order).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefMap {
    entries: Vec<(String, String)>,
}

impl RefMap {
    /// No refs.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The address `name` stands for.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, a)| a.as_str())
    }

    /// Whether `name` is one.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// `name` standing for `address`: in its place when it is one already, else last.
    pub fn insert(&mut self, name: &str, address: &str) {
        match self.entries.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => address.clone_into(&mut entry.1),
            None => self.entries.push((name.to_owned(), address.to_owned())),
        }
    }

    /// Every ref of `other` as here (`dict.update`).
    pub fn update(&mut self, other: &RefMap) {
        for (name, address) in other.iter() {
            self.insert(name, address);
        }
    }

    /// The refs, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(n, a)| (n.as_str(), a.as_str()))
    }

    /// The names, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(n, _)| n.as_str())
    }

    /// How many.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `$name` of each, sorted: a refusal's options.
    #[must_use]
    pub fn options(&self) -> Vec<String> {
        let mut names: Vec<String> = self.names().map(|n| format!("${n}")).collect();
        names.sort();
        names
    }

    /// As JSON, `{name: address}` in order.
    #[must_use]
    pub fn to_json(&self) -> Map<String, Value> {
        self.iter().map(|(n, a)| (n.to_owned(), Value::from(a))).collect()
    }
}

impl<'a> FromIterator<(&'a str, &'a str)> for RefMap {
    fn from_iter<I: IntoIterator<Item = (&'a str, &'a str)>>(iter: I) -> Self {
        let mut out = RefMap::new();
        for (name, address) in iter {
            out.insert(name, address);
        }
        out
    }
}

/// What a scoped undo or redo did to a document's states (ooxml-edit's `ScopedStep`): the
/// states the step went from and to when it was made, the state the document is in now,
/// and, for an undo, each rewritten state's old number and its new one, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedStep {
    /// The state the step was made on.
    pub before: u64,
    /// The state it made.
    pub after: u64,
    /// The document's state now.
    pub version: u64,
    /// Each later state's old number and its new one (an undo's).
    pub renumbered: Vec<(u64, u64)>,
}

/// One document's refs: as they are now, as each state had them, and who defined each.
#[derive(Debug, Clone, Default)]
pub struct RefTable {
    current: Arc<RefMap>,
    states: BTreeMap<u64, Arc<RefMap>>,
    owners: HashMap<(String, String), RefOwner>,
}

impl RefTable {
    /// No refs.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The refs as they are now (`entry.refs`).
    #[must_use]
    pub fn refs(&self) -> &RefMap {
        &self.current
    }

    /// The refs as they are now, shared (a reader keeps them while others change the table).
    #[must_use]
    pub fn shared(&self) -> Arc<RefMap> {
        Arc::clone(&self.current)
    }

    /// The refs the state `number` had, where the table keeps them (`ref_states`).
    #[must_use]
    pub fn at(&self, number: u64) -> Option<Arc<RefMap>> {
        self.states.get(&number).cloned()
    }

    /// Who defined ref `name` as it is now; `None` for a shared ref (defined by a call with
    /// no caller), or a name the document has not (`ref_owner`).
    #[must_use]
    pub fn owner(&self, name: &str) -> Option<&RefOwner> {
        let address = self.current.get(name)?;
        self.owners.get(&(name.to_owned(), address.to_owned()))
    }

    /// Keep the refs a call defined, by `owner` (`None`: shared), and the set as the state
    /// `version`'s (`keep_refs`). A ref named again at the address it has keeps its owner.
    pub fn keep(&mut self, defined: &RefMap, owner: Option<&RefOwner>, version: u64) {
        let mut current = (*self.current).clone();
        for (name, address) in defined.iter() {
            if current.get(name) == Some(address) {
                continue;
            }
            let key = (name.to_owned(), address.to_owned());
            match owner {
                Some(owner) => {
                    self.owners.insert(key, Arc::clone(owner));
                }
                None => {
                    self.owners.remove(&key);
                }
            }
        }
        current.update(defined);
        let current = Arc::new(current);
        self.states.insert(version, Arc::clone(&current));
        self.current = current;
    }

    /// The refs of the state the document is at after an undo or redo (`restore_refs`).
    pub fn restore(&mut self, version: u64) {
        self.current = self.states.get(&version).cloned().unwrap_or_default();
    }

    /// The refs after a scoped undo or redo (`scoped_refs`): the names the step defined go
    /// (undo) or come back (redo); other refs are kept.
    pub fn scoped(&mut self, step: &ScopedStep, undone: bool) {
        let empty = RefMap::new();
        let before = self.states.get(&step.before).cloned().unwrap_or_default();
        let after = self.states.get(&step.after).cloned().unwrap_or_default();
        let made: RefMap = after
            .iter()
            .filter(|(name, address)| before.get(name) != Some(*address))
            .collect();
        if undone {
            for (old, new) in &step.renumbered {
                if self.states.contains_key(new) {
                    continue;
                }
                let found = self.states.get(old).map_or(&empty, |s| &**s);
                let kept: RefMap = found
                    .iter()
                    .filter(|(name, address)| made.get(name) != Some(*address))
                    .collect();
                self.states.insert(*new, Arc::new(kept));
            }
        } else {
            let mut back = (*self.current).clone();
            back.update(&made);
            self.states.insert(step.version, Arc::new(back));
        }
        self.current = self.states.get(&step.version).cloned().unwrap_or_default();
    }
}

/// What a call (or every op of a batch) did with refs so far, to go back to after a failed
/// op ([`CallRefs::mark`], [`CallRefs::reset`]).
#[derive(Debug, Clone)]
pub struct RefsMark(BTreeMap<String, RefMap>);

/// One call's (or batch's) use of refs: what it defined, not yet kept (a failure drops
/// them); the names its failed ops would have made; its caller.
#[derive(Debug, Clone, Default)]
pub struct CallRefs {
    pending: BTreeMap<String, RefMap>,
    failed: HashMap<String, usize>,
    read: bool,
    caller: Option<RefOwner>,
}

/// A ref resolved by [`CallRefs`]: the value, and the note of a camelCase use read as its
/// ref, to be the call's warning.
pub type Resolved = (Value, Vec<String>);

impl CallRefs {
    /// A call made by `caller` (`None`: no caller context).
    #[must_use]
    pub fn new(caller: Option<RefOwner>) -> Self {
        Self {
            caller,
            ..Self::default()
        }
    }

    /// The call's caller.
    #[must_use]
    pub fn caller(&self) -> Option<&RefOwner> {
        self.caller.as_ref()
    }

    /// The refs `doc`'s calls defined so far, not yet kept: the effect to keep once the
    /// call's change is published.
    #[must_use]
    pub fn defined(&self, doc: &str) -> Option<&RefMap> {
        self.pending.get(doc)
    }

    /// The documents with refs defined so far.
    pub fn defined_docs(&self) -> impl Iterator<Item = &str> {
        self.pending.keys().map(String::as_str)
    }

    /// Whether a call checked a ref name against a document's refs (whose it is): what a
    /// call re-run on a newer version checks again.
    #[must_use]
    pub fn read_owners(&self) -> bool {
        self.read
    }

    /// What the ops so far defined.
    #[must_use]
    pub fn mark(&self) -> RefsMark {
        RefsMark(self.pending.clone())
    }

    /// Forget what a failed op defined: back to `mark`.
    pub fn reset(&mut self, mark: RefsMark) {
        self.pending = mark.0;
    }

    /// Op `op` of the batch failed: the refs its arguments ask for are its, for a later op's
    /// `$name` to say so (`_ref_names`).
    pub fn failed(&mut self, op: usize, arguments: &Value) {
        for name in ref_names(arguments) {
            self.failed.insert(name, op);
        }
    }

    /// Every ref of `doc` the call can use: the document's (`table`), then its own.
    #[must_use]
    pub fn known(&self, doc: &str, table: Option<&RefTable>) -> RefMap {
        let mut known = table.map(|t| t.refs().clone()).unwrap_or_default();
        if let Some(own) = self.pending.get(doc) {
            known.update(own);
        }
        known
    }

    /// The refusal of `$name`, which a failed op of the batch would have made.
    fn missing(&self, name: &str, field: Option<&str>, retry: &TryCall) -> Option<ToolError> {
        let op = *self.failed.get(name)?;
        let error = ToolError::refusal(
            Refusal::NotFound,
            format!("${name} was to be made by ops[{op}], which failed: fix that op and resend both"),
            WayOut::call(retry.clone()),
        )
        .with_detail("op", Value::from(op));
        Some(match field {
            Some(field) => error.at(field),
            None => error,
        })
    }

    /// `no ref $name in doc`, the refs there are its options.
    fn unknown(&self, name: &str, doc: &str, known: &RefMap, field: Option<&str>, retry: &TryCall) -> ToolError {
        self.missing(name, field, retry).unwrap_or_else(|| {
            let error = ToolError::refusal(
                Refusal::NotFound,
                format!("no ref ${name} in {doc}"),
                way_out(known.options(), retry),
            );
            match field {
                Some(field) => error.at(field),
                None => error,
            }
        })
    }

    /// `value`, a string on one of the paths a tool declares as refs (`Tool.refs`) of `doc`,
    /// as an address (`_with_refs`): `$name/rest` its address and the rest; a camelCase
    /// `$recNow` that folds to a ref read as it, with a note; a `$Name` that is no ref name but
    /// stands for one refused asking whether that one was meant; anything else as given.
    ///
    /// # Errors
    /// `not_found` at `field`, as above.
    pub fn resolve_value(
        &self,
        value: &str,
        doc: &str,
        table: Option<&RefTable>,
        field: &str,
        retry: &TryCall,
    ) -> Result<(String, Option<String>), ToolError> {
        let known = self.known(doc, table);
        let mut note = None;
        let mut used = ref_use(value).map(|(n, r)| (n.to_owned(), r.to_owned()));
        if used.is_none() {
            if let Some(folded) = folded_use(value, |n| known.contains(n), |n| self.failed.contains_key(n)) {
                note = Some(format!("{field} {} read as {}", repr_str(value), repr_str(&folded)));
                used = ref_use(&folded).map(|(n, r)| (n.to_owned(), r.to_owned()));
            }
        }
        let Some((name, rest)) = used else {
            if let Some(meant) = misspelt_ref(value, known.names()) {
                return Err(ToolError::refusal(
                    Refusal::NotFound,
                    format!(
                        "{} is no ref; did you mean ${meant}? A ref is lowercase",
                        repr_str(value)
                    ),
                    way_out(known.options(), retry),
                )
                .at(field));
            }
            return Ok((value.to_owned(), None));
        };
        match known.get(&name) {
            Some(address) => Ok((format!("{address}{rest}"), note)),
            None => Err(self.unknown(&name, doc, &known, Some(field), retry)),
        }
    }

    /// `arguments` with every `$name` on the paths a tool declares as refs (`paths`,
    /// `items[].target`) of `doc` replaced by its address, and the notes of the camelCase
    /// uses read as refs (`_with_refs`).
    ///
    /// # Errors
    /// The first value's refusal ([`CallRefs::resolve_value`]).
    pub fn resolve_declared(
        &self,
        arguments: &Value,
        paths: &[String],
        doc: &str,
        table: Option<&RefTable>,
        retry: &TryCall,
    ) -> Result<Resolved, ToolError> {
        let mut notes = Vec::new();
        let mut out = arguments.clone();
        let mut resolve = |value: &Value, field: &str| -> Result<Value, ToolError> {
            let Value::String(text) = value else {
                return Ok(value.clone());
            };
            // a span's ends, each resolved as a handler resolves a ref (docx-agent's `refs`)
            if let (Some((first, last)), true) = (text.split_once(".."), text.contains('$')) {
                let mut end = |part: &str| -> Result<String, ToolError> {
                    let trimmed = part.trim();
                    if !trimmed.starts_with('$') {
                        return Ok(part.to_owned());
                    }
                    let (address, note) = self.resolve_ref(trimmed, doc, table, Some(field), retry)?;
                    notes.extend(note);
                    Ok(address)
                };
                let (first, last) = (end(first)?, end(last)?);
                return Ok(Value::String(format!("{first}..{last}")));
            }
            let (resolved, note) = self.resolve_value(text, doc, table, field, retry)?;
            notes.extend(note);
            Ok(Value::String(resolved))
        };
        for path in paths {
            let parts: Vec<&str> = path.split('.').collect();
            out = walk(&out, &parts, "", &mut resolve)?;
        }
        Ok((out, notes))
    }

    /// `value` with a leading `$name` replaced by its address, as a handler resolves one
    /// (`Call.resolve_ref`): a camelCase use read as its ref, with a warning; no misspelt
    /// check; anything else as given.
    ///
    /// # Errors
    /// `not_found` (at `field` where given).
    pub fn resolve_ref(
        &self,
        value: &str,
        doc: &str,
        table: Option<&RefTable>,
        field: Option<&str>,
        retry: &TryCall,
    ) -> Result<(String, Option<String>), ToolError> {
        let known = self.known(doc, table);
        let mut warning = None;
        let used = match ref_use(value) {
            Some((name, rest)) => (name.to_owned(), rest.to_owned()),
            None => {
                let Some(folded) = folded_use(value, |n| known.contains(n), |n| self.failed.contains_key(n)) else {
                    return Ok((value.to_owned(), None));
                };
                let lead = field.map_or(String::new(), |f| format!("{f} "));
                warning = Some(format!("{lead}{} read as {}", repr_str(value), repr_str(&folded)));
                let (name, rest) = ref_use(&folded).unwrap_or_default();
                (name.to_owned(), rest.to_owned())
            }
        };
        let (name, rest) = used;
        match known.get(&name) {
            Some(address) => Ok((format!("{address}{rest}"), warning)),
            None => Err(self.unknown(&name, doc, &known, field, retry)),
        }
    }

    /// A span `$a..$b` (or one end a ref) with each end that is a ref resolved
    /// ([`CallRefs::resolve_ref`], the end stripped), as the tools resolve a span's ends
    /// (docx-agent's `reads.refs`); a lone `$name` resolved whole; anything else as given.
    ///
    /// # Errors
    /// An end's refusal.
    pub fn resolve_span(
        &self,
        value: &str,
        doc: &str,
        table: Option<&RefTable>,
        retry: &TryCall,
    ) -> Result<(String, Vec<String>), ToolError> {
        let mut warnings = Vec::new();
        if let (Some((first, last)), true) = (value.split_once(".."), value.contains('$')) {
            let mut ends = Vec::with_capacity(2);
            for end in [first, last] {
                let stripped = crate::facts::py_strip(end);
                if stripped.starts_with('$') {
                    let (resolved, warning) = self.resolve_ref(stripped, doc, table, None, retry)?;
                    warnings.extend(warning);
                    ends.push(resolved);
                } else {
                    ends.push(end.to_owned());
                }
            }
            return Ok((ends.join(".."), warnings));
        }
        if value.starts_with('$') {
            let (resolved, warning) = self.resolve_ref(value, doc, table, None, retry)?;
            warnings.extend(warning);
            return Ok((resolved, warnings));
        }
        Ok((value.to_owned(), warnings))
    }

    /// The ref name `name` is kept as: itself, or the name a camelCase or hyphenated name
    /// folds to; refused where it folds to none, or to a name that already names an address
    /// other than `address` (`None`: any, for a call that has not made what it names yet)
    /// (`Call.ref_name`).
    ///
    /// # Errors
    /// `invalid_arguments` at `ref`, as above.
    pub fn ref_name(
        &self,
        name: &str,
        address: Option<&str>,
        doc: &str,
        table: Option<&RefTable>,
        retry: &TryCall,
    ) -> Result<String, ToolError> {
        if is_ref_name(name) {
            return Ok(name.to_owned());
        }
        let Some(folded) = fold_ref(name) else {
            return Err(
                ToolError::refusal(Refusal::InvalidArguments, ref_rule(name), WayOut::call(retry.clone())).at("ref"),
            );
        };
        let known = self.known(doc, table);
        match known.get(&folded) {
            Some(taken) if Some(taken) != address => Err(ToolError::refusal(
                Refusal::InvalidArguments,
                format!(
                    "ref {} reads as {}, which already names {taken}: give ref {} to point it here instead, or a name \
                     of its own",
                    repr_str(name),
                    repr_str(&folded),
                    repr_str(&folded)
                ),
                WayOut::call(retry.clone()),
            )
            .at("ref")
            .with_detail("ref", Value::from(folded.as_str()))
            .with_detail("address", Value::from(taken))),
            _ => Ok(folded),
        }
    }

    /// Name `address` `name` in `doc`, so later calls may write `$name`: the name kept, and
    /// the warning of a name folded ([`folded_warning`]) (`Call.define_ref`).
    ///
    /// # Errors
    /// [`CallRefs::ref_name`]'s; the ownership refusal ([`CallRefs::own_first`]).
    pub fn define(
        &mut self,
        name: &str,
        address: &str,
        doc: &str,
        table: Option<&RefTable>,
        retry: &TryCall,
    ) -> Result<(String, Option<String>), ToolError> {
        let kept = self.ref_name(name, Some(address), doc, table, retry)?;
        let warning = (kept != name).then(|| folded_warning(name, &kept));
        self.own(doc, &kept, Some(address), table, retry)?;
        self.pending.entry(doc.to_owned()).or_default().insert(&kept, address);
        Ok((kept, warning))
    }

    /// Before a changing call's handler runs: refuse a ref name it is given (`ref`, or
    /// `ref` in the objects of a list argument) that another caller's ref of `doc` has --
    /// what it makes is somewhere new, so it would point that ref elsewhere -- before
    /// anything is made (`_own_refs_first`).
    ///
    /// # Errors
    /// The ownership refusal.
    pub fn own_first(
        &mut self,
        arguments: &Value,
        doc: &str,
        table: Option<&RefTable>,
        retry: &TryCall,
    ) -> Result<(), ToolError> {
        if self.caller.is_none() {
            return Ok(());
        }
        let Some(map) = arguments.as_object() else {
            return Ok(());
        };
        let mut names: Vec<&Value> = map.get("ref").into_iter().collect();
        for value in map.values() {
            if let Value::Array(items) = value {
                names.extend(items.iter().filter_map(|item| item.as_object()?.get("ref")));
            }
        }
        for name in names.into_iter().filter_map(Value::as_str) {
            let folded = if is_ref_name(name) {
                Some(name.to_owned())
            } else {
                fold_ref(name)
            };
            if let Some(folded) = folded {
                self.own(doc, &folded, None, table, retry)?;
            }
        }
        Ok(())
    }

    /// Refuse to point `name` at `address` (`None`: somewhere new) when another caller's ref
    /// has that name and names another address; a call with no caller points any, and a
    /// ref this call defined already is its own (`_own_ref`).
    fn own(
        &mut self,
        doc: &str,
        name: &str,
        address: Option<&str>,
        table: Option<&RefTable>,
        retry: &TryCall,
    ) -> Result<(), ToolError> {
        let Some(caller) = self.caller.as_ref() else {
            return Ok(());
        };
        if self.pending.get(doc).is_some_and(|p| p.contains(name)) {
            return Ok(());
        }
        self.read = true;
        let Some(table) = table else {
            return Ok(());
        };
        let Some(now) = table.refs().get(name).filter(|now| Some(*now) != address) else {
            return Ok(());
        };
        match table.owner(name) {
            None => return Ok(()),
            Some(owner) if owner == caller => return Ok(()),
            Some(_) => {}
        }
        let taken = self.known(doc, Some(table));
        let stem: String = name.chars().take(28).collect();
        let other = (2..)
            .map(|n| format!("{stem}_{n}"))
            .find(|o| !taken.contains(o))
            .unwrap_or_default();
        // the owner is not named: a caller's context is the application's, never the model's
        Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!(
                "ref {} was defined by another caller (it names {now}), and only its definer points it elsewhere: \
                 give this one another name, such as {}",
                repr_str(name),
                repr_str(&other)
            ),
            WayOut::names([other]).unwrap_or_else(|| WayOut::call(retry.clone())),
        )
        .at("ref")
        .with_detail("ref", Value::from(name))
        .with_detail("address", Value::from(now)))
    }
}

/// A way out naming `options`, else the call itself.
fn way_out(options: Vec<String>, retry: &TryCall) -> WayOut {
    WayOut::names(options).unwrap_or_else(|| WayOut::call(retry.clone()))
}

/// `node` with `resolve(value, field)` applied at the argument path `parts`
/// (`items[].target` split at the dots), each value named by where it is (`_walk`).
fn walk(
    node: &Value,
    parts: &[&str],
    field: &str,
    resolve: &mut dyn FnMut(&Value, &str) -> Result<Value, ToolError>,
) -> Result<Value, ToolError> {
    let Some((part, rest)) = parts.split_first() else {
        return match node {
            Value::Array(items) => items
                .iter()
                .enumerate()
                .map(|(i, item)| resolve(item, &format!("{field}[{i}]")))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            other => resolve(other, field),
        };
    };
    let (name, many) = part.strip_suffix("[]").map_or((*part, false), |n| (n, true));
    let Some(value) = node.as_object().and_then(|map| map.get(name)) else {
        return Ok(node.clone());
    };
    let path = if field.is_empty() {
        name.to_owned()
    } else {
        format!("{field}.{name}")
    };
    let walked = match value {
        Value::Array(items) if many => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let at = format!("{path}[{i}]");
                    if rest.is_empty() {
                        resolve(item, &at)
                    } else {
                        walk(item, rest, &at, resolve)
                    }
                })
                .collect::<Result<_, _>>()?,
        ),
        _ => walk(value, rest, &path, resolve)?,
    };
    let mut map = node.as_object().cloned().unwrap_or_default();
    map.insert(name.to_owned(), walked);
    Ok(Value::Object(map))
}
