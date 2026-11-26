//! Owns: a scale as the caller gives it, read or refused with the way out (ooxml-edit's
//! `scales.parse`, ported exactly): its name, its steps -- each a fill and a text colour
//! (white, black, or the one that reads better), a label, and what it takes: a `min` (and
//! `max`), or one `value` -- its targets and what its texts show; the steps' order, gaps and
//! overlaps checked; what the reading has to say (a gap meant, a step's colours below WCAG's
//! 4.5:1).
//!
//! Must not: read a document.

use super::contrast::{hex_colour, ratio, readable_text, BLACK, TEXT_MINIMUM, WHITE};
use super::{folded, num, By, Scale, Step, DEFAULT_NAME, MAX_STEPS, MAX_TARGETS, PARAM};
use ooxml_base::pyfmt::{repr_float, round2};
use ooxml_tools::pyfmt::{repr, str_of};
use ooxml_tools::{Refusal, ToolError, WayOut};
use serde_json::{json, Map, Value};

/// The shape of a scale, in a refusal of one (`EXAMPLE`).
#[must_use]
pub fn example() -> Value {
    json!({"name": "Score",
        "steps": [{"min": 1, "fill": "C62828", "label": "Low"},
                  {"min": 3, "fill": "F9A825", "text": "black", "label": "Medium"},
                  {"min": 4, "fill": "2E7D32", "label": "High"}],
        "targets": ["<a table>/col2", "{{d:<dataset>/*/<column>}}"]})
}

/// `show` as an agent may send it (rule 1), and what it means (`_SHOW_SAID`).
fn show_said(given: &str) -> Option<bool> {
    match given {
        "" | "value" | "values" => Some(false),
        "label" | "labels" => Some(true),
        _ => None,
    }
}

/// A scale refused at `scale`, the example scale in `details` where `example`.
fn refused(message: impl Into<String>, example_given: bool, retry: &WayOut) -> ToolError {
    let error = ToolError::refusal(Refusal::InvalidArguments, message, retry.clone()).at(PARAM);
    if example_given {
        error.with_detail("example", example())
    } else {
        error
    }
}

/// Whether a scale shows its steps' labels: its `show` read as `label` (`reads_values`).
#[must_use]
pub fn shows_labels(given: &Value) -> bool {
    let shown = given.get("show").and_then(Value::as_str);
    shown.is_some_and(|s| show_said(&crate::facts::py_strip(s).to_lowercase()) == Some(true))
}

/// The scale `given` describes, or `invalid_arguments` saying what is wrong and the way out
/// (`way_out`: the call as given), with an example scale in `details` (`parse`).
///
/// # Errors
/// The refusal of a scale that is not one.
pub fn parse(given: &Value, way_out: &WayOut) -> Result<Scale, ToolError> {
    let refuse = |message: String, example: bool| refused(message, example, way_out);
    let Some(fields) = given.as_object() else {
        return Err(refuse(
            "scale is an object: {name, steps: [{min or value, fill, text, label}], targets}".into(),
            true,
        ));
    };
    let mut unknown: Vec<&str> = fields
        .keys()
        .map(String::as_str)
        .filter(|k| !["name", "steps", "targets", "show"].contains(k))
        .collect();
    unknown.sort_unstable();
    if !unknown.is_empty() {
        return Err(refuse(
            format!(
                "scale has no {}: it takes name, steps, targets and show",
                unknown.join(", ")
            ),
            true,
        ));
    }
    let labels = show_of(fields.get("show"), &refuse)?;
    let name = name_of(fields.get("name"), &refuse)?;
    let steps = match fields.get("steps") {
        Some(Value::Array(steps)) if !steps.is_empty() => steps,
        _ => {
            return Err(refuse(
                "scale steps: a list of at least one step, lowest first".into(),
                true,
            ))
        }
    };
    if steps.len() > MAX_STEPS {
        return Err(refuse(
            format!("{} steps: at most {MAX_STEPS} in one scale", steps.len()),
            false,
        ));
    }
    let targets = targets_of(fields.get("targets"), &refuse)?;
    let raw = raw_steps(steps, &refuse)?;
    let mut notes = Vec::new();
    let mut out = steps_of(&raw, &mut notes, &refuse)?;
    let ranges = raw.first().is_some_and(|s| !s.get("min").is_none_or(Value::is_null));
    let by = if ranges {
        ranged(&out, &mut notes, &refuse)?;
        By::Range
    } else {
        valued(&mut out, &refuse)?
    };
    if labels {
        labelled(&out, &refuse)?;
    }
    Ok(Scale {
        name,
        steps: out,
        targets,
        by,
        given: fields.clone(),
        notes,
        labels,
    })
}

type Refuse<'a> = dyn Fn(String, bool) -> ToolError + 'a;

