//! Owns: contact images (ooxml-edit's `tools/tiles.py`): many rendered views tiled into
//! one image, each labelled -- the options as checked ([`Tiling`], [`tiling_of`]), tiles
//! narrowed to fit the image's edge ([`fit`]), the last size check ([`check_size`]) and the
//! SVG that places each tile's PNG at its own pixel size, so nothing is resampled
//! ([`tiled_svg`]).
//!
//! The width is each tile's, not the whole image's: what the model chooses is how legible
//! one view is. With the defaults (4 columns of 360 px) the image is 1,480 px wide. An
//! image that would be over the edge limit on either side is drawn with narrower tiles --
//! the widest that fit, never under [`MIN_WIDTH`] -- and [`Tiling::shrunk`] says from what
//! width and why. Only tiles that would not fit even at [`MIN_WIDTH`] are refused, naming
//! what would.
//!
//! Must not: know formats or draw pixels (a format library renders the tiles and
//! rasterises [`tiled_svg`]'s document once).

use crate::{Failure, Refusal, ToolError, WayOut};
use serde_json::{json, Value};
use std::fmt::Write as _;

/// Views one contact image may hold.
pub const MAX_TILES: usize = 30;
/// Columns at least.
pub const MIN_COLUMNS: i64 = 1;
/// Columns at most.
pub const MAX_COLUMNS: i64 = 8;
/// Columns by default.
pub const DEFAULT_COLUMNS: i64 = 4;
/// A tile's width at least, in pixels.
pub const MIN_WIDTH: i64 = 160;
/// A tile's width at most, in pixels.
pub const MAX_WIDTH: i64 = 1280;
/// A tile's width by default, in pixels.
pub const DEFAULT_WIDTH: i64 = 360;
/// The gap round every tile, in pixels.
pub const GAP: i64 = 8;
/// The label strip's height, in pixels.
pub const LABEL_HEIGHT: i64 = 20;
/// The label's text size, in pixels.
pub const LABEL_SIZE: i64 = 13;
const BACKGROUND: &str = "#d4d4d4";
const FRAME: &str = "#8c8c8c";
const INK: &str = "#1a1a1a";

/// The options as checked: `columns` (no more than the tiles) and each tile's `width` in
/// pixels; `asked` the width asked for when it was narrowed to fit, and `why`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tiling {
    /// Columns.
    pub columns: i64,
    /// Each tile's width.
    pub width: i64,
    /// The width first asked for, where narrowed.
    pub asked: Option<i64>,
    /// Why it was narrowed.
    pub why: Option<String>,
}

/// The tiling's options as the call gave them (`{"columns", "width"}`, either left out).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TileOptions {
    /// Columns.
    pub columns: Option<i64>,
    /// Each tile's width.
    pub width: Option<i64>,
}

/// What [`tiling_of`] checks the options against.
#[derive(Debug, Clone, Copy)]
pub struct TileAsk<'a> {
    /// The argument's name (`sheet`): what errors name.
    pub name: &'a str,
    /// What the tiles are (`pages`): the field a count error names.
    pub what: &'a str,
    /// The call's own width, taken as the tile's where the options give none.
    pub width: Option<i64>,
    /// The image's edge limit, in pixels.
    pub max_edge: i64,
}

impl Tiling {
    /// The image's width.
    #[must_use]
    pub fn total_width(&self) -> i64 {
        self.columns * self.width + (self.columns + 1) * GAP
    }

    /// `{"from": <width asked>, "why": ...}` where the tiles are narrower than asked.
    #[must_use]
    pub fn shrunk(&self) -> Option<Value> {
        self.asked.map(|from| json!({"from": from, "why": self.why}))
    }

    /// This tiling at `width`, remembering the width first asked for.
    #[must_use]
    pub fn narrowed(&self, width: i64, why: &str) -> Tiling {
        if width == self.width {
            return self.clone();
        }
        let reason = match &self.why {
            Some(before) => format!("{before}; then {why}"),
            None => why.to_owned(),
        };
        Tiling {
            columns: self.columns,
            width,
            asked: Some(self.asked.unwrap_or(self.width)),
            why: Some(reason),
        }
    }

    /// What a summary adds for tiles narrower than asked (` (narrowed from 1280 px: ...)`).
    #[must_use]
    pub fn narrowed_note(&self) -> String {
        match (self.asked, &self.why) {
            (Some(asked), Some(why)) => format!(" (narrowed from {asked} px: {why})"),
            _ => String::new(),
        }
    }
}

/// A `limit` refusal at `field`, `most` the limit and `given` what the call asked
/// (its details).
fn limit(message: String, field: &str, (most, given): (usize, usize)) -> ToolError {
    ToolError::failure(Failure::Limit, message)
        .with_limit(most, given)
        .at(field)
}

/// A count of tiles or columns (never negative) as a limit's detail.
fn counted(n: i64) -> usize {
    usize::try_from(n).unwrap_or(0)
}

fn invalid(message: String, field: &str, low: i64, high: i64) -> ToolError {
    let way_out = WayOut::one(json!({"minimum": low, "maximum": high}));
    ToolError::refusal(Refusal::InvalidArguments, message, way_out).at(field)
}

