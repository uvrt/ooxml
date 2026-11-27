//! Owns: `check`'s `include: ["numbers"]` and its `scale`, done by the host for every format
//! around the format's own check (design phase 3, section 5; ooxml-edit's
//! `numbers.with_numbers` and `scales.with_scale`, ported exactly). Numbers: the texts the
//! call's document gives ([`FormatDoc::number_texts`], within the check's scope), compared
//! with the session's index of values -- its datasets, and the tables the open documents list
//! ([`FormatDoc::value_sources`]), kept while none of them changes ([`KeptIndex`]). A scale:
//! read or refused, the values its targets hold as the document's format reads them
//! ([`FormatDoc::scale_values`]), each checked against its step. Each fact is added to the
//! format's reply (`numbers`, `scale`), numbers' seconds where the reply says where its time
//! went; the format's check not run when nothing else is asked of it.
//!
//! Must not: read a document (its format does), or run the format's check with `numbers` or
//! a `scale` asked of it.

use crate::session::HostSession;
use ooxml_numfmt::Locale;
use ooxml_session::numbers::{self, Index, NumberRules, NumbersEnv, Source, INCLUDE};
use ooxml_session::scales::{self, ScaleEnv};
use ooxml_session::{AnyVersion, CallCancel, CallContext, DocId, FormatDoc, ReadTexts, ValueRead};
use ooxml_tools::{Done, Outcome, Refusal, ToolError, ToolResult, TryCall, WayOut};
use serde_json::{Map, Value};
use std::sync::{Arc, Weak};

/// The arguments that narrow a check, handed to the format's read.
const SCOPE: [&str; 3] = ["slides", "pages", "range"];
/// The arguments that ask the format's check for something besides.
const ASKS: [&str; 6] = ["boxes", "spacing", "contrast", "min_size", "styles", "copies"];

/// The index of values a session keeps while no dataset or listed document changes: what it
/// was built from (each dataset by identity, each document by identity and version), the
/// index, and what could not be read for it.
pub struct KeptIndex {
    key: Vec<(String, usize, u64)>,
    docs: Vec<Weak<dyn FormatDoc>>,
    index: Arc<Index>,
    unread: Vec<String>,
}

impl KeptIndex {
    /// The index kept.
    #[must_use]
    pub fn index(&self) -> &Arc<Index> {
        &self.index
    }
}