/// Each step read, a note for each whose colours read below WCAG's minimum.
fn steps_of(raw: &[&Map<String, Value>], notes: &mut Vec<String>, refuse: &Refuse<'_>) -> Result<Vec<Step>, ToolError> {
    let mut out = Vec::new();
    for (k, step) in raw.iter().enumerate() {
        let made = step_of(k, step, refuse)?;
        if made.contrast < TEXT_MINIMUM {
            let rule = if made.rule == "auto" {
                ""
            } else {
                "; text auto chooses the higher of white and black"
            };
            notes.push(format!(
                "step {}: {} text on {} has contrast {}:1, below WCAG's {}:1 for text{rule}",
                k + 1,
                made.rule,
                made.fill,
                repr_float(made.contrast),
                repr_float(TEXT_MINIMUM)
            ));
        }
        out.push(made);
    }
    Ok(out)
}

/// `show`: whether the texts show labels.
fn show_of(shown: Option<&Value>, refuse: &Refuse<'_>) -> Result<bool, ToolError> {
    let said = match shown {
        None | Some(Value::Null) => String::new(),
        Some(value) => crate::facts::py_strip(&str_of(value)).to_lowercase(),
    };
    show_said(&said).ok_or_else(|| {
        refuse(
            format!(
                "scale show {}: value (the text is the value, the default) or label (the text becomes its step's label)",
                shown.map_or_else(|| "None".to_owned(), repr)
            ),
            false,
        )
    })
}

/// The scale's name: `Step` where none is given.
fn name_of(given: Option<&Value>, refuse: &Refuse<'_>) -> Result<String, ToolError> {
    let name = match given {
        None | Some(Value::Null) => DEFAULT_NAME.to_owned(),
        Some(Value::String(s)) if s.is_empty() => DEFAULT_NAME.to_owned(),
        Some(value) => crate::facts::py_strip(&str_of(value)).to_owned(),
    };
    let last_digit = name.chars().next_back().is_some_and(ooxml_base::pystr::is_decimal);
    if name.is_empty() || name.chars().count() > 40 || last_digit {
        return Err(refuse(
            format!(
                "scale name {}: 1 to 40 characters, not ending in a digit (the styles are named '<name> <step> <label>')",
                given.map_or_else(|| "None".to_owned(), repr)
            ),
            true,
        ));
    }
    Ok(name)
}

/// The scale's targets: one address or a list of them, none blank.
fn targets_of(given: Option<&Value>, refuse: &Refuse<'_>) -> Result<Vec<String>, ToolError> {
    let items: Option<Vec<&Value>> = match given {
        Some(one @ Value::String(_)) => Some(vec![one]),
        Some(Value::Array(items)) if !items.is_empty() => Some(items.iter().collect()),
        _ => None,
    };
    let targets: Option<Vec<String>> = items.and_then(|items| {
        items
            .iter()
            .map(|t| {
                t.as_str()
                    .filter(|s| !crate::facts::py_strip(s).is_empty())
                    .map(str::to_owned)
            })
            .collect()
    });
    let Some(targets) = targets else {
        return Err(refuse(
            "scale targets: the values to style, a list of addresses -- a paragraph, a range, a shape or a cell, \
             a table column (/col2), {{...}} references"
                .into(),
            true,
        ));
    };
    if targets.len() > MAX_TARGETS {
        return Err(refuse(
            format!(
                "{} targets: at most {MAX_TARGETS}; a table column (/col2) or a reference with * names many values at \
                 once",
                targets.len()
            ),
            false,
        ));
    }
    Ok(targets.iter().map(|t| crate::facts::py_strip(t).to_owned()).collect())
}

/// The steps as objects, each with the keys a step takes and a min or a value, all of one
/// kind.
fn raw_steps<'a>(steps: &'a [Value], refuse: &Refuse<'_>) -> Result<Vec<&'a Map<String, Value>>, ToolError> {
    let mut raw = Vec::new();
    for (k, step) in steps.iter().enumerate() {
        let Some(step) = step.as_object() else {
            return Err(refuse(
                format!("steps[{k}] is an object: {{min or value, fill, text, label}}"),
                true,
            ));
        };
        let mut extra: Vec<&str> = step
            .keys()
            .map(String::as_str)
            .filter(|key| !["min", "max", "value", "fill", "text", "label"].contains(key))
            .collect();
        extra.sort_unstable();
        if !extra.is_empty() {
            return Err(refuse(
                format!(
                    "steps[{k}] has no {}: a step takes min (and max) or value, fill, text and label",
                    extra.join(", ")
                ),
                true,
            ));
        }
        raw.push(step);
    }
    let kind = |s: &Map<String, Value>| {
        if !s.get("min").is_none_or(Value::is_null) {
            Some("min")
        } else if !matches!(s.get("value"), None | Some(Value::Null)) && s.get("value") != Some(&json!("")) {
            Some("value")
        } else {
            None
        }
    };
    let kinds: Vec<Option<&str>> = raw.iter().map(|s| kind(s)).collect();
    if let Some(missing) = kinds.iter().position(Option::is_none) {
        return Err(refuse(
            format!(
                "steps[{missing}] says no values: give min (a step from there up to the next step's min) or value (one \
                 number or text)"
            ),
            true,
        ));
    }
    if kinds.iter().any(|k| *k != kinds.first().copied().flatten()) {
        return Err(refuse(
            "steps mix min and value: give every step a min (ranges), or every step a value (exact numbers or texts)"
                .into(),
            true,
        ));
    }
    Ok(raw)
}

