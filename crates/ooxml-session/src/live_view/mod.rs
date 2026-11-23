//! Owns: an application's live view of a Word document (docx-agent's `liveview`, round 35
//! R27, on the session): [`Session::view`] -- the head version's pages, each with the ids of
//! the body's top-level blocks it draws and its size, the pages whose drawing changed since
//! an earlier view's token, and a token naming this version -- and [`Session::page_svg`],
//! one page of the version a token names as SVG, tagged with that version's ids, drawn from
//! its layout even when the document moved on since.
//!
//! A token is `<session nonce>-<doc id>-<state number>`: a state is laid out once whoever
//! asks, an undo's state is the token of the state it brings back, and a token from another
//! session (one before a restart) names nothing here. Pages changed are those
//! [`ooxml_word_pages::PageEngine::compare`] says differ (ids not followed: the same ids
//! are the same blocks) and those the version before did not have.
//!
//! The head is read without a lock; the version is laid out by the host's engine within
//! the call's limit on laying out, as `render` lays it out (`laid_to_draw`: a version the
//! engine does not lay out yet is refused, the way out a read or a save). A kept version's
//! page is painted by the engine only while it is the engine's last (its generation's model
//! -- the styles, the numbering -- is the last version's: an older version painted there
//! would draw with a later version's styles); an older one is laid out again on a spare
//! engine
//! ([`ooxml_tools_word::WordLayout::seed`]) once, that layout kept with the version within
//! the session's memory budget for its other pages: an older version never becomes the
//! engine's last. A version whose layout the budget has no room for is not kept: its token
//! is stale to a later view, and its pages are laid out on demand.
//!
//! Must not: hold the document's kept versions' lock while laying out or painting, lay a
//! version out but through the host's engine or a spare of it, or draw text as outlines yet
//! (refused, the way out `text="text"`).

mod kept;
mod tag;

pub use kept::VIEW_TOKENS;

use crate::context::{CallCancel, LayoutCancel};
use crate::document::{At, Word};
use crate::{CallContext, DocId, DocKind, Session, SessionError};
pub(crate) use kept::LiveViews;
use kept::{Kept, Spare};
use ooxml_base::Cancel;
use ooxml_tools::{Refusal, ToolError, WayOut};
use ooxml_tools_word::{laid_to_draw, EnginePages, LayoutLimit, WordLayout};
use ooxml_word_pages::LaidVersion;
use std::collections::HashMap;
use std::sync::Arc;

/// What a refusal names as not drawing the document.
const WHO: &str = "the live view";

/// One page of a view.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewPage {
    /// Its 1-based number.
    pub number: usize,
    /// The ids of the body's top-level blocks it draws something of, in document order.
    pub blocks: Vec<String>,
    /// Its width, points.
    pub width: f64,
    /// Its height, points.
    pub height: f64,
}

/// What [`Session::view`] answers.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// This version's token: `since` for the next view, and [`Session::page_svg`]'s.
    pub token: String,
    /// Every page, in order.
    pub pages: Arc<[ViewPage]>,
    /// The 1-based pages whose drawing changed since `since`: every page without one, or
    /// with a stale one. Pages `since`'s version had past the last are not listed.
    pub changed: Vec<usize>,
    /// The token the pages were compared with, as given.
    pub since: Option<String>,
    /// `since` names no version kept for the document: every page is in `changed`.
    pub stale: bool,
    /// Why every page is listed as changed, where `since` was given.
    pub note: Option<String>,
    /// The state number of the version laid out.
    pub version: u64,
}

/// How [`Session::page_svg`] draws text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMode {
    /// As `<text>`, drawn with the viewer's fonts.
    Text,
    /// As the glyphs' paths (not drawn yet: refused).
    Outlines,
}

impl TextMode {
    /// The mode named `text` (`"text"`, `"outlines"`).
    ///
    /// # Errors
    /// [`ViewError::Mode`] for another name.
    pub fn parse(text: &str) -> Result<TextMode, ViewError> {
        match text {
            "text" => Ok(TextMode::Text),
            "outlines" => Ok(TextMode::Outlines),
            other => Err(ViewError::Mode(other.to_owned())),
        }
    }
}

/// What [`Session::page_svg`] answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSvg {
    /// The 1-based page.
    pub page: usize,
    /// The token of the version drawn.
    pub token: String,
    /// The page as SVG, its paragraph groups carrying `data-docx-agent-id`, its pictures
    /// embedded.
    pub svg: String,
    /// What is not drawn as Word draws it yet.
    pub warnings: Vec<String>,
}