/// The options checked for `count` tiles. Tiles too wide side by side for the edge are
/// narrowed to the widest that fit.
///
/// # Errors
/// `limit` for more than [`MAX_TILES`] tiles or columns too many even at [`MIN_WIDTH`];
/// `invalid_arguments` for columns or a width out of bounds.
pub fn tiling_of(options: TileOptions, count: usize, ask: TileAsk<'_>) -> Result<Tiling, ToolError> {
    let TileAsk {
        name,
        what,
        width,
        max_edge,
    } = ask;
    if count > MAX_TILES {
        return Err(limit(
            format!(
                "one {name} holds at most {MAX_TILES} {what}, not {count}: render the first {MAX_TILES}, and the rest in another call"
            ),
            what,
            (MAX_TILES, count),
        ));
    }
    let columns = options.columns.unwrap_or(DEFAULT_COLUMNS);
    if !(MIN_COLUMNS..=MAX_COLUMNS).contains(&columns) {
        return Err(invalid(
            format!("{name}.columns is {MIN_COLUMNS}-{MAX_COLUMNS}, not {columns}"),
            &format!("{name}.columns"),
            MIN_COLUMNS,
            MAX_COLUMNS,
        ));
    }
    let field = if options.width.is_some() || width.is_none_or(|w| w == 0) {
        format!("{name}.width")
    } else {
        "width".to_owned()
    };
    let (mut asked, mut why) = (None, None);
    let mut tile = options.width;
    if tile.is_none() {
        if let Some(w) = width.filter(|w| *w > MAX_WIDTH) {
            asked = Some(w);
            why = Some(format!("width {w} is over a tile's {MAX_WIDTH} px"));
            tile = Some(MAX_WIDTH);
        }
    }
    let tile = tile.unwrap_or_else(|| width.filter(|w| *w != 0).unwrap_or(DEFAULT_WIDTH));
    if !(MIN_WIDTH..=MAX_WIDTH).contains(&tile) {
        return Err(invalid(
            format!("a {name}'s tile width ({field}) is {MIN_WIDTH}-{MAX_WIDTH} px, not {tile}"),
            &field,
            MIN_WIDTH,
            MAX_WIDTH,
        ));
    }
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    let result = Tiling {
        columns: columns.min(count).max(1),
        width: tile,
        asked,
        why,
    };
    if result.total_width() <= max_edge {
        return Ok(result);
    }
    let widest = (max_edge - (result.columns + 1) * GAP) / result.columns;
    if widest < MIN_WIDTH {
        let most = ((max_edge - GAP) / (MIN_WIDTH + GAP)).max(1);
        return Err(limit(
            format!(
                "{} columns make the {name} over the {max_edge} px limit even at a tile width of {MIN_WIDTH}: at most {most} columns",
                result.columns
            ),
            &format!("{name}.columns"),
            (counted(most), counted(result.columns)),
        ));
    }
    let why = format!(
        "{} columns of {} px make the {name} {} px wide, over the {max_edge} px limit",
        result.columns,
        result.width,
        result.total_width()
    );
    Ok(result.narrowed(widest, &why))
}

/// The image's height for tiles of these heights in `columns` columns.
fn tall(columns: i64, heights: &[i64]) -> i64 {
    let step = usize::try_from(columns.max(1)).unwrap_or(1);
    let rows: Vec<i64> = heights
        .chunks(step)
        .map(|row| row.iter().copied().max().unwrap_or(0) + LABEL_HEIGHT)
        .collect();
    let count = i64::try_from(rows.len()).unwrap_or(i64::MAX);
    rows.iter().sum::<i64>() + (count + 1) * GAP
}

/// Each tile's height at `width` for these shapes (height over width), as the rasteriser
/// sizes it.
fn heights(width: i64, ratios: &[f64]) -> Vec<i64> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    ratios
        .iter()
        .map(|ratio| (width as f64 * ratio - 1e-9).ceil() as i64)
        .collect()
}

