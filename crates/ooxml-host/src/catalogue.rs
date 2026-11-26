//! Owns: the host's table of tools ([`Catalogue`]): every tool a call may name, in the
//! host's order -- the formats' tools and the host's own ([`Own`]) -- each with its spec and
//! what the host holds of it beside ([`HostFacts`]: the roles of its actions, the calls of a
//! changing tool that only read); how a name a model writes is read (ooxml-edit's
//! `Toolbox.tool_named`: a namespace stripped, another spelling of one tool read as it with
//! a note, a withdrawn tool kept for its refusal); the refusal of a name that is no tool
//! ([`Catalogue::no_tool`]: withdrawn, with its way out; else the tools and actions it most
//! likely means, of the call's roles); and the role checks before and after a call's
//! arguments ([`Catalogue::check_role`]), in today's words.
//!
//! Must not: run a tool, or know a format: its tools, hints and withdrawn names come from
//! the [`ooxml_session::Format`]s the host is made with.

use crate::hints::{action_hints, tool_alias, tool_hints, Hint, Named};
use ooxml_session::DocKind;
use ooxml_tools::pyfmt::repr_str;
use ooxml_tools::roles::{HostFacts, Needs, Role, RoleSet};
use ooxml_tools::schema::Kind;
use ooxml_tools::{close_matches, Refusal, ToolError, ToolSpec, TryCall, WayOut};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// A tool the host runs itself (the session's own tools: ooxml-edit's `SESSION_TOOLS`,
/// `new_document` and `read_batch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Own {
    /// Open an input blob as a document.
    OpenDocument,
    /// Make a new document (by its format).
    NewDocument,
    /// Close a document.
    CloseDocument,
    /// Read a text blob a page at a time.
    ReadBlob,
    /// Undo or redo a document's latest steps (the session's).
    Undo,
    /// Changing calls as one step (design section 4: lane D2).
    Batch,
    /// Reading calls, each answered as alone.
    ReadBatch,
}

/// One tool of the catalogue.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Its definition.
    pub spec: ToolSpec,
    /// What the host holds of it beside.
    pub facts: HostFacts,
    /// The kinds whose format registered it (none for the host's own).
    pub kinds: Vec<DocKind>,
    /// The host's own tool it is, if it is one.
    pub own: Option<Own>,
    /// The values of its `action` enum.
    pub actions: Vec<String>,
}

impl Entry {
    /// A tool of `spec`.
    #[must_use]
    pub fn new(spec: ToolSpec, facts: HostFacts, own: Option<Own>) -> Entry {
        let actions = spec
            .arguments
            .property("action")
            .and_then(|p| match &p.schema.kind {
                Kind::String(node) => Some(node.values.clone()),
                _ => None,
            })
            .unwrap_or_default();
        Entry {
            spec,
            facts,
            kinds: Vec::new(),
            own,
            actions,
        }
    }

