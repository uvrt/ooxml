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

use crate::host_replay::{frozen_rows, Outcome, Replayer};
use ooxml_numfmt::Locale;
use ooxml_session::numbers::{report, Index, NumberRules, NumberText, NumbersEnv, Source};
use ooxml_session::scales::{self, ScaleEnv, ScaleValue};
use serde_json::{Map, Value};
use std::sync::Arc;

/// The known differences' stream the frozen answers are matched as.
const STREAM: &str = "frozen/numbers";

impl Replayer {
    /// A frozen answer of a check of numbers or a scale, as the Rust host or the session's
    /// service answers it.
    #[must_use]
    pub fn frozen_numbers(&self, case: &Value) -> Outcome {
        match case.get("service").and_then(Value::as_str) {
            Some(service) => self.service(service, case),
            None => self.frozen_in(STREAM, case),
        }
    }

    /// A service case (`numbers`, `scale`): the fact the session's service computes from what
    /// the Python host's was computed of, or its refusal.
    fn service(&self, service: &str, case: &Value) -> Outcome {
        let mut case = case.clone();
        let test = case.get("test").cloned();
        if let (Value::Object(map), Some(test)) = (&mut case, test) {
            map.insert("case".into(), test);
        }
        let found = if service == "scale" {
            scale_fact(&case)
        } else {
            numbers_fact(&case).map(Ok)
        };
        match found {
            Ok(Ok(data)) => self.compare(STREAM, &case, "data", &Value::Object(data)),
            Ok(Err(raised)) => self.compare(STREAM, &case, "raised", &raised),
            Err(why) => Outcome::Different(vec!["inputs".into()], Value::Null, Value::from(why)),
        }
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

impl ScaleEnv for Frozen {
    fn locale(&self) -> &Locale {
        &self.locale
    }

    fn doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    fn fact(&self, address: &str) -> Result<String, String> {
        match self.facts.get(address) {
            Some(Value::String(text)) => Ok(text.clone()),
            Some(Value::Object(refused)) => Err(refused["message"].as_str().unwrap_or_default().to_owned()),
            _ => Err(format!("{address} was not read when the answer was frozen")),
        }
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

/// A frozen value a scale's target held (`freeze_numbers`'s values).
fn value_of(given: &Value) -> Option<ScaleValue> {
    let text = |key: &str| given.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(ScaleValue {
        target: text("target")?,
        text: text("text").unwrap_or_default(),
        given: text("given"),
        now: given
            .get("now")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok()),
        exact: given.get("exact").and_then(Value::as_bool).unwrap_or(true),
        differs: text("differs"),
        skip: text("skip"),
        place: given
            .get("where")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
        kept: given.get("kept").and_then(Value::as_object).cloned(),
        source: text("source"),
    })
}

/// The fact the service computes from a scale case's inputs, or its refusal as the
/// freezer writes one (`{"code", "message"}`).
fn scale_fact(case: &Value) -> Result<Result<Map<String, Value>, Value>, String> {
    let way_out = ooxml_tools::WayOut::call(ooxml_tools::TryCall {
        tool: "check".into(),
        arguments: Map::new(),
        fill: Vec::new(),
    });
    let scale = match scales::parse(&case["given"], &way_out) {
        Ok(scale) => scale,
        Err(error) => {
            return Ok(Err(
                serde_json::json!({"code": error.code().as_str(), "message": error.message()}),
            ));
        }
    };
    let Some(values) = case["values"].as_array() else {
        // (refused before the format read any: it reads none)
        let reading: Vec<&str> = case["reading"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let error = scales::no_reading(&reading, case["kind"].as_str().unwrap_or_default(), way_out);
        return Ok(Err(
            serde_json::json!({"code": error.code().as_str(), "message": error.message()}),
        ));
    };
    let values = values
        .iter()
        .map(value_of)
        .collect::<Option<Vec<_>>>()
        .ok_or("a value unreadable")?;
    let env = Frozen {
        locale: locale_of(&case["locale"]),
        doc: case["doc"].as_str().map(str::to_owned),
        facts: case["facts"].as_object().cloned().unwrap_or_default(),
    };
    Ok(Ok(scales::report(&env, &scale, &values)))
}
