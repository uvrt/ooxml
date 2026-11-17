//! The metafile players' differential (phase P3, lane P3b1): ooxml-common's drawings, as
//! `python/metafile_record.py` records them, against `ooxml-metafile`'s.
//!
//! Owns: reading a case (a metafile or a DIB, and what the Python drew of it), playing it in
//! Rust, and naming every difference by its cause. Embedded PNGs are compared twice: as
//! bytes, and by their header and inflated pixel rows -- the Rust PNGs are deflated by
//! zlib-rs, whose stream is not always the C zlib's byte for byte.
//!
//! Must not: be depended on by a shipped crate.

use flate2::read::ZlibDecoder;
use ooxml_base::{Digest, DigestKey};
use ooxml_metafile::{dib, draw, FontMapping, Options};
use std::io::Read;

/// What a case's comparison found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Equal byte for byte.
    Same,
    /// Equal but for the deflate bytes of embedded PNGs (their pixels are equal).
    SamePixels,
    /// Different: the cause and a detail.
    Different(String, String),
}

/// One recorded case.
#[derive(Debug, Clone)]
pub enum Case {
    /// A metafile: its bytes, the font mapping (`None` for the default), the drawing's
    /// metadata as JSON (`None` when Python returned none) and its SVG.
    Metafile {
        /// The metafile.
        data: Vec<u8>,
        /// The font mapping, `None` for the default.
        mapping: Option<Vec<(String, String)>>,
        /// The drawing's metadata, `None` when Python drew none.
        meta: Option<serde_json::Value>,
        /// Python's SVG.
        svg: Vec<u8>,
    },
    /// A DIB: header, bits, the image's type (`None` for no image) and bytes.
    Dib {
        /// The `BITMAPINFO` block.
        header: Vec<u8>,
        /// The pixels.
        bits: Vec<u8>,
        /// The image's type, `None` for no image.
        mime: Option<String>,
        /// The image's bytes.
        image: Vec<u8>,
    },
}

fn fields(bytes: &[u8]) -> Result<Vec<&[u8]>, String> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let len = bytes
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or("a truncated field length")? as usize;
        out.push(bytes.get(at + 4..at + 4 + len).ok_or("a truncated field")?);
        at += 4 + len;
    }
    Ok(out)
}

fn json(bytes: &[u8]) -> Result<Option<serde_json::Value>, String> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    Ok((!v.is_null()).then_some(v))
}

/// Reads a case file.
pub fn read_case(bytes: &[u8]) -> Result<Case, String> {
    let f = fields(bytes)?;
    let get = |i: usize| f.get(i).copied().ok_or_else(|| format!("field {i} missing"));
    match get(0)? {
        b"metafile" => {
            let mapping = json(get(2)?)?.map(|m| {
                m.as_object()
                    .map(|o| {
                        o.iter()
                            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned()))
                            .collect()
                    })
                    .unwrap_or_default()
            });
            Ok(Case::Metafile {
                data: get(1)?.to_vec(),
                mapping,
                meta: json(get(3)?)?,
                svg: get(4)?.to_vec(),
            })
        }
        b"dib" => {
            let meta = json(get(3)?)?;
            Ok(Case::Dib {
                header: get(1)?.to_vec(),
                bits: get(2)?.to_vec(),
                mime: meta.and_then(|m| m.get("mime").and_then(|v| v.as_str()).map(str::to_owned)),
                image: get(4)?.to_vec(),
            })
        }
        other => Err(format!("unknown case kind {:?}", String::from_utf8_lossy(other))),
    }
}

/// Plays a case in Rust and compares.
pub fn compare(case: &Case) -> Verdict {
    match case {
        Case::Metafile {
            data,
            mapping,
            meta,
            svg,
        } => compare_metafile(data, mapping.as_ref(), meta.as_ref(), svg),
        Case::Dib {
            header,
            bits,
            mime,
            image,
        } => {
            let ours = dib::to_image(header, bits);
            match (ours, mime) {
                (None, None) => Verdict::Same,
                (Some(o), Some(m)) if o.mime == m => compare_bytes(&o.data, image, o.mime == "image/png"),
                (o, m) => Verdict::Different("dib".into(), format!("rust {:?} python {m:?}", o.map(|o| o.mime))),
            }
        }
    }
}

