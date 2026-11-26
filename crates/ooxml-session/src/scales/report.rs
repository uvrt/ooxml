//! Owns: a scale's check (ooxml-edit's `scales.read`, `report`, `_fix` and `summary`, ported
//! exactly): each target's value -- from its text, or for a scale of labels from the label it
//! shows and the record kept with it, its source read again where the session can -- and its
//! step; the values styled wrong or not at all, the texts that are not their value's label,
//! the labels with no value recorded, capped; the one `style_by_value` call that restyles
//! them.
//!
//! Must not: read a document (its format gives the values), or style one.

use super::{folded, num, quoted, read_value, By, Scale, Step, CHECK_LISTED, MAX_TARGETS, PARAM, SKIPPED_LISTED, TOOL};
use ooxml_numfmt::Locale;
use serde_json::{Map, Value};

/// One value a format reads at a target (`ScaleValue`): where it is, as written; the target
/// as the scale names it where that named many values (a column, a reference); the step it
/// is styled with now and whether exactly so (else what differs); why it cannot be styled;
/// what a reader is told of its place; the label record kept where it is; the reference it
/// was written from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScaleValue {
    /// Where it is, as the format's tools take it.
    pub target: String,
    /// The value as written.
    pub text: String,
    /// The target as the scale names it, where that named many values.
    pub given: Option<String>,
    /// The step it is styled with now (`None`: none of this scale's).
    pub now: Option<usize>,
    /// Whether it is styled as that step says.
    pub exact: bool,
    /// What is not (`fill FFC000`).
    pub differs: Option<String>,
    /// Why it cannot be styled.
    pub skip: Option<String>,
    /// What a reader is told of its place (`{"page": 3}`).
    pub place: Map<String, Value>,
    /// The label record kept where it is.
    pub kept: Option<Map<String, Value>>,
    /// The reference it was written from.
    pub source: Option<String>,
}

/// What a scale's check reads beside the values: the call's locale, its document's name, and
/// what a reference writes now (`Call.fact`).
pub trait ScaleEnv {
    /// The call's locale.
    fn locale(&self) -> &Locale;
    /// The call's document, as the call names it.
    fn doc(&self) -> Option<&str>;
    /// The text the reference `address` writes now, or why it could not be read.
    ///
    /// # Errors
    /// The refusal's message.
    fn fact(&self, address: &str) -> Result<String, String>;
}

/// A value of a scale: a number, or a step's text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Held {
    Number(f64),
    Text(String),
}

impl Held {
    fn of(value: &Value) -> Option<Held> {
        match value {
            Value::Number(n) => n.as_f64().map(Held::Number),
            Value::String(s) => Some(Held::Text(s.clone())),
            Value::Bool(b) => Some(Held::Text(if *b { "True" } else { "False" }.to_owned())),
            _ => None,
        }
    }

    /// As a reply says it (`_said`): a number in the locale, a text as it is.
    fn said(&self, locale: &Locale) -> String {
        match self {
            Held::Text(text) => text.clone(),
            Held::Number(n) => {
                let text = num(*n);
                if text.contains('e') {
                    text
                } else {
                    text.replace('.', &locale.decimal_separator.to_string())
                }
            }
        }
    }

    /// `_same_value`.
    fn same(&self, other: &Value) -> bool {
        match (self, Held::of(other)) {
            (Held::Number(a), Some(Held::Number(b))) if !other.is_boolean() => {
                (a - b).abs() <= 1e-12_f64.max(1e-12 * a.abs().max(b.abs()))
            }
            (_, None) => false,
            (a, Some(b)) => folded(&a.text()) == folded(&b.text()),
        }
    }

    fn text(&self) -> String {
        match self {
            Held::Text(t) => t.clone(),
            Held::Number(n) => ooxml_base::pyfmt::repr_float(*n),
        }
    }
}

impl Scale {
    /// The step whose label `text` is (case and surrounding spaces aside).
    pub(crate) fn step_of_label(&self, text: &str) -> Option<&Step> {
        let wanted = folded(text);
        self.steps
            .iter()
            .find(|s| s.label.as_deref().is_some_and(|l| folded(l) == wanted))
    }

    /// `text` as a value of this scale: a number, or a step's text (`value_of`).
    pub(crate) fn value_of(&self, text: &str, locale: &Locale) -> Option<Held> {
        if self.by == By::Text {
            let wanted = folded(text);
            return self
                .steps
                .iter()
                .find_map(|s| s.value.as_deref().filter(|v| folded(v) == wanted))
                .map(|v| Held::Text(v.to_owned()));
        }
        read_value(text, locale).map(Held::Number)
    }

