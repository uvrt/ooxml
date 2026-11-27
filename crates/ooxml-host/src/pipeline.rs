//! Owns: one call's pipeline (design phase 3, section 6), each step refusing with its way
//! out: 1. the tool's name read (a namespace stripped, another spelling read as the one tool
//! it names, a withdrawn tool refused with what to call instead, an unknown one with the
//! tools it most likely means); 2. the role check; 3. the one acceptance (the tool's spec:
//! rule 1's intent rules, their notes the reply's first warnings; the host's words for an
//! action or a key the tool lacks); 4.-5. the document and its refs (`$name` on the
//! declared paths, a changing call's ref names folded and owned first); a retry key's
//! repeat; 9. the handler, on the version the call reads, run by the document's format; 10.
//! its effects applied once its change is published (the refs it defined, kept with the
//! version it left; its images counted); 11. the checks its format makes of a changing call;
//! then the reply's warnings in today's order, and the call's record.
//!
//! Must not: hold the session's stores while the handler runs, or reach a document's head
//! for a handler: the format runs the handler on the version the call reads.

use crate::catalogue::{Catalogue, Entry, Own};
use crate::own::OwnCall;
use crate::records::{digest, shape, CallRecord};
use crate::session::{CallAs, HostSession};
use crate::values::WithValues;
use ooxml_session::datasets::CallDatasets;
use ooxml_session::{CallContext, CallRefs, DocId, RefOwner};
use ooxml_store::Caller;
use ooxml_tools::blobs::BlobList;
use ooxml_tools::roles::RoleSet;
use ooxml_tools::written::CallValues;
use ooxml_tools::{Holding, OpEffects, Outcome, PlaceKind, ToolError, ToolResult, TryCall};
use serde_json::{Map, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

/// Why a call was not run at all: the application's mistake, never the model's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotRun {
    /// The roles asked for are not all the session's.
    Roles(String),
}

/// A call's arguments with their refs resolved.
pub(crate) struct WithRefs {
    /// The arguments.
    pub(crate) arguments: Map<String, Value>,
    /// What was read as what.
    pub(crate) notes: Vec<String>,
    /// Each ref name the call gives that it keeps folded: `(kept, given)`.
    pub(crate) folded: Vec<(String, String)>,
}

/// What one call gathers as it goes through the pipeline, for its reply and its record.
pub(crate) struct Going {
    /// The tool as the host read its name.
    tool: String,
    /// The notes on what was read as what: the reply's first warnings.
    pub(crate) notes: Vec<String>,
    /// The documents it named.
    pub(crate) docs: Vec<String>,
    /// Its arguments as taken (as given, where it failed before they were).
    arguments: Value,
    /// The values the application resolved for the call itself (the sources of documents
    /// it holds), handed to the format with the host's own.
    pub(crate) values: ooxml_tools::written::CallValues,
    /// The versions it reads, where a batch pins them (each document at the version the
    /// batch's first op on it read); `None`: each document's head.
    pub(crate) pinned: Option<Vec<ooxml_session::Opened>>,
    /// How long its commits held its documents' commit locks and waited for them and their
    /// gates.
    pub(crate) lock: ooxml_session::LockSpent,
    /// The moment the application dates the call's writes by ([`CallAs::date`]); `None`:
    /// the session's clock.
    pub(crate) date: Option<String>,
}

impl Going {
    /// A call of `tool` not yet read, with `arguments` as given.
    pub(crate) fn new(tool: &str, arguments: Value) -> Going {
        Going {
            tool: tool.to_owned(),
            notes: Vec::new(),
            docs: Vec::new(),
            arguments,
            pinned: None,
            lock: ooxml_session::LockSpent::default(),
            values: ooxml_tools::written::CallValues::default(),
            date: None,
        }
    }
}

