//! Owns: [`Placer`], the port a reading tool asks where blocks are laid out through (the
//! host's layout of the version read: docx-agent's, phase PL step L4, until the session
//! holds the layout itself), and the filling of the places a reply keeps for the answers.
//!
//! Must not: lay out, or read the layout's internals: a placer answers in the reply's own
//! words (`placements` or `unknown`, as docx-agent's `state` writes them).

use serde_json::{Map, Value};

/// Where blocks are laid out, for the version a call reads.
pub trait Placer: Send + Sync {
    /// For each entry of `asked` -- a block's id, then the ids of the paragraphs it joins
    /// (the layout draws a joined paragraph's lines under the first one's id) -- the keys
    /// its block shows for where it is placed: `placements` (a list of pages, tops,
    /// bottoms, lines, stories and columns) or `unknown` (why, and the page it is after).
    /// `None` where the layout could not be read: the call then fails, and the host says
    /// why.
    fn place(&self, asked: &[Vec<String>]) -> Option<Vec<Map<String, Value>>>;
}

/// The value a block holds at its placements' key until they are known: the `n`th asked.
pub(crate) fn pending(n: usize) -> Value {
    Value::from(format!("{PENDING}{n}"))
}

const PENDING: &str = "\u{1}placements:";

/// `value` with each pending placements key (and the keys after it, in order) replaced by
/// the answer it waits for.
pub(crate) fn fill(value: &mut Value, answers: &[Map<String, Value>]) {
    match value {
        Value::Array(items) => {
            for item in items {
                fill(item, answers);
            }
        }
        Value::Object(map) => {
            let waiting = map
                .get("placements")
                .and_then(Value::as_str)
                .and_then(|s| s.strip_prefix(PENDING))
                .and_then(|n| n.parse::<usize>().ok());
            if let Some(n) = waiting {
                let old = std::mem::take(map);
                for (key, item) in old {
                    if key == "placements" {
                        if let Some(answer) = answers.get(n) {
                            map.extend(answer.clone());
                        }
                    } else {
                        map.insert(key, item);
                    }
                }
            }
            for item in map.values_mut() {
                fill(item, answers);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{fill, pending};
    use serde_json::{json, Map, Value};

    #[test]
    fn an_answer_takes_its_place_with_the_keys_after_it_after_it() {
        let mut value = json!([{"id": "p:1", "placements": pending(1), "xml": "x",
                                "text_boxes": [{"blocks": [{"id": "p:2", "placements": pending(0)}]}]}]);
        let answer = |v: Value| -> Map<String, Value> { v.as_object().cloned().unwrap_or_default() };
        let answers = [
            answer(json!({"unknown": {"reason": "not-drawn", "after_page": null}})),
            answer(json!({"placements": [{"page": 1}]})),
        ];
        fill(&mut value, &answers);
        assert_eq!(
            serde_json::to_string(&value).unwrap_or_default(),
            r#"[{"id":"p:1","placements":[{"page":1}],"xml":"x","text_boxes":[{"blocks":[{"id":"p:2","unknown":{"reason":"not-drawn","after_page":null}}]}]}]"#
        );
    }
}