    /// The step `text` (a value as written) is due, or why not (`due`).
    pub(crate) fn due(&self, text: &str, locale: &Locale) -> Result<&Step, String> {
        if self.by == By::Text {
            let wanted = folded(text);
            return self
                .steps
                .iter()
                .find(|s| s.value.as_deref().is_some_and(|v| folded(v) == wanted))
                .ok_or_else(|| {
                    let takes: Vec<String> = self
                        .steps
                        .iter()
                        .map(|s| ooxml_base::pyfmt::repr_str(s.value.as_deref().unwrap_or_default()))
                        .collect();
                    format!("{} is no step's text; steps take {}", quoted(text), takes.join(", "))
                });
        }
        match read_value(text, locale) {
            Some(number) => self.due_number(number),
            None => Err(format!("{} is not one number", quoted(text))),
        }
    }

    /// The step a value -- a number, or a scale of texts' text -- is due (`due_value`).
    pub(crate) fn due_value(&self, value: &Held, locale: &Locale) -> Result<&Step, String> {
        match value {
            Held::Text(text) if self.by == By::Text => self.due(text, locale),
            Held::Text(text) => Err(format!("{} is not one number", quoted(text))),
            Held::Number(n) => self.due_number(*n),
        }
    }

    /// The step a number is due, or why not (`due_number`).
    pub(crate) fn due_number(&self, number: f64) -> Result<&Step, String> {
        if self.by == By::Value {
            return self
                .steps
                .iter()
                .find(|s| s.number.is_some_and(|n| (n - number).abs() <= 1e-9))
                .ok_or_else(|| {
                    let takes: Vec<String> = self.steps.iter().filter_map(|s| s.number.map(num)).collect();
                    format!("{} is no step's value; steps take {}", num(number), takes.join(", "))
                });
        }
        let first = self.steps.first().ok_or_else(|| "no steps".to_owned())?;
        if let Some(min) = first.min.filter(|m| number < *m) {
            return Err(format!("{} is below the first step's min {}", num(number), num(min)));
        }
        let chosen = self
            .steps
            .iter()
            .rfind(|s| s.min.is_some_and(|m| m <= number))
            .unwrap_or(first);
        match chosen.max.filter(|m| number > *m) {
            None => Ok(chosen),
            Some(max) => {
                let following = self.steps.get(chosen.index + 1);
                let place = match following.and_then(|f| f.min.map(|m| (f, m))) {
                    Some((next, min)) => format!(
                        "between step {}'s max {} and step {}'s min {}",
                        chosen.n(),
                        num(max),
                        next.n(),
                        num(min)
                    ),
                    None => format!("above the last step's max {}", num(max)),
                };
                Err(format!("{} is in no step: {place}", num(number)))
            }
        }
    }
}

/// A target's value as [`read`] found it (`Reading`).
#[derive(Debug, Default)]
struct Reading<'s> {
    value: Option<Held>,
    said: String,
    step: Option<&'s Step>,
    why: Option<String>,
    label: bool,
    source: Option<String>,
    changed: Option<String>,
    recorded: bool,
    unread: Option<String>,
    unrecorded: Option<&'s Step>,
    several: Vec<&'s Step>,
}

/// The value `value`'s target stands for and its step (`read`).
fn read<'s>(env: &dyn ScaleEnv, scale: &'s Scale, value: &ScaleValue) -> Reading<'s> {
    let locale = env.locale();
    let text = value.text.as_str();
    let due = |step: Result<&'s Step, String>| match step {
        Ok(step) => (Some(step), None),
        Err(why) => (None, Some(why)),
    };
    if !scale.labels {
        let (step, mut why) = due(scale.due(text, locale));
        if step.is_none() {
            if let Some(kept) = value.kept.as_ref().filter(|k| !k.is_empty()) {
                let kept_step = kept
                    .get("step")
                    .map_or_else(|| "None".to_owned(), ooxml_tools::pyfmt::str_of);
                why = Some(format!(
                    "{}; it is step {kept_step}'s label: give the scale show label",
                    why.unwrap_or_default()
                ));
            }
        }
        return Reading {
            value: scale.value_of(text, locale),
            said: text.to_owned(),
            step,
            why,
            source: value.source.clone(),
            ..Reading::default()
        };
    }
    if let Some(shown) = scale.value_of(text, locale) {
        let (step, why) = due(scale.due_value(&shown, locale));
        return Reading {
            value: Some(shown),
            said: text.to_owned(),
            step,
            why,
            source: value.source.clone(),
            ..Reading::default()
        };
    }
    if let Some(reading) = recorded(env, scale, value) {
        return reading;
    }
    unlabelled(scale, text)
}