/// One step read: its colours, label, min and max, or value.
fn step_of(k: usize, step: &Map<String, Value>, refuse: &Refuse<'_>) -> Result<Step, ToolError> {
    let given = |key: &str| step.get(key).map_or_else(|| "None".to_owned(), repr);
    let fill = step
        .get("fill")
        .and_then(Value::as_str)
        .and_then(hex_colour)
        .ok_or_else(|| {
            refuse(
                format!(
                    "steps[{k}].fill {} is not a colour: give RRGGBB, e.g. 2E7D32",
                    given("fill")
                ),
                true,
            )
        })?;
    let rule = match step.get("text") {
        Some(value) if truthy(value) => crate::facts::py_strip(&str_of(value)).to_lowercase(),
        _ => "auto".to_owned(),
    };
    let (text, rule, contrast) = match rule.as_str() {
        "auto" => {
            let (text, contrast) = readable_text(&fill);
            (text.to_owned(), rule, contrast)
        }
        "white" | "black" => {
            let text = if rule == "white" { WHITE } else { BLACK };
            (text.to_owned(), rule, round2(ratio(text, &fill)))
        }
        other => match hex_colour(other).filter(|c| c == WHITE || c == BLACK) {
            Some(text) => {
                let rule = if text == WHITE { "white" } else { "black" }.to_owned();
                let contrast = round2(ratio(&text, &fill));
                (text, rule, contrast)
            }
            None => {
                return Err(refuse(
                    format!(
                        "steps[{k}].text {}: white, black or auto (the one that contrasts more with the fill)",
                        given("text")
                    ),
                    false,
                ))
            }
        },
    };
    let label = match step.get("label") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(value) => Some(words(&str_of(value))),
    };
    if label.as_ref().is_some_and(|l| l.chars().count() > 40) {
        return Err(refuse(format!("steps[{k}].label: at most 40 characters"), false));
    }
    let min = number(step.get("min"), &format!("steps[{k}].min"), refuse)?;
    let max = number(step.get("max"), &format!("steps[{k}].max"), refuse)?;
    if max.is_some() && min.is_none() {
        return Err(refuse(
            format!("steps[{k}].max with no min: a step's range is min to max; or give value"),
            true,
        ));
    }
    let value = match step.get("value") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(if *b { "True" } else { "False" }.to_owned()),
        Some(value) => Some(words(&str_of(value))),
    };
    Ok(Step {
        index: k,
        fill,
        text,
        rule,
        contrast,
        label,
        min,
        max,
        value,
        number: None,
    })
}

