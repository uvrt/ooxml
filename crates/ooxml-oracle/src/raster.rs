//! Owns: the PNG differential's Rust side -- a call to resvg-py as
//! `python/raster_record.py` wrote it ([`Call`]), the same call drawn by `ooxml-raster`
//! ([`replay`]), and the two compared pixel for pixel ([`compare`]).
//!
//! Must not: change what is drawn; the fonts and options are the call's.

use ooxml_raster::{rasterise, Families, Fit, FontLoader, FontSet, RasterOptions, Rgba};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tiny_skia::Pixmap;

/// One recorded call to resvg-py's `svg_to_bytes`.
#[derive(Debug, Clone)]
pub struct Call {
    /// The document the renderer drew, and the call's number within it.
    pub document: String,
    /// The call's number within the document.
    pub number: u64,
    /// The SVG handed over.
    pub svg: String,
    /// resvg-py's PNG.
    pub png: Vec<u8>,
    /// The size asked for.
    pub fit: Fit,
    /// The background.
    pub background: Option<Rgba>,
    /// The font files, in order.
    pub files: Vec<PathBuf>,
    /// The font folders, in order.
    pub folders: Vec<PathBuf>,
    /// Whether the system's fonts take part.
    pub system: bool,
    /// The generic families.
    pub families: Families,
    /// resvg-py's time, seconds.
    pub python_seconds: f64,
}

fn strings(value: Option<&Value>) -> Vec<PathBuf> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(PathBuf::from).collect())
        .unwrap_or_default()
}

fn families(case: &Value) -> Families {
    let mut out = Families::platform_default();
    let named = |key: &str| case.get(key).and_then(Value::as_str).map(str::to_owned);
    let slots: [(&str, &mut String); 6] = [
        ("font_family", &mut out.default),
        ("serif_family", &mut out.serif),
        ("sans_serif_family", &mut out.sans_serif),
        ("cursive_family", &mut out.cursive),
        ("fantasy_family", &mut out.fantasy),
        ("monospace_family", &mut out.monospace),
    ];
    for (key, slot) in slots {
        if let Some(value) = named(key) {
            *slot = value;
        }
    }
    out
}

fn fit(case: &Value) -> Fit {
    let int = |key: &str| {
        case.get(key)
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
    };
    match (int("width"), int("height")) {
        (Some(w), Some(h)) => Fit::Size(w, h),
        (Some(w), None) => Fit::Width(w),
        (None, Some(h)) => Fit::Height(h),
        #[allow(clippy::cast_possible_truncation)]
        (None, None) => case
            .get("zoom")
            .and_then(Value::as_f64)
            .map_or(Fit::Original, |z| Fit::Zoom(z as f32)),
    }
}

fn background(case: &Value) -> Option<Rgba> {
    let channels: Vec<u8> = case
        .get("background")?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_u64().and_then(|c| u8::try_from(c).ok()))
        .collect();
    let [red, green, blue, alpha] = channels.as_slice() else {
        return None;
    };
    Some(Rgba {
        red: *red,
        green: *green,
        blue: *blue,
        alpha: *alpha,
    })
}

/// The call recorded as `stem.json`, `stem.svg` and `stem.png`.
///
/// # Errors
/// A file cannot be read or the JSON is not the recorder's.
pub fn read_call(stem: &Path) -> Result<Call, String> {
    let read = |ext: &str| std::fs::read(stem.with_extension(ext)).map_err(|e| format!("{ext}: {e}"));
    let case: Value = serde_json::from_slice(&read("json")?).map_err(|e| e.to_string())?;
    Ok(Call {
        document: case
            .get("document")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        number: case.get("call").and_then(Value::as_u64).unwrap_or_default(),
        svg: String::from_utf8(read("svg")?).map_err(|e| e.to_string())?,
        png: read("png")?,
        fit: fit(&case),
        background: background(&case),
        files: strings(case.get("font_files")),
        folders: strings(case.get("font_dirs")),
        system: !case.get("skip_system_fonts").and_then(Value::as_bool).unwrap_or(false),
        families: families(&case),
        python_seconds: case.get("python_seconds").and_then(Value::as_f64).unwrap_or_default(),
    })
}

/// What a replay drew, and how long the font set and the drawing took.
#[derive(Debug, Clone)]
pub struct Replayed {
    /// The PNG.
    pub png: Vec<u8>,
    /// Building the font set, seconds (done once per font setup by a caller that keeps it).
    pub fonts_seconds: f64,
    /// Drawing and encoding, seconds.
    pub draw_seconds: f64,
}

/// `call` drawn by `ooxml-raster`, its fonts set up as resvg-py set them up (`system`: the
/// system's fonts, read once by the caller).
///
/// # Errors
/// The drawing fails.
pub fn replay(call: &Call, system: &FontSet) -> Result<Replayed, String> {
    let start = Instant::now();
    let files: Vec<&Path> = call.files.iter().map(PathBuf::as_path).collect();
    let folders: Vec<&Path> = call.folders.iter().map(PathBuf::as_path).collect();
    let fonts = FontLoader::resvg_py(call.system.then_some(system), &files, &folders, call.families.clone());
    let fonts_seconds = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let options = RasterOptions {
        fit: call.fit,
        background: call.background,
    };
    let raster = rasterise(&call.svg, &fonts, &options).map_err(|e| e.to_string())?;
    let png = raster.png().map_err(|e| e.to_string())?;
    Ok(Replayed {
        png,
        fonts_seconds,
        draw_seconds: start.elapsed().as_secs_f64(),
    })
}

/// How two PNGs' pixels differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PixelDiff {
    /// Pixels in which any channel differs.
    pub pixels: usize,
    /// The largest difference of one channel.
    pub largest: u8,
}

/// `ours` against `theirs`, decoded: `None` where their sizes differ or one does not decode.
pub fn compare(ours: &[u8], theirs: &[u8]) -> Option<PixelDiff> {
    let (a, b) = (Pixmap::decode_png(ours).ok()?, Pixmap::decode_png(theirs).ok()?);
    if (a.width(), a.height()) != (b.width(), b.height()) {
        return None;
    }
    let mut diff = PixelDiff::default();
    for (p, q) in a.data().as_chunks::<4>().0.iter().zip(b.data().as_chunks::<4>().0) {
        let largest = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        if largest > 0 {
            diff.pixels += 1;
            diff.largest = diff.largest.max(largest);
        }
    }
    Some(diff)
}