fn compare_metafile(
    data: &[u8],
    mapping: Option<&Vec<(String, String)>>,
    meta: Option<&serde_json::Value>,
    svg: &[u8],
) -> Verdict {
    let options = Options {
        font_mapping: mapping.map_or_else(FontMapping::office_default, |m| FontMapping::new(m.iter().cloned())),
        ..Options::default()
    };
    let drawing = draw(data, &options);
    let (drawing, meta) = match (drawing, meta) {
        (None, None) => return Verdict::Same,
        (Some(d), Some(m)) => (d, m),
        (d, m) => return Verdict::Different("none".into(), format!("rust {} python {}", d.is_some(), m.is_some())),
    };
    let skipped: Vec<serde_json::Value> = drawing.skipped.iter().map(|(n, c)| serde_json::json!([n, c])).collect();
    let ours = serde_json::json!({
        "width": drawing.width, "height": drawing.height, "skipped": skipped,
        "emf_plus": drawing.emf_plus, "gdi_fallback": drawing.gdi_fallback,
        "truncated": drawing.truncated, "drawn": drawing.drawn,
    });
    for key in [
        "width",
        "height",
        "skipped",
        "emf_plus",
        "gdi_fallback",
        "truncated",
        "drawn",
    ] {
        let (a, b) = (ours.get(key), meta.get(key));
        // The size is written as text, parsed exactly here.
        let exact = b.and_then(|v| v.as_str()).and_then(|t| t.parse::<f64>().ok());
        let same = match (a.and_then(|v| v.as_f64()), exact.or_else(|| b.and_then(|v| v.as_f64()))) {
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        };
        if !same {
            return Verdict::Different(format!("meta:{key}"), format!("rust {a:?} python {b:?}"));
        }
    }
    compare_bytes(&drawing.svg(), svg, true)
}

fn compare_bytes(ours: &[u8], theirs: &[u8], pngs: bool) -> Verdict {
    if ours == theirs {
        return Verdict::Same;
    }
    if pngs {
        let (a, b) = (normalise(ours), normalise(theirs));
        if a == b {
            return Verdict::SamePixels;
        }
        let at = a
            .iter()
            .zip(&b)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(b.len()));
        let ctx = |v: &[u8]| {
            String::from_utf8_lossy(v.get(at.saturating_sub(60)..(at + 60).min(v.len())).unwrap_or(&[])).into_owned()
        };
        return Verdict::Different(
            "svg".into(),
            format!("at {at}:\n  rust   {}\n  python {}", ctx(&a), ctx(&b)),
        );
    }
    Verdict::Different("bytes".into(), format!("{} against {} bytes", ours.len(), theirs.len()))
}

const PNG_URI: &[u8] = b"data:image/png;base64,";

/// The SVG (or a PNG) with each PNG written as its header and inflated rows' digest.
pub fn normalise(svg: &[u8]) -> Vec<u8> {
    if svg.starts_with(b"\x89PNG") {
        return png_identity(svg).into_bytes();
    }
    let mut out = Vec::with_capacity(svg.len());
    let mut at = 0usize;
    while let Some(found) = find(svg, PNG_URI, at) {
        let start = found + PNG_URI.len();
        let end = svg
            .get(start..)
            .and_then(|s| s.iter().position(|&c| c == b'"'))
            .map_or(svg.len(), |p| start + p);
        out.extend_from_slice(svg.get(at..start).unwrap_or(&[]));
        let png = base64_decode(svg.get(start..end).unwrap_or(&[]));
        out.extend_from_slice(png_identity(&png).as_bytes());
        at = end;
    }
    out.extend_from_slice(svg.get(at..).unwrap_or(&[]));
    out
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn base64_decode(text: &[u8]) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for v in text.iter().filter_map(|&c| value(c)) {
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// A PNG as its `IHDR` and the digest of its inflated `IDAT` stream.
fn png_identity(png: &[u8]) -> String {
    let mut at = 8usize;
    let (mut ihdr, mut idat) = (Vec::new(), Vec::new());
    while at + 8 <= png.len() {
        let len = png
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map_or(0, u32::from_be_bytes) as usize;
        let tag = png.get(at + 4..at + 8).unwrap_or(&[]);
        let body = png.get(at + 8..(at + 8 + len).min(png.len())).unwrap_or(&[]);
        if tag == b"IHDR" {
            ihdr = body.to_vec();
        } else if tag == b"IDAT" {
            idat.extend_from_slice(body);
        }
        at += 12 + len;
    }
    let mut raw = Vec::new();
    let _ = ZlibDecoder::new(&idat[..]).read_to_end(&mut raw);
    let digest = Digest::of(DigestKey::new(1, 2), &raw);
    let head: String = ihdr.iter().map(|b| format!("{b:02x}")).collect();
    format!("png:{head}:{digest}")
}