impl HostSession {
    /// Run one call of the tool `name` with `arguments` (a JSON object, or its text), with
    /// the roles and context `how` gives: never fails for the model's mistakes -- what went
    /// wrong is the reply's error, with its way out -- and never panics (a panic is the
    /// reply's `internal` error).
    ///
    /// # Errors
    /// [`NotRun`] for the application's mistake (roles the session does not hold).
    pub fn call(&self, name: &str, arguments: Value, how: &CallAs) -> Result<ToolResult<Value>, NotRun> {
        self.call_recorded(name, arguments, how).map(|(result, _)| result)
    }

    /// [`HostSession::call`], with the call's record as the session's log keeps it.
    ///
    /// # Errors
    /// As [`HostSession::call`].
    pub fn call_recorded(
        &self,
        name: &str,
        arguments: Value,
        how: &CallAs,
    ) -> Result<(ToolResult<Value>, CallRecord), NotRun> {
        let roles = self.narrowed(how.roles).map_err(NotRun::Roles)?;
        let clock = &self.session.context().clock;
        let started = clock.monotonic();
        let cpu = clock.thread_cpu();
        let mut going = Going::new(name, arguments.clone());
        going.values = how.values.clone();
        going.date.clone_from(&how.date);
        let ran = catch_unwind(AssertUnwindSafe(|| self.go(name, arguments, roles, how, &mut going)));
        let mut result = ran.unwrap_or_else(|panic| {
            let said = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic".to_owned());
            let mut result = ToolResult::failed(ToolError::internal(
                format!("{} failed: {said}", going.tool),
                &format!("host:{}", going.tool),
            ));
            result.app.panic = Some(said);
            result
        });
        let mut warnings = std::mem::take(&mut going.notes);
        warnings.append(&mut result.warnings);
        result.warnings = warnings;
        result.app.tool.clone_from(&going.tool);
        let duration = clock.monotonic().saturating_sub(started);
        let cpu = cpu.zip(clock.thread_cpu()).map(|(then, now)| now.saturating_sub(then));
        result.app.timing = Some(timing(duration, cpu, going.lock));
        let record = self.record(&going, &result, roles, how, duration);
        Ok((result, record))
    }

    /// The pipeline, its notes and what it named gathered in `going`.
    fn go(&self, name: &str, arguments: Value, roles: RoleSet, how: &CallAs, going: &mut Going) -> ToolResult<Value> {
        let catalogue = &self.host.catalogue;
        let retry = TryCall {
            tool: name.to_owned(),
            arguments: arguments.as_object().cloned().unwrap_or_default(),
            fill: Vec::new(),
        };
        // 1. the tool's name
        going.tool = catalogue.tool_named(name, &mut going.notes, true);
        let Some(entry) = catalogue.get(&going.tool) else {
            return ToolResult::failed(catalogue.no_tool(name, roles, false, "batch", &retry));
        };
        let given = TryCall {
            tool: going.tool.clone(),
            ..retry
        };
        let retry = &given;
        // 2. the role, then 3. the one acceptance, then the call's own role
        if let Err(error) = Catalogue::check_role(entry, None, roles, None, retry) {
            return ToolResult::failed(error);
        }
        // a batch's op is its tool and its arguments: what else it gives is not read (today's
        // host reads those two alone)
        let arguments = match entry.own {
            Some(Own::Batch | Own::ReadBatch) => ops_read(arguments),
            _ => arguments,
        };
        let accepted = match entry.spec.accept(arguments) {
            Ok(accepted) => accepted,
            Err(error) => return ToolResult::failed(self.reworded(entry, retry, error, roles)),
        };
        going.notes.extend(accepted.notes);
        let arguments = accepted.arguments;
        going.arguments = Value::Object(arguments.clone());
        if let Err(error) = Catalogue::check_role(entry, Some(&arguments), roles, None, retry) {
            return ToolResult::failed(error);
        }
        let caller = how.context.as_ref().filter(|c| !c.is_empty());
        match entry.own {
            Some(Own::Undo) => return self.undo(arguments, caller, going),
            Some(Own::ReadBatch) => return self.read_batch(&arguments, roles, caller),
            Some(Own::Batch) => return self.batch(&arguments, roles, caller, going),
            Some(own) => {
                let call = OwnCall {
                    own,
                    arguments: &arguments,
                    session: self,
                    caller,
                    retry,
                    date: going.date.as_deref(),
                };
                return self.host.own.run(&call);
            }
            None => {}
        }
        self.run_format(entry, arguments, caller, going, retry)
    }

