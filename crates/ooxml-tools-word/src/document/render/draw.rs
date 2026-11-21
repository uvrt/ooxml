//! Owns: a version's pages drawn as PNG for `render` -- the version laid out by the host's
//! page engine (from the version it laid out last), each page's SVG at the width asked
//! (docx2svg's `render_page`), the faces the pages draw handed to a raster made for them
//! (docx2svg's `convert_docx_to_png`), the pages the store does not hold rasterised at once
//! on scoped threads, and each image kept in the store for the next call; the layout's
//! coverage and why it was laid out whole, as the reply says them.
//!
//! Must not: hold the engine's lock while rasterising, or keep anything but in the store.

use super::store::{page_key, ImageKey};
use crate::{NotLaid, Owned, WordLayout};
use ooxml_base::Cancel;
use ooxml_opc::Package;
use ooxml_tools::{Failure, Refusal, ToolError, TryCall, WayOut};
use ooxml_word_pages::png::{sized::sized, Fit, PageRaster};
use ooxml_word_pages::{DiskFonts, LaidVersion};
use serde_json::{json, Map, Value};
use std::sync::{Arc, PoisonError};

/// What a call draws pages with: the host's raster (the system's fonts and font folders)
/// and the session's store of the document's images.
#[derive(Clone, Copy)]
pub struct Drawing<'a> {
    /// The faces pages are drawn with beside each document's own.
    pub raster: &'a PageRaster,
    /// The document's images kept between calls.
    pub images: &'a super::store::PageImages,
    /// How many images the call may still give (`None`: no budget).
    pub images_left: Option<usize>,
}

impl std::fmt::Debug for Drawing<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Drawing")
            .field("images_left", &self.images_left)
            .finish_non_exhaustive()
    }
}

/// One page drawn.
#[derive(Debug, Clone)]
pub(super) struct Drawn {
    /// Its 1-based number.
    pub page: usize,
    /// Its PNG.
    pub png: Arc<[u8]>,
    /// Its pixel size.
    pub width: i64,
    /// Its pixel height.
    pub height: i64,
    /// Whether the store held it.
    pub cached: bool,
    /// What the store keeps it by.
    pub key: ImageKey,
}

/// `save_document` and `word_read`'s way out of a version the page engine does not lay out.
fn not_supported(doc: &str, why: &str) -> ToolError {
    let call = TryCall {
        tool: "word_read".to_owned(),
        arguments: json!({"doc": doc}).as_object().cloned().unwrap_or_default(),
        fill: Vec::new(),
    };
    ToolError::refusal(
        Refusal::Refused,
        format!(
            "render does not draw this document yet ({why}): read it as text (word_read), or save it \
             (save_document) and open it in Word"
        ),
        WayOut::call(call),
    )
}

/// The version `package` laid out by `layout` within the call's layout limit.
///
/// # Errors
/// `refused` where the engine does not lay it out yet (naming why, the way out a read or a
/// save); `timeout` past the call's layout limit or when the call is stopped.
pub(super) fn laid(
    layout: &dyn WordLayout,
    owned: &Owned<'_>,
    package: &Package,
    doc: &str,
    cancel: &dyn Cancel,
) -> Result<Arc<LaidVersion>, ToolError> {
    let stop = owned.layout_limit.map_or(cancel, |limit| limit.cancel);
    match layout.laid(package, stop) {
        Ok(laid) => Ok(laid),
        Err(NotLaid::Other(why)) => Err(not_supported(doc, &why)),
        Err(NotLaid::Stopped) => {
            cancel
                .check()
                .map_err(|_| ToolError::failure(Failure::Timeout, "the call was stopped"))?;
            let seconds = owned.layout_limit.map_or(0.0, |l| l.timeout.as_secs_f64());
            Err(ToolError::failure(
                Failure::Timeout,
                format!("laying the document out took longer than its {seconds:.0} s limit"),
            ))
        }
    }
}

