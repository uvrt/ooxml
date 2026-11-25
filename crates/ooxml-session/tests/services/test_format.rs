//! `TestFormat`: a made-up format for the session's services, as ooxml-edit's tests use their
//! toy formats (`tools_toys.TEXT`, a list of items; `DECK`, pages with a title each, every
//! page its own undo scope). Its documents number their states as the tool layer does: each
//! change a new number, undo and redo back to the number of the state they return to, a
//! scoped undo renumbering the later states. A toy host runs its tools as ooxml-edit's
//! dispatcher does around a handler: the ownership check first, the declared refs resolved,
//! the handler, then the refs it defined kept with the version the call left -- or nothing
//! kept when it failed.
//!
//! Every name and figure here is made up.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_session::{CallRefs, RefMap, RefOwner, RefTable, ScopedStep};
use ooxml_tools::{ToolError, TryCall};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// One state of a toy document: its items and its pages' titles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Content {
    pub items: Vec<String>,
    pub titles: BTreeMap<String, String>,
}

/// One step of a toy document's history.
#[derive(Debug, Clone)]
struct Step {
    scope: Option<String>,
    before: u64,
    after: u64,
}

/// A toy document: its states by number, its history.
#[derive(Debug, Default)]
pub struct TestDoc {
    states: BTreeMap<u64, Content>,
    steps: Vec<Step>,
    redo: Vec<Step>,
    scoped_redo: Vec<Step>,
    pub version: u64,
    highest: u64,
}

impl TestDoc {
    fn new() -> Self {
        let mut doc = TestDoc::default();
        doc.states.insert(0, Content::default());
        doc
    }

    pub fn content(&self) -> &Content {
        &self.states[&self.version]
    }

    /// A change in `scope` (`None`: the whole document's): a new state.
    pub fn change(&mut self, scope: Option<&str>, edit: impl FnOnce(&mut Content)) -> u64 {
        let mut next = self.content().clone();
        edit(&mut next);
        self.highest += 1;
        let after = self.highest;
        self.states.insert(after, next);
        self.steps.push(Step {
            scope: scope.map(str::to_owned),
            before: self.version,
            after,
        });
        self.redo.clear();
        self.version = after;
        after
    }

    pub fn undo(&mut self) -> Option<u64> {
        let step = self.steps.pop()?;
        self.version = step.before;
        self.redo.push(step);
        Some(self.version)
    }

    pub fn redo(&mut self) -> Option<u64> {
        let step = self.redo.pop()?;
        self.version = step.after;
        self.steps.push(step);
        Some(self.version)
    }

    /// Take back page `scope`'s latest change from under the later ones, which get new
    /// numbers.
    pub fn undo_scope(&mut self, scope: &str) -> Option<ScopedStep> {
        let at = self.steps.iter().rposition(|s| s.scope.as_deref() == Some(scope))?;
        let step = self.steps.remove(at);
        let was = self.states[&step.before].titles.get(scope).cloned();
        let mut renumbered = Vec::new();
        let mut before = step.before;
        for later in self.steps.iter_mut().skip(at) {
            let mut content = self.states[&later.after].clone();
            match &was {
                Some(title) => content.titles.insert(scope.to_owned(), title.clone()),
                None => content.titles.remove(scope),
            };
            self.highest += 1;
            renumbered.push((later.after, self.highest));
            self.states.insert(self.highest, content);
            later.before = before;
            later.after = self.highest;
            before = self.highest;
        }
        self.version = before;
        self.scoped_redo.push(step.clone());
        Some(ScopedStep {
            before: step.before,
            after: step.after,
            version: self.version,
            renumbered,
        })
    }

