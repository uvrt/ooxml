//! Owns: reading docx2svg's differential records (`python/docx2svg/tools/text_differential.py`)
//! into the text engine's input, and their expected answers.
//!
//! Must not: measure or compare.

use ooxml_text::{
    BreakKind, CharFormat, FaceKey, Geometry, Hyphenation, Label, LabelAlign, LabelBody, LabelSuffix, Line, Mark,
    Paragraph, Run, RunFormats, TabAlign, Units, VerticalAlign,
};
use serde_json::Value;
use std::collections::HashMap;

/// Why a record could not be read.
pub type RecordError = String;

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, RecordError> {
    value.get(name).ok_or_else(|| format!("no {name:?} in {value}"))
}

fn int(value: &Value) -> Result<i64, RecordError> {
    value.as_i64().ok_or_else(|| format!("not an integer: {value}"))
}

fn units(value: &Value) -> Result<Units, RecordError> {
    value.as_str().ok_or_else(|| format!("not a length: {value}"))?.parse()
}

fn optional_units(value: Option<&Value>) -> Result<Option<Units>, RecordError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(v) => units(v).map(Some),
    }
}

fn flag(value: &Value, name: &str) -> bool {
    value.get(name).and_then(Value::as_bool).unwrap_or(false)
}

/// A document's character formats, by the number the record gives them.
pub type Formats = HashMap<i64, CharFormat>;

/// Add a `{"formats": [[id, face, bold, italic, half points, vertical align], ...]}` record.
pub fn add_formats(formats: &mut Formats, list: &Value) -> Result<(), RecordError> {
    for row in list.as_array().ok_or("formats: not a list")? {
        let get = |k: usize| row.get(k).ok_or_else(|| format!("short format row {row}"));
        let face = get(1)?.as_str().map(|family| FaceKey {
            family: family.into(),
            bold: get(2).ok().and_then(Value::as_bool).unwrap_or(false),
            italic: get(3).ok().and_then(Value::as_bool).unwrap_or(false),
        });
        let vertical_align = match get(5)?.as_str() {
            Some("superscript") => Some(VerticalAlign::Superscript),
            Some("subscript") => Some(VerticalAlign::Subscript),
            _ => None,
        };
        let half_points = u32::try_from(int(get(4)?)?).map_err(|e| e.to_string())?;
        formats.insert(
            int(get(0)?)?,
            CharFormat {
                face,
                half_points,
                vertical_align,
            },
        );
    }
    Ok(())
}

fn format(formats: &Formats, id: &Value) -> Result<CharFormat, RecordError> {
    formats.get(&int(id)?).cloned().ok_or_else(|| format!("no format {id}"))
}

fn mark(kind: &str) -> Result<Mark, RecordError> {
    Ok(match kind {
        "tab" => Mark::Tab,
        "br" | "textWrapping" | "cr" => Mark::Break(BreakKind::Line),
        "page" => Mark::Break(BreakKind::Page),
        "column" => Mark::Break(BreakKind::Column),
        "softHyphen" => Mark::SoftHyphen,
        "noBreakHyphen" => Mark::NoBreakHyphen,
        "drawing" => Mark::UnmeasuredDrawing,
        "separator" => Mark::Separator,
        "continuationSeparator" => Mark::ContinuationSeparator,
        other => match other.strip_prefix("drawing:") {
            Some(rest) => {
                let v: Vec<i64> = rest
                    .split(':')
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|e| format!("{e}"))?;
                let at = |k: usize| v.get(k).copied().unwrap_or_default();
                Mark::Drawing {
                    cx: at(0),
                    cy: at(1),
                    effect: [at(2), at(3), at(4), at(5)],
                }
            }
            None => Mark::Other,
        },
    })
}