/// A label's value read from the record kept with it: its source read again now where the
/// session can, else the value recorded.
fn recorded<'s>(env: &dyn ScaleEnv, scale: &'s Scale, value: &ScaleValue) -> Option<Reading<'s>> {
    let locale = env.locale();
    let kept = value.kept.as_ref()?;
    let recorded = kept.get("value").filter(|v| !v.is_null())?;
    let held = Held::of(recorded)?;
    let source = kept.get("source").and_then(Value::as_str).map(str::to_owned);
    let mut reading = Reading {
        said: held.said(locale),
        value: Some(held.clone()),
        label: true,
        source: source.clone(),
        ..Reading::default()
    };
    if let Some(source) = source {
        match source_value(env, scale, &source) {
            Err(unread) => {
                reading.recorded = true;
                reading.unread = Some(unread);
            }
            Ok(now) => {
                if !now.same(recorded) {
                    reading.changed = Some(format!("recorded {}, now {}", held.said(locale), now.said(locale)));
                }
                reading.said = now.said(locale);
                reading.value = Some(now);
            }
        }
    }
    match reading.value.as_ref().map(|v| scale.due_value(v, locale)) {
        Some(Ok(step)) => reading.step = Some(step),
        Some(Err(why)) => reading.why = Some(why),
        None => {}
    }
    Some(reading)
}

/// A text that is no value and has no record: a step's label (unrecorded), the labels of
/// several steps one per line (a merged cell), or nothing this scale reads.
fn unlabelled<'s>(scale: &'s Scale, text: &str) -> Reading<'s> {
    if let Some(step) = scale.step_of_label(text) {
        return Reading {
            said: text.to_owned(),
            label: true,
            unrecorded: Some(step),
            why: Some(format!(
                "{} is step {}'s label, and no value is recorded for it: write its value there (typed, or by \
                 reference {{{{d:...}}}}), then label it",
                quoted(text),
                step.n()
            )),
            ..Reading::default()
        };
    }
    let lines: Vec<&str> = text
        .split('\n')
        .filter(|l| !crate::facts::py_strip(l).is_empty())
        .collect();
    let steps: Vec<Option<&Step>> = lines.iter().map(|l| scale.step_of_label(l)).collect();
    if lines.len() > 1 && steps.iter().all(Option::is_some) {
        let held: Vec<&Step> = steps.into_iter().flatten().collect();
        let numbers: Vec<String> = held.iter().map(|s| s.n().to_string()).collect();
        return Reading {
            said: text.to_owned(),
            label: true,
            unrecorded: held.first().copied(),
            why: Some(format!(
                "{} holds the labels of steps {}, one per line, and no value is recorded for it: write its one value \
                 there (typed, or by reference {{{{d:...}}}}), then label it",
                quoted(text),
                numbers.join(", ")
            )),
            several: held,
            ..Reading::default()
        };
    }
    Reading {
        said: text.to_owned(),
        why: Some(format!("{} is not one number, nor a step's label", quoted(text))),
        ..Reading::default()
    }
}

/// The value `source` (a reference, without braces) names now, as this scale reads it, or
/// why it could not be read (`_source_value`).
fn source_value(env: &dyn ScaleEnv, scale: &Scale, source: &str) -> Result<Held, String> {
    let text = env.fact(source)?;
    scale
        .value_of(&text, env.locale())
        .ok_or_else(|| format!("{source} now reads {}, no value of this scale", quoted(&text)))
}

/// Whether a text reads as a label: the same, spaces around and between aside (`_same_text`).
fn same_text(a: &str, b: &str) -> bool {
    fn words(t: &str) -> Vec<&str> {
        t.split(crate::facts::is_py_space).filter(|w| !w.is_empty()).collect()
    }
    words(a) == words(b)
}