/// Whether a value asks for something, as Python's `value not in (None, False, [], {})`
/// reads it (`0` is `False`).
fn asks(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

/// A check's fact (of numbers, of a scale): its data, the warnings its document's read gave,
/// and the version it read.
struct Fact {
    data: Map<String, Value>,
    warnings: Vec<String>,
    read: Read,
}

/// A call's document at the version it reads: pinned in its context, else the head.
pub(crate) struct Read {
    pub(crate) doc: Arc<dyn FormatDoc>,
    pub(crate) at: AnyVersion,
}

impl HostSession {
    /// The rules of this session's checks of numbers.
    #[must_use]
    pub fn number_rules(&self) -> NumberRules {
        self.numbers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The application sets this session's rules for checks of numbers (`numbers=` of
    /// ooxml-edit's session).
    pub fn set_number_rules(&self, rules: NumberRules) {
        *self.numbers.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = rules;
    }

    /// A call of `tool` on `doc` with `arguments` (taken, refs resolved): a `check` with a
    /// `scale` or `numbers` asked for, each done here and added to the format's reply (`run`
    /// runs the format's tool with what is left of the arguments; `alone` makes a reply of the
    /// facts alone; `result` reads a reply); else the format's tool alone.
    pub(crate) fn check_values<T, R>(
        &self,
        (tool, doc): (&str, &str),
        mut arguments: Map<String, Value>,
        context: &CallContext,
        retry: &TryCall,
        calls: (
            impl FnOnce(Map<String, Value>) -> T,
            impl FnOnce(ToolResult<Value>) -> T,
            R,
        ),
    ) -> T
    where
        R: Fn(&mut T) -> &mut ToolResult<Value>,
    {
        let (run, alone, result_of) = calls;
        if tool != "check" {
            return run(arguments);
        }
        let Some(given) = arguments.remove(scales::PARAM).filter(|s| !s.is_null()) else {
            return self.with_numbers(doc, arguments, context, retry, (run, alone, &result_of));
        };
        let fact = match self.scale_fact(doc, &given, &arguments, context, retry) {
            Ok(found) => found,
            Err(error) => return alone(self.refused_at_read(error, doc, context)),
        };
        let said = scales::summary(&fact.data);
        let included = arguments.get("include").is_some_and(|i| asks(Some(i)));
        if !included && !ASKS.iter().any(|name| asks(arguments.get(*name))) {
            return alone(fact_alone(scales::PARAM, fact, said, doc));
        }
        let mut returned = self.with_numbers(doc, arguments, context, retry, (run, alone, &result_of));
        let result = result_of(&mut returned);
        added(result, scales::PARAM, fact, &said, None);
        returned
    }

    /// [`HostSession::check_values`] of a check without a scale: `numbers`, where asked for.
    fn with_numbers<T, R>(
        &self,
        doc: &str,
        mut arguments: Map<String, Value>,
        context: &CallContext,
        retry: &TryCall,
        calls: (
            impl FnOnce(Map<String, Value>) -> T,
            impl FnOnce(ToolResult<Value>) -> T,
            &R,
        ),
    ) -> T
    where
        R: Fn(&mut T) -> &mut ToolResult<Value>,
    {
        let (run, alone, result_of) = calls;
        let include: Vec<Value> = arguments
            .get("include")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !include.iter().any(|i| i == INCLUDE) {
            return run(arguments);
        }
        let rest: Vec<Value> = include.into_iter().filter(|i| i != INCLUDE).collect();
        let clock = &self.session.context().clock;
        let began = clock.monotonic();
        let fact = match self.numbers_fact(doc, &arguments, context, retry) {
            Ok(found) => found,
            Err(error) => return alone(self.refused_at_read(error, doc, context)),
        };
        let took = clock.monotonic().saturating_sub(began).as_secs_f64();
        let said = numbers::summary(&fact.data);
        if rest.is_empty() && !ASKS.iter().any(|name| asks(arguments.get(*name))) {
            return alone(fact_alone(INCLUDE, fact, said, doc));
        }
        if rest.is_empty() {
            arguments.remove("include");
        } else {
            arguments.insert("include".into(), Value::Array(rest));
        }
        let mut returned = run(arguments);
        added(result_of(&mut returned), INCLUDE, fact, &said, Some(took));
        returned
    }

    /// The fact of `given`, a scale, for a check of `doc` with `arguments`: the values its
    /// targets hold, read by the document's format, each checked against its step.
    fn scale_fact(
        &self,
        doc: &str,
        given: &Value,
        arguments: &Map<String, Value>,
        context: &CallContext,
        retry: &TryCall,
    ) -> Result<Fact, ToolError> {
        let read = self
            .read_of(doc, context)
            .ok_or_else(|| ToolError::internal(format!("{doc} is not open"), "host:scale"))?;
        let scale = scales::parse(given, &WayOut::call(retry.clone()))?;
        let scope = self.scope_of(doc, arguments, retry)?;
        let cancel = CallCancel::new(context, &*self.session.context().cancel, &*self.session.context().clock);
        let value_read = ValueRead {
            named: doc,
            scope: &scope,
            context,
            cancel: &cancel,
        };
        let values = match read.doc.scale_values(&read.at, &scale, &value_read) {
            Some(found) => found?,
            None => return Err(self.no_reading(doc, retry)),
        };
        let locale = self.session.config().locale.clone().unwrap_or_default();
        let data = self.with_facts(retry, &context.values.held, |fact| {
            let env = Env {
                locale: &locale,
                doc,
                fact,
            };
            scales::report(&env, &scale, &values.values)
        });
        Ok(Fact {
            data,
            warnings: values.warnings,
            read,
        })
    }

    /// A scale refused for a document whose format reads none, naming the kinds that do.
    fn no_reading(&self, doc: &str, retry: &TryCall) -> ToolError {
        let mut kinds: Vec<&str> = self
            .host
            .formats
            .iter()
            .filter(|f| f.gives_scale_values())
            .map(|f| f.kind().as_str())
            .collect();
        kinds.sort_unstable();
        let kind = DocId::parse(doc)
            .and_then(|id| self.session.kind_of(&id).ok())
            .map_or("call without a document", |k| k.as_str());
        scales::no_reading(&kinds, kind, WayOut::call(retry.clone()))
    }

    /// A refusal of a check's fact as the call's reply, naming `doc` at the version it read.
    fn refused_at_read(&self, error: ToolError, doc: &str, context: &CallContext) -> ToolResult<Value> {
        let mut result = ToolResult::failed(error);
        result.doc = Some(doc.to_owned());
        result.version = self.read_of(doc, context).map(|r| r.at.number());
        result
    }

    /// `doc` at the version the call reads: pinned in `context`, else its head.
    pub(crate) fn read_of(&self, doc: &str, context: &CallContext) -> Option<Read> {
        let id = DocId::parse(doc)?;
        let format = self.answering(&id, self.session.format_doc(&id).ok()?, &context.values.held);
        let at = match context.at.iter().find(|o| *o.id() == id) {
            Some(pinned) => pinned.any().clone(),
            None => self.session.snapshot(&id).ok()?.any().clone(),
        };
        Some(Read { doc: format, at })
    }

    /// The fact of numbers for a check of `doc` with `arguments`: its data, the warnings its
    /// document's read gave, and the version it read.
    fn numbers_fact(
        &self,
        doc: &str,
        arguments: &Map<String, Value>,
        context: &CallContext,
        retry: &TryCall,
    ) -> Result<Fact, ToolError> {
        let read = self
            .read_of(doc, context)
            .ok_or_else(|| ToolError::internal(format!("{doc} is not open"), "host:numbers"))?;
        let scope = self.scope_of(doc, arguments, retry)?;
        let texts = self.texts_of(doc, &read, &scope, context)?;
        let (index, notes) = self.index_of(context, retry);
        let narrower: Vec<&str> = SCOPE.to_vec();
        let rules = self.number_rules();
        let locale = self.session.config().locale.clone().unwrap_or_default();
        let data = self.with_facts(retry, &context.values.held, |fact| {
            let env = Env {
                locale: &locale,
                doc,
                fact,
            };
            numbers::report(&env, &index, notes, &texts.texts, &rules, &narrower)
        });
        let data = data.map_err(|why| ToolError::internal(why, "host:numbers"))?;
        Ok(Fact {
            data,
            warnings: texts.warnings,
            read,
        })
    }

    /// The check's arguments that narrow it, as given, a `range`'s refs resolved (`$x`, and
    /// both ends of `$a..$b`).
    fn scope_of(
        &self,
        doc: &str,
        arguments: &Map<String, Value>,
        retry: &TryCall,
    ) -> Result<Map<String, Value>, ToolError> {
        let mut scope: Map<String, Value> = SCOPE
            .iter()
            .filter_map(|name| {
                Some((
                    (*name).to_owned(),
                    arguments.get(*name).filter(|v| !v.is_null())?.clone(),
                ))
            })
            .collect();
        let Some(range) = scope.get("range").cloned() else {
            return Ok(scope);
        };
        let stores = self.stores();
        let table = stores.refs.get(doc);
        let refs = ooxml_session::CallRefs::new(None);
        let resolve = |item: &str| -> Result<String, ToolError> {
            let one = |end: &str| -> Result<String, ToolError> {
                let end = end.trim();
                if end.starts_with('$') {
                    Ok(refs.resolve_ref(end, doc, table, Some("range"), retry)?.0)
                } else {
                    Ok(end.to_owned())
                }
            };
            match item.split_once("..") {
                Some((first, last)) if item.contains('$') => Ok(format!("{}..{}", one(first)?, one(last)?)),
                _ if item.starts_with('$') => one(item),
                _ => Ok(item.to_owned()),
            }
        };
        let resolved = match &range {
            Value::String(item) => Value::from(resolve(item)?),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|i| i.as_str().map_or(Ok(i.clone()), |s| resolve(s).map(Value::from)))
                    .collect::<Result<_, _>>()?,
            ),
            other => other.clone(),
        };
        scope.insert("range".into(), resolved);
        Ok(scope)
    }

    /// The texts `doc` gives a check of numbers within `scope` (`texts_of`): refused, with
    /// the way out, where its format gives none.
    fn texts_of(
        &self,
        doc: &str,
        read: &Read,
        scope: &Map<String, Value>,
        context: &CallContext,
    ) -> Result<ReadTexts, ToolError> {
        let cancel = CallCancel::new(context, &*self.session.context().cancel, &*self.session.context().clock);
        let value_read = ValueRead {
            named: doc,
            scope,
            context,
            cancel: &cancel,
        };
        if let Some(found) = read.doc.number_texts(&read.at, &value_read) {
            return found;
        }
        let mut kinds: Vec<&str> = self
            .host
            .formats
            .iter()
            .filter(|f| f.gives_number_texts())
            .map(|f| f.kind().as_str())
            .collect();
        kinds.sort_unstable();
        let kind = DocId::parse(doc)
            .and_then(|id| self.session.kind_of(&id).ok())
            .map_or("call without a document", |k| k.as_str());
        let listed = if kinds.is_empty() {
            "no".to_owned()
        } else {
            kinds.join(" and ")
        };
        Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!(
                "numbers are read from {listed} documents here; a {kind} gives no texts for them: leave numbers out \
                 of include"
            ),
            WayOut::names(["validate", "layout"]).unwrap_or_else(|| WayOut::one(Value::Null)),
        )
        .at("include"))
    }

    /// The session's index of values (`index_of`), built once while no dataset or listed
    /// document changes, and what could not be read for it.
    fn index_of(&self, context: &CallContext, retry: &TryCall) -> (Arc<Index>, Vec<String>) {
        let entries: Vec<(String, Arc<ooxml_tools::datasets::Dataset>)> = self
            .stores()
            .datasets
            .entries()
            .iter()
            .map(|e| (e.address(), Arc::clone(&e.dataset)))
            .collect();
        let listed = self.listed(context);
        let mut key: Vec<(String, usize, u64)> = entries
            .iter()
            .map(|(address, dataset)| (address.clone(), Arc::as_ptr(dataset) as usize, 0))
            .collect();
        key.extend(
            listed
                .iter()
                .map(|(doc, read, _)| (doc.clone(), Arc::as_ptr(&read.doc).cast::<()>() as usize, read.at.seq())),
        );
        if let Some(kept) = &self.stores().numbers {
            if kept.key == key && kept.docs.iter().all(|d| d.upgrade().is_some()) {
                return (Arc::clone(&kept.index), kept.unread.clone());
            }
        }
        let mut unread = Vec::new();
        let sources = self.sources_of(entries, &listed, &mut unread, retry);
        let index = Arc::new(Index::new(sources));
        self.stores().numbers = Some(KeptIndex {
            key,
            docs: listed.iter().map(|(_, read, _)| Arc::downgrade(&read.doc)).collect(),
            index: Arc::clone(&index),
            unread: unread.clone(),
        });
        (index, unread)
    }

    /// The open documents whose format lists tables of values, each at the version the call
    /// reads, with the prefixes its format takes.
    fn listed(&self, context: &CallContext) -> Vec<(String, Read, Vec<String>)> {
        let mut out = Vec::new();
        for id in self.session.documents() {
            let doc = id.to_string();
            let Some(read) = self.read_of(&doc, context) else {
                continue;
            };
            if read.doc.value_sources(&read.at).is_none() {
                continue;
            }
            let prefixes = self
                .session
                .kind_of(&id)
                .ok()
                .and_then(|kind| self.host.formats.iter().find(|f| f.kind() == kind))
                .map(|f| f.source_prefixes())
                .unwrap_or_default();
            out.push((doc, read, prefixes));
        }
        out
    }

    /// The datasets, and the tables each listed document lists, read by its format; what
    /// could not be read said in `unread` (`_read`).
    fn sources_of(
        &self,
        entries: Vec<(String, Arc<ooxml_tools::datasets::Dataset>)>,
        listed: &[(String, Read, Vec<String>)],
        unread: &mut Vec<String>,
        retry: &TryCall,
    ) -> Vec<Source> {
        let mut out: Vec<Source> = entries
            .into_iter()
            .map(|(address, dataset)| Source {
                shown: address.clone(),
                address,
                dataset,
            })
            .collect();
        let mut takers: Vec<(String, usize)> = Vec::new();
        for prefix in listed.iter().flat_map(|(_, _, p)| p) {
            match takers.iter_mut().find(|(p, _)| p == prefix) {
                Some(found) => found.1 += 1,
                None => takers.push((prefix.clone(), 1)),
            }
        }
        for (doc, read, _) in listed {
            for address in read.doc.value_sources(&read.at).unwrap_or_default() {
                let found = match read.doc.source(&read.at, &address) {
                    Some(found) => found,
                    None => {
                        let kind = DocId::parse(doc).and_then(|id| self.session.kind_of(&id).ok());
                        Err(kind.map_or_else(
                            || ToolError::internal("no kind", "host:numbers"),
                            |k| ooxml_session::no_source(k, &address, retry),
                        ))
                    }
                };
                let dataset = match found {
                    Ok(dataset) => dataset,
                    Err(error) => {
                        unread.push(format!("{doc}/{address} was not read ({})", error.message()));
                        continue;
                    }
                };
                let count = takers
                    .iter()
                    .find(|(p, _)| address.starts_with(p.as_str()))
                    .map_or(0, |(_, n)| *n);
                let qualified = format!("{doc}/{address}");
                out.push(Source {
                    shown: if count == 1 { address.clone() } else { qualified.clone() },
                    address: qualified,
                    dataset: Arc::new(dataset),
                });
            }
        }
        out
    }
}

