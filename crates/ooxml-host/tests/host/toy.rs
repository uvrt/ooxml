//! `Toy`: a made-up format for the host's tests (design phase 3, section 7.4: the generic
//! host tests run against a toy format, as ooxml-edit's run against `tools_toys`). A toy
//! document is a package whose parts hold text; its tools read a part (`toy_read`), write
//! one (`toy_set`, defining a ref to it and taking a retry key) and list or change its notes
//! (`toy_notes`, whose `list` a reviewer may call). Its format declares its tools' host
//! facts, a hints table and a withdrawn tool, as a real format does.
//!
//! Every name and figure here is made up.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_base::context::{FaceData, FaceRequest, Timestamp};
use ooxml_base::{BaseError, CacheBudget, Cancel, Clock, Context, FontSource, PartStore};
use ooxml_host::{CallAs, Host, HostSession, HostSettings, NotYet};
use ooxml_opc::Package;
use ooxml_session::{
    AnyVersion, DocKind, Finished, Format, FormatDoc, FormatRun, OpenEnv, PackageDoc, PartUpdate, Registry,
    SessionConfig, SessionError, StoreClock, Stored, StoredDoc,
};
use ooxml_store::{Caller, Store, StoreConfig};
use ooxml_tools::roles::{HostFacts, Needs, Role};
use ooxml_tools::{
    ArgEnum, CallEnv, Done, FormatCall, FormatTypes, Refusal, Registrar, Run, Tool, ToolArgs, ToolError, ToolResult,
    WayOut,
};
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Map, Value};
use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

/// The toy format's types.
pub struct Toy;

/// What a toy reading tool reads: the version's package.
pub struct ToyVersion<'a> {
    pub package: &'a Package,
}

/// What a toy editing tool edits through: a part written as one step.
pub struct ToyEditing<'a> {
    write: &'a dyn Fn(&str, &str) -> Result<(), ToolError>,
}

impl FormatTypes for Toy {
    type Version<'a> = ToyVersion<'a>;
    type Editing<'a> = ToyEditing<'a>;
    type Tracked = ();
}

/// `toy_read`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "toy_read", description = "Read a part's text.")]
pub struct ToyRead {
    /// Document id.
    pub doc: String,
    /// The part, or a $ref to one.
    #[arg(refs)]
    pub target: String,
}

/// `toy_set`'s arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "toy_set", description = "Write a part's text.", mutates)]
pub struct ToySet {
    /// Document id.
    pub doc: String,
    /// The part.
    #[arg(alias = "where")]
    pub target: String,
    /// The text.
    pub text: String,
    /// A ref to define for the part.
    #[arg(rename = "ref", alias = "name")]
    pub reference: Option<String>,
    /// A retry key.
    pub key: Option<String>,
}

/// `toy_notes`' actions.
#[derive(ArgEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesAction {
    /// List them.
    List,
    /// Add one.
    Add,
    /// Delete one.
    Delete,
}

/// `toy_notes`' arguments.
#[derive(ToolArgs, Debug, Clone, PartialEq, Eq)]
#[tool(name = "toy_notes", description = "List, add or delete notes.", mutates)]
pub struct ToyNotes {
    /// Document id.
    pub doc: String,
    /// What to do.
    pub action: NotesAction,
    /// A note's text.
    pub text: Option<String>,
}

fn part(target: &str) -> String {
    format!("toy/{target}.xml")
}

fn toy_read(args: &ToyRead, version: &ToyVersion<'_>, _cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    let Some(bytes) = version.package.read(&part(&args.target)).ok().flatten() else {
        return Err(ToolError::refusal(
            Refusal::NotFound,
            format!("no part {}", args.target),
            WayOut::names(version.package.part_names()).unwrap_or_else(|| WayOut::one(json!("toy/a.xml"))),
        )
        .at("target"));
    };
    let mut done = Done::new(format!("Read {}", args.target));
    done.data = Some(Value::from(String::from_utf8_lossy(&bytes).into_owned()));
    Ok(done)
}

fn toy_set(args: &ToySet, editing: &ToyEditing<'_>) -> ToolResult<Value> {
    if let Err(error) = (editing.write)(&part(&args.target), &format!("<t>{}</t>", args.text)) {
        return ToolResult::failed(error);
    }
    let mut done = Done::new(format!("Set {}", args.target));
    done.changed.push(args.target.clone());
    if let Some(name) = &args.reference {
        done.refs.push((name.clone(), args.target.clone()));
    }
    ToolResult::done(done)
}