    /// Bring page `scope`'s undone change back, as a new state.
    pub fn redo_scope(&mut self, scope: &str) -> Option<ScopedStep> {
        let at = self
            .scoped_redo
            .iter()
            .rposition(|s| s.scope.as_deref() == Some(scope))?;
        let step = self.scoped_redo.remove(at);
        let title = self.states[&step.after].titles.get(scope).cloned();
        let version = self.change(Some(scope), |c| {
            if let Some(title) = title {
                c.titles.insert(scope.to_owned(), title);
            }
        });
        Some(ScopedStep {
            before: step.before,
            after: step.after,
            version,
            renumbered: Vec::new(),
        })
    }
}

/// What a toy call answered.
#[derive(Debug, Default)]
pub struct Reply {
    pub error: Option<ToolError>,
    pub refs: RefMap,
    pub warnings: Vec<String>,
    pub data: Value,
    pub summary: String,
}

impl Reply {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }

    pub fn error(&self) -> Value {
        self.error.as_ref().map_or(Value::Null, ToolError::to_json)
    }
}

/// What a toy handler is given: the call's refs, its document, its arguments.
pub struct Run<'a> {
    pub refs: &'a mut CallRefs,
    pub doc: &'a str,
    pub table: Option<&'a RefTable>,
    pub content: &'a mut Content,
    pub changed: &'a mut Option<String>,
    pub warnings: &'a mut Vec<String>,
    pub retry: &'a TryCall,
}

impl Run<'_> {
    /// `call.define_ref`.
    pub fn define(&mut self, name: &str, address: &str) -> Result<String, ToolError> {
        let (kept, warning) = self.refs.define(name, address, self.doc, self.table, self.retry)?;
        self.warnings.extend(warning);
        Ok(kept)
    }

    /// `call.resolve_ref`.
    pub fn resolve(&mut self, value: &str) -> Result<String, ToolError> {
        let (resolved, warning) = self.refs.resolve_ref(value, self.doc, self.table, None, self.retry)?;
        self.warnings.extend(warning);
        Ok(resolved)
    }

    /// A change to the whole document (`None`) or a page.
    pub fn change(&mut self, scope: Option<&str>) {
        *self.changed = Some(scope.unwrap_or("").to_owned());
    }
}

/// A toy tool's handler: its summary and data, or its refusal.
pub type Handler = fn(&mut Run<'_>, &Map<String, Value>) -> Result<(String, Value), ToolError>;

/// A toy tool: its name, whether it changes its document, the paths it declares as refs,
/// its handler.
#[derive(Clone, Copy)]
pub struct Tool {
    pub name: &'static str,
    pub mutates: bool,
    pub refs: &'static [&'static str],
    pub run: Handler,
}

/// The format: its documents by id, each one's refs beside it (kept after a close).
#[derive(Default)]
pub struct TestFormat {
    pub docs: BTreeMap<String, TestDoc>,
    pub refs: BTreeMap<String, RefTable>,
}

/// A caller as a ref's owner.
pub fn caller(name: &str) -> Option<RefOwner> {
    Some(Arc::from(name))
}

fn retry_of(tool: &str, arguments: &Value) -> TryCall {
    TryCall {
        tool: tool.to_owned(),
        arguments: arguments.as_object().cloned().unwrap_or_default(),
        fill: Vec::new(),
    }
}

impl TestFormat {
    pub fn open(&mut self, doc: &str) {
        self.docs.insert(doc.to_owned(), TestDoc::new());
    }

    pub fn doc(&self, doc: &str) -> &TestDoc {
        &self.docs[doc]
    }

    pub fn table(&self, doc: &str) -> RefMap {
        self.refs.get(doc).map(|t| t.refs().clone()).unwrap_or_default()
    }

    pub fn owner(&self, doc: &str, name: &str) -> Option<String> {
        self.refs.get(doc)?.owner(name).map(|o| o.to_string())
    }