/// A report's surroundings in the host.
struct Env<'a> {
    locale: &'a Locale,
    doc: &'a str,
    fact: &'a dyn Fn(&str) -> Result<String, String>,
}

impl NumbersEnv for Env<'_> {
    fn locale(&self) -> &Locale {
        self.locale
    }

    fn doc(&self) -> Option<&str> {
        Some(self.doc)
    }

    fn fact(&self, address: &str) -> Option<String> {
        (self.fact)(address).ok()
    }
}

impl ScaleEnv for Env<'_> {
    fn locale(&self) -> &Locale {
        self.locale
    }

    fn doc(&self) -> Option<&str> {
        Some(self.doc)
    }

    fn fact(&self, address: &str) -> Result<String, String> {
        (self.fact)(address)
    }
}

/// A reply of a check's fact alone, `key` its data's one key, on the version it read.
fn fact_alone(key: &str, fact: Fact, said: String, doc: &str) -> ToolResult<Value> {
    let mut data = Map::new();
    data.insert(key.to_owned(), Value::Object(fact.data));
    let mut done = Done::new(said);
    done.data = Some(Value::Object(data));
    done.warnings = fact.warnings;
    let mut result = ToolResult::done(done);
    result.doc = Some(doc.to_owned());
    result.version = Some(fact.read.at.number());
    result
}

