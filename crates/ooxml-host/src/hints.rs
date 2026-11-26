//! Owns: what a model meant by a tool name the host does not have (ooxml-edit's `hints.py`,
//! ported exactly): [`tool_hints`], the tools and actions a name most likely means, ranked;
//! [`tool_alias`], the one tool a name is another spelling of; and [`action_hints`], what an
//! `action` value a tool lacks means. Words are a name's parts between underscores, compared
//! ignoring case and a plural `s`; a verb a model writes for another is read as the one it
//! means ([`verb`]); a format's prefix -- the first word of three tools or more -- is no word
//! of the thing meant. A format adds the cases the rules miss as data (its hints table).
//!
//! Ranked, best first, and only the best rank is returned: 1. the format's own table; 2. a
//! tool whose name says the same, verb read; 3. an action whose words are in the name, a word
//! of the name left over being one of the tool's; 4. an action of two words or more that is
//! the whole name but its prefix; 5. a tool whose name is the name's start; 6. an action only
//! one tool has, named as it is.
//!
//! Must not: refuse a call or word a message (the catalogue does).

use std::collections::BTreeMap;
use std::fmt;

/// The most hints a message names.
pub const MAX_HINTS: usize = 3;

/// A tool, and the action of it, that does what an unknown name says.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hint {
    /// The tool.
    pub tool: String,
    /// Its action, where one is meant.
    pub action: Option<String>,
}

impl fmt::Display for Hint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.tool)?;
        if let Some(action) = &self.action {
            write!(f, " with action {action}")?;
        }
        Ok(())
    }
}

/// A tool as hints read it: its name and its `action` values, in the host's order.
#[derive(Debug, Clone, Copy)]
pub struct Named<'a> {
    /// Its name.
    pub name: &'a str,
    /// The values of its `action` enum (none without one).
    pub actions: &'a [String],
}

/// The verb a model writes, read as the verb a tool's name or action uses for it.
#[must_use]
pub fn verb(word: &str) -> &str {
    match word {
        "add" | "insert" | "create" | "new" | "append" | "make" => "add",
        "delete" | "remove" | "drop" | "del" | "erase" => "delete",
        "set" | "update" | "change" | "modify" | "edit" => "set",
        "read" | "get" | "list" | "show" | "view" | "fetch" => "read",
        "move" | "reorder" | "reposition" => "move",
        "duplicate" | "copy" | "clone" => "duplicate",
        other => other,
    }
}

fn is_verb(word: &str) -> bool {
    verb(word) != word || matches!(word, "add" | "delete" | "set" | "read" | "move" | "duplicate")
}

/// `word` without a plural ending (`pages` `page`, `categories` `category`, `boxes` `box`);
/// `series`, `ss` and short words as they are.
#[must_use]
pub fn singular(word: &str) -> String {
    let count = word.chars().count();
    if word == "series" || count <= 3 || word.ends_with("ss") {
        return word.to_owned();
    }
    if let Some(stem) = word.strip_suffix("ies") {
        return format!("{stem}y");
    }
    if ["ches", "shes", "xes", "sses"].iter().any(|end| word.ends_with(end)) {
        return word[..word.len() - 2].to_owned();
    }
    word.strip_suffix('s').unwrap_or(word).to_owned()
}

/// A name's words.
#[must_use]
pub fn words(name: &str) -> Vec<String> {
    name.to_lowercase()
        .split('_')
        .filter(|part| !part.is_empty())
        .map(singular)
        .collect()
}

/// The first words that are a format's prefix: each the first of three tools or more.
fn prefixes(tools: &[Named<'_>]) -> Vec<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for tool in tools {
        if let Some(first) = words(tool.name).into_iter().next() {
            *counts.entry(first).or_default() += 1;
        }
    }
    counts.into_iter().filter(|(_, n)| *n >= 3).map(|(w, _)| w).collect()
}

/// A name's words without its format prefix.
fn core(name: &str, prefix: &[String]) -> Vec<String> {
    let found = words(name);
    match found.split_first() {
        Some((first, rest)) if !rest.is_empty() && prefix.contains(first) => rest.to_vec(),
        _ => found,
    }
}

/// A name's words as compared: the first, a verb, read as the verb it means (only where more
/// words follow).
fn key(core: &[String]) -> Vec<String> {
    match core.split_first() {
        Some((first, rest)) if !rest.is_empty() => {
            let mut out = vec![verb(first).to_owned()];
            out.extend(rest.iter().cloned());
            out
        }
        _ => core.to_vec(),
    }
}

/// The keys `name` may stand for: its own, and without a first word that is some other prefix.
fn keys(name: &str, prefix: &[String]) -> Vec<Vec<String>> {
    let core = core(name, prefix);
    let mut found = vec![key(&core)];
    if let (true, Some((_, rest))) = (core.len() > 2, core.split_first()) {
        found.push(key(rest));
    }
    found
}