    /// Steps 4 to 11 of a call of a format's tool, its arguments taken as meant.
    pub(crate) fn run_format(
        &self,
        entry: &Entry,
        mut arguments: Map<String, Value>,
        caller: Option<&Map<String, Value>>,
        going: &mut Going,
        retry: &TryCall,
    ) -> ToolResult<Value> {
        // 4. the document, and the rules of its kind
        going.docs = ooxml_tools::documents::named(&entry.spec.documents, &arguments);
        let Some(doc) = going.docs.first().cloned() else {
            return ToolResult::failed(ToolError::internal(
                format!("{} names no document", entry.name()),
                "host:documents",
            ));
        };
        let kind = DocId::parse(&doc).and_then(|id| self.session.kind_of(&id).ok());
        let mut ignored = entry.spec.ignored_for(kind.map(|k| k.as_str()), &mut arguments);
        going.notes.append(&mut ignored);
        // 5. its refs
        let changing = entry.changes(&arguments);
        let owner: Option<RefOwner> = caller.map(|c| Arc::from(caller_key(c).as_str()));
        let mut refs = CallRefs::new(owner.clone());
        // (a changing call of one document, no retry key and no value by reference is the
        // one today's host serves beside the others: refused, it names no document; any other
        // call refused names its document at the version it read)
        let keyed = entry.spec.flags.retry_key && arguments.get("key").is_some_and(Value::is_string);
        let served =
            changing && going.docs.len() == 1 && !keyed && !Value::Object(arguments.clone()).to_string().contains("{{");
        let WithRefs {
            arguments,
            mut notes,
            folded,
        } = match self.with_refs(entry, &mut refs, &doc, arguments, changing, retry) {
            Ok(found) => found,
            Err(error) => return self.refused_at(error, (!served).then_some(doc.as_str())),
        };
        going.notes.append(&mut notes);
        // 6.-8. its sources, the datasets it names, its references
        let mut datasets = CallDatasets::new();
        let (WithValues { arguments, values, .. }, served) =
            match self.values_of(entry, &doc, arguments, (retry, &going.values), &mut datasets, served) {
                Ok(found) => found,
                Err(refused) => return *refused,
            };
        // a retry key's repeat
        let key = (changing && entry.spec.flags.retry_key)
            .then(|| arguments.get("key").and_then(Value::as_str).map(str::to_owned))
            .flatten();
        if let Some(repeat) = key.as_deref().and_then(|key| self.repeated(&doc, entry.name(), key)) {
            return repeat;
        }
        // 9. the handler, by the document's format, on the version the call reads
        let mut context = self.context(caller, going.date.as_deref());
        self.pin(going, &mut context);
        context.values = values;
        context.values.merge(going.values.clone());
        let (mut result, made) = self.handler(entry, &doc, arguments, &context, retry);
        crate::values::report_facts(&mut result, &context.values.written);
        if let Some(made) = &made {
            going.lock = made.lock;
        }
        if !result.is_error() {
            match self.finish_datasets(datasets) {
                Ok(mut stale) => result.warnings.append(&mut stale),
                Err(error) => return ToolResult::failed(error),
            }
        }
        if served && result.is_error() {
            result.doc = None;
            result.version = None;
        }
        // 10. its effects, applied now its change is published
        if let Err(error) = self.apply_effects(&mut result, &mut refs, &doc, owner.as_ref(), changing, retry) {
            return ToolResult::failed(error);
        }
        let mut first: Vec<String> = folded
            .iter()
            .filter(|(kept, _)| matches!(&result.outcome, Outcome::Done(d) if d.refs.iter().any(|(n, _)| n == kept)))
            .map(|(kept, given)| ooxml_tools::refs::folded_warning(given, kept))
            .collect();
        first.append(&mut result.warnings);
        result.warnings = first;
        // 11. the checks of a changing call, and what it touched
        if changing && !result.is_error() {
            self.checks(&doc, made.as_ref(), &mut result);
        }
        if let Some(made) = &made {
            self.touched(&doc, made, &mut result);
        }
        self.host_added(entry.name(), &doc, &mut result);
        if let (Some(key), false) = (&key, result.is_error()) {
            self.stores().keys.keep(&doc, entry.name(), key, &result);
        }
        result
    }