/// `tiling` with its tiles narrowed to the widest whose image is at most `ask.max_edge`
/// tall, for tiles of these shapes; `tiling` itself where it fits.
///
/// # Errors
/// `limit` where even [`MIN_WIDTH`] is too tall, naming how many fit there.
pub fn fit(tiling: &Tiling, ratios: &[f64], ask: TileAsk<'_>) -> Result<Tiling, ToolError> {
    let TileAsk {
        name, what, max_edge, ..
    } = ask;
    if ratios.is_empty() || tall(tiling.columns, &heights(tiling.width, ratios)) <= max_edge {
        return Ok(tiling.clone());
    }
    let (mut low, mut high) = (MIN_WIDTH, tiling.width - 1);
    let narrowest = heights(low, ratios);
    if tall(tiling.columns, &narrowest) > max_edge {
        let row = narrowest.iter().copied().max().unwrap_or(0) + LABEL_HEIGHT + GAP;
        let rows = ((max_edge - GAP) / row).max(1);
        return Err(limit(
            format!(
                "{} {what} in {} column(s) make the {name} over the {max_edge} px limit even at a tile width of {MIN_WIDTH}: at most {} {what} at these columns; or more columns",
                ratios.len(),
                tiling.columns,
                rows * tiling.columns
            ),
            what,
            (counted(rows * tiling.columns), ratios.len()),
        ));
    }
    while low < high {
        let middle = (low + high + 1) / 2;
        if tall(tiling.columns, &heights(middle, ratios)) <= max_edge {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let total = tall(tiling.columns, &heights(tiling.width, ratios));
    let why = format!(
        "{} {what} in {} column(s) of {} px make the {name} {total} px tall, over the {max_edge} px limit",
        ratios.len(),
        tiling.columns,
        tiling.width
    );
    Ok(tiling.narrowed(low, &why))
}

/// The image's pixel size for tiles of these heights (each [`Tiling::width`] wide).
#[must_use]
pub fn tiled_size(tiling: &Tiling, heights: &[i64]) -> (i64, i64) {
    (tiling.total_width(), tall(tiling.columns, heights))
}

/// [`tiled_size`], refused where the image is over `ask.max_edge` tall.
///
/// # Errors
/// `limit`, naming how many tiles fit.
pub fn check_size(tiling: &Tiling, heights: &[i64], ask: TileAsk<'_>) -> Result<(i64, i64), ToolError> {
    let TileAsk {
        name, what, max_edge, ..
    } = ask;
    let (width, height) = tiled_size(tiling, heights);
    if height > max_edge {
        let row = heights.iter().copied().max().unwrap_or(0) + LABEL_HEIGHT + GAP;
        let rows = ((max_edge - GAP) / row).max(1);
        return Err(limit(
            format!(
                "{} {what} in {} column(s) make the {name} {width}x{height} px, over the {max_edge} px limit: at most {} {what} at this width and columns; or more columns, or a smaller {name}.width",
                heights.len(),
                tiling.columns,
                rows * tiling.columns
            ),
            what,
            (counted(rows * tiling.columns), heights.len()),
        ));
    }
    Ok((width, height))
}

/// One tile: its PNG, pixel size and label.
#[derive(Debug, Clone, Copy)]
pub struct Tile<'a> {
    /// The PNG's bytes.
    pub png: &'a [u8],
    /// Its width in pixels.
    pub width: i64,
    /// Its height in pixels.
    pub height: i64,
    /// Its label (`page 3`).
    pub label: &'a str,
}

/// One SVG document of the tiles, in order, and its pixel size: each PNG at its own size
/// on a whole pixel (drawn, not resampled), framed, its label under it, cut to the tile's
/// width. `base64` appends bytes in base64 to a string.
#[must_use]
pub fn tiled_svg(tiles: &[Tile<'_>], tiling: &Tiling, base64: fn(&[u8], &mut String)) -> (String, i64, i64) {
    let all: Vec<i64> = tiles.iter().map(|t| t.height).collect();
    let (width, height) = tiled_size(tiling, &all);
    let mut out = String::new();
    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><rect width=\"{width}\" height=\"{height}\" fill=\"{BACKGROUND}\"/>"
    );
    let mut top = GAP;
    let step = usize::try_from(tiling.columns.max(1)).unwrap_or(1);
    for row in tiles.chunks(step) {
        let tallest = row.iter().map(|t| t.height).max().unwrap_or(0);
        for (column, tile) in (0_i64..).zip(row) {
            let left = GAP + column * (tiling.width + GAP);
            let _ = write!(
                out,
                // `left - 0.5` and `top - 0.5`, as Python writes them.
                "<rect x=\"{}.5\" y=\"{}.5\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{FRAME}\"/>",
                left - 1,
                top - 1,
                tile.width + 1,
                tile.height + 1
            );
            let _ = write!(
                out,
                "<image x=\"{left}\" y=\"{top}\" width=\"{}\" height=\"{}\" href=\"data:image/png;base64,",
                tile.width, tile.height
            );
            base64(tile.png, &mut out);
            let _ = write!(
                out,
                "\"/><text x=\"{}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"{LABEL_SIZE}\" fill=\"{INK}\">{}</text>",
                left + 2,
                top + tallest + LABEL_SIZE + 3,
                escape(&fit_label(tile.label, tiling.width))
            );
        }
        top += tallest + LABEL_HEIGHT + GAP;
    }
    out.push_str("</svg>");
    (out, width, height)
}

/// `label` cut to what fits `width` px at [`LABEL_SIZE`] (about 0.55 em a character), its
/// end an ellipsis.
fn fit_label(label: &str, width: i64) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let room = (((width - 4) as f64 / (LABEL_SIZE as f64 * 0.55)).floor() as usize).max(4);
    if label.chars().count() <= room {
        return label.to_owned();
    }
    let kept: String = label.chars().take(room - 1).collect();
    format!("{}…", kept.trim_end())
}

/// `<`, `>` and `&` escaped, as `xml.sax.saxutils.escape` escapes them.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
#[path = "tiles_tests.rs"]
mod tests;
