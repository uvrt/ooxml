//! Owns: what the host adds to a format tool's reply (ooxml-edit's wrappers of the shared
//! tools and docx-agent's host parts, ported exactly): `save_document`'s file handed to the
//! application behind the save gate -- the document's problems now against those it opened
//! with, a new one refusing the save unless the application allows it -- as an output with
//! the format's state beside its bytes (a document, not its text copy), the model told its
//! name, format, size and the gate's report, never its bytes ([`HostSession::output`]);
//! `check`'s `validation`, what the save would refuse as new, over the whole document
//! whatever the check asked, and its summary's note ([`validation_fact`]); `describe`'s
//! `problems`, those the document opened with, and the session's datasets.
//!
//! Must not: find problems (the format does), or keep a file's bytes in the reply.

use crate::session::HostSession;
use ooxml_session::outputs::Output;
use ooxml_session::problems::Validation;
use ooxml_session::{AnyVersion, DocId, FormatDoc};
use ooxml_tools::{Outcome, Refusal, ToolError, ToolResult, WayOut};
use serde_json::{Map, Value};
use std::sync::Arc;

/// At most this many new problems listed in `check`'s `validation` (rule 4); `cut` counts
/// the rest, all of which `save_document`'s refusal lists.
const LISTED: usize = 20;

/// The formats `save_document` writes a document's text copy in: it carries no state.
const TEXT_COPIES: [&str; 2] = ["markdown", "outline"];

/// The gate's report as today's replies say it: each problem as its text.
fn report(validation: &Validation) -> Map<String, Value> {
    let texts = |items: &[ooxml_session::problems::Problem]| {
        Value::Array(items.iter().map(|p| Value::from(p.message.as_str())).collect())
    };
    let mut out = Map::new();
    out.insert("new".into(), texts(&validation.new));
    out.insert("fixed".into(), texts(&validation.fixed));
    out.insert("baseline".into(), Value::from(validation.baseline));
    out
}

/// `check`'s `validation`: the problems saving would refuse as new (at most [`LISTED`],
/// `cut` the rest), the count the document opened with, and those of them fixed since
/// (when any).
#[must_use]
pub fn validation_fact(validation: &Validation) -> Map<String, Value> {
    let texts = |items: &[ooxml_session::problems::Problem]| {
        Value::Array(
            items
                .iter()
                .take(LISTED)
                .map(|p| Value::from(p.message.as_str()))
                .collect(),
        )
    };
    let mut out = Map::new();
    out.insert("new".into(), texts(&validation.new));
    out.insert("baseline".into(), Value::from(validation.baseline));
    if validation.new.len() > LISTED {
        out.insert("cut".into(), Value::from(validation.new.len() - LISTED));
    }
    if !validation.fixed.is_empty() {
        out.insert("fixed".into(), texts(&validation.fixed));
    }
    out
}

impl HostSession {
    /// What the host adds to the reply of the format tool `tool` on `doc`.
    pub(crate) fn host_added(&self, tool: &str, doc: &str, result: &mut ToolResult<Value>) {
        if result.is_error() {
            return;
        }
        let Some((format, at)) = self.version_read(doc, result.version) else {
            return;
        };
        self.host_added_at(tool, doc, &*format, &at, result);
    }

    /// [`HostSession::host_added`] of a call that read `at` of `doc` (a batch's op: the version
    /// the ops before it left, which only its batch reads).
    pub(crate) fn host_added_at(
        &self,
        tool: &str,
        doc: &str,
        format: &dyn FormatDoc,
        at: &AnyVersion,
        result: &mut ToolResult<Value>,
    ) {
        if result.is_error() {
            return;
        }
        let (format, at) = (format, at);
        match tool {
            "save_document" => self.output(doc, format, at, result),
            "check" => self.check_validation(format, at, result),
            "describe" => {
                describe_problems(format, result);
                self.describe_datasets(result);
            }
            _ => {}
        }
    }

    /// The document `doc` and the version a call read of it (numbered `version`; else the
    /// head).
    fn version_read(&self, doc: &str, version: Option<u64>) -> Option<(Arc<dyn FormatDoc>, AnyVersion)> {
        let id = DocId::parse(doc)?;
        let format = self.session.format_doc(&id).ok()?;
        let at = match version.map(|n| self.session.numbered(&id, n)) {
            Some(Ok(Some(opened))) => opened.any().clone(),
            _ => format.stored().head(),
        };
        Some((format, at))
    }