    /// Step 9: the handler, run by the document's format on the version the call reads, with
    /// what the host does around it (check's numbers).
    fn handler(
        &self,
        entry: &Entry,
        doc: &str,
        arguments: Map<String, Value>,
        context: &CallContext,
        retry: &TryCall,
    ) -> (ToolResult<Value>, Option<ooxml_session::CallMade>) {
        self.check_values(
            (entry.name(), doc),
            arguments,
            context,
            retry,
            (
                |arguments| self.session.edit_accepted(entry.name(), arguments, context),
                |alone| (alone, None),
                |ran: &mut (ToolResult<Value>, Option<ooxml_session::CallMade>)| &mut ran.0,
            ),
        )
    }

    /// Steps 6 to 8 ([`HostSession::with_values`]), and whether the call is still one
    /// today's host serves beside the others (`served`: not where it reads another
    /// document); a refusal as the call's reply -- naming its document where it is not
    /// served, none where it came before the documents were held.
    fn values_of(
        &self,
        entry: &Entry,
        doc: &str,
        arguments: Map<String, Value>,
        (retry, given): (&TryCall, &CallValues),
        datasets: &mut CallDatasets,
        served: bool,
    ) -> Result<(WithValues, bool), Box<ToolResult<Value>>> {
        let other = |held: &[String]| held.iter().any(|h| h != doc);
        match self.with_values(entry, doc, arguments, retry, datasets, given) {
            Ok(found) => {
                let served = served && !other(&found.held);
                Ok((found, served))
            }
            Err(refused) => {
                let named = !refused.before && !(served && !other(&refused.held));
                Err(Box::new(self.refused_at(refused.error, named.then_some(doc))))
            }
        }
    }

    /// The reply a retry key's earlier call left, for its repeat: at the document's version
    /// now, as a call that changed nothing.
    fn repeated(&self, doc: &str, tool: &str, key: &str) -> Option<ToolResult<Value>> {
        let mut repeat = self.stores().keys.repeat(doc, tool, key)?;
        repeat.doc = Some(doc.to_owned());
        repeat.version = DocId::parse(doc)
            .and_then(|id| self.session.snapshot(&id).ok())
            .map(|head| head.number());
        Some(repeat)
    }

    /// `error` as a call's reply, naming `doc` at its head where the call names one.
    fn refused_at(&self, error: ToolError, doc: Option<&str>) -> ToolResult<Value> {
        let mut result = ToolResult::failed(error);
        if let Some(doc) = doc {
            result.version = DocId::parse(doc)
                .and_then(|id| self.session.snapshot(&id).ok())
                .map(|head| head.number());
            result.doc = Some(doc.to_owned());
        }
        result
    }

    /// In a batch that pins versions, each document the call names read at the version the
    /// batch's first op on it read (pinned now where none did yet).
    fn pin(&self, going: &mut Going, context: &mut CallContext) {
        let Some(pinned) = &mut going.pinned else { return };
        for doc in &going.docs {
            let Some(id) = DocId::parse(doc) else { continue };
            if pinned.iter().all(|o| *o.id() != id) {
                if let Ok(head) = self.session.snapshot(&id) {
                    pinned.push(head);
                }
            }
        }
        context.at.clone_from(pinned);
    }

