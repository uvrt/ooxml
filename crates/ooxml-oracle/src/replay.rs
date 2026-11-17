//! Owns: [`ReplayGlyphs`], the text engine's [`Glyphs`] port answered from what docx2svg's
//! breaker was answered (a replay record's `glyphs`): the engine compared alone, whatever
//! faces a test laid its document out with.
//!
//! Must not: answer anything docx2svg was not asked (an unasked advance is unknown, an
//! unasked kern pair zero).

use ooxml_base::Digest;
use ooxml_text::{Advance, FaceKey, Glyphs, ScriptMetrics};
use serde_json::Value;
use std::collections::HashMap;

type Face = (String, bool, bool);

/// Recorded answers.
#[derive(Debug, Default)]
pub struct ReplayGlyphs {
    advances: HashMap<(Face, char), Option<Advance>>,
    kerns: HashMap<(Face, char, char), Option<i32>>,
    metrics: HashMap<Face, Option<ScriptMetrics>>,
}

fn face(row: &Value) -> Option<Face> {
    Some((
        row.get(0)?.as_str()?.to_owned(),
        row.get(1)?.as_bool()?,
        row.get(2)?.as_bool()?,
    ))
}

fn ch(value: Option<&Value>) -> Option<char> {
    let text = value?.as_str()?;
    let mut chars = text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

fn small(value: Option<&Value>) -> Option<i32> {
    value.and_then(Value::as_i64).and_then(|v| i32::try_from(v).ok())
}

impl ReplayGlyphs {
    /// The answers of a record's `glyphs`.
    pub fn new(glyphs: &Value) -> ReplayGlyphs {
        let mut out = ReplayGlyphs::default();
        let rows = |name: &str| glyphs.get(name).and_then(Value::as_array).cloned().unwrap_or_default();
        for row in rows("advance") {
            let (Some(f), Some(c)) = (face(&row), ch(row.get(3))) else {
                continue;
            };
            let advance = row
                .get(4)
                .and_then(Value::as_u64)
                .zip(row.get(5).and_then(Value::as_u64));
            let advance = advance.and_then(|(a, u)| {
                Some(Advance {
                    advance: u32::try_from(a).ok()?,
                    units_per_em: u32::try_from(u).ok()?,
                })
            });
            out.advances.insert((f, c), advance);
        }
        for row in rows("kern") {
            let (Some(f), Some(l), Some(r)) = (face(&row), ch(row.get(3)), ch(row.get(4))) else {
                continue;
            };
            out.kerns.insert((f, l, r), small(row.get(5)));
        }
        for row in rows("metrics") {
            let Some(f) = face(&row) else { continue };
            let m = row.get(3).and_then(Value::as_array).and_then(|m| {
                Some(ScriptMetrics {
                    units_per_em: u32::try_from(m.first()?.as_u64()?).ok()?,
                    superscript_size: small(m.get(1)),
                    subscript_size: small(m.get(2)),
                    superscript_offset: small(m.get(3)),
                    subscript_offset: small(m.get(4)),
                })
            });
            out.metrics.insert(f, m);
        }
        out
    }
}

fn key(face: &FaceKey) -> Face {
    (face.family.to_string(), face.bold, face.italic)
}

impl Glyphs for ReplayGlyphs {
    fn advance(&self, face: &FaceKey, ch: char) -> Option<Advance> {
        self.advances.get(&(key(face), ch)).copied().flatten()
    }

    fn kern(&self, face: &FaceKey, left: char, right: char) -> Option<i32> {
        self.kerns.get(&(key(face), left, right)).copied().unwrap_or(Some(0))
    }

    fn script_metrics(&self, face: &FaceKey) -> Option<ScriptMetrics> {
        self.metrics.get(&key(face)).copied().flatten()
    }

    fn epoch(&self) -> Digest {
        Digest::default()
    }
}