/// A check's fact added to the format's reply `result` as `key`: its read's warnings first,
/// its summary after the reply's, its seconds where the reply says where its time went.
fn added(result: &mut ToolResult<Value>, key: &str, fact: Fact, said: &str, took: Option<f64>) {
    let mut first = fact.warnings;
    first.append(&mut result.warnings);
    result.warnings = first;
    if let Outcome::Done(done) = &mut result.outcome {
        if let Some(Value::Object(out)) = &mut done.data {
            out.insert(key.to_owned(), Value::Object(fact.data));
            if let Some(took) = took {
                timed(out, key, took);
            }
        }
        done.summary = if done.summary.is_empty() {
            said.to_owned()
        } else {
            format!("{}; {said}", done.summary)
        };
    }
}

/// `part`'s seconds in a reply's data that says where its time went (`data.timing`), kept
/// its last key; none added to data without one (round 57, R124: `_timed`).
fn timed(data: &mut Map<String, Value>, part: &str, seconds: f64) {
    let Some(timing) = data.shift_remove("timing") else {
        return;
    };
    let timing = match timing {
        Value::Object(mut timing) => {
            let before = timing.get(part).and_then(Value::as_f64).unwrap_or(0.0);
            timing.insert(part.to_owned(), Value::from(before + seconds));
            Value::Object(timing)
        }
        other => other,
    };
    data.insert("timing".into(), timing);
}