    /// A call's effects applied once its change is published: the refs it defined, kept
    /// with the version it left (a changing call's version keeps the refs as they are); the
    /// images it shows counted; its files and datasets back in its reply for the application.
    fn apply_effects(
        &self,
        result: &mut ToolResult<Value>,
        refs: &mut CallRefs,
        doc: &str,
        owner: Option<&RefOwner>,
        changing: bool,
        retry: &TryCall,
    ) -> Result<(), ToolError> {
        let mut effects = OpEffects::default();
        effects.take_from(result);
        let version = result.version.unwrap_or(0);
        let mut defined = Vec::new();
        {
            let mut stores = self.stores();
            let table = stores.refs.get(doc);
            for (name, address) in &effects.refs {
                match refs.define(name, address, doc, table, retry) {
                    Ok((kept, _)) => defined.push((kept, address.clone())),
                    Err(_) if result.is_error() => {}
                    Err(error) => return Err(error),
                }
            }
            if !result.is_error() {
                if let Some(table) = stores.refs.get_mut(doc) {
                    match refs.defined(doc) {
                        Some(own) => table.keep(own, owner, version),
                        None if changing => table.keep(&ooxml_session::RefMap::new(), owner, version),
                        None => {}
                    }
                }
                if effects.images > 0 {
                    // (the format refused a call past the budgets before it drew anything)
                    let _ = stores.images.reserve(effects.images);
                }
            }
        }
        // a PNG the tool gives back is an image the model is shown
        let mut images = Vec::new();
        for file in &effects.files {
            if let (true, Some((width, height))) = (file.format == "png", crate::images::png_size(&file.bytes)) {
                images.push(crate::images::descriptor(&file.name, width, height));
            }
        }
        if !images.is_empty() && !result.is_error() {
            // (the tool refused a call past the budgets before it drew anything)
            let _ = self.stores().images.reserve(images.len());
        }
        result.app.files = std::mem::take(&mut effects.files);
        if let Outcome::Done(done) = &mut result.outcome {
            done.refs = defined;
            done.datasets = std::mem::take(&mut effects.datasets);
            done.images.extend(images);
        }
        Ok(())
    }

    /// The acceptance's refusal in the host's words where it knows more: the action a name
    /// meant, or why a tool takes no key.
    fn reworded(&self, entry: &Entry, retry: &TryCall, error: ToolError, roles: RoleSet) -> ToolError {
        match error.field() {
            Some("action") => {
                let given = retry.arguments.get("action").and_then(Value::as_str).unwrap_or("");
                let message = self.host.catalogue.action_message(entry, given, error.message(), roles);
                error.with_message(message)
            }
            Some("key") if entry.spec.flags.mutates => error.with_message(format!(
                "{} takes no key: a retry key is for the tools that create something, so a repeat makes \
                 nothing twice; this one is safe to repeat as it is -- resend without key",
                entry.name()
            )),
            _ => error,
        }
    }

