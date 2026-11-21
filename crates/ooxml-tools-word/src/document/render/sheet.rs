//! Owns: `render` with `sheet` (docx-agent's `_render_sheet`): the pages tiled into one
//! image, in order, each labelled `page 3` (`ooxml_tools::tiles`); `pages` left out is the
//! first [`MAX_TILES`], or all there are. `sheet.width` is each tile's, `width` standing in
//! for it. Tiles too wide or too tall for the image's edge are drawn narrower -- the widest
//! that fit, from the pages' own shapes, before anything is drawn -- and `sheet.shrunk`
//! says from what width and why; only tiles too tall even at the narrowest are refused.
//! Pages named are refused so before anything is laid out, at the shortest page the
//! sections declare (docx-agent's order: `limit` before `not_found`). The
//! tiles are the images single renders keep (`cached_tiles` lists those held); the sheet is
//! kept whole, one entry.
//!
//! Must not: draw a page but through [`super::draw`].

use super::{draw, sheet_key, Drawer, RenderSheet, FINAL_VIEW};
use ooxml_tools::tiles::{check_size, fit, tiled_svg, tiling_of, Tile, TileAsk, TileOptions, MAX_TILES};
use ooxml_tools::{Done, File, ToolError};
use ooxml_word_pages::LaidVersion;
use serde_json::{json, Map, Value};
use std::sync::{Arc, PoisonError};

/// Appends `bytes` in base64 (the tiles' `data:` URIs).
fn base64(bytes: &[u8], out: &mut String) {
    ooxml_scene::base64::encode_into(bytes, out);
}

/// Each page's shape (its height over its width, from its section's `w:pgSz` in twips, as
/// docx-agent's `_shortest_ratio` reads it), for pages 1-based. Not the whole-pixel
/// frame's: its rounding makes a tile look taller than it is drawn, and the fit a pixel
/// narrower than docx-agent's.
fn shapes(call: &Drawer<'_>, laid: &LaidVersion, pages: &[usize]) -> Vec<f64> {
    let sections = &laid.layout().sections;
    let mut engine = call.layout.engine().lock().unwrap_or_else(PoisonError::into_inner);
    pages
        .iter()
        .filter_map(|page| engine.paint(laid, page - 1))
        .map(|paint| {
            let frame = paint.geometry.frame;
            let declared = sections
                .get(frame.section)
                .map(|s| (s.props.page_size.width, s.props.page_size.height))
                .filter(|(width, height)| *width > 0 && *height > 0);
            #[allow(clippy::cast_precision_loss)]
            let shape = match declared {
                Some((width, height)) => height as f64 / width as f64,
                None => frame.height_px as f64 / frame.width_px.max(1) as f64,
            };
            shape
        })
        .collect()
}

/// `render` of `pages` as one sheet.
pub(super) fn render_sheet(
    call: &Drawer<'_>,
    pages: Option<Vec<i64>>,
    width: Option<i64>,
    sheet: RenderSheet,
    warnings: &mut Vec<String>,
) -> Result<Done<Value>, ToolError> {
    let ask = TileAsk {
        name: "sheet",
        what: "pages",
        width,
        max_edge: call.limits.image_edge,
    };
    let pages = pages.map(|mut asked| {
        let mut seen = std::collections::HashSet::new();
        asked.retain(|p| seen.insert(*p));
        asked
    });
    let numbers: Vec<usize> = match pages {
        Some(asked) => {
            // Before laying out: the tiles fitted to the pages asked at the shortest page
            // the sections declare (too tall even so is refused whatever the pages are),
            // and the image budget; only then whether the pages exist.
            let tiling = tiling_of(options(sheet), asked.len(), ask)?;
            if let Some(shortest) = call.shortest_declared()? {
                fit(&tiling, &vec![shortest; asked.len()], ask)?;
            }
            call.check_images(1)?;
            let count = call.laid()?.page_count();
            asked
                .iter()
                .map(|p| usize::try_from(*p).ok().filter(|p| (1..=count).contains(p)))
                .collect::<Option<_>>()
                .ok_or_else(|| call.past_the_end(count))?
        }
        None => {
            let numbers: Vec<usize> = (1..=call.laid()?.page_count().min(MAX_TILES)).collect();
            tiling_of(options(sheet), numbers.len(), ask)?;
            call.check_images(1)?;
            numbers
        }
    };
    let laid = call.laid()?;
    let tiling = tiling_of(options(sheet), numbers.len(), ask)?;
    let tiling = fit(&tiling, &shapes(call, laid, &numbers), ask)?;
    let drawn = draw::draw(call.layout, laid, call.drawing, (&numbers, tiling.width), warnings)?;
    let heights: Vec<i64> = drawn.iter().map(|d| d.height).collect();
    check_size(&tiling, &heights, ask)?;
    let key = sheet_key(
        &drawn.iter().map(|d| d.key).collect::<Vec<_>>(),
        tiling.columns,
        tiling.width,
    );
    let (png, cached) = sheet_png(call, &drawn, &numbers, &tiling, key)?;
    Ok(reply(call, laid, &drawn, &numbers, &tiling, (png, cached)))
}

