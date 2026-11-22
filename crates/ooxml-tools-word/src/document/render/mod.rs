//! Owns: `render` of a Word document (docx-agent's `render` in `tools/shared.py`): its
//! definition; the pages asked (`pages`, or `slides` read as pages with a warning; the
//! pages a `range`'s blocks fall on, `range` reporting each range's blocks and pages), each
//! its own image or tiled into one labelled sheet ([`sheet`]); the reply's `pages`,
//! `cached` (the pages the store held), `coverage` and `relaid`; refusals with their way
//! out. The images travel as the call's files (`page 3`, `sheet of pages 1-12`, PNG); the
//! host turns each into an image of the reply. `fast` draws exactly: the exact image costs
//! less in Rust than today's approximate one.
//!
//! Must not: lay out but through the host's engine, or keep images but in the session's
//! store ([`PageImages`]).

mod draw;
mod sheet;
mod store;

pub use draw::{laid_to_draw, Drawing};
pub use store::{page_key, sheet_key, ImageKey, PageImages, MAX_IMAGES};

use crate::check::MAX_RANGES;
use crate::version::Version;
use ooxml_base::Cancel;
use ooxml_tools::{Done, Failure, File, IntOrStr, Refusal, ToolArgs, ToolError, WayOut};
use ooxml_word_pages::layout_facts::{Scope, ScopeError};
use ooxml_word_pages::LaidVersion;
use serde_json::{json, Map, Value};

/// A page's width when the call gives none.
pub const DEFAULT_WIDTH: i64 = 1000;
/// What the summary says of the view drawn.
const FINAL_VIEW: &str = "(final view: changes accepted, comments hidden)";

/// What `unported` says of a call on a document the session does not own.
pub const RENDER_NOT_OWNED: &str = "render: not owned";
/// What `unported` says of a call the host handed no page engine or raster.
pub const RENDER_NO_LAYOUT: &str = "render: no layout";

/// `render`'s sheet: the pages tiled into one labelled image.
#[allow(missing_docs)]
#[derive(ToolArgs, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderSheet {
    #[arg(minimum = 1, maximum = 8)]
    pub columns: Option<i64>,
    /// Each tile's px.
    #[arg(minimum = 160, maximum = 1280)]
    pub width: Option<i64>,
}

/// `render`'s arguments, shared by every format.
#[derive(ToolArgs, Debug, Clone, PartialEq)]
#[tool(name = "render", description = include_str!("render.md"), batchable = false, read_batchable)]
pub struct Render {
    /// Document id.
    pub doc: String,
    /// Decks: slide numbers from 1, or ids (s:256, $ref); layouts L2, masters M0.
    #[arg(kinds("pptx"), refs)]
    pub slides: Option<Vec<IntOrStr<1>>>,
    /// Documents: page numbers from 1.
    #[arg(kinds("docx"), minimum = 1)]
    pub pages: Option<Vec<i64>>,
    /// Pixels, 200-2576. Default 1280 (slides), 1000 (pages).
    #[arg(minimum = 200, maximum = 2576)]
    pub width: Option<i64>,
    /// Up to 30 in one labelled image; {} for defaults.
    pub sheet: Option<RenderSheet>,
    /// Documents: blocks, spans p:A..t:B or sections s:2; their facts, pages.
    #[arg(kinds("docx"), single_as_list)]
    pub range: Option<Vec<String>>,
    /// Quicker image while building: same layout, approximate pixels; the result says so.
    /// Default false.
    #[arg(kinds("docx"))]
    pub fast: Option<bool>,
}

fn unported(why: &str) -> Done<Value> {
    let mut done = Done::new("");
    done.unported.push(why.to_owned());
    done
}