fn run(formats: &Formats, value: &Value) -> Result<Run, RecordError> {
    let text = field(value, "text")?.as_str().unwrap_or_default().to_owned();
    let mut marks = Vec::new();
    for m in field(value, "marks")?.as_array().into_iter().flatten() {
        let at = usize::try_from(int(m.get(0).unwrap_or(&Value::Null))?).map_err(|e| e.to_string())?;
        marks.push((at, mark(m.get(1).and_then(Value::as_str).unwrap_or_default())?));
    }
    let blank = CharFormat {
        face: None,
        half_points: 0,
        vertical_align: None,
    };
    if flag(value, "vanish") {
        return Ok(Run {
            text,
            marks,
            vanish: true,
            formats: RunFormats::Uniform(blank.clone()),
            hyphen_format: blank,
            tracking: 0,
            scale: 100,
            kern: 0,
            caps: false,
            note_number: false,
            lang: None,
            no_proof: false,
        });
    }
    let ids = field(value, "formats")?.as_array().cloned().unwrap_or_default();
    let mut list: Vec<CharFormat> = ids.iter().map(|id| format(formats, id)).collect::<Result<_, _>>()?;
    let formats_of = if list.len() == 1 {
        RunFormats::Uniform(list.pop().unwrap_or(blank))
    } else {
        RunFormats::PerChar(list)
    };
    let number = |name: &str| value.get(name).and_then(Value::as_i64).unwrap_or_default();
    Ok(Run {
        text,
        marks,
        vanish: false,
        formats: formats_of,
        hyphen_format: format(formats, field(value, "hyphen_format")?)?,
        tracking: i32::try_from(number("tracking")).map_err(|e| e.to_string())?,
        scale: u32::try_from(number("scale")).map_err(|e| e.to_string())?,
        kern: u32::try_from(number("kern")).map_err(|e| e.to_string())?,
        caps: flag(value, "caps"),
        note_number: flag(value, "note_number"),
        lang: value.get("lang").and_then(Value::as_str).map(str::to_owned),
        no_proof: flag(value, "no_proof"),
    })
}

fn label(formats: &Formats, value: &Value) -> Result<Label, RecordError> {
    let body = match value.get("picture") {
        Some(Value::Null) => LabelBody::UnreadPicture,
        Some(picture) => LabelBody::Picture {
            half_points: u32::try_from(int(picture.get(0).unwrap_or(&Value::Null))?).map_err(|e| e.to_string())?,
            pixel_height: u32::try_from(int(picture.get(1).unwrap_or(&Value::Null))?).map_err(|e| e.to_string())?,
        },
        None => {
            let mut chars = Vec::new();
            for pair in field(value, "chars")?.as_array().into_iter().flatten() {
                let ch = pair
                    .get(0)
                    .and_then(Value::as_str)
                    .and_then(|s| s.chars().next())
                    .ok_or("label char")?;
                chars.push((ch, format(formats, pair.get(1).unwrap_or(&Value::Null))?));
            }
            LabelBody::Text(chars)
        }
    };
    Ok(Label {
        body,
        caps: flag(value, "caps"),
        alignment: match value.get("alignment").and_then(Value::as_str) {
            Some("center") => LabelAlign::Center,
            Some("right" | "end") => LabelAlign::Right,
            _ => LabelAlign::Left,
        },
        suffix: match value.get("suffix").and_then(Value::as_str) {
            Some("tab") => LabelSuffix::Tab,
            Some("space") => LabelSuffix::Space,
            _ => LabelSuffix::Nothing,
        },
        space_format: format(formats, field(value, "space_format")?)?,
    })
}

