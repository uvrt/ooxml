//! Owns: the replay of the Python host's frozen answers for `check`'s numbers and scale
//! (`frozen/numbers.jsonl`, made by `python/freeze_numbers.py` over ooxml-edit's and
//! docx-agent's tests of them): a check of a Word document replayed by the Rust host from the
//! state it froze ([`Replayer::frozen_numbers`] of a host case); a check of a made-up
//! format's document replayed by the session's service from what the fact was computed of
//! (a service case: the texts the format gave, the values indexed, the rules, the locale, the
//! references read).
//!
//! Must not: answer a reference the Python host did not read when the case was frozen (it is
//! refused, as one the host refused).

use crate::host_replay::{frozen_rows, Outcome, Replayer, Waits};
use ooxml_numfmt::Locale;
use ooxml_session::numbers::{report, Index, NumberRules, NumberText, NumbersEnv, Source};
use serde_json::{Map, Value};
use std::sync::Arc;

/// The known differences' stream the frozen answers are matched as.
const STREAM: &str = "frozen/numbers";

impl Replayer {
    /// A frozen answer of a check of numbers or a scale, as the Rust host or the session's
    /// service answers it.
    #[must_use]
    pub fn frozen_numbers(&self, case: &Value) -> Outcome {
        let scale = case.get("service").and_then(Value::as_str) == Some("scale")
            || case.pointer("/arguments/scale").is_some_and(|s| !s.is_null());
        if scale {
            return Outcome::Waiting(Waits::G1, false);
        }
        if case.get("service").is_some() {
            return self.numbers_service(case);
        }
        self.frozen_in(STREAM, case)
    }

    /// A service case of numbers: the fact the session's service computes from what the
    /// Python host's was computed of.
    fn numbers_service(&self, case: &Value) -> Outcome {
        let mut case = case.clone();
        let test = case.get("test").cloned();
        if let (Value::Object(map), Some(test)) = (&mut case, test) {
            map.insert("case".into(), test);
        }
        let have = match numbers_fact(&case) {
            Ok(data) => Value::Object(data),
            Err(why) => return Outcome::Different(vec!["inputs".into()], Value::Null, Value::from(why)),
        };
        self.compare(STREAM, &case, "data", &have)
    }
}

/// The frozen surroundings of a service case.
struct Frozen {
    locale: Locale,
    doc: Option<String>,
    facts: Map<String, Value>,
}

impl NumbersEnv for Frozen {
    fn locale(&self) -> &Locale {
        &self.locale
    }

    fn doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    fn fact(&self, address: &str) -> Option<String> {
        self.facts.get(address).and_then(Value::as_str).map(str::to_owned)
    }
}

/// A frozen locale (`freeze_numbers._locale`); en-US where none was.
fn locale_of(given: &Value) -> Locale {
    let Some(fields) = given.as_object() else {
        return Locale::default();
    };
    let char_of = |key: &str, default: char| {
        fields
            .get(key)
            .and_then(Value::as_str)
            .and_then(|s| s.chars().next())
            .unwrap_or(default)
    };
    let text = |key: &str, default: &'static str| {
        fields
            .get(key)
            .and_then(Value::as_str)
            .map_or(std::borrow::Cow::Borrowed(default), |s| {
                std::borrow::Cow::Owned(s.to_owned())
            })
    };
    Locale {
        decimal_separator: char_of("decimal_separator", '.'),
        thousands_separator: char_of("thousands_separator", ','),
        date_order: text("date_order", "mdy"),
        language: text("language", "en-US"),
        list_separator: char_of("list_separator", ','),
    }
}

/// A frozen text (`freeze_numbers`'s items).
fn text_of(given: &Value) -> Option<NumberText> {
    let written = given
        .get("written")?
        .as_array()?
        .iter()
        .map(|span| {
            let at = |k: usize| span.get(k)?.as_u64().and_then(|n| usize::try_from(n).ok());
            Some((at(0)?, at(1)?))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(NumberText {
        at: given.get("at")?.as_str()?.to_owned(),
        text: given.get("text")?.as_str()?.to_owned(),
        written,
        scope: given.get("scope")?.as_object()?.clone(),
        place: given.get("where")?.as_object()?.clone(),
    })
}

/// The fact the service computes from a service case's inputs.
fn numbers_fact(case: &Value) -> Result<Map<String, Value>, String> {
    let items = case["items"]
        .as_array()
        .ok_or("no items")?
        .iter()
        .map(text_of)
        .collect::<Option<Vec<_>>>()
        .ok_or("an item unreadable")?;
    let sources = case["sources"]
        .as_array()
        .ok_or("no sources")?
        .iter()
        .map(|s| {
            Some(Source {
                address: s.get("address")?.as_str()?.to_owned(),
                shown: s.get("shown")?.as_str()?.to_owned(),
                dataset: Arc::new(frozen_rows(s.get("dataset")?)?),
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or("a source unreadable")?;
    let notes: Vec<String> = case["notes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n.as_str().map(str::to_owned))
        .collect();
    let rules = NumberRules::from_json(&case["rules"])?;
    let narrower: Vec<&str> = case["narrower"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let env = Frozen {
        locale: locale_of(&case["locale"]),
        doc: case["doc"].as_str().map(str::to_owned),
        facts: case["facts"].as_object().cloned().unwrap_or_default(),
    };
    report(&env, &Index::new(sources), notes, &items, &rules, &narrower)
}
