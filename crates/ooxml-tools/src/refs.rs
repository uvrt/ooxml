//! Owns: the grammar of a ref (design phase 3, section 5; ooxml-edit's `session.REF`,
//! `REF_USE`, `fold_ref`, `_misspelt_ref`, `_folded_use` and `dispatch.ref_rule`, ported
//! exactly): what a ref's name may be ([`is_ref_name`]), a ref used in an argument
//! (`$name`, `$name/rest`: [`ref_use`]), the name a camelCase or hyphenated name stands for
//! ([`fold_ref`]) and the warning of a name kept so ([`folded_warning`]), a use read through
//! that fold ([`folded_use`]), the known ref a misspelt `$Name` stands for
//! ([`misspelt_ref`]), why a name is none in words with one that would do ([`ref_rule`]),
//! and the ref names an op's arguments ask for ([`ref_names`]).
//!
//! Python's `$` also matches before one final newline; the checks here keep that
//! (`"x\n"` is a ref name as today, and `$x\n` the use of `x`).
//!
//! Must not: hold refs or resolve them (the session's ref table does), or know a format.

use ooxml_base::pyfmt::repr_str;
use serde_json::Value;

/// The longest a ref's name is, in characters.
pub const REF_MAX: usize = 32;

/// `text` with one final newline taken off, as Python's `$` reads a match's end.
fn before_end(text: &str) -> [&str; 2] {
    [text, text.strip_suffix('\n').unwrap_or(text)]
}