/// Pages `pages` (1-based, each one the version has) of `version` drawn `width` pixels
/// wide; `notes` says of a page what is not drawn as Word draws it yet.
///
/// # Errors
/// `internal` where a page's SVG cannot be sized or drawn.
pub(super) fn draw(
    layout: &dyn WordLayout,
    version: &LaidVersion,
    drawing: Drawing<'_>,
    (pages, width): (&[usize], i64),
    notes: &mut Vec<String>,
) -> Result<Vec<Drawn>, ToolError> {
    let indexes: Vec<usize> = pages.iter().map(|p| p - 1).collect();
    let (svgs, faces) = {
        let mut engine = layout.engine().lock().unwrap_or_else(PoisonError::into_inner);
        let mut svgs = Vec::with_capacity(pages.len());
        for (&page, &index) in pages.iter().zip(&indexes) {
            let internal = || ToolError::internal(format!("page {page} could not be drawn"), "render.draw");
            let (svg, missing) = engine.page_svg(version, index).ok_or_else(internal)?;
            let frame = engine.paint(version, index).ok_or_else(internal)?.geometry.frame;
            let (svg, height) = sized(&svg, &frame, width).ok_or_else(internal)?;
            if !missing.is_empty() {
                let mut reasons = missing;
                reasons.dedup();
                notes.push(format!(
                    "page {page}: drawn, but not yet as Word draws it ({})",
                    reasons.join(", ")
                ));
            }
            svgs.push((svg, height));
        }
        (svgs, engine.raster_faces(version, &indexes))
    };
    let faces_key = faces.key();
    let raster = drawing
        .images
        .raster(faces_key, || drawing.raster.for_faces(&DiskFonts, &faces));
    let mut out: Vec<Drawn> = Vec::with_capacity(pages.len());
    for (&page, (svg, height)) in pages.iter().zip(&svgs) {
        let key = page_key(svg, width, faces_key);
        let held = drawing.images.get(&key);
        out.push(Drawn {
            page,
            cached: held.is_some(),
            png: held.unwrap_or_else(|| Arc::from(Vec::new())),
            width,
            height: *height,
            key,
        });
    }
    let wanted: Vec<&str> = svgs
        .iter()
        .zip(&out)
        .filter(|(_, d)| !d.cached)
        .map(|((svg, _), _)| svg.as_str())
        .collect();
    let mut drawn = rasterise(&raster, &wanted).into_iter();
    for page in out.iter_mut().filter(|d| !d.cached) {
        let made = drawn
            .next()
            .unwrap_or_else(|| Err("the rasteriser gave no image".to_owned()));
        let png: Arc<[u8]> = Arc::from(made.map_err(|e| {
            ToolError::internal(format!("page {} could not be drawn: {e}", page.page), "render.raster")
        })?);
        drawing.images.put(page.key, Arc::clone(&png));
        page.png = png;
    }
    Ok(out)
}

/// Each SVG as PNG, drawn at once over scoped threads (at most as many as the machine has
/// cores); in order.
pub(super) fn rasterise(raster: &PageRaster, svgs: &[&str]) -> Vec<Result<Vec<u8>, String>> {
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(svgs.len())
        .max(1);
    let chunk = svgs.len().div_ceil(workers).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = svgs
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .map(|svg| raster.png(svg, Fit::Original, None).map_err(|e| e.to_string()))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| {
                h.join()
                    .unwrap_or_else(|_| vec![Err("the rasteriser panicked".to_owned())])
            })
            .collect()
    })
}

/// Why `version` was laid out whole although the version before was laid out, in the tool's
/// ids (docx-agent's `relaid_facts`): `reason`, `at` (the block's id, else `p:` and its
/// paragraph id, else the path) and `part`; `None` where it was laid out from the version
/// before where the edit reached, or without one.
pub(super) fn relaid(version: &LaidVersion) -> Option<Value> {
    let cause = version.relaid_cause()?;
    let mut out = Map::new();
    out.insert("reason".into(), Value::from(cause.reason));
    if let Some(path) = &cause.at {
        let at = version
            .id_of(path)
            .map(str::to_owned)
            .or_else(|| cause.para_id.as_ref().map(|id| format!("p:{}", id.to_uppercase())))
            .unwrap_or_else(|| path.clone());
        out.insert("at".into(), Value::from(at));
    }
    if let Some(part) = &cause.part {
        out.insert("part".into(), Value::from(part.as_str()));
    }
    Some(Value::Object(out))
}
