//! What a definition costs in every request: characters and estimated tokens per tool and
//! per group, held against budgets.
//!
//! Owns: the offline estimate today's tests use (`docx-agent/tests/test_tools_schema.py`:
//! characters / 3.5 of the compact JSON, times 1.48 for what Anthropic's count-tokens
//! counts) applied to a tool's name, description and canonical schema, and the check of a
//! set of tools against [`Budget`]s. The main session's online count stays the authority.
//!
//! Must not: build a provider's definition; the estimate counts the parts every provider
//! is sent, so a provider's own wrapping is outside it.

use crate::spec::ToolSpec;
use serde_json::{Map, Value};

/// Characters per estimated token.
pub const CHARS_PER_TOKEN: f64 = 3.5;
/// Counted tokens per estimated one (Sonnet 5.5, measured in T5 and T5b).
pub const COUNTED_PER_ESTIMATED: f64 = 1.48;

/// What one definition costs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cost {
    /// Characters of its compact JSON (name, description, canonical schema).
    pub chars: usize,
    /// Characters / 3.5, rounded up.
    pub estimated: u64,
}

impl Cost {
    /// The cost of `chars` characters.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn of_chars(chars: usize) -> Self {
        Self {
            chars,
            estimated: (chars as f64 / CHARS_PER_TOKEN).ceil() as u64,
        }
    }

    /// The estimate as Anthropic would count it.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn counted(self) -> f64 {
        self.estimated as f64 * COUNTED_PER_ESTIMATED
    }
}

impl ToolSpec {
    /// The definition every provider is sent the parts of, for a toolbox serving `served`.
    #[must_use]
    pub fn definition(&self, served: Option<&[&str]>) -> Value {
        let mut out = Map::new();
        out.insert("name".into(), Value::from(self.name.as_str()));
        out.insert("description".into(), Value::from(self.description.as_str()));
        out.insert("input_schema".into(), self.canonical_for(served));
        Value::Object(out)
    }

    /// What the definition costs.
    #[must_use]
    pub fn cost(&self, served: Option<&[&str]>) -> Cost {
        Cost::of_chars(self.definition(served).to_string().chars().count())
    }
}

/// Budgets in estimated tokens: per tool, and per group (all its tools together).
#[derive(Debug, Clone, PartialEq)]
pub struct Budget {
    /// The most one tool may cost.
    pub per_tool: u64,
    /// The most each group may cost: `(group, budget)`; a group not named has none.
    pub per_group: Vec<(String, u64)>,
    /// The most one tool description may have, in words.
    pub description_words: usize,
    /// The most one property description may have, in words.
    pub property_words: usize,
}

/// Every way `tools` exceed `budget`; empty when they keep it.
#[must_use]
pub fn over_budget(tools: &[ToolSpec], served: Option<&[&str]>, budget: &Budget) -> Vec<String> {
    let mut found = Vec::new();
    let mut groups: Vec<(&str, u64)> = Vec::new();
    for tool in tools {
        let cost = tool.cost(served);
        if cost.estimated > budget.per_tool {
            found.push(format!(
                "{}: {} estimated tokens; at most {}",
                tool.name, cost.estimated, budget.per_tool
            ));
        }
        if tool.description.split_whitespace().count() > budget.description_words {
            found.push(format!(
                "{}: the description is over {} words",
                tool.name, budget.description_words
            ));
        }
        long_descriptions(
            &tool.canonical_for(served),
            &tool.name,
            budget.property_words,
            &mut found,
        );
        match groups.iter_mut().find(|(g, _)| *g == tool.group) {
            Some(entry) => entry.1 += cost.estimated,
            None => groups.push((&tool.group, cost.estimated)),
        }
    }
    for (group, cost) in groups {
        if let Some((_, most)) = budget.per_group.iter().find(|(g, _)| g == group) {
            if cost > *most {
                found.push(format!("group {group}: {cost} estimated tokens; at most {most}"));
            }
        }
    }
    found
}

fn long_descriptions(node: &Value, at: &str, most: usize, found: &mut Vec<String>) {
    let Value::Object(map) = node else { return };
    if let Some(Value::Object(properties)) = map.get("properties") {
        for (name, child) in properties {
            let where_ = format!("{at}.{name}");
            if let Some(text) = child.get("description").and_then(Value::as_str) {
                if text.split_whitespace().count() > most {
                    found.push(format!("{where_}: the description is over {most} words"));
                }
            }
            long_descriptions(child, &where_, most, found);
        }
    }
    if let Some(items) = map.get("items") {
        long_descriptions(items, &format!("{at}[]"), most, found);
    }
}