fn toy_notes(args: &ToyNotes, editing: &ToyEditing<'_>) -> ToolResult<Value> {
    match args.action {
        NotesAction::List => ToolResult::done(Done::new("0 notes")),
        NotesAction::Add | NotesAction::Delete => {
            let text = args.text.clone().unwrap_or_default();
            if let Err(error) = (editing.write)(&part("notes"), &format!("<n>{text}</n>")) {
                return ToolResult::failed(error);
            }
            ToolResult::done(Done::new("Notes changed"))
        }
    }
}

fn listing(arguments: &Map<String, Value>) -> bool {
    arguments.get("action").and_then(Value::as_str) == Some("list")
}

/// The toy format.
pub struct ToyFormat;

impl Format for ToyFormat {
    fn kind(&self) -> DocKind {
        DocKind::Pptx
    }

    fn register(&self, tools: &mut Registry) {
        let mut r = tools.registrar(self.kind());
        Registrar::<Toy>::read::<ToyRead>(&mut r, toy_read);
        Registrar::<Toy>::edit::<ToySet>(&mut r, toy_set);
        Registrar::<Toy>::edit::<ToyNotes>(&mut r, toy_notes);
    }

    fn host_facts(&self) -> Vec<(String, HostFacts)> {
        vec![(
            "toy_notes".into(),
            HostFacts {
                action_roles: vec![("list".into(), Needs::Role(Role::Review))],
                reads: Some(listing),
                ..HostFacts::default()
            },
        )]
    }

    fn tool_hints(&self) -> Vec<(String, String)> {
        vec![("toy_add_comment".into(), "toy_notes add".into())]
    }

    fn withdrawn(&self) -> Vec<(String, String)> {
        vec![("toy_draw".into(), "write the text with toy_set instead.".into())]
    }

    fn open(&self, package: Package, env: &OpenEnv<'_>) -> Result<Arc<dyn FormatDoc>, SessionError> {
        let clock = Arc::new(StoreClock(Arc::clone(&env.context.clock)));
        Ok(Arc::new(ToyDoc(Stored::new(Store::new(
            PackageDoc::new(package),
            env.store,
            clock,
        )))))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A toy document: its package's versions.
pub struct ToyDoc(Stored<PackageDoc>);

/// What a toy edit made.
struct Made {
    first: u64,
    before: u64,
    last: AnyVersion,
}

/// A toy call's format context.
struct ToyCall<'a> {
    doc: &'a ToyDoc,
    run: &'a FormatRun<'a>,
}

impl FormatCall for ToyCall<'_> {
    fn run(&self, tool: &str, _arguments: &Map<String, Value>, run: &dyn Any) -> ToolResult<Value> {
        let Some(run) = run.downcast_ref::<Run<Toy>>() else {
            return ToolResult::failed(ToolError::internal(format!("{tool} is no toy tool"), "toy"));
        };
        match run {
            Run::Read(read) => {
                let version = ToyVersion {
                    package: self.run.doc.package(),
                };
                match read(&version, self.run.cancel) {
                    Ok(done) => ToolResult::done(done),
                    Err(error) => ToolResult::failed(error),
                }
            }
            Run::Edit(edit) => {
                let write = |name: &str, text: &str| -> Result<(), ToolError> {
                    let before = self.run.doc.number();
                    let bytes: Arc<[u8]> = text.as_bytes().into();
                    let update = PartUpdate::new(
                        vec![(name.to_owned(), Some(bytes))],
                        ooxml_base::Limits::default(),
                        self.run.cancel,
                    );
                    let (version, _, _, _) =
                        FormatDoc::update(&self.doc.0, &self.run.context.caller, Some(self.run.tool), &update)
                            .map_err(|e| ToolError::internal(e.to_string(), "toy:update"))?;
                    *self.run.made.lock().unwrap() = Some(Box::new(Made {
                        first: version.seq(),
                        before,
                        last: version,
                    }));
                    Ok(())
                };
                edit(&ToyEditing { write: &write })
            }
            Run::Tracking(_) => ToolResult::failed(ToolError::internal("no tracking in toys", "toy")),
        }
    }
}

impl FormatDoc for ToyDoc {
    fn stored(&self) -> &dyn StoredDoc {
        &self.0
    }