/// Why a view or a page was not given.
#[derive(Debug, thiserror::Error)]
pub enum ViewError {
    /// The session's own: the document is not open.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// The document is not a Word document.
    #[error("{doc} is a {kind} document; the live view draws a Word document's pages")]
    NotWord {
        /// Its id.
        doc: String,
        /// Its kind.
        kind: DocKind,
    },
    /// Not drawn, as a tool says so: `refused` for a version the engine does not lay out
    /// yet (or text as outlines), `timeout` past the call's limit on laying out.
    #[error("{0}")]
    NotDrawn(ToolError),
    /// The token names no version kept for the document.
    #[error(
        "token {token:?} names no layout kept for this document (another document's, one from before a \
         restart, or older than the last {VIEW_TOKENS}); call view() for a current token and the pages it changed"
    )]
    UnknownToken {
        /// The token given.
        token: String,
    },
    /// A page the version does not have.
    #[error("page {page}: the version {token:?} names has {count} page(s), 1 to {count}")]
    NoPage {
        /// The page asked.
        page: usize,
        /// The token given.
        token: String,
        /// Its pages.
        count: usize,
    },
    /// A text mode there is none of.
    #[error("text must be one of text, outlines, not {0:?}")]
    Mode(String),
}

impl Session {
    /// The Word document `doc`'s pages at its head version, the pages changed since the
    /// view whose token is `since` (every page without one), and this version's token;
    /// laid out by `layout` (the host's engine for the document) within `context`'s limit on
    /// laying out (the module says how).
    ///
    /// # Errors
    /// [`ViewError::Session`] for a document not open, [`ViewError::NotWord`],
    /// [`ViewError::NotDrawn`] where the version is not laid out.
    pub fn view(
        &self,
        doc: &DocId,
        since: Option<&str>,
        layout: &dyn WordLayout,
        context: &CallContext,
    ) -> Result<View, ViewError> {
        let document = self.document(doc)?;
        let word = word_of(doc, document.kind, document.word())?;
        let At::Word(head) = document.head() else {
            return Err(not_word(doc, document.kind));
        };
        let number = head.number();
        let token = format!("{}-{doc}-{number}", self.nonce);
        // `since` first: made the most recently used before this version is kept; one whose
        // layout the budget had no room for is stale
        let old = since
            .filter(|s| *s != token)
            .and_then(|s| word.views.get(s))
            .filter(|old| old.version().is_some());
        let cancel = CallCancel::new(context, &*self.context.cancel, &*self.context.clock);
        let laying = cancel.layout();
        let limit = laying.as_ref().map(LayoutCancel::limit);
        let found = word.views.get(&token);
        let laid = match found.as_ref().and_then(|kept| kept.version()) {
            Some(laid) => Arc::clone(laid),
            None => {
                let laid = laid_to_draw(WHO, layout, limit, (head.doc().package(), &doc.to_string()), &cancel)
                    .map_err(ViewError::NotDrawn)?;
                // a later view compares with it: digested before the engine stops painting it
                lock(layout).watch(&laid);
                laid
            }
        };
        let now = match found {
            Some(kept) => kept,
            None => {
                let pages = pages_of(&laid);
                word.views
                    .keep(Kept::new(token.clone(), Arc::clone(&laid), head, pages))
            }
        };
        let every = || now.pages.iter().map(|p| p.number).collect::<Vec<_>>();
        let (changed, stale, note) = match (since, &old) {
            (None, _) => (every(), false, None),
            (Some(s), _) if s == token => (Vec::new(), false, None),
            (Some(s), None) => (
                every(),
                true,
                Some(format!(
                    "since {s:?} names no layout kept for this document (another document's, one from before a \
                     restart, older than the last {VIEW_TOKENS}, or one the memory budget had no room for): every \
                     page is listed as changed"
                )),
            ),
            (Some(_), Some(old)) => match changed(layout, old, (&laid, now.pages.len()), (limit, &cancel)) {
                Some(changed) => (changed, false, None),
                None => (
                    every(),
                    false,
                    Some("the pages could not be compared with since's: every page is listed as changed".to_owned()),
                ),
            },
        };
        Ok(View {
            token,
            pages: Arc::clone(&now.pages),
            changed,
            since: since.map(str::to_owned),
            stale,
            note,
            version: number,
        })
    }