/// Python's `" ".join(text.split())`.
fn words(text: &str) -> String {
    text.split(crate::facts::is_py_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// A step's `min` or `max` as a number (`_number`).
fn number(value: Option<&Value>, at: &str, refuse: &Refuse<'_>) -> Result<Option<f64>, ToolError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => Ok(n.as_f64()),
        Some(other) => as_number(&str_of(other))
            .map(Some)
            .ok_or_else(|| refuse(format!("{at} {} is not a number", repr(other)), false)),
    }
}

/// A text read as Python's `float` reads it, `−` as `-` (`_as_number`).
pub(crate) fn as_number(text: &str) -> Option<f64> {
    let text = crate::facts::py_strip(text).replace('\u{2212}', "-");
    let lower = text.to_ascii_lowercase();
    let body = lower.strip_prefix(['-', '+']).unwrap_or(&lower);
    if matches!(body, "inf" | "infinity" | "nan") {
        return lower.parse().ok().or_else(|| text.parse().ok());
    }
    if text.is_empty()
        || !text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
    {
        return None;
    }
    text.parse().ok()
}

/// A scale of ranges' steps checked: lowest first, no max below its min, no overlap; each
/// gap meant noted.
fn ranged(out: &[Step], notes: &mut Vec<String>, refuse: &Refuse<'_>) -> Result<(), ToolError> {
    for pair in out.windows(2) {
        let [a, b] = pair else { continue };
        if let (Some(a_min), Some(b_min)) = (a.min, b.min) {
            if b_min <= a_min {
                let mut order: Vec<f64> = out.iter().map(|s| s.min.unwrap_or(0.0)).collect();
                order.sort_by(f64::total_cmp);
                let listed: Vec<String> = order.iter().map(|m| repr_float(*m)).collect();
                return Err(refuse(
                    format!(
                        "steps are listed lowest first, numbered as listed: step {}'s min {} is not above step {}'s {}; \
                         in order the mins are [{}]",
                        b.n(),
                        num(b_min),
                        a.n(),
                        num(a_min),
                        listed.join(", ")
                    ),
                    false,
                ));
            }
        }
    }
    for step in out {
        if let (Some(max), Some(min)) = (step.max, step.min) {
            if max < min {
                return Err(refuse(
                    format!("step {}: max {} is below its min {}", step.n(), num(max), num(min)),
                    false,
                ));
            }
        }
    }
    for pair in out.windows(2) {
        let [a, b] = pair else { continue };
        if let (Some(a_max), Some(b_min)) = (a.max, b.min) {
            if a_max >= b_min {
                return Err(refuse(
                    format!(
                        "step {}'s max {} reaches step {}'s min {}: steps overlap; end it below the next min, or leave \
                         max out",
                        a.n(),
                        num(a_max),
                        b.n(),
                        num(b_min)
                    ),
                    false,
                ));
            }
            notes.push(format!(
                "values above {} and below {} are in no step (step {}'s max): skipped",
                num(a_max),
                num(b_min),
                a.n()
            ));
        }
    }
    Ok(())
}

/// A scale of exact values' steps checked: numbers -- each once, lowest first, no whole
/// number skipped -- or texts, each once.
fn valued(out: &mut [Step], refuse: &Refuse<'_>) -> Result<By, ToolError> {
    let numbers: Option<Vec<f64>> = out.iter().map(|s| s.value.as_deref().and_then(as_number)).collect();
    let Some(values) = numbers else {
        let mut seen: Vec<String> = Vec::new();
        for step in out.iter() {
            let key = crate::numbers::casefold(step.value.as_deref().unwrap_or_default());
            if seen.contains(&key) {
                return Err(refuse(
                    format!(
                        "two steps take {}: each text is one step's",
                        ooxml_base::pyfmt::repr_str(step.value.as_deref().unwrap_or_default())
                    ),
                    false,
                ));
            }
            seen.push(key);
        }
        return Ok(By::Text);
    };
    for (step, value) in out.iter_mut().zip(&values) {
        step.number = Some(*value);
    }
    let mut sorted = values.clone();
    sorted.sort_by(f64::total_cmp);
    sorted.dedup();
    if sorted.len() < values.len() {
        return Err(refuse(
            "two steps take the same value: each value is one step's".into(),
            false,
        ));
    }
    let listed = |v: &[f64]| format!("[{}]", v.iter().map(|n| repr_float(*n)).collect::<Vec<_>>().join(", "));
    if values != sorted {
        return Err(refuse(
            format!(
                "steps are listed lowest first, numbered as listed: the values are {}, in order {}",
                listed(&values),
                listed(&sorted)
            ),
            false,
        ));
    }
    if values.iter().all(|v| v.fract() == 0.0) {
        let skipped = values.windows(2).find(|w| matches!(w, [v, w] if w - v > 1.0));
        if let Some([v, _]) = skipped {
            let whole: Vec<String> = values.iter().map(|v| num(*v)).collect();
            return Err(refuse(
                format!(
                    "no step takes {}: whole values {} skip it. Add its step, or give min/max ranges where a gap is meant",
                    num(v.trunc() + 1.0),
                    whole.join(", ")
                ),
                false,
            ));
        }
    }
    Ok(By::Value)
}

/// A scale that shows labels: every step labelled, no two alike.
fn labelled(out: &[Step], refuse: &Refuse<'_>) -> Result<(), ToolError> {
    if let Some(unnamed) = out.iter().find(|s| s.label.is_none()) {
        return Err(refuse(
            format!(
                "show label writes each step's label, and step {} has none: give every step a label",
                unnamed.n()
            ),
            false,
        ));
    }
    let mut seen: Vec<(String, usize)> = Vec::new();
    for step in out {
        let label = step.label.clone().unwrap_or_default();
        let key = folded(&label);
        if let Some((_, twin)) = seen.iter().find(|(k, _)| *k == key) {
            return Err(refuse(
                format!(
                    "steps {twin} and {} are both labelled {}: with show label a label names one step",
                    step.n(),
                    ooxml_base::pyfmt::repr_str(&label)
                ),
                false,
            ));
        }
        seen.push((key, step.n()));
    }
    Ok(())
}