/// Whether `name` is a ref's name in full: a lowercase letter, then up to 31 lowercase
/// letters, digits or `_` (`^[a-z][a-z0-9_]{0,31}$`).
fn name_body(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && name.len() <= REF_MAX
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Whether `name` is a ref's name (`REF.match`).
#[must_use]
pub fn is_ref_name(name: &str) -> bool {
    before_end(name).iter().any(|body| name_body(body))
}

/// A ref used in an argument: `$name` or `$name/rest` (`REF_USE.match`), the name and the
/// rest (`""` or from its `/`); `None` for anything else.
#[must_use]
pub fn ref_use(value: &str) -> Option<(&str, &str)> {
    let body = value.strip_prefix('$')?;
    // the name is the longest run of name characters, at most 32, then `/...` or the end
    let end = body
        .char_indices()
        .take(REF_MAX)
        .take_while(|(i, c)| {
            if *i == 0 {
                c.is_ascii_lowercase()
            } else {
                c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_'
            }
        })
        .count();
    if end == 0 {
        return None;
    }
    let (name, rest) = body.split_at(end);
    match rest {
        "" | "\n" => Some((name, "")),
        _ if rest.starts_with('/') => {
            // `.` takes no newline: the rest runs to the end, or to one final newline
            let [_, kept] = before_end(rest);
            (!kept.contains('\n')).then_some((name, kept))
        }
        _ => None,
    }
}

/// Whether `name` is letters, digits, `_` and `-`, from a letter (`_FOLDABLE`).
fn foldable(name: &str) -> bool {
    before_end(name).iter().any(|body| {
        let mut chars = body.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    })
}

/// The ref name a camelCase or hyphenated name stands for: `recNow` is `rec_now`,
/// `scale-note` `scale_note`, `HTTPLink` `http_link`; `None` when it folds to no ref name,
/// holds anything but letters, digits, `_` and `-`, or has no lowercase letter (`A1` is an
/// address, not a ref) (`fold_ref`).
#[must_use]
pub fn fold_ref(name: &str) -> Option<String> {
    if !foldable(name) || !name.chars().any(|c| c.is_ascii_lowercase()) {
        return None;
    }
    // `_` between a lowercase letter or digit and a capital, and between a capital and a
    // capital that starts a word (`HTTPLink`: `HTTP_Link`)
    let chars: Vec<char> = name.chars().collect();
    let mut folded = String::with_capacity(name.len() + 4);
    let mut before: Option<char> = None;
    for (i, c) in chars.iter().enumerate() {
        if let Some(before) = before.filter(|_| c.is_ascii_uppercase()) {
            let camel = before.is_ascii_lowercase() || before.is_ascii_digit();
            let word = before.is_ascii_uppercase() && chars.get(i + 1).is_some_and(char::is_ascii_lowercase);
            if camel || word {
                folded.push('_');
            }
        }
        folded.push(*c);
        before = Some(*c);
    }
    let folded = folded.replace('-', "_").to_lowercase();
    is_ref_name(&folded).then_some(folded)
}

/// The warning of a call whose ref `name` is kept as `folded` (`folded_warning`).
#[must_use]
pub fn folded_warning(name: &str, folded: &str) -> String {
    format!("ref {} read as {}: write ${folded}", repr_str(name), repr_str(folded))
}

/// `$recNow` (or `$recNow/rest`) as `$rec_now` when it folds ([`fold_ref`]) to a ref
/// `known` has -- or one a failed op of the batch would have made (`failed`) --; `None` for
/// anything else (`_folded_use`).
#[must_use]
pub fn folded_use(value: &str, known: impl Fn(&str) -> bool, failed: impl Fn(&str) -> bool) -> Option<String> {
    let body = value.strip_prefix('$')?;
    let (head, slash, rest) = match body.split_once('/') {
        Some((head, rest)) => (head, "/", rest),
        None => (body, "", ""),
    };
    let folded = fold_ref(head)?;
    if folded == head || (!known(&folded) && !failed(&folded)) {
        return None;
    }
    Some(format!("${folded}{slash}{rest}"))
}

/// The known ref a `$Name` that is no ref name stands for (`$catChart` for a ref `catchart`
/// or `cat_chart`); `None` for anything else, which stays an address (`_misspelt_ref`).
/// `known`: the names, in the order the document has them.
#[must_use]
pub fn misspelt_ref<'a, I: IntoIterator<Item = &'a str>>(value: &str, known: I) -> Option<&'a str> {
    let body = value.strip_prefix('$')?;
    let head = body.split('/').next().unwrap_or(body);
    let folded = head.to_lowercase().replace(['_', '-'], "");
    let mut found = known.into_iter().filter(|name| name.replace('_', "") == folded);
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// Why `name` is no ref name, in words, with one that would do (`dispatch.ref_rule`).
#[must_use]
pub fn ref_rule(name: &str) -> String {
    // `_` between a lowercase letter or digit and a capital, then lower case
    let chars: Vec<char> = name.chars().collect();
    let mut split = String::with_capacity(name.len() + 4);
    let mut before: Option<char> = None;
    for c in &chars {
        if before.is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) && c.is_ascii_uppercase() {
            split.push('_');
        }
        split.push(*c);
        before = Some(*c);
    }
    let lowered = split.to_lowercase();
    // every run of other characters one `_`, then none at either end
    let mut joined = String::with_capacity(lowered.len());
    let mut in_run = false;
    for c in lowered.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
            joined.push(c);
            in_run = false;
        } else if !in_run {
            joined.push('_');
            in_run = true;
        }
    }
    let mut suggestion = joined.trim_matches('_').to_owned();
    if suggestion.is_empty() {
        suggestion = "r1".to_owned();
    } else if !suggestion.chars().next().is_some_and(char::is_alphabetic) {
        suggestion = format!("r_{suggestion}");
    }
    let suggestion: String = suggestion.chars().take(REF_MAX).collect();
    let suggestion = suggestion.trim_end_matches('_');
    format!(
        "ref {} is not a ref name: a ref starts with a lowercase letter, then up to 31 lowercase letters, digits or _ \
         (no capitals, spaces or -); e.g. {}",
        repr_str(name),
        repr_str(suggestion)
    )
}