    /// `arguments` with their refs resolved on the tool's declared paths (and the notes of
    /// what was read as what); for a changing call, its ref names checked as the call keeps
    /// them (`(kept, given)` for each folded) and owned first.
    pub(crate) fn with_refs(
        &self,
        entry: &Entry,
        refs: &mut CallRefs,
        doc: &str,
        arguments: Map<String, Value>,
        changing: bool,
        retry: &TryCall,
    ) -> Result<WithRefs, ToolError> {
        let paths = entry.spec.paths(Holding::Refs);
        let stores = self.stores();
        let table = stores.refs.get(doc);
        let (resolved, notes) = if paths.is_empty() {
            (Value::Object(arguments), Vec::new())
        } else {
            refs.resolve_declared(&Value::Object(arguments), &paths, doc, table, retry)?
        };
        let Value::Object(mut arguments) = resolved else {
            return Err(ToolError::internal("refs resolved to no object", "host:refs"));
        };
        let mut notes = notes;
        if let Some(places) = &entry.spec.places {
            let fields: &[&str] = if places.kind == PlaceKind::Sides {
                &["after", "before"]
            } else {
                &["at"]
            };
            for field in fields {
                let Some(Value::String(place)) = arguments.get(*field) else {
                    continue;
                };
                if !place.contains('$') {
                    continue;
                }
                let resolved = place_refs(place, places.kind == PlaceKind::Blocks, retry, |name| {
                    let (address, note) = refs.resolve_ref(name, doc, table, Some(field), retry)?;
                    notes.extend(note);
                    Ok(address)
                })?;
                arguments.insert((*field).to_owned(), Value::from(resolved));
            }
        }
        let mut folded = Vec::new();
        if changing {
            refs.own_first(&Value::Object(arguments.clone()), doc, table, retry)?;
            // A name that folds to one naming another address is refused before the handler
            // runs, so the refusal changes nothing (docx-agent's `_kept_refs`); one that folds
            // to none is the tool's to refuse (it may be a place written as a ref).
            kept_names(&mut arguments, |name| {
                match refs.ref_name(name, None, doc, table, retry) {
                    Ok(kept) if kept != name => {
                        folded.push((kept.clone(), name.to_owned()));
                        Ok(Some(kept))
                    }
                    Ok(_) => Ok(None),
                    Err(taken) if taken.details().contains_key("address") => Err(taken),
                    Err(_) => Ok(None),
                }
            })?;
        }
        Ok(WithRefs {
            arguments,
            notes,
            folded,
        })
    }

    /// The context a call of this session runs with: who calls, the host's limits, the
    /// session's blobs and image budgets as they stand, and the moment its writes are dated
    /// by (`date`, the application's; `None`: the session's clock).
    pub(crate) fn context(&self, caller: Option<&Map<String, Value>>, date: Option<&str>) -> CallContext {
        let stores = self.stores();
        CallContext {
            caller: caller.map_or_else(Caller::anonymous, |c| Caller::named(&caller_key(c))),
            caller_context: caller.map(|c| Arc::new(c.clone())),
            limits: self.host.settings.limits,
            layout_timeout: self.host.settings.layout_timeout,
            image_budgets: stores.images.budgets(),
            blobs: Some(Arc::new(BlobList(stores.blobs.blobs().to_vec()))),
            date: date.map(str::to_owned),
            ..CallContext::default()
        }
    }

    /// The checks a changing call's reply carries, its format's of the version it left
    /// (else of the head), the tool's own checks kept over them.
    fn checks(&self, doc: &str, made: Option<&ooxml_session::CallMade>, result: &mut ToolResult<Value>) {
        let Some(id) = DocId::parse(doc) else { return };
        let Ok(format) = self.session.format_doc(&id) else {
            return;
        };
        let found = match made {
            Some(made) => format.checks(made.last.any(), made.format.as_deref()),
            None => format.checks(&format.stored().head(), None),
        };
        if let (Some(mut checks), Outcome::Done(done)) = (found, &mut result.outcome) {
            checks.extend(std::mem::take(&mut done.checks));
            done.checks = checks;
        }
    }

    /// What a call that made versions touched, for the application: its format's entries
    /// and the parts whose bytes changed, each with the document, from the state its first
    /// edit was applied to through the version its last made.
    fn touched(&self, doc: &str, made: &ooxml_session::CallMade, result: &mut ToolResult<Value>) {
        let Some(id) = DocId::parse(doc) else { return };
        let Ok(format) = self.session.format_doc(&id) else {
            return;
        };
        let before = &made.before;
        self.say_touched(
            doc,
            &*format,
            before.any(),
            made.last.any(),
            made.format.as_deref(),
            result,
        );
    }