/// `render` of a Word document at one version.
///
/// # Errors
/// `limit`, `not_found`, `invalid_arguments` with their way out (the pages there are, the
/// pages a range is on, the bounds); `refused` for a document the engine does not lay out
/// yet; `timeout` past the call's layout limit.
pub fn render(args: &Render, version: &Version<'_>, cancel: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    let Some(owned) = version.owned else {
        return Ok(unported(RENDER_NOT_OWNED));
    };
    let (Some(layout), Some(drawing)) = (owned.layout, owned.drawing) else {
        return Ok(unported(RENDER_NO_LAYOUT));
    };
    let mut warnings: Vec<String> = Vec::new();
    let asked = pages_of(args, &mut warnings)?;
    let ranges: Vec<String> = args
        .range
        .iter()
        .flatten()
        .filter(|r| !r.trim().is_empty())
        .cloned()
        .collect();
    if ranges.len() > MAX_RANGES {
        return Err(ToolError::failure(
            Failure::Limit,
            format!(
                "{} ranges: at most {MAX_RANGES} per render; a span p:A..t:B covers many blocks",
                ranges.len()
            ),
        )
        .at("range"));
    }
    let call = Drawer {
        layout,
        laying: Laying {
            owned,
            package: version.package,
            cancel,
            laid: std::cell::OnceCell::new(),
        },
        drawing,
        limits: version.limits,
        doc: &args.doc,
    };
    let mut sheet = args.sheet;
    let (pages, scoped) = if ranges.is_empty() {
        (asked, None)
    } else {
        let (on, scoped) = range_pages(&call, &ranges, asked.as_deref())?;
        let per_call = version.limits.images_per_call;
        if sheet.is_none() && on.len() > per_call {
            warnings.push(format!(
                "the range is on {} pages, more than {per_call} images a call: drawn as one sheet",
                on.len()
            ));
            sheet = Some(RenderSheet {
                columns: None,
                width: None,
            });
        }
        (Some(on), Some(scoped))
    };
    let mut done = match sheet {
        Some(sheet) => sheet::render_sheet(&call, pages, args.width, sheet, &mut warnings)?,
        None => render_pages(&call, pages, args.width, &mut warnings)?,
    };
    if let (Some(scoped), Some(Value::Object(data))) = (scoped, done.data.as_mut()) {
        data.insert("range".into(), scoped);
    }
    done.warnings.extend(warnings);
    Ok(done)
}

/// What a call draws with: the engine, the version to lay out (laid out when first
/// needed, so a refusal the arguments alone decide costs no layout), the raster and store,
/// the host's limits on images.
pub(super) struct Drawer<'a> {
    layout: &'a dyn crate::WordLayout,
    laying: Laying<'a>,
    drawing: Drawing<'a>,
    limits: ooxml_tools::Limits,
    doc: &'a str,
}

/// The version a call lays out, and its layout once made.
struct Laying<'a> {
    owned: crate::Owned<'a>,
    package: &'a ooxml_opc::Package,
    cancel: &'a dyn Cancel,
    laid: std::cell::OnceCell<std::sync::Arc<LaidVersion>>,
}

impl Drawer<'_> {
    /// The version laid out by the host's engine, at the first call that needs it.
    ///
    /// # Errors
    /// [`draw::laid`]'s: `refused` for a document the engine does not lay out yet,
    /// `timeout` past the call's layout limit.
    fn laid(&self) -> Result<&LaidVersion, ToolError> {
        let laying = &self.laying;
        if let Some(laid) = laying.laid.get() {
            return Ok(laid);
        }
        let laid = draw::laid(self.layout, &laying.owned, laying.package, self.doc, laying.cancel)?;
        Ok(laying.laid.get_or_init(|| laid))
    }

    /// The height over the width of the shortest page the version's sections declare
    /// (`w:pgSz`, read without laying out: docx-agent's `_shortest_ratio`), or `None` where
    /// a section declares no size or a side of 0.
    ///
    /// # Errors
    /// `internal` where the version cannot be read.
    fn shortest_declared(&self) -> Result<Option<f64>, ToolError> {
        let docx = self
            .laying
            .owned
            .doc
            .docx()
            .map_err(|e| ToolError::internal(format!("the document could not be read: {e}"), "render.sheet"))?;
        let mut shortest: Option<f64> = None;
        for section in ooxml_word_model::docx::sections::sections(&docx) {
            match section.page() {
                Some((width, height)) if width != 0.0 && height != 0.0 => {
                    let ratio = height / width;
                    shortest = Some(shortest.map_or(ratio, |s| s.min(ratio)));
                }
                _ => return Ok(None),
            }
        }
        Ok(shortest)
    }

    /// `count` images more refused, before anything is drawn, where the call's limit or
    /// the host's budgets have not room for them (docx-agent's `call.check_images`).
    fn check_images(&self, count: usize) -> Result<(), ToolError> {
        self.limits.check_images(count, self.drawing.budgets)
    }

    /// `not_found` for pages past the version's last (`count` pages), naming the pages there
    /// are.
    fn past_the_end(&self, count: usize) -> ToolError {
        let options: Vec<Value> = (1..=count.min(50)).map(Value::from).collect();
        ToolError::refusal(
            Refusal::NotFound,
            format!("the document has {count} page(s)"),
            WayOut::options(options).unwrap_or_else(|| WayOut::one(json!({"doc": self.doc}))),
        )
        .at("pages")
    }

    /// The reply's `coverage` and `relaid` of `laid`, into `data`.
    fn facts(&self, laid: &LaidVersion, data: &mut Map<String, Value>) {
        let coverage = self
            .layout
            .engine()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .coverage(laid);
        if let Some(coverage) = coverage {
            data.insert(
                "coverage".into(),
                crate::coverage::coverage_facts(&coverage, |path| laid.id_of(path).map(str::to_owned)),
            );
        }
        if let Some(relaid) = draw::relaid(laid) {
            data.insert("relaid".into(), relaid);
        }
    }
}