    /// Its name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.spec.name
    }

    /// The role of a call whose action names none of its own: the declared one, else `edit`
    /// for a changing tool and `review` for a reading one.
    #[must_use]
    pub fn base_role(&self) -> Needs {
        match self.spec.role.as_deref().and_then(Needs::parse) {
            Some(needs) => needs,
            None if self.spec.flags.mutates => Needs::Role(Role::Edit),
            None => Needs::Role(Role::Review),
        }
    }

    fn action_role(&self, action: &str) -> Needs {
        self.facts.action_role(action).unwrap_or_else(|| self.base_role())
    }

    /// The roles whose toolsets hold it: every role one of its actions needs (all for `any`).
    #[must_use]
    pub fn roles(&self) -> RoleSet {
        let needed: Vec<Needs> = if self.actions.is_empty() {
            vec![self.base_role()]
        } else {
            self.actions.iter().map(|a| self.action_role(a)).collect()
        };
        if needed.contains(&Needs::Any) {
            return RoleSet::all();
        }
        RoleSet::of(needed.into_iter().filter_map(|n| match n {
            Needs::Role(role) => Some(role),
            Needs::Any => None,
        }))
        .unwrap_or_default()
    }

    /// The role a call with `arguments` needs.
    #[must_use]
    pub fn role_of(&self, arguments: &Map<String, Value>) -> Needs {
        if let Some(needed) = self.facts.call_role.and_then(|role| role(arguments)) {
            return needed;
        }
        match arguments.get("action").and_then(Value::as_str) {
            Some(action) => self.facts.action_role(action).unwrap_or_else(|| self.base_role()),
            None => self.base_role(),
        }
    }

    /// The actions `roles` may call (none for a tool without `action`).
    #[must_use]
    pub fn actions_for(&self, roles: RoleSet) -> Vec<String> {
        self.actions
            .iter()
            .filter(|a| roles.allows(self.action_role(a)))
            .cloned()
            .collect()
    }

    /// Whether a call with `arguments` changes its document: a changing tool, unless its
    /// facts say this call only reads.
    #[must_use]
    pub fn changes(&self, arguments: &Map<String, Value>) -> bool {
        self.spec.flags.mutates && !self.facts.reads.is_some_and(|reads| reads(arguments))
    }

    /// Whether it may run as an op of `within` (`batch` or `read_batch`).
    #[must_use]
    pub fn runs_in(&self, within: &str) -> bool {
        if matches!(self.own, Some(Own::Batch | Own::ReadBatch)) {
            return false;
        }
        self.spec.flags.batchable || (within == "read_batch" && self.spec.flags.read_batchable)
    }

    /// Whether some call of it only reads, and may run in a `read_batch`.
    #[must_use]
    pub fn reads_some(&self) -> bool {
        self.runs_in("read_batch") && (!self.spec.flags.mutates || self.facts.reads.is_some())
    }
}

/// The host's tools, in its order, with the formats' hints and withdrawn names.
#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    entries: Vec<Entry>,
    /// A withdrawn tool's name and what to call instead.
    withdrawn: Vec<(String, String)>,
    /// The formats' hints table: a name, and the tool (and action) it means.
    hints: Vec<(String, String)>,
}

impl Catalogue {
    /// A catalogue of `entries` (in the host's order), `withdrawn` and the `hints` table.
    #[must_use]
    pub fn new(entries: Vec<Entry>, withdrawn: Vec<(String, String)>, hints: Vec<(String, String)>) -> Catalogue {
        Catalogue {
            entries,
            withdrawn,
            hints,
        }
    }