    /// `result`'s touched and changed parts between `before` and `after` of `doc`.
    pub(crate) fn say_touched(
        &self,
        doc: &str,
        format: &dyn ooxml_session::FormatDoc,
        before: &ooxml_session::AnyVersion,
        after: &ooxml_session::AnyVersion,
        made: Option<&(dyn std::any::Any + Send + Sync)>,
        result: &mut ToolResult<Value>,
    ) {
        let with_doc = |mut entry: Map<String, Value>| {
            let mut out = Map::new();
            out.insert("doc".into(), Value::from(doc));
            out.append(&mut entry);
            Value::Object(out)
        };
        result.app.touched = format
            .touched(before, after, made)
            .into_iter()
            .map(|entry| match entry {
                Value::Object(entry) => with_doc(entry),
                other => other,
            })
            .collect();
        let mut parts = ooxml_session::changed_parts(before.package(), after.package());
        parts.sort();
        result.app.changed_parts = parts
            .into_iter()
            .map(|part| {
                let mut entry = Map::new();
                entry.insert("part".into(), Value::from(part));
                with_doc(entry)
            })
            .collect();
    }

    /// `undo`, the session's, its document's refs brought back to the state it returns to.
    fn undo(
        &self,
        arguments: Map<String, Value>,
        caller: Option<&Map<String, Value>>,
        going: &mut Going,
    ) -> ToolResult<Value> {
        going.docs = arguments
            .get("doc")
            .and_then(Value::as_str)
            .map(|d| vec![d.to_owned()])
            .unwrap_or_default();
        let context = self.context(caller, going.date.as_deref());
        let id = going.docs.first().and_then(|d| DocId::parse(d));
        let before = id.as_ref().and_then(|id| self.session.snapshot(id).ok());
        let mut result = self.session.call(&going.tool, Value::Object(arguments), &context);
        if let (false, Some(doc), Some(version)) = (result.is_error(), going.docs.first(), result.version) {
            if let Some(table) = self.stores().refs.get_mut(doc) {
                table.restore(version);
            }
            let after = id.as_ref().and_then(|id| self.session.snapshot(id).ok());
            let format = id.as_ref().and_then(|id| self.session.format_doc(id).ok());
            if let (Some(before), Some(after), Some(format)) = (before, after, format) {
                self.say_touched(doc, &*format, before.any(), after.any(), None, &mut result);
            }
        }
        result
    }

    /// The call's record, kept in the log.
    fn record(
        &self,
        going: &Going,
        result: &ToolResult<Value>,
        roles: RoleSet,
        how: &CallAs,
        duration: std::time::Duration,
    ) -> CallRecord {
        let arguments = match &going.arguments {
            Value::Object(map) => Value::Object(map.clone()),
            other => {
                let mut raw = Map::new();
                raw.insert("raw".into(), Value::from(other.to_string()));
                Value::Object(raw)
            }
        };
        let (code, summary) = match &result.outcome {
            Outcome::Failed(error) => (Some(error.code().as_str().to_owned()), error.message().to_owned()),
            Outcome::Done(done) => (None, done.summary.clone()),
        };
        let record = CallRecord {
            tool: going.tool.clone(),
            docs: going.docs.clone(),
            digest: digest(&arguments),
            shape: shape(&arguments),
            ok: !result.is_error(),
            code,
            summary,
            duration,
            version: result.version,
            arguments: self
                .host
                .settings
                .log_arguments
                .then(|| arguments.as_object().cloned())
                .flatten(),
            roles: (roles != self.roles()).then(|| roles.roles().iter().map(|r| r.as_str().to_owned()).collect()),
            context: how.context.clone().filter(|c| !c.is_empty()),
            panic: result.app.panic.clone(),
        };
        self.stores().log.push(record.clone());
        record
    }
}