/// The sheet's reply: its data, summary and image.
fn reply(
    call: &Drawer<'_>,
    laid: &LaidVersion,
    drawn: &[draw::Drawn],
    numbers: &[usize],
    tiling: &ooxml_tools::tiles::Tiling,
    (png, cached): (Arc<[u8]>, bool),
) -> Done<Value> {
    let (first, last) = (
        numbers.first().copied().unwrap_or(1),
        numbers.last().copied().unwrap_or(1),
    );
    let label = if numbers.len() > 1 {
        format!("sheet of pages {first}-{last}")
    } else {
        format!("sheet of page {first}")
    };
    let mut data = Map::new();
    data.insert("pages".into(), json!(numbers));
    let mut described = Map::new();
    described.insert("columns".into(), Value::from(tiling.columns));
    described.insert("width".into(), Value::from(tiling.width));
    if let Some(shrunk) = tiling.shrunk() {
        described.insert("shrunk".into(), shrunk);
    }
    data.insert("sheet".into(), Value::Object(described));
    data.insert("cached".into(), Value::from(cached));
    let tiles_cached: Vec<usize> = drawn.iter().filter(|d| d.cached).map(|d| d.page).collect();
    if !cached && !tiles_cached.is_empty() {
        data.insert("cached_tiles".into(), json!(tiles_cached));
    }
    call.facts(laid, &mut data);
    let mut done = Done::new(format!(
        "Rendered {} page(s) as one sheet, {} column(s) of {} px{}, labelled with page numbers {FINAL_VIEW}",
        numbers.len(),
        tiling.columns,
        tiling.width,
        tiling.narrowed_note()
    ));
    done.files = vec![File {
        name: label,
        format: "png".to_owned(),
        bytes: png.to_vec(),
    }];
    done.data = Some(Value::Object(data));
    done
}

/// The sheet's PNG: the one the store keeps under `key`, else drawn from the tiles and
/// kept; and whether it was kept.
fn sheet_png(
    call: &Drawer<'_>,
    drawn: &[draw::Drawn],
    numbers: &[usize],
    tiling: &ooxml_tools::tiles::Tiling,
    key: super::ImageKey,
) -> Result<(Arc<[u8]>, bool), ToolError> {
    let held = call.drawing.images.get(&key);
    let cached = held.is_some();
    let png = match held {
        Some(png) => png,
        None => {
            let labels: Vec<String> = numbers.iter().map(|p| format!("page {p}")).collect();
            let tiles: Vec<Tile<'_>> = drawn
                .iter()
                .zip(&labels)
                .map(|(d, label)| Tile {
                    png: &d.png,
                    width: d.width,
                    height: d.height,
                    label,
                })
                .collect();
            let (svg, _, _) = tiled_svg(&tiles, tiling, base64);
            let made = draw::rasterise(call.drawing.raster, &[svg.as_str()]);
            let png: Arc<[u8]> = match made.into_iter().next() {
                Some(Ok(png)) => Arc::from(png),
                Some(Err(e)) => {
                    return Err(ToolError::internal(
                        format!("the sheet could not be drawn: {e}"),
                        "render.sheet",
                    ))
                }
                None => return Err(ToolError::internal("the sheet could not be drawn", "render.sheet")),
            };
            call.drawing.images.put(key, Arc::clone(&png));
            png
        }
    };
    Ok((png, cached))
}

fn options(sheet: RenderSheet) -> TileOptions {
    TileOptions {
        columns: sheet.columns,
        width: sheet.width,
    }
}