/// Whether a label scale's text is its value's label, and its record says so (`_label_right`).
fn label_right(value: &ScaleValue, reading: &Reading<'_>, step: &Step) -> bool {
    let Some(kept) = &value.kept else { return false };
    let label = step.label.as_deref().unwrap_or_default();
    let same_value = match (&reading.value, kept.get("value")) {
        (Some(held), Some(recorded)) if !recorded.is_null() => held.same(recorded),
        _ => false,
    };
    let kept_source = kept.get("source").and_then(Value::as_str).filter(|s| !s.is_empty());
    same_text(&value.text, label) && same_value && kept_source == reading.source.as_deref().filter(|s| !s.is_empty())
}

/// Why a label scale's text is not its value's label (`_label_why`).
fn label_why(value: &ScaleValue, reading: &Reading<'_>, step: &Step) -> String {
    if !reading.label {
        return if value.kept.is_none() {
            "shows its value, not its label".to_owned()
        } else {
            "shows a value typed over its label".to_owned()
        };
    }
    if let Some(changed) = &reading.changed {
        return format!("its value changed: {changed}");
    }
    let kept = value.kept.clone().unwrap_or_default();
    if let Some(written) = kept.get("label").and_then(Value::as_str) {
        if !same_text(&value.text, written) {
            return format!("edited: written {}", quoted(written));
        }
        if Some(written) != step.label.as_deref() {
            let at = kept
                .get("step")
                .map_or_else(|| "None".to_owned(), ooxml_tools::pyfmt::str_of);
            return format!("the steps changed: written as step {at}'s label {}", quoted(written));
        }
    }
    "not its value's label".to_owned()
}

/// One value's entry in the fact: its target, the scale's target it came from, its place.
fn entry_of(value: &ScaleValue) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("target".into(), Value::from(value.target.as_str()));
    if let Some(given) = value.given.as_deref().filter(|g| !g.is_empty() && *g != value.target) {
        out.insert("from".into(), Value::from(given));
    }
    for (key, place) in &value.place {
        out.insert(key.clone(), place.clone());
    }
    out
}

fn with(mut entry: Map<String, Value>, pairs: &[(&str, Value)]) -> Map<String, Value> {
    for (key, value) in pairs {
        entry.insert((*key).to_owned(), value.clone());
    }
    entry
}

/// What a check found, list by list.
#[derive(Default)]
struct Found {
    wrong: Vec<Map<String, Value>>,
    none: Vec<Map<String, Value>>,
    labels: Vec<Map<String, Value>>,
    unrecorded: Vec<Map<String, Value>>,
    skipped: Vec<Map<String, Value>>,
    recorded: usize,
    right: usize,
}

/// One value checked into `found`.
fn checked(env: &dyn ScaleEnv, scale: &Scale, value: &ScaleValue, found: &mut Found) {
    let entry = entry_of(value);
    if let Some(skip) = &value.skip {
        found.skipped.push(with(entry, &[("why", Value::from(skip.as_str()))]));
        return;
    }
    let reading = read(env, scale, value);
    if let Some(step) = reading.unrecorded {
        let label_of = if reading.several.is_empty() {
            Value::from(step.n())
        } else {
            Value::from(reading.several.iter().map(|s| s.n()).collect::<Vec<_>>())
        };
        found.unrecorded.push(with(
            entry,
            &[("shows", Value::from(value.text.as_str())), ("label_of", label_of)],
        ));
        return;
    }
    let mut entry = with(entry, &[("value", Value::from(reading.said.as_str()))]);
    if scale.labels && reading.label {
        entry.insert("shows".into(), Value::from(value.text.as_str()));
    }
    let Some(step) = reading.step else {
        found.skipped.push(with(
            entry,
            &[("why", Value::from(reading.why.clone().unwrap_or_default()))],
        ));
        return;
    };
    found.recorded += usize::from(reading.recorded);
    let styled = value.now == Some(step.index) && value.exact;
    let text_right = !scale.labels || label_right(value, &reading, step);
    if styled && text_right {
        found.right += 1;
    }
    match value.now {
        _ if styled => {}
        None => found.none.push(with(entry.clone(), &[("due", Value::from(step.n()))])),
        Some(now) => {
            let mut item = with(
                entry.clone(),
                &[("step", Value::from(now + 1)), ("due", Value::from(step.n()))],
            );
            if now == step.index {
                if let Some(differs) = &value.differs {
                    item.insert("differs".into(), Value::from(differs.as_str()));
                }
            }
            found.wrong.push(item);
        }
    }
    if !text_right {
        found.labels.push(with(
            entry,
            &[
                ("shows", Value::from(value.text.as_str())),
                ("label", step.label.clone().map_or(Value::Null, Value::from)),
                ("due", Value::from(step.n())),
                ("why", Value::from(label_why(value, &reading, step))),
            ],
        ));
    }
}