/// Two nouns that name the same thing: equal, or one ends with the other, four letters or more.
fn same(a: &str, b: &str) -> bool {
    a == b || (a.chars().count().min(b.chars().count()) >= 4 && (a.ends_with(b) || b.ends_with(a)))
}

/// The tools (and actions) `name`, which is no tool, most likely means, best first: at most
/// [`MAX_HINTS`], all of the best rank; empty when none. `table` is the formats' own
/// (`(name, "tool")` or `(name, "tool action")`); `exclude` names tools never to suggest.
#[must_use]
pub fn tool_hints(name: &str, tools: &[Named<'_>], table: &[(String, String)], exclude: &[&str]) -> Vec<Hint> {
    if name.trim().is_empty() {
        return Vec::new();
    }
    let name = name.rsplit('.').next().unwrap_or(name);
    let known: Vec<Named<'_>> = tools.iter().copied().filter(|t| !exclude.contains(&t.name)).collect();
    let prefix = prefixes(tools);
    let table_tier: Vec<(Vec<i64>, Hint)> = table_hints(name, &known, table, &prefix)
        .into_iter()
        .map(|h| (vec![0], h))
        .collect();
    let (same_names, starts) = same_names(name, &known, &prefix);
    let same_tier: Vec<(Vec<i64>, Hint)> = same_names.into_iter().map(|h| (vec![0], h)).collect();
    let (mut worded, mut whole, mut only) = (Vec::new(), Vec::new(), Vec::new());
    let given = core(name, &prefix);
    let verbs: Vec<String> = given.iter().map(|w| verb(w).to_owned()).collect();
    let mut owners: BTreeMap<&str, usize> = BTreeMap::new();
    for tool in &known {
        for action in tool.actions {
            *owners.entry(action.as_str()).or_default() += 1;
        }
    }
    for tool in &known {
        let nouns = nouns(&core(tool.name, &prefix));
        for action in tool.actions {
            let needed: Vec<String> = words(action).iter().map(|w| verb(w).to_owned()).collect();
            let Some(rest) = unmatched(&given, &verbs, &needed) else {
                continue;
            };
            let matched = rest.iter().filter(|w| nouns.iter().any(|n| same(w, n))).count();
            let hint = Hint {
                tool: tool.name.to_owned(),
                action: Some(action.clone()),
            };
            let len = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
            if matched > 0 {
                worded.push((
                    vec![len(matched), len(needed.len()), -(len(rest.len()) - len(matched))],
                    hint,
                ));
            } else if rest.is_empty() && needed.len() > 1 {
                whole.push((vec![len(needed.len())], hint));
            } else if owners.get(action.as_str()) == Some(&1) && words(action).iter().all(|w| given.contains(w)) {
                only.push((vec![len(needed.len()), -len(rest.len())], hint));
            }
        }
    }
    for mut tier in [table_tier, same_tier, worded, whole, starts, only] {
        if tier.is_empty() {
            continue;
        }
        // Python's sorted(..., reverse=True) is stable: equal keys keep their order
        tier.sort_by(|a, b| b.0.cmp(&a.0));
        let mut out: Vec<Hint> = Vec::new();
        for (_, hint) in tier {
            if !out.contains(&hint) {
                out.push(hint);
            }
        }
        out.truncate(MAX_HINTS);
        return out;
    }
    Vec::new()
}

/// The tool `name`, which is no tool, is another spelling of: the one tool whose name says
/// the same (rank 2), when no entry of the format's `table` reads the name otherwise; `None`
/// when no tool, or more than one, does.
#[must_use]
pub fn tool_alias(name: &str, tools: &[Named<'_>], table: &[(String, String)]) -> Option<String> {
    if name.trim().is_empty() || name.contains('.') {
        return None;
    }
    let prefix = prefixes(tools);
    if !table_hints(name, tools, table, &prefix).is_empty() {
        return None;
    }
    let (same, _) = same_names(name, tools, &prefix);
    match same.as_slice() {
        [one] => Some(one.tool.clone()),
        _ => None,
    }
}

/// The format's table entries `name` is a spelling of (rank 1).
fn table_hints(name: &str, known: &[Named<'_>], table: &[(String, String)], prefix: &[String]) -> Vec<Hint> {
    let wanted = keys(name, prefix);
    let mut found = Vec::new();
    for (entry, target) in table {
        let (tool_name, action) = target.split_once(' ').unwrap_or((target.as_str(), ""));
        let Some(tool) = known.iter().find(|t| t.name == tool_name) else {
            continue;
        };
        if !action.is_empty() && !tool.actions.iter().any(|a| a == action) {
            continue;
        }
        if wanted.iter().any(|want| key(&core(entry, prefix)) == *want) {
            found.push(Hint {
                tool: tool_name.to_owned(),
                action: (!action.is_empty()).then(|| action.to_owned()),
            });
        }
    }
    found
}

/// The tools whose names say the same as `name` (rank 2), and those whose names are its
/// start, each with its length in words (rank 5).
fn same_names(name: &str, known: &[Named<'_>], prefix: &[String]) -> (Vec<Hint>, Vec<(Vec<i64>, Hint)>) {
    let wanted = keys(name, prefix);
    let mut same = Vec::new();
    let mut starts = Vec::new();
    for tool in known {
        let tool_key = key(&core(tool.name, prefix));
        let hint = Hint {
            tool: tool.name.to_owned(),
            action: None,
        };
        if wanted.contains(&tool_key) {
            same.push(hint);
        } else if wanted
            .iter()
            .any(|want| want.len() > tool_key.len() && want.starts_with(&tool_key))
        {
            starts.push((vec![i64::try_from(tool_key.len()).unwrap_or(i64::MAX)], hint));
        }
    }
    (same, starts)
}

/// The words of a name's core that name a thing: not its verbs, unless the verb is all there is.
fn nouns(core: &[String]) -> Vec<String> {
    let found: Vec<String> = core.iter().filter(|w| !is_verb(w)).cloned().collect();
    if !found.is_empty() && core.len() > 1 {
        found
    } else {
        core.to_vec()
    }
}

/// The words of `given` (`verbs`: each read as a verb) the action's words leave over, in the
/// name's own spelling; `None` when one of the action's words is not in the name.
fn unmatched(given: &[String], verbs: &[String], needed: &[String]) -> Option<Vec<String>> {
    let mut taken = needed.to_vec();
    let mut rest = Vec::new();
    for (word, as_verb) in given.iter().zip(verbs) {
        if let Some(at) = taken.iter().position(|t| t == as_verb) {
            taken.remove(at);
        } else {
            rest.push(word.clone());
        }
    }
    taken.is_empty().then_some(rest)
}

/// For a call of `tool` whose `action` `value` it does not have: the action of its own the
/// value means with a verb read (`add_row`: `insert_row`), or `None`; and the other tools
/// whose action it is.
#[must_use]
pub fn action_hints(
    tool: Named<'_>,
    value: &str,
    tools: &[Named<'_>],
    table: &[(String, String)],
) -> (Option<String>, Vec<Hint>) {
    if value.is_empty() {
        return (None, Vec::new());
    }
    let read = |text: &str| -> Vec<String> { words(text).iter().map(|w| verb(w).to_owned()).collect() };
    let given = read(value);
    if let Some(own) = tool.actions.iter().find(|a| read(a) == given) {
        return (Some(own.clone()), Vec::new());
    }
    let prefix = prefixes(tools);
    let mut names = vec![value.to_owned()];
    names.extend(
        nouns(&core(tool.name, &prefix))
            .iter()
            .map(|noun| format!("{value}_{noun}")),
    );
    let mut found: Vec<Hint> = Vec::new();
    for name in names {
        for hint in tool_hints(&name, tools, table, &[tool.name]) {
            if !found.contains(&hint) {
                found.push(hint);
            }
        }
    }
    found.truncate(MAX_HINTS);
    (None, found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn a_name_is_read_by_its_words_verbs_and_plurals() {
        let chart = actions(&["add", "read", "set_values"]);
        let pages = actions(&["add", "delete", "move"]);
        let none: Vec<String> = Vec::new();
        let tools = [
            Named {
                name: "x_chart",
                actions: &chart,
            },
            Named {
                name: "x_pages",
                actions: &pages,
            },
            Named {
                name: "x_read",
                actions: &none,
            },
            Named {
                name: "x_add_picture",
                actions: &none,
            },
        ];
        let hint = |tool: &str, action: Option<&str>| Hint {
            tool: tool.to_owned(),
            action: action.map(str::to_owned),
        };
        assert_eq!(
            tool_alias("x_insert_picture", &tools, &[]),
            Some("x_add_picture".to_owned())
        );
        assert_eq!(
            tool_hints("x_remove_pages", &tools, &[], &[]),
            vec![hint("x_pages", Some("delete"))]
        );
        assert_eq!(tool_hints("x_read_cells", &tools, &[], &[]), vec![hint("x_read", None)]);
        let table = [("x_drop_box".to_owned(), "x_pages delete".to_owned())];
        assert_eq!(
            tool_hints("x_remove_box", &tools, &table, &[]),
            vec![hint("x_pages", Some("delete"))]
        );
        assert_eq!(singular("categories"), "category");
        assert_eq!(singular("boxes"), "box");
        assert_eq!(singular("series"), "series");
    }
}
