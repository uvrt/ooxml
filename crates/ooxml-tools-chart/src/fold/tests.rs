use super::*;

#[test]
fn folds_as_python_does() {
    // full-width forms, ligatures and compatibility letters (NFKC), then case folding
    assert_eq!(loose("ＡＢＣ　１２").as_deref(), Some("abc 12"));
    assert_eq!(loose("ﬁnance").as_deref(), Some("finance"));
    assert_eq!(loose("\u{212A}elvin").as_deref(), Some("kelvin"));
    // where Python's casefold is not its lower case
    assert_eq!(loose("Straße").as_deref(), Some("strasse"));
    assert_eq!(loose("ΣΟΦΟΣ σοφος").as_deref(), Some("σοφοσ σοφοσ"));
    assert_eq!(loose("\u{13A0}\u{AB70}").as_deref(), Some("\u{13A0}\u{13A0}"));
    assert_eq!(loose("North").as_deref(), Some("north"));
}

#[test]
fn folds_sorted() {
    assert!(FOLDS.windows(2).all(|w| w[0].0 < w[1].0));
    assert!(NEWER.windows(2).all(|w| w[0].1 < w[1].0));
}

/// Unicode 15.0 (Python 3.12) left these unassigned; unicode-normalization 0.1.26 (Unicode
/// 18.0) or the standard library's case data changes each one (or composes or reorders
/// around it), so a text holding one has no answer.
#[test]
fn newer_characters_have_no_answer() {
    assert_eq!(unicode_normalization::UNICODE_VERSION, (18, 0, 0));
    for &(low, high) in NEWER {
        for c in low..=high {
            assert_eq!(loose(&format!("a{c}")), None, "U+{:04X}", c as u32);
        }
    }
    // the same characters as the fold would see them
    let changed = NEWER
        .iter()
        .flat_map(|&(low, high)| low..=high)
        .filter(|c| {
            let text = c.to_string();
            text.nfkc().collect::<String>() != text || c.to_lowercase().collect::<String>() != text
        })
        .count();
    assert!(changed > 0);
}