/// Where a call's time went (R142, as 0.1's `Result.block`): the whole call, the time it
/// waited for its documents' commit locks and gates, the rest, and how long it held the
/// locks, and the CPU time of the thread the call ran on (`cpu`, where the clock gives it).
fn timing(
    total: std::time::Duration,
    cpu: Option<std::time::Duration>,
    lock: ooxml_session::LockSpent,
) -> ooxml_tools::Timing {
    let total = total.as_secs_f64();
    let lock_wait = lock.waited.as_secs_f64().min(total);
    ooxml_tools::Timing {
        total,
        lock_wait,
        work: total - lock_wait,
        cpu: cpu.map(|cpu| cpu.as_secs_f64()),
        held: lock.held.as_secs_f64(),
    }
}

/// A batch's arguments with each op held to what is read of it: its `tool` and `arguments`.
fn ops_read(mut arguments: Value) -> Value {
    if let Some(Value::Array(ops)) = arguments.get_mut("ops") {
        for op in ops.iter_mut() {
            if let Value::Object(op) = op {
                op.retain(|key, _| key == "tool" || key == "arguments");
            }
        }
    }
    arguments
}

/// A caller's context as the session's caller key: its JSON, keys sorted.
#[must_use]
pub fn caller_key(context: &Map<String, Value>) -> String {
    let sorted: std::collections::BTreeMap<&String, &Value> = context.iter().collect();
    serde_json::to_string(&sorted).unwrap_or_default()
}

/// A place (`after:$x`, `before:$a..$b`, `replace:$x`, `$x`) with each `$name` of its
/// address resolved by `resolve` (docx-agent's `blocks_at` and `sided`): the side kept, a
/// span's ends resolved each; for a place of new blocks (`blocks`), a span placed beside
/// read as its end on that side -- after its last block, before its first -- where it is
/// no range of characters.
fn place_refs(
    place: &str,
    blocks: bool,
    retry: &TryCall,
    mut resolve: impl FnMut(&str) -> Result<String, ToolError>,
) -> Result<String, ToolError> {
    let (side, rest) = match place.split_once(':') {
        Some((side, rest)) if matches!(side, "after" | "before" | "replace") => (Some(side), rest),
        _ => (None, place),
    };
    let parts: Result<Vec<String>, ToolError> = rest
        .split("..")
        .map(|part| {
            if part.starts_with('$') {
                resolve(part)
            } else {
                Ok(part.to_owned())
            }
        })
        .collect();
    let parts = parts?;
    if blocks && parts.iter().any(|p| p.starts_with("h:")) {
        let form = "placing at a heading's range by reference";
        return Err(ToolError::refusal(
            ooxml_tools::Refusal::Refused,
            format!("{form} is not supported yet; give the heading's first or last block's id"),
            ooxml_tools::WayOut::call(retry.clone()),
        )
        .with_detail("not_supported", Value::from(form))
        .at("at"));
    }
    let mut joined = parts.join("..");
    if let (true, Some(side @ ("after" | "before")), Some((first, last))) = (blocks, side, joined.split_once("..")) {
        if !joined.contains('@') {
            let end = if side == "before" { first } else { last }.trim();
            if !end.is_empty() {
                joined = end.to_owned();
            }
        }
    }
    Ok(match side {
        Some(side) => format!("{side}:{joined}"),
        None => joined,
    })
}

/// Each ref name a call gives (`ref`, `items[].ref`) as `keep` keeps it, where it says one;
/// the first refusal `keep` gives.
fn kept_names(
    arguments: &mut Map<String, Value>,
    mut keep: impl FnMut(&str) -> Result<Option<String>, ToolError>,
) -> Result<(), ToolError> {
    let mut kept = |holder: &mut Map<String, Value>| -> Result<(), ToolError> {
        let found = holder
            .get("ref")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .map(str::to_owned);
        if let Some(name) = found {
            if let Some(kept) = keep(&name)? {
                holder.insert("ref".into(), Value::from(kept));
            }
        }
        Ok(())
    };
    kept(arguments)?;
    if let Some(Value::Array(items)) = arguments.get_mut("items") {
        for item in items.iter_mut() {
            if let Value::Object(item) = item {
                kept(item)?;
            }
        }
    }
    Ok(())
}