    /// Every tool, in order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The tool `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name() == name)
    }

    /// What `name` was withdrawn for, if it was.
    #[must_use]
    pub fn withdrawn(&self, name: &str) -> Option<&str> {
        self.withdrawn.iter().find(|(n, _)| n == name).map(|(_, w)| w.as_str())
    }

    fn named(&self) -> Vec<Named<'_>> {
        self.entries
            .iter()
            .map(|e| Named {
                name: e.name(),
                actions: &e.actions,
            })
            .collect()
    }

    /// The tool name `name` stands for: itself; the name after the last dot (a namespace a
    /// provider shows a model); with `aliases`, the one tool whose name says the same in
    /// other words. `notes` gets a line for each name read as another; a name that is none
    /// of these is returned as it is.
    #[must_use]
    pub fn tool_named(&self, name: &str, notes: &mut Vec<String>, aliases: bool) -> String {
        if self.get(name).is_some() {
            return name.to_owned();
        }
        let bare = name.rsplit_once('.').map_or(name, |(_, b)| b);
        if self.withdrawn(bare).is_some() {
            return bare.to_owned();
        }
        let meant = if self.get(bare).is_some() {
            Some(bare.to_owned())
        } else if aliases {
            tool_alias(bare, &self.named(), &self.hints)
        } else {
            None
        };
        let Some(meant) = meant else {
            return name.to_owned();
        };
        notes.push(format!("tool {} read as {}", repr_str(name), repr_str(&meant)));
        meant
    }

    /// The role a hint's call needs that `roles` lack; `None` when they have it.
    fn hint_role(&self, hint: &Hint, roles: RoleSet) -> Option<Needs> {
        let tool = self.get(&hint.tool)?;
        match &hint.action {
            None => {
                if tool.roles().and(roles).is_some() {
                    None
                } else {
                    Some(Needs::Role(first_needed(tool.roles())))
                }
            }
            Some(action) => {
                let needed = tool.action_role(action);
                (!roles.allows(needed)).then_some(needed)
            }
        }
    }

    /// The refusal of `name`, which is no tool: `refused` for a withdrawn one (its way out);
    /// else `invalid_arguments` naming what it most likely means, of `roles`' tools. In a
    /// batch (`within`: `batch` or `read_batch`) the refusal is at `tool`, and a hint the
    /// batch cannot run is to be called alone.
    #[must_use]
    pub fn no_tool(&self, name: &str, roles: RoleSet, batch: bool, within: &str, retry: &TryCall) -> ToolError {
        let bare = name.rsplit_once('.').map_or(name, |(_, b)| b);
        if let Some(instead) = self.withdrawn(bare) {
            let error = ToolError::refusal(
                Refusal::Refused,
                format!("{bare} is withdrawn until the Rust session has it; {instead}"),
                WayOut::call(retry.clone()),
            )
            .with_detail("not_supported", Value::from(format!("{bare} (withdrawn)")));
            return if batch { error.at("tool") } else { error };
        }
        let tools: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|t| t.roles().and(roles).is_some() && (within != "read_batch" || t.reads_some()))
            .collect();
        let found = tool_hints(name, &self.named(), &self.hints, &[]);
        let usable: Vec<&Hint> = found.iter().filter(|h| self.hint_role(h, roles).is_none()).collect();
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        let near = close_matches(bare, &names, 5, 0.5);
        let mut message = format!("no tool {}", repr_str(name));
        let all: Vec<&str> = self.entries.iter().map(Entry::name).collect();
        if !usable.is_empty() {
            let said: Vec<String> = usable
                .iter()
                .map(|hint| {
                    let runs = self.get(&hint.tool).is_some_and(|t| t.runs_in(within));
                    if batch && !runs {
                        format!("{hint} on its own ({within} cannot run it)")
                    } else {
                        hint.to_string()
                    }
                })
                .collect();
            message.push_str(&format!(": use {}", said.join(", or ")));
        } else if let Some(first) = found.first() {
            let needed = self.hint_role(first, roles).map_or("", Needs::as_str);
            message.push_str(&format!(
                ": {first} does that, and needs the {needed} role; this session has {}",
                roles.listed()
            ));
        } else if batch && close_matches(bare, &all, 1, 0.8).is_empty() {
            message.push_str(&format!(
                " that {within} can run: it runs only the tools in valid_options; call {name} on its own"
            ));
        } else if !near.is_empty() {
            message.push_str(&format!(
                "; the nearest: {}",
                near.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        let mut options = hinted_options(&found, &usable, &near);
        options.truncate(5);
        if options.is_empty() && found.is_empty() {
            options = tools
                .iter()
                .filter(|t| !batch || t.runs_in(within))
                .map(|t| t.name().to_owned())
                .collect();
            options.sort();
        }
        let way_out = WayOut::names(options).unwrap_or_else(|| WayOut::call(retry.clone()));
        let error = ToolError::refusal(Refusal::InvalidArguments, message, way_out);
        if batch {
            error.at("tool")
        } else {
            error
        }
    }

    /// The message for an `action` `tool` lacks: the action of its own it means with a verb
    /// read, else the other tools that have it, of `roles`; `message` otherwise.
    #[must_use]
    pub fn action_message(&self, tool: &Entry, value: &str, message: &str, roles: RoleSet) -> String {
        let named = Named {
            name: tool.name(),
            actions: &tool.actions,
        };
        let (own, found) = action_hints(named, value, &self.named(), &self.hints);
        let base = message.split("; did you mean").next().unwrap_or(message);
        if let Some(own) = own {
            return format!("{base}; did you mean {}?", repr_str(&own));
        }
        let usable: Vec<String> = found
            .iter()
            .filter(|h| self.hint_role(h, roles).is_none())
            .map(ToString::to_string)
            .collect();
        if usable.is_empty() {
            return message.to_owned();
        }
        format!("{base}; use {}", usable.join(", or "))
    }

    /// Refuse a call outside `roles`: with `arguments` `None`, a tool in none of the roles'
    /// toolsets; else the call's own role. `field`: where a batch's op names it.
    ///
    /// # Errors
    /// `refused`, naming the role it needs and what the session has, with the actions the
    /// roles may call.
    pub fn check_role(
        tool: &Entry,
        arguments: Option<&Map<String, Value>>,
        roles: RoleSet,
        field: Option<&str>,
        retry: &TryCall,
    ) -> Result<(), ToolError> {
        let (needed, action) = match arguments {
            None => {
                if tool.roles().and(roles).is_some() {
                    return Ok(());
                }
                (first_needed(tool.roles()), None)
            }
            Some(arguments) => match tool.role_of(arguments) {
                Needs::Any => return Ok(()),
                Needs::Role(role) if roles.has(role) => return Ok(()),
                Needs::Role(role) => (role, arguments.get("action").and_then(Value::as_str)),
            },
        };
        let what = if arguments.is_some_and(|a| tool.facts.call_role.is_some_and(|r| r(a).is_some())) {
            format!("this {} call", tool.name())
        } else if let (Some(action), false) = (action, tool.actions.is_empty()) {
            format!("{} {action}", tool.name())
        } else {
            tool.name().to_owned()
        };
        let mut message = format!("{what} needs the {needed} role; this session has {}. ", roles.listed());
        message.push_str(match needed {
            Role::Edit => "Do not make the change: report it with the call that would make it (tool and arguments).",
            Role::Resolve => "Leave the thread as it is and say it should be resolved.",
            Role::Accept => "Leave the tracked changes as they are and say which should be accepted or rejected.",
            Role::Review => "Reading needs the review role.",
        });
        let allowed = if action.is_some() {
            tool.actions_for(roles)
        } else {
            Vec::new()
        };
        let way_out = WayOut::names(allowed).unwrap_or_else(|| WayOut::call(retry.clone()));
        let at = field.map(str::to_owned).or_else(|| action.map(|_| "action".to_owned()));
        let error = ToolError::refusal(Refusal::Refused, message, way_out)
            .with_detail("role", Value::from(needed.as_str()))
            .with_detail(
                "roles",
                json!(roles.roles().iter().map(|r| r.as_str()).collect::<Vec<_>>()),
            );
        Err(match at {
            Some(at) => error.at(at),
            None => error,
        })
    }

    /// The tools' names by kind, for a refusal of a tool that does not serve a kind.
    #[must_use]
    pub fn serving(&self) -> BTreeMap<String, Vec<DocKind>> {
        self.entries
            .iter()
            .map(|e| (e.name().to_owned(), e.kinds.clone()))
            .collect()
    }
}

/// The role named first of `roles` as today's `sorted(tool.roles, key=(role != EDIT, role))`
/// picks it: `edit` where it is one, else the first by name.
fn first_needed(roles: RoleSet) -> Role {
    if roles.has(Role::Edit) {
        return Role::Edit;
    }
    let mut names = roles.roles();
    names.sort_by_key(|r| r.as_str());
    names.first().copied().unwrap_or(Role::Edit)
}

/// The options of a refusal of a name that is no tool: the usable hints' tools, then the
/// names close to it (not those a hint names that the roles bar), each once, in order.
fn hinted_options(found: &[Hint], usable: &[&Hint], near: &[String]) -> Vec<String> {
    let barred: Vec<&str> = found
        .iter()
        .map(|h| h.tool.as_str())
        .filter(|t| !usable.iter().any(|u| u.tool == *t))
        .collect();
    let mut options: Vec<String> = Vec::new();
    for option in usable
        .iter()
        .map(|h| h.tool.clone())
        .chain(near.iter().filter(|n| !barred.contains(&n.as_str())).cloned())
    {
        if !options.contains(&option) {
            options.push(option);
        }
    }
    options
}