    fn save(&self, at: &AnyVersion, checked: bool) -> Result<Vec<u8>, SessionError> {
        FormatDoc::save(&self.0, at, checked)
    }

    fn update(
        &self,
        caller: &Caller,
        tool: Option<&str>,
        update: &PartUpdate<'_>,
    ) -> Result<(AnyVersion, bool, u32, bool), SessionError> {
        FormatDoc::update(&self.0, caller, tool, update)
    }

    fn readings(&self) -> Arc<dyn Any + Send + Sync> {
        FormatDoc::readings(&self.0)
    }

    fn run(&self, tool: &dyn Tool, run: &FormatRun<'_>) -> ToolResult<Value> {
        let call = ToyCall { doc: self, run };
        let mut effects = ooxml_tools::OpEffects::default();
        let mut env = CallEnv {
            cancel: run.cancel,
            format: &call,
            effects: &mut effects,
        };
        let mut result = tool.run(run.arguments, &mut env);
        effects.give_to(&mut result);
        result
    }

    fn finish(&self, made: Box<dyn Any + Send>, _run: &FormatRun<'_>) -> Option<Finished> {
        let made = made.downcast::<Made>().ok()?;
        Some(Finished {
            first: made.first,
            before: made.before,
            last: made.last,
            batch: false,
            format: None,
        })
    }

    fn checks(&self, at: &AnyVersion, _made: Option<&(dyn Any + Send + Sync)>) -> Option<Map<String, Value>> {
        let mut checks = Map::new();
        checks.insert("parts".into(), Value::from(at.package().part_names().len()));
        Some(checks)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct Fixed;

impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp {
            seconds: 1_767_603_600,
            nanos: 0,
        }
    }
    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

struct NoFonts;

impl FontSource for NoFonts {
    fn face(&self, _: &FaceRequest) -> Result<Option<FaceData>, BaseError> {
        Ok(None)
    }
}

struct NoParts;

impl PartStore for NoParts {
    fn load(&self, _: &str) -> Result<Option<Arc<[u8]>>, BaseError> {
        Ok(None)
    }
    fn store(&self, _: &str, _: Arc<[u8]>) -> Result<(), BaseError> {
        Ok(())
    }
}

struct Unlimited;

impl CacheBudget for Unlimited {
    fn reserve(&self, _: usize) -> bool {
        true
    }
    fn release(&self, _: usize) {}
    fn used(&self) -> usize {
        0
    }
}

struct Never;

impl Cancel for Never {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// A context with a fixed clock and no fonts.
fn context() -> Context {
    Context {
        fonts: Arc::new(NoFonts),
        parts: Arc::new(NoParts),
        clock: Arc::new(Fixed),
        budget: Arc::new(Unlimited),
        cancel: Arc::new(Never),
        limits: ooxml_base::Limits::default(),
        locale: ooxml_base::Locale::default(),
    }
}

/// An empty toy package's bytes.
pub fn package() -> Arc<[u8]> {
    let mut writer = Writer::new();
    let entry = NewEntry {
        name: "[Content_Types].xml".into(),
        method: Method::Deflated,
        dos_time: 0,
        dos_date: 33,
        external_attr: 0,
        create_system: 3,
    };
    writer.add(&entry, b"<Types/>").unwrap();
    writer.finish(b"").unwrap().into()
}

/// A host of the toy format with `settings`, and a session of it with one toy document open
/// (`d1`).
pub fn session_with(settings: HostSettings) -> HostSession {
    let host = Arc::new(Host::new(vec![Arc::new(ToyFormat)], settings, Arc::new(NotYet)));
    let session = host.session(
        SessionConfig {
            store: StoreConfig::default(),
            locale: None,
        },
        context(),
    );
    session.open(package(), DocKind::Pptx, "deck.pptx").unwrap();
    session
}

/// [`session_with`] the default settings.
pub fn session() -> HostSession {
    session_with(HostSettings::default())
}

/// A call's reply as the model reads it.
pub fn call(session: &HostSession, tool: &str, arguments: Value) -> Value {
    call_as(session, tool, arguments, &CallAs::default())
}

/// [`call`] with roles and a context.
pub fn call_as(session: &HostSession, tool: &str, arguments: Value, how: &CallAs) -> Value {
    let result = session.call(tool, arguments, how).unwrap();
    Value::Object(result.to_json())
}