/// `pages`, or `slides` read as pages (a model that renders decks too often names a
/// document's pages `slides`); given both, `slides` is ignored. The warnings say which.
fn pages_of(args: &Render, warnings: &mut Vec<String>) -> Result<Option<Vec<i64>>, ToolError> {
    let Some(slides) = &args.slides else {
        return Ok(args.pages.clone());
    };
    if args.pages.is_some() {
        warnings.push("slides ignored: a document has pages".to_owned());
        return Ok(args.pages.clone());
    }
    warnings.push("slides read as pages: a document has pages".to_owned());
    let mut out = Vec::with_capacity(slides.len());
    for (index, item) in slides.iter().enumerate() {
        let number = match item {
            IntOrStr::Int(n) => Some(*n),
            IntOrStr::Str(text) => text
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|_| text.trim().chars().all(|c| c.is_ascii_digit())),
        };
        let Some(number) = number else {
            let shown = match item {
                IntOrStr::Str(text) => ooxml_tools::pyfmt::repr(&Value::from(text.as_str())),
                IntOrStr::Int(n) => n.to_string(),
            };
            return Err(ToolError::refusal(
                Refusal::InvalidArguments,
                format!("slides[{index}] is {shown}: a document has pages, numbered from 1 (pages: [1])"),
                WayOut::one(json!({"pages": [1]})),
            )
            .at("pages"));
        };
        out.push(number);
    }
    Ok(Some(out))
}

/// The pages `ranges` fall on (with `pages`, those of them) and each range as reported
/// (`{"range", "blocks", "pages"}`): the ranges resolved before the version is laid out, as
/// docx-agent does.
fn range_pages(call: &Drawer<'_>, ranges: &[String], pages: Option<&[i64]>) -> Result<(Vec<i64>, Value), ToolError> {
    let doc = call.doc;
    let docx = call
        .laying
        .owned
        .doc
        .docx()
        .map_err(|e| ToolError::internal(format!("the document could not be read: {e}"), "render.range"))?;
    let scope = Scope::of(&docx, ranges).map_err(|e| scope_refusal(e, doc))?;
    let laid = call.laid()?;
    let mut every: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
    let mut reported = Vec::with_capacity(scope.ranges.len());
    for (range, on) in scope.ranges.iter().zip(scope.pages(laid)) {
        let on: Vec<i64> = on.into_iter().filter_map(|p| i64::try_from(p).ok()).collect();
        every.extend(on.iter().copied());
        reported.push(json!({"range": range.address, "blocks": range.blocks.len(), "pages": on}));
    }
    let on: Vec<i64> = every.into_iter().collect();
    let count = laid.page_count();
    if on.is_empty() {
        let options: Vec<Value> = (1..=count.min(50)).map(Value::from).collect();
        return Err(ToolError::refusal(
            Refusal::NotFound,
            "the range's blocks are drawn on no page (hidden, or past where the layout stopped)",
            WayOut::options(options).unwrap_or_else(|| WayOut::one(json!({"doc": doc}))),
        )
        .at("range"));
    }
    let Some(pages) = pages else {
        return Ok((on, Value::from(reported)));
    };
    let wanted: Vec<i64> = on.iter().copied().filter(|p| pages.contains(p)).collect();
    if wanted.is_empty() {
        let options: Vec<Value> = on.iter().take(50).copied().map(Value::from).collect();
        return Err(ToolError::refusal(
            Refusal::InvalidArguments,
            format!(
                "pages {} hold none of the range's blocks: they are on pages {}; leave pages out to draw them",
                python_list(pages),
                python_list(&on)
            ),
            WayOut::options(options).unwrap_or_else(|| WayOut::one(json!({"doc": doc}))),
        )
        .at("pages"));
    }
    Ok((wanted, Value::from(reported)))
}