/// `check`'s fact for `scale` (`report`), of the values its format read.
#[must_use]
pub fn report(env: &dyn ScaleEnv, scale: &Scale, values: &[ScaleValue]) -> Map<String, Value> {
    let mut found = Found::default();
    for value in values {
        checked(env, scale, value, &mut found);
    }
    let listed_count = found.wrong.len() + found.none.len() + found.labels.len() + found.unrecorded.len();
    let mut room = CHECK_LISTED;
    let mut cap = |list: &[Map<String, Value>]| -> Value {
        let kept: Vec<Value> = list.iter().take(room).cloned().map(Value::Object).collect();
        room -= kept.len();
        Value::Array(kept)
    };
    let fixes: Vec<String> = found
        .wrong
        .iter()
        .chain(&found.none)
        .chain(&found.labels)
        .filter_map(|e| e.get("target").and_then(Value::as_str).map(str::to_owned))
        .collect();
    let mut data = Map::new();
    data.insert("checked".into(), Value::from(values.len() - found.skipped.len()));
    data.insert("right".into(), Value::from(found.right));
    data.insert("wrong".into(), cap(&found.wrong));
    data.insert("none".into(), cap(&found.none));
    if scale.labels {
        data.insert("labels".into(), cap(&found.labels));
        data.insert("unrecorded".into(), cap(&found.unrecorded));
    }
    if listed_count > CHECK_LISTED {
        data.insert("left_out".into(), Value::from(listed_count - CHECK_LISTED));
    }
    if !found.skipped.is_empty() {
        let skipped: Vec<Value> = found
            .skipped
            .iter()
            .take(SKIPPED_LISTED)
            .cloned()
            .map(Value::Object)
            .collect();
        data.insert("skipped".into(), Value::Array(skipped));
        if found.skipped.len() > SKIPPED_LISTED {
            data.insert(
                "skipped_left_out".into(),
                Value::from(found.skipped.len() - SKIPPED_LISTED),
            );
        }
    }
    if !fixes.is_empty() {
        data.insert("fix".into(), Value::Object(fix(env.doc(), scale, fixes)));
    }
    let mut notes = scale.notes.clone();
    if !found.unrecorded.is_empty() {
        notes.push(
            "unrecorded: a step's label with no value kept for it, so its step cannot be checked; write its value \
             there (typed, or by reference {{d:...}}), then style_by_value with show label"
                .to_owned(),
        );
    }
    if found.recorded > 0 {
        notes.push(format!(
            "{} label(s) checked against the value recorded: their source could not be read now",
            found.recorded
        ));
    }
    if !notes.is_empty() {
        data.insert("notes".into(), Value::from(notes));
    }
    data
}

/// The one `style_by_value` call that restyles `targets` (`_fix`): those addresses, or --
/// more than a scale takes -- the scale's own targets.
fn fix(doc: Option<&str>, scale: &Scale, targets: Vec<String>) -> Map<String, Value> {
    let mut given = scale.given.clone();
    let mut unique: Vec<String> = Vec::new();
    for target in targets {
        if !unique.contains(&target) {
            unique.push(target);
        }
    }
    if unique.len() <= MAX_TARGETS {
        given.insert("targets".into(), Value::from(unique));
    }
    let mut arguments = Map::new();
    arguments.insert("doc".into(), doc.map_or(Value::Null, Value::from));
    arguments.insert(PARAM.into(), Value::Object(given));
    let mut out = Map::new();
    out.insert("tool".into(), Value::from(TOOL));
    out.insert("arguments".into(), Value::Object(arguments));
    out
}

/// The fact's summary (`summary`).
#[must_use]
pub fn summary(data: &Map<String, Value>) -> String {
    let count = |key: &str| data.get(key).and_then(Value::as_u64).unwrap_or(0);
    let (checked, right) = (count("checked"), count("right"));
    let skipped = data.get("skipped").and_then(Value::as_array).map_or(0, Vec::len);
    let said = if skipped > 0 {
        format!(
            ", {} skipped",
            u64::try_from(skipped).unwrap_or(0) + count("skipped_left_out")
        )
    } else {
        String::new()
    };
    format!(
        "scale: {right} of {checked} value(s) in their step, {} not{said}",
        checked.saturating_sub(right)
    )
}