    /// One call, as the dispatcher runs it.
    pub fn call(&mut self, by: Option<RefOwner>, tool: Tool, arguments: Value) -> Reply {
        let mut refs = CallRefs::new(by.clone());
        let mut reply = Reply::default();
        match self.op(&mut refs, tool, &arguments, &mut reply.warnings) {
            Ok((summary, data, changed)) => {
                let doc = arguments["doc"].as_str().unwrap_or_default().to_owned();
                self.keep(&refs, by.as_ref(), &doc, changed);
                reply.refs = refs.defined(&doc).cloned().unwrap_or_default();
                reply.summary = summary;
                reply.data = data;
            }
            Err(error) => reply.error = Some(error),
        }
        reply
    }

    /// One op: the ownership check, the declared refs, the handler; the change it made is
    /// applied to the document, its refs left in `refs` for the caller to keep.
    fn op(
        &mut self,
        refs: &mut CallRefs,
        tool: Tool,
        arguments: &Value,
        warnings: &mut Vec<String>,
    ) -> Result<(String, Value, Option<String>), ToolError> {
        let doc = arguments["doc"].as_str().unwrap_or_default().to_owned();
        let retry = retry_of(tool.name, arguments);
        let table = self.refs.get(&doc);
        if tool.mutates {
            refs.own_first(arguments, &doc, table, &retry)?;
        }
        let paths: Vec<String> = tool.refs.iter().map(|p| (*p).to_owned()).collect();
        let (arguments, notes) = refs.resolve_declared(arguments, &paths, &doc, table, &retry)?;
        warnings.extend(notes);
        let mark = refs.mark();
        let mut content = self.docs[&doc].content().clone();
        let mut changed = None;
        let mut run = Run {
            refs,
            doc: &doc,
            table,
            content: &mut content,
            changed: &mut changed,
            warnings,
            retry: &retry,
        };
        let answer = (tool.run)(&mut run, arguments.as_object().unwrap_or(&Map::new()));
        let (summary, data) = match answer {
            Ok(found) => found,
            Err(error) => {
                refs.reset(mark);
                return Err(error);
            }
        };
        if let Some(scope) = &changed {
            let scope = (!scope.is_empty()).then_some(scope.as_str());
            let doc_state = self.docs.get_mut(&doc).unwrap();
            doc_state.change(scope, |c| *c = content);
        }
        Ok((summary, data, changed))
    }

    /// The refs a call defined kept, and the version a changed document is at recorded.
    fn keep(&mut self, refs: &CallRefs, by: Option<&RefOwner>, doc: &str, changed: Option<String>) {
        let version = self.docs[doc].version;
        if changed.is_some() || refs.defined(doc).is_some() {
            let defined = refs.defined(doc).cloned().unwrap_or_default();
            self.refs.entry(doc.to_owned()).or_default().keep(&defined, by, version);
        }
    }

    /// A batch of ops on one document as one step: a failed op is skipped, what it defined
    /// forgotten, the refs it would have made named for a later op's refusal.
    pub fn batch(&mut self, by: Option<RefOwner>, ops: &[(Tool, Value)]) -> (Reply, Vec<Value>) {
        let mut refs = CallRefs::new(by.clone());
        let mut reply = Reply::default();
        let mut errors = Vec::new();
        let doc = ops[0].1["doc"].as_str().unwrap_or_default().to_owned();
        let start = self.docs[&doc].version;
        let mut changed = false;
        for (i, (tool, arguments)) in ops.iter().enumerate() {
            let mark = refs.mark();
            let mut warnings = Vec::new();
            match self.op(&mut refs, *tool, arguments, &mut warnings) {
                Ok((_, data, made)) => {
                    changed |= made.is_some();
                    reply
                        .warnings
                        .extend(warnings.into_iter().map(|w| format!("ops[{i}]: {w}")));
                    errors.push(json!({"op": i, "data": data}));
                }
                Err(error) => {
                    refs.reset(mark);
                    refs.failed(i, arguments);
                    errors.push(json!({"op": i, "error": error.to_json()}));
                }
            }
        }
        // one step: the batch's states folded into one
        if changed {
            let doc_state = self.docs.get_mut(&doc).unwrap();
            let content = doc_state.content().clone();
            while doc_state.version != start {
                doc_state.undo();
            }
            doc_state.change(None, |c| *c = content);
        }
        self.keep(&refs, by.as_ref(), &doc, changed.then(String::new));
        reply.refs = refs.defined(&doc).cloned().unwrap_or_default();
        (reply, errors)
    }