/// Numbers as Python writes a list of them (`[1, 2]`).
fn python_list(numbers: &[i64]) -> String {
    let inner: Vec<String> = numbers.iter().map(i64::to_string).collect();
    format!("[{}]", inner.join(", "))
}

/// A range refused: what the resolver says, the call that draws the first page.
fn scope_refusal(error: ScopeError, doc: &str) -> ToolError {
    let call = ooxml_tools::TryCall {
        tool: "render".to_owned(),
        arguments: json!({"doc": doc}).as_object().cloned().unwrap_or_default(),
        fill: Vec::new(),
    };
    match error {
        ScopeError::Address { error, .. } => crate::refusal::address_refusal(error, "range", call),
        ScopeError::NotBody { address, story, .. } => ToolError::refusal(
            Refusal::InvalidArguments,
            format!("range {address} is in {story}, not the body: a range names the body's blocks"),
            WayOut::call(call),
        )
        .at("range"),
        ScopeError::Empty { address, .. } => ToolError::refusal(
            Refusal::InvalidArguments,
            format!("range {address} holds no paragraph"),
            WayOut::call(call),
        )
        .at("range"),
    }
}

/// `render` of `pages` (default the first), each its own image.
fn render_pages(
    call: &Drawer<'_>,
    pages: Option<Vec<i64>>,
    width: Option<i64>,
    warnings: &mut Vec<String>,
) -> Result<Done<Value>, ToolError> {
    let mut pages = pages.unwrap_or_else(|| vec![1]);
    let mut seen = std::collections::HashSet::new();
    pages.retain(|p| seen.insert(*p));
    let width = width.unwrap_or(DEFAULT_WIDTH);
    let per_call = call.limits.images_per_call;
    if pages.len() > per_call {
        return Err(ToolError::failure(
            Failure::Limit,
            format!(
                "at most {per_call} pages per call; sheet: {{}} tiles up to {} into one image",
                ooxml_tools::tiles::MAX_TILES
            ),
        )
        .at("pages"));
    }
    call.check_images(pages.len())?;
    let laid = call.laid()?;
    let count = laid.page_count();
    let numbers: Vec<usize> = pages
        .iter()
        .map(|p| usize::try_from(*p).ok().filter(|p| (1..=count).contains(p)))
        .collect::<Option<_>>()
        .ok_or_else(|| call.past_the_end(count))?;
    let drawn = draw::draw(call.layout, laid, call.drawing, (&numbers, width), warnings)?;
    let mut done = Done::new(format!(
        "Rendered page(s) {} {FINAL_VIEW}",
        pages.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
    ));
    let mut data = Map::new();
    data.insert("pages".into(), json!(pages));
    let cached: Vec<usize> = drawn.iter().filter(|d| d.cached).map(|d| d.page).collect();
    data.insert("cached".into(), json!(cached));
    call.facts(laid, &mut data);
    done.files = drawn
        .into_iter()
        .map(|d| File {
            name: format!("page {}", d.page),
            format: "png".to_owned(),
            bytes: d.png.to_vec(),
        })
        .collect();
    done.data = Some(Value::Object(data));
    Ok(done)
}

/// Registers `render` (the group's [`super::register`]): the host hands its calls to the
/// session with its page engine.
pub(crate) fn register(r: &mut impl crate::Registrar) {
    r.hosted_read::<Render>(render, crate::Hosting::SERVED.layout());
}
