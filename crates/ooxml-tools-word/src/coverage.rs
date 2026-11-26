//! Owns: a version's coverage as the tools report it (docx-agent's
//! `layout.coverage_facts`, read by `check`'s `reflow` and by `render`): whether every
//! block was laid out, its status, the fonts substituted or missing, and the first
//! approximations and placeholders, each message shortened as docx-agent shortens it
//! ([`short`]), a placeholder's block named by its id where the version gives it one.
//!
//! Must not: lay out or read the engine (the coverage is given).

use ooxml_word_pages::coverage::Coverage;
use serde_json::{Map, Value};

/// The approximations, story stops and placeholders a coverage lists at most.
const LISTED: usize = 5;
/// How long a listed message or warning may be, characters.
pub(crate) const SHORT: usize = 160;

/// `text` with its white space runs made one space, cut to `limit` characters with an
/// ellipsis (docx-agent's `short`).
pub(crate) fn short(text: &str, limit: usize) -> String {
    let joined = text
        .split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.chars().count() <= limit {
        return joined;
    }
    let mut out: String = joined.chars().take(limit.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// `layout.coverage_facts()` of a version laid out in Rust, `id_of` naming a block by its key
/// (through the version's names) or, where it has none, by its
/// element path (`ids`).
pub(crate) fn coverage_facts(
    coverage: &Coverage,
    id_of: impl Fn(Option<ooxml_word_model::ids::BlockKey>, &str) -> Option<String>,
) -> Value {
    let mut out = Map::new();
    let status = if !coverage.complete {
        "partial"
    } else if coverage.approximations.is_empty() {
        "complete"
    } else {
        "approximate"
    };
    out.insert("complete".into(), Value::from(coverage.complete));
    out.insert("status".into(), Value::from(status));
    out.insert("pages".into(), Value::from(coverage.pages));
    out.insert(
        "blocks_laid_out".into(),
        Value::from(vec![coverage.laid_out, coverage.blocks]),
    );
    if !coverage.substituted.is_empty() {
        let named: Vec<String> = coverage
            .substituted
            .iter()
            .map(|s| {
                let approximate = if s.metric_compatible { "" } else { " (approximate)" };
                format!("{} -> {}{approximate}", s.family, s.substitute)
            })
            .collect();
        out.insert("substituted_fonts".into(), Value::from(named));
    }
    if !coverage.missing.is_empty() {
        out.insert("missing_fonts".into(), Value::from(coverage.missing.clone()));
    }
    if !coverage.approximations.is_empty() {
        let listed: Vec<Value> = coverage
            .approximations
            .iter()
            .take(LISTED)
            .map(|a| {
                let (reason, message) = a.listed();
                let mut entry = Map::new();
                entry.insert("page".into(), a.page.map_or(Value::Null, Value::from));
                entry.insert("reason".into(), Value::from(reason));
                entry.insert("path".into(), a.path.clone().map_or(Value::Null, Value::from));
                entry.insert("message".into(), Value::from(short(message, SHORT)));
                Value::Object(entry)
            })
            .collect();
        out.insert("approximations".into(), Value::from(listed));
        if coverage.approximations.len() > LISTED {
            out.insert(
                "approximations_total".into(),
                Value::from(coverage.approximations.len()),
            );
        }
    }
    if !coverage.placeholders.is_empty() {
        let listed: Vec<Value> = coverage
            .placeholders
            .iter()
            .take(LISTED)
            .map(|p| {
                let at = p
                    .path
                    .as_deref()
                    .map(|path| id_of(p.key, path).unwrap_or_else(|| path.to_owned()));
                let mut entry = Map::new();
                entry.insert("page".into(), p.page.map_or(Value::Null, Value::from));
                entry.insert("reason".into(), Value::from(p.reason()));
                entry.insert("at".into(), at.map_or(Value::Null, Value::from));
                entry.insert("message".into(), Value::from(short(&p.message, SHORT)));
                Value::Object(entry)
            })
            .collect();
        out.insert("placeholders".into(), Value::from(listed));
        if coverage.placeholders.len() > LISTED {
            out.insert("placeholders_total".into(), Value::from(coverage.placeholders.len()));
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_word_pages::coverage::glyph_approximation;
    use ooxml_word_pages::warnings::Approximation;

    fn coverage(approximations: Vec<Approximation>) -> Coverage {
        Coverage {
            complete: true,
            pages: 1,
            blocks: 1,
            laid_out: 1,
            approximations,
            ..Coverage::default()
        }
    }

    #[test]
    fn a_glyph_a_face_lacks_is_listed_as_docx_agent_lists_it() {
        // A face with no glyph for the character (no MS Gothic on Linux: simulated).
        let facts = coverage_facts(
            &coverage(vec![glyph_approximation("MS Gothic", '\u{2612}', ".notdef")]),
            |_, _| None,
        );
        assert_eq!(facts["status"], "approximate");
        assert_eq!(
            facts["approximations"][0],
            serde_json::json!({
                "page": null,
                "reason": "'MS Gothic' has no glyph for '\u{2612}': measured with its missing glyph",
                "path": null,
                "message": "",
            })
        );
        let other = coverage_facts(
            &coverage(vec![glyph_approximation("Calibri", 'x', "Carlito")]),
            |_, _| None,
        );
        assert_eq!(
            other["approximations"][0]["reason"],
            "'Calibri' has no glyph for 'x': measured with 'Carlito'"
        );
    }

    #[test]
    fn a_layout_approximation_is_listed_by_the_reason_its_code_gives() {
        let approximation = |code: &str| Approximation {
            code: code.to_owned(),
            message: "a frame (w:framePr) is  laid out\nin the flow".to_owned(),
            page: Some(2),
            path: Some("w:body/w:p[3]".to_owned()),
        };
        let facts = coverage_facts(
            &coverage(vec![approximation("layout-approximate:frame"), approximation("odd")]),
            |_, _| None,
        );
        assert_eq!(
            facts["approximations"][0],
            serde_json::json!({
                "page": 2,
                "reason": "frame",
                "path": "w:body/w:p[3]",
                "message": "a frame (w:framePr) is laid out in the flow",
            })
        );
        assert_eq!(facts["approximations"][1]["reason"], "odd");
    }
}