    /// Undo (or redo) the whole document's latest step: the refs of the state it returns to.
    pub fn undo(&mut self, doc: &str, redo: bool) -> bool {
        let state = self.docs.get_mut(doc).unwrap();
        let Some(version) = (if redo { state.redo() } else { state.undo() }) else {
            return false;
        };
        self.refs.entry(doc.to_owned()).or_default().restore(version);
        true
    }

    /// A scoped undo (or redo) of page `scope`.
    pub fn undo_scope(&mut self, doc: &str, scope: &str, redo: bool) -> bool {
        let state = self.docs.get_mut(doc).unwrap();
        let Some(step) = (if redo {
            state.redo_scope(scope)
        } else {
            state.undo_scope(scope)
        }) else {
            return false;
        };
        self.refs.entry(doc.to_owned()).or_default().scoped(&step, !redo);
        true
    }
}

fn text(arguments: &Map<String, Value>, name: &str) -> Option<String> {
    arguments.get(name).and_then(Value::as_str).map(str::to_owned)
}

/// `toy_ref_make`: append an item and name it.
pub const MAKE: Tool = Tool {
    name: "toy_ref_make",
    mutates: true,
    refs: &[],
    run: |run, args| {
        run.content.items.push(text(args, "text").unwrap_or_default());
        run.change(None);
        let address = format!("item:{}", run.content.items.len());
        run.define(&text(args, "ref").unwrap_or_default(), &address)?;
        Ok((format!("added {}", text(args, "text").unwrap_or_default()), Value::Null))
    },
};

/// `toy_ref_name`: name an address, changing nothing.
pub const NAME: Tool = Tool {
    name: "toy_ref_name",
    mutates: true,
    refs: &[],
    run: |run, args| {
        let address = text(args, "address").unwrap_or_default();
        let name = text(args, "name").or_else(|| text(args, "ref")).unwrap_or_default();
        run.define(&name, &address)?;
        Ok((format!("named {address}"), Value::Null))
    },
};

/// `toy_ref_after`: append an item saying the target it was given (declared as a ref).
pub const AFTER: Tool = Tool {
    name: "toy_ref_after",
    mutates: true,
    refs: &["target"],
    run: |run, args| {
        let target = text(args, "target").unwrap_or_default();
        run.content.items.push(format!("after {target}"));
        run.change(None);
        if let Some(name) = text(args, "ref") {
            let address = format!("item:{}", run.content.items.len());
            run.define(&name, &address)?;
        }
        Ok((format!("added after {target}"), Value::Null))
    },
};

/// `toy_ref_refuse`: refuse the target it was given.
pub const REFUSE: Tool = Tool {
    name: "toy_ref_refuse",
    mutates: true,
    refs: &["target"],
    run: |_run, args| {
        let target = text(args, "target").unwrap_or_default();
        Err(ToolError::refusal(
            ooxml_tools::Refusal::InvalidArguments,
            format!("{target} is not for this"),
            ooxml_tools::WayOut::names([target.clone()]).unwrap(),
        )
        .at("target"))
    },
};

/// `t_make`: make an item, named `ref` when given; say the target it was given (a ref).
pub const T_MAKE: Tool = Tool {
    name: "t_make",
    mutates: true,
    refs: &["target"],
    run: |run, args| {
        if args.contains_key("nope") {
            return Err(ToolError::refusal(
                ooxml_tools::Refusal::InvalidArguments,
                "nope is no argument",
                ooxml_tools::WayOut::names(["doc"]).unwrap(),
            ));
        }
        run.change(None);
        if let Some(name) = text(args, "ref") {
            run.define(&name, "item:1")?;
        }
        let data = text(args, "target").map_or(Value::Null, |t| json!({"target": t}));
        Ok(("made".to_owned(), data))
    },
};