/// A paragraph record's input.
pub fn paragraph(formats: &Formats, value: &Value) -> Result<(Paragraph, Geometry, bool), RecordError> {
    let runs = field(value, "runs")?
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| run(formats, r))
        .collect::<Result<_, _>>()?;
    let label = match value.get("label") {
        None | Some(Value::Null) => None,
        Some(l) => Some(label(formats, l)?),
    };
    let hyphenation = match value.get("hyphenation") {
        None | Some(Value::Null) => None,
        Some(h) => Some(Hyphenation {
            mode15: flag(h, "mode15"),
            no_caps: flag(h, "no_caps"),
        }),
    };
    let g = field(value, "geometry")?;
    let mut tabs = Vec::new();
    for stop in field(g, "tabs")?.as_array().into_iter().flatten() {
        let align = stop
            .get(1)
            .and_then(Value::as_str)
            .and_then(TabAlign::parse)
            .ok_or("tab alignment")?;
        tabs.push((units(stop.get(0).unwrap_or(&Value::Null))?, align));
    }
    let geometry = Geometry {
        first_start: units(field(g, "first_start")?)?,
        start: units(field(g, "start")?)?,
        right: units(field(g, "right")?)?,
        tabs,
        default_tab: units(field(g, "default_tab")?)?,
        hanging_stop: optional_units(g.get("hanging_stop"))?,
        squeeze: flag(g, "squeeze"),
        tab_origin: units(field(g, "tab_origin")?)?,
        hyphen_zone: optional_units(g.get("hyphen_zone"))?,
        hyphen_next: flag(g, "hyphen_next"),
        hyphen_gap: optional_units(g.get("hyphen_gap"))?,
        hyphen_limit: u32::try_from(g.get("hyphen_limit").and_then(Value::as_i64).unwrap_or(0)).unwrap_or(0),
    };
    Ok((
        Paragraph {
            runs,
            label,
            hyphenation,
        },
        geometry,
        flag(value, "columns"),
    ))
}

/// One expected piece, as docx2svg made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedPiece {
    /// `g`, `s`, `t`, `b`, `h`, `o`.
    pub kind: String,
    /// The character (`None`: the label's shift, an empty string).
    pub ch: Option<char>,
    /// Width.
    pub width: Units,
    /// Source index.
    pub source: i64,
    /// Hyphen width.
    pub hyphen: Units,
    /// Run.
    pub run: i64,
    /// Automatic hyphen.
    pub auto: bool,
    /// `w:noProof` end.
    pub proof_end: Option<i64>,
    /// Object height.
    pub height: i64,
    /// The kern pair charged to it (`None`: unknown).
    pub kern: Option<Units>,
}

/// What docx2svg answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// Unmeasurable, with its message.
    Error(String),
    /// Pieces and lines.
    Measured(Vec<ExpectedPiece>, Vec<Line>),
}

fn expected_piece(value: &Value) -> Result<ExpectedPiece, RecordError> {
    let at = |k: usize| value.get(k).unwrap_or(&Value::Null);
    Ok(ExpectedPiece {
        kind: at(0).as_str().unwrap_or_default().to_owned(),
        ch: at(1).as_str().and_then(|s| s.chars().next()),
        width: units(at(2))?,
        source: int(at(3))?,
        hyphen: units(at(4))?,
        run: int(at(5))?,
        auto: at(6).as_bool().unwrap_or(false),
        proof_end: at(7).as_i64(),
        height: at(8).as_i64().unwrap_or_default(),
        kern: optional_units(Some(at(9)))?,
    })
}

/// A paragraph record's expected answer.
pub fn expected(value: &Value) -> Result<Expected, RecordError> {
    let e = field(value, "expected")?;
    if let Some(message) = e.get("error").and_then(Value::as_str) {
        return Ok(Expected::Error(message.to_owned()));
    }
    let pieces = field(e, "pieces")?
        .as_array()
        .into_iter()
        .flatten()
        .map(expected_piece)
        .collect::<Result<_, _>>()?;
    let mut lines = Vec::new();
    for l in field(e, "lines")?.as_array().into_iter().flatten() {
        let at = |k: usize| l.get(k).unwrap_or(&Value::Null);
        lines.push(Line {
            start: usize::try_from(int(at(0))?).map_err(|e| e.to_string())?,
            end: usize::try_from(int(at(1))?).map_err(|e| e.to_string())?,
            forced: at(2).as_bool().unwrap_or(false),
            emergency: at(3).as_bool().unwrap_or(false),
            hyphenated: at(4).as_bool().unwrap_or(false),
        });
    }
    Ok(Expected::Measured(pieces, lines))
}
