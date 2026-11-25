//! Owns: a length in points as a tool gives it, rounded to the whole unit a command writes
//! (docx-agent's `emu` and `round(points * 20)`: half to even), so a tool converts at its
//! edge and the commands take [`Emu`] and [`Twips`].
//!
//! Must not: hold a length a command still checks in points (a chart's 1-1584 pt bound, a
//! picture's size from its width and the image's aspect): those stay points until the check
//! is made, so the refusal and its bound read as before.

pub use ooxml_base::units::{Emu, Twips};

/// EMU per point.
const EMU_PER_POINT: f64 = 12_700.0;

/// `points` as EMU, rounded half to even (docx-agent's `emu`).
#[must_use]
pub fn emu(points: f64) -> Emu {
    #[allow(clippy::cast_possible_truncation)]
    let value = (points * EMU_PER_POINT).round_ties_even() as i64;
    Emu(value)
}

/// `points` as twips, rounded half to even (Python's `round(points * 20)`).
#[must_use]
pub fn twips(points: f64) -> Twips {
    #[allow(clippy::cast_possible_truncation)]
    let value = (points * 20.0).round_ties_even() as i64;
    Twips(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_round_half_to_even() {
        assert_eq!(emu(1.0), Emu(12_700));
        assert_eq!(emu(-30.5), Emu(-387_350));
        assert_eq!(twips(0.125), Twips(2));
        assert_eq!(twips(0.375), Twips(8));
        assert_eq!(twips(12.5), Twips(250));
    }

    /// The conversions as the commands made them inside before they took units (frozen).
    #[allow(clippy::cast_possible_truncation)]
    fn frozen_emu(points: f64) -> i64 {
        (points * 12700.0).round_ties_even() as i64
    }

    #[allow(clippy::cast_possible_truncation)]
    fn frozen_twips(points: f64) -> i64 {
        (points * 20.0).round_ties_even() as i64
    }

    /// Every length in points the gates' cases give (`crates/ooxml-oracle/cases`), with
    /// odd values beside them, converts at the edge as the command converted it inside.
    #[test]
    fn every_length_of_the_cases_converts_as_before() {
        use std::path::{Path, PathBuf};
        const KEYS: [&str; 16] = [
            "width", "height", "x", "y", "indent", "page_width", "page_height", "top", "right", "bottom", "left",
            "number_at", "text_at", "tab_at", "size", "margins",
        ];
        fn files(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    files(&path, out);
                } else if path.extension().is_some_and(|e| e == "jsonl") {
                    out.push(path);
                }
            }
        }
        fn numbers(value: &serde_json::Value, key: &str, out: &mut Vec<f64>) {
            match value {
                serde_json::Value::Object(map) => map.iter().for_each(|(k, v)| numbers(v, k, out)),
                serde_json::Value::Array(items) => items.iter().for_each(|v| numbers(v, key, out)),
                serde_json::Value::Number(n) if KEYS.contains(&key) => out.extend(n.as_f64()),
                _ => {}
            }
        }
        let cases = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ooxml-oracle/cases");
        let mut paths = Vec::new();
        files(&cases, &mut paths);
        let mut lengths = vec![0.0, -0.0, 0.025, 0.05, 0.075, 1.0 / 3.0, 12.345_67, -30.5, 1584.0, 1e12, -1e12];
        for path in paths {
            for line in std::fs::read_to_string(&path).unwrap_or_default().lines() {
                if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
                    numbers(&entry["case"], "", &mut lengths);
                }
            }
        }
        let differ: Vec<f64> = lengths
            .iter()
            .copied()
            .filter(|&p| emu(p).0 != frozen_emu(p) || twips(p).0 != frozen_twips(p))
            .collect();
        assert!(differ.is_empty(), "converted otherwise: {differ:?}");
        assert!(lengths.len() > 50, "{} lengths compared", lengths.len());
    }
}
