//! What the descriptions cost: characters and estimated tokens per tool and per group,
//! held against budgets (characters / 3.5 of the definition's compact JSON, as today's
//! `test_tools_schema.py` estimates; the main session's online count is the authority).
//! The ported tools' figures equal what today's Python definitions cost.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod ported;

use ooxml_tools::cost::{over_budget, Budget, Cost};

fn budget() -> Budget {
    Budget {
        per_tool: 1_000,
        per_group: vec![("core".into(), 1_000), ("word_style".into(), 1_000)],
        description_words: 40,
        property_words: 15,
    }
}

#[test]
fn each_tool_and_group_is_within_its_budget() {
    let specs: Vec<_> = ported::ported().into_iter().map(|(spec, _, _)| spec).collect();
    assert_eq!(over_budget(&specs, Some(&["docx"]), &budget()), Vec::<String>::new());
}

#[test]
fn the_costs_are_counted_as_today() {
    // Characters of {"name", "description", "input_schema": canonical} for a Word toolbox;
    // Python's json.dumps of the same three definitions counts 1,090, 2,937 and 875.
    let costs: Vec<(String, Cost)> = ported::ported()
        .into_iter()
        .map(|(spec, _, served)| (spec.name.clone(), spec.cost(Some(served))))
        .collect();
    let chars: Vec<(&str, usize, u64)> = costs.iter().map(|(n, c)| (n.as_str(), c.chars, c.estimated)).collect();
    assert_eq!(
        chars,
        [
            ("word_set_text", 1090, 312),
            ("word_format", 2937, 840),
            ("replace_text", 875, 250)
        ]
    );
}

#[test]
fn a_tool_over_its_budget_is_named() {
    let specs: Vec<_> = ported::ported().into_iter().map(|(spec, _, _)| spec).collect();
    let tight = Budget {
        per_tool: 500,
        per_group: vec![("core".into(), 500)],
        ..budget()
    };
    let found = over_budget(&specs, Some(&["docx"]), &tight);
    assert_eq!(
        found,
        [
            "word_format: 840 estimated tokens; at most 500",
            "group core: 562 estimated tokens; at most 500"
        ]
    );
}