    /// `save_document`'s file handed to the application behind the save gate, its state
    /// beside it where it is the document itself; the reply says the output, never its bytes.
    fn output(&self, doc: &str, format: &dyn FormatDoc, at: &AnyVersion, result: &mut ToolResult<Value>) {
        let files = std::mem::take(&mut result.app.files);
        let Some(file) = files.into_iter().next() else {
            return;
        };
        let report = format.validation(at).map(|v| report(&v));
        let new: Vec<Value> = report
            .as_ref()
            .and_then(|r| r.get("new"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !new.is_empty() && !self.host.settings.allow_new_problems {
            let message = format!("saving would add {} validation problem(s); fix them first", new.len());
            let way_out = WayOut::options(new).unwrap_or_else(|| WayOut::one(Value::from("check")));
            let error = ToolError::refusal(Refusal::Refused, message, way_out)
                .with_detail("validate", Value::Object(report.unwrap_or_default()));
            let (doc, version) = (result.doc.take(), result.version);
            *result = ToolResult::failed(error);
            result.doc = doc;
            result.version = version;
            return;
        }
        let state = (!TEXT_COPIES.contains(&file.format.as_str()))
            .then(|| format.state(at))
            .flatten()
            .map(|s| Arc::new(s) as Arc<dyn std::any::Any + Send + Sync>);
        let output = Output {
            name: file.name,
            format: file.format,
            data: Arc::from(file.bytes),
            doc: Some(doc.to_owned()),
            validate: report.map(Value::Object),
            state,
        };
        let mut data = output.describe();
        if let Outcome::Done(done) = &mut result.outcome {
            if let Some(Value::Object(said)) = done.data.take() {
                for key in ["cleaned", "coverage"] {
                    if let Some(value) = said.get(key) {
                        data.insert(key.into(), value.clone());
                    }
                }
            }
            done.data = Some(Value::Object(data));
        }
        self.stores().outputs.add(output);
    }

    /// `describe`'s datasets: the session's, not a document's -- each its id, columns, type
    /// labels, row count and where it came from, never its values (ooxml-edit's
    /// `_with_datasets`); no key without any.
    fn describe_datasets(&self, result: &mut ToolResult<Value>) {
        let heads: Vec<(String, u64)> = self
            .session
            .documents()
            .into_iter()
            .filter_map(|id| Some((id.to_string(), self.session.snapshot(&id).ok()?.number())))
            .collect();
        let heads = |doc: &str| heads.iter().find(|(d, _)| d == doc).map(|(_, n)| *n);
        let briefs = self.stores().datasets.briefs(&heads);
        if let Outcome::Done(done) = &mut result.outcome {
            if let Some(Value::Object(data)) = &mut done.data {
                data.extend(briefs);
            }
        }
    }

    /// `check`'s whole-document `validation` and its summary's note.
    fn check_validation(&self, format: &dyn FormatDoc, at: &AnyVersion, result: &mut ToolResult<Value>) {
        let clock = &self.session.context().clock;
        let began = clock.monotonic();
        let Some(validation) = format.validation(at) else {
            return;
        };
        let took = clock.monotonic().saturating_sub(began).as_secs_f64();
        let Outcome::Done(done) = &mut result.outcome else {
            return;
        };
        let Some(Value::Object(data)) = done.data.take() else {
            return;
        };
        // before the tool's timing, which stays the data's last key and says this part's
        // seconds too
        let mut out: Map<String, Value> = data
            .iter()
            .filter(|(k, _)| *k != "timing")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.insert("validation".into(), Value::Object(validation_fact(&validation)));
        if let Some(timing) = data.get("timing") {
            let timing = match timing {
                Value::Object(timing) => {
                    let mut timing = timing.clone();
                    timing.insert("validation".into(), Value::from(took));
                    Value::Object(timing)
                }
                other => other.clone(),
            };
            out.insert("timing".into(), timing);
        }
        done.data = Some(Value::Object(out));
        if !validation.new.is_empty() {
            done.summary.push_str(&format!(
                "; saving would refuse {} new validation problem(s): see validation",
                validation.new.len()
            ));
        }
    }
}

/// `describe`'s `problems`: those the document opened with (at most 20), after its controls.
fn describe_problems(format: &dyn FormatDoc, result: &mut ToolResult<Value>) {
    let Outcome::Done(done) = &mut result.outcome else {
        return;
    };
    let Some(Value::Object(data)) = &mut done.data else {
        return;
    };
    if !data.contains_key("controls") {
        return;
    }
    let problems: Vec<Value> = format
        .baseline()
        .iter()
        .take(20)
        .map(|p| Value::from(p.message.as_str()))
        .collect();
    let mut out = Map::new();
    for (key, value) in std::mem::take(data) {
        let controls = key == "controls";
        out.insert(key, value);
        if controls {
            out.insert("problems".into(), Value::Array(problems.clone()));
        }
    }
    *data = out;
}