/// The ref names an op's arguments ask for: every `ref` at any depth, as the name it is or
/// folds to, so a later op's `$name` can say which failed op would have made it
/// (`dispatch._ref_names`). Text is read as JSON first, as today.
#[must_use]
pub fn ref_names(node: &Value) -> Vec<String> {
    let parsed;
    let node = match node {
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(value) => {
                parsed = value;
                &parsed
            }
            Err(_) => return Vec::new(),
        },
        other => other,
    };
    let mut found = Vec::new();
    let mut stack = vec![node];
    while let Some(item) = stack.pop() {
        match item {
            Value::Object(map) => {
                if let Some(Value::String(name)) = map.get("ref") {
                    let usable = if is_ref_name(name) {
                        Some(name.clone())
                    } else {
                        fold_ref(name)
                    };
                    if let Some(usable) = usable.filter(|u| !u.is_empty()) {
                        found.push(usable);
                    }
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items),
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_ref_name_is_lowercase_from_a_letter_up_to_32() {
        for name in ["a", "rec_now", "x2", &"a".repeat(32), "x\n"] {
            assert!(is_ref_name(name), "{name:?}");
        }
        for name in ["", "A", "2nd", "rec-now", "big chart", &"a".repeat(33), "x\n\n", "\nx"] {
            assert!(!is_ref_name(name), "{name:?}");
        }
    }

    #[test]
    fn a_use_is_the_name_and_its_rest() {
        assert_eq!(ref_use("$x"), Some(("x", "")));
        assert_eq!(ref_use("$first/r0"), Some(("first", "/r0")));
        assert_eq!(ref_use("$x/a\n"), Some(("x", "/a")));
        assert_eq!(ref_use("$x\n"), Some(("x", "")));
        for value in [
            "x",
            "$",
            "$A$1",
            "$recNow",
            "$a..$b",
            "$x/a\nb",
            &format!("${}", "a".repeat(33)),
        ] {
            assert_eq!(ref_use(value), None, "{value:?}");
        }
    }

    /// `test_fold_ref`.
    #[test]
    fn fold_ref_folds_camel_case_and_hyphens() {
        let cases: [(&str, Option<&str>); 10] = [
            ("recNow", Some("rec_now")),
            ("scale-note", Some("scale_note")),
            ("HTTPLink", Some("http_link")),
            ("Sales-2024", Some("sales_2024")),
            ("rec_now", Some("rec_now")),
            ("A1", None),
            ("big chart", None),
            ("2nd", None),
            (&"x".repeat(40), None),
            ("$x", None),
        ];
        for (name, folded) in cases {
            assert_eq!(fold_ref(name).as_deref(), folded, "{name}");
        }
    }

    /// `test_ref_rule_suggests_a_valid_name`.
    #[test]
    fn ref_rule_suggests_a_valid_name() {
        let cases = [
            ("catChart", "cat_chart".to_owned()),
            ("Sales-2024", "sales_2024".to_owned()),
            ("2nd", "r_2nd".to_owned()),
            ("big chart", "big_chart".to_owned()),
            (&"x".repeat(40), "x".repeat(32)),
            ("--", "r1".to_owned()),
        ];
        for (name, suggestion) in cases {
            let rule = ref_rule(name);
            assert!(rule.ends_with(&format!("e.g. '{suggestion}'")), "{rule}");
            assert!(is_ref_name(&suggestion));
        }
        // `test_a_ref_name_refusal_says_the_rule_and_a_name_that_would_do`
        let rule = ref_rule("cat chart");
        assert!(rule.contains("starts with a lowercase letter") && rule.contains("'cat_chart'"));
        assert!(!rule.contains('^'));
    }

    #[test]
    fn a_folded_use_needs_a_known_or_failed_ref() {
        let known = |n: &str| n == "rec_now";
        let none = |_: &str| false;
        assert_eq!(folded_use("$recNow/p1", known, none).as_deref(), Some("$rec_now/p1"));
        assert_eq!(folded_use("$rec-now", known, none).as_deref(), Some("$rec_now"));
        assert_eq!(folded_use("$otherName", known, none), None);
        assert_eq!(folded_use("$rec_now", known, none), None);
        assert_eq!(
            folded_use("$newOne", none, |n| n == "new_one").as_deref(),
            Some("$new_one")
        );
    }

    #[test]
    fn a_misspelt_use_names_the_one_ref_it_stands_for() {
        assert_eq!(misspelt_ref("$catChart", ["catchart"]), Some("catchart"));
        assert_eq!(misspelt_ref("$REC_NOW", ["rec_now"]), Some("rec_now"));
        assert_eq!(misspelt_ref("$A$1", ["catchart"]), None);
        assert_eq!(misspelt_ref("$catChart", ["catchart", "cat_chart"]), None);
        assert_eq!(misspelt_ref("catChart", ["catchart"]), None);
    }

    #[test]
    fn the_refs_an_op_asks_for_are_found_at_any_depth() {
        let op = json!({"ref": "catChart", "items": [{"text": "a", "ref": "x"}, {"ref": "big chart"}]});
        let mut names = ref_names(&op);
        names.sort();
        assert_eq!(names, ["cat_chart", "x"]);
        assert_eq!(ref_names(&json!("{\"ref\": \"z\"}")), ["z"]);
        assert!(ref_names(&json!("not json")).is_empty());
    }
}