    /// Page `page` (1-based) of the Word document `doc`'s version that `token` names (a
    /// view's), as SVG drawn with `text`, tagged with that version's ids: painted by
    /// `layout`'s engine, else laid out again on a spare of it within `context`'s limit on
    /// laying out.
    ///
    /// # Errors
    /// [`ViewError::Session`], [`ViewError::NotWord`]; [`ViewError::NotDrawn`] for text as
    /// outlines (refused, the way out `text="text"`) or a version not laid out again;
    /// [`ViewError::UnknownToken`], [`ViewError::NoPage`].
    pub fn page_svg(
        &self,
        doc: &DocId,
        page: usize,
        token: &str,
        layout: &dyn WordLayout,
        text: TextMode,
        context: &CallContext,
    ) -> Result<PageSvg, ViewError> {
        let document = self.document(doc)?;
        let word = word_of(doc, document.kind, document.word())?;
        if text == TextMode::Outlines {
            return Err(ViewError::NotDrawn(outlines_refused()));
        }
        let kept = word.views.get(token).ok_or_else(|| ViewError::UnknownToken {
            token: token.to_owned(),
        })?;
        let count = kept.pages.len();
        if !(1..=count).contains(&page) {
            return Err(ViewError::NoPage {
                page,
                token: token.to_owned(),
                count,
            });
        }
        // only the engine's last version: its generation's model (the styles, the numbering)
        // is the last version's, so an older one painted there would draw with them
        let painted = kept.version().and_then(|laid| {
            let mut engine = lock(layout);
            let last = engine.last().is_some_and(|last| Arc::ptr_eq(last, laid));
            if last {
                engine.page_svg(laid, page - 1)
            } else {
                None
            }
        });
        let (svg, mut warnings) = match painted {
            Some(painted) => painted,
            None => {
                let spare = match kept.spare() {
                    Some(spare) => spare,
                    None => {
                        let cancel = CallCancel::new(context, &*self.context.cancel, &*self.context.clock);
                        let laying = cancel.layout();
                        let limit = laying.as_ref().map(LayoutCancel::limit);
                        let package = (kept.at.doc().package(), &*doc.to_string());
                        let spare = spared(layout, package, (limit, &cancel)).map_err(ViewError::NotDrawn)?;
                        // the next page of this version is painted from it, not laid out again
                        word.views.keep_spare(&kept, &spare);
                        spare
                    }
                };
                let painted = lock(&*spare.engine).page_svg(&spare.version, page - 1);
                painted.ok_or_else(|| {
                    ViewError::NotDrawn(ToolError::internal(
                        format!("page {page} could not be drawn"),
                        "view.page",
                    ))
                })?
            }
        };
        warnings.dedup();
        let docx = kept.at.doc().docx().map_err(SessionError::Word)?;
        Ok(PageSvg {
            page,
            token: token.to_owned(),
            svg: tag::tagged(&svg, &docx),
            warnings,
        })
    }
}

/// The Word document's state, or why there is none.
fn word_of<'d>(doc: &DocId, kind: DocKind, word: Option<&'d Word>) -> Result<&'d Word, ViewError> {
    word.ok_or_else(|| not_word(doc, kind))
}

fn not_word(doc: &DocId, kind: DocKind) -> ViewError {
    ViewError::NotWord {
        doc: doc.to_string(),
        kind,
    }
}

/// The refusal of text drawn as outlines (not drawn yet), the way out text.
fn outlines_refused() -> ToolError {
    ToolError::refusal(
        Refusal::Refused,
        "the live view does not draw text as outlines yet: draw it as text (text=\"text\")",
        WayOut::one(serde_json::Value::from("text")),
    )
    .at("text")
}

fn lock(layout: &dyn WordLayout) -> std::sync::MutexGuard<'_, ooxml_word_pages::PageEngine> {
    ooxml_word_pages::lock_engine(layout.engine())
}

/// Each page of `laid`: its number, its top-level blocks' ids and its size in points.
fn pages_of(laid: &LaidVersion) -> Vec<ViewPage> {
    laid.facts()
        .pages
        .iter()
        .enumerate()
        .map(|(k, page)| ViewPage {
            number: k + 1,
            blocks: laid.top_blocks(k).into_iter().map(str::to_owned).collect(),
            width: points(page.frame.width_pt),
            height: points(page.frame.height_pt),
        })
        .collect()
}

/// `value` as the nearest float.
fn points(value: ooxml_base::Q) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let (n, d) = (*value.numer() as f64, *value.denom() as f64);
    n / d
}

/// `package` (a version of the document, and the id a refusal names it by) laid out on a
/// spare of `layout`'s engine (`stop`: the call's limit on laying out and its
/// cancellation).
fn spared(
    layout: &dyn WordLayout,
    package: (&ooxml_opc::Package, &str),
    (limit, cancel): (Option<LayoutLimit<'_>>, &dyn Cancel),
) -> Result<Spare, ToolError> {
    let engine = Arc::new(EnginePages::of(layout.seed().spare()));
    let version = laid_to_draw(WHO, &*engine, limit, package, cancel)?;
    Ok(Spare { engine, version })
}

/// The 1-based pages of `now` (its layout and its count of pages) whose drawing differs
/// from `old`'s page of the same number, and those `old` did not have: compared by the
/// host's engine, else -- where it no longer paints `old` and made no summary of it --
/// with `old`'s spare layout, laid out on a spare engine where none is kept (`stop`: the
/// call's limit on laying out and its cancellation). `None` where neither compares them.
fn changed(
    layout: &dyn WordLayout,
    old: &Kept,
    (now, count): (&Arc<LaidVersion>, usize),
    stop: (Option<LayoutLimit<'_>>, &dyn Cancel),
) -> Option<Vec<usize>> {
    let before = old.version()?;
    if Arc::ptr_eq(before, now) {
        return Some(Vec::new());
    }
    let none = HashMap::new();
    let compared = lock(layout).compare(now, before, &none);
    let mut changed = match compared {
        Some(reflow) => reflow.changed,
        None => {
            let summary = lock(layout).summary(now)?;
            let spare = match old.spare() {
                Some(spare) => spare,
                None => spared(layout, (old.at.doc().package(), ""), stop).ok()?,
            };
            let before = lock(&*spare.engine).summary(&spare.version)?;
            summary.compare(&before, &none).changed
        }
    };
    changed.extend(old.pages.len() + 1..=count);
    Some(changed)
}