/// `t_name`: name an address (`ref`, `address`), or resolve a target in the handler.
pub const T_NAME: Tool = Tool {
    name: "t_name",
    mutates: true,
    refs: &[],
    run: |run, args| {
        if let Some(target) = text(args, "target") {
            let resolved = run.resolve(&target)?;
            return Ok(("read".to_owned(), json!({"target": resolved})));
        }
        let address = text(args, "address").unwrap_or_default();
        run.define(&text(args, "ref").unwrap_or_default(), &address)?;
        Ok(("named".to_owned(), Value::Null))
    },
};

/// `t_ref_name`: the name a ref would be kept as, asked before anything is made.
pub const T_REF_NAME: Tool = Tool {
    name: "t_ref_name",
    mutates: false,
    refs: &[],
    run: |run, args| {
        let name = text(args, "ref").unwrap_or_default();
        let kept = run.refs.ref_name(&name, None, run.doc, run.table, run.retry)?;
        Ok(("kept".to_owned(), json!({"kept": kept})))
    },
};

/// `t_add`: add items, each named by its `ref` when given.
pub const T_ADD: Tool = Tool {
    name: "t_add",
    mutates: true,
    refs: &[],
    run: |run, args| {
        let items = args.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        for item in &items {
            let Some(given) = item.get("text").and_then(Value::as_str) else {
                return Err(ToolError::refusal(
                    ooxml_tools::Refusal::InvalidArguments,
                    "items[].text takes text",
                    ooxml_tools::WayOut::names(["text"]).unwrap(),
                ));
            };
            run.content.items.push(given.to_owned());
            run.change(None);
            if let Some(name) = item.get("ref").and_then(Value::as_str) {
                let address = format!("item:{}", run.content.items.len());
                run.define(name, &address)?;
            }
        }
        Ok((format!("Added {} item(s)", items.len()), Value::Null))
    },
};

/// `t_set`: set the text of the item a target names (a ref).
pub const T_SET: Tool = Tool {
    name: "t_set",
    mutates: true,
    refs: &["target"],
    run: |run, args| {
        let target = text(args, "target").unwrap_or_default();
        let Some(n) = target.strip_prefix("item:").and_then(|n| n.parse::<usize>().ok()) else {
            return Err(ToolError::refusal(
                ooxml_tools::Refusal::NotFound,
                format!("no item {target}"),
                ooxml_tools::WayOut::names(["item:1"]).unwrap(),
            ));
        };
        run.content.items[n - 1] = text(args, "text").unwrap_or_default();
        run.change(None);
        Ok(("set".to_owned(), Value::Null))
    },
};

/// `t_half`: name what it would make, then give up half way.
pub const T_HALF: Tool = Tool {
    name: "t_half",
    mutates: true,
    refs: &[],
    run: |run, args| {
        if let Some(name) = text(args, "ref") {
            run.define(&name, "item:9")?;
        }
        Err(ToolError::failure(ooxml_tools::Failure::Unit, "gave up half way"))
    },
};

/// `t_title`: set a page's title (its own undo scope), named `ref` when given.
pub const T_TITLE: Tool = Tool {
    name: "t_title",
    mutates: true,
    refs: &[],
    run: |run, args| {
        let page = text(args, "page").unwrap_or_default();
        run.content
            .titles
            .insert(page.clone(), text(args, "text").unwrap_or_default());
        run.change(Some(&page));
        if let Some(name) = text(args, "ref") {
            run.define(&name, &format!("page:{page}"))?;
        }
        Ok(("titled".to_owned(), Value::Null))
    },
};
