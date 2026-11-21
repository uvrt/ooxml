//! Owns: what changed between the package the model opened before and the next, read from
//! the packages themselves: the parts the model's context is read from, and the notes and
//! stories a page draws. A part is the same where it is the same part by identity, or,
//! where the two packages were made apart (one read from bytes, one a session's version),
//! where its bytes are the same.
//!
//! Must not: decide what is laid out again.

use crate::causes::part_reason;
use ooxml_opc::Package;

/// Whether the model's context may change from `opened` (the package it opened last) to
/// `package`: one of the parts it reads its context from, in either, is not the same part.
pub(super) fn context_may_change(opened: Option<&Package>, package: &Package) -> bool {
    let Some(opened) = opened else {
        return false;
    };
    let names = |p: &Package| ooxml_word_model::context_parts(p).unwrap_or_default();
    let (before, now) = (names(opened), names(package));
    before != now || now.iter().any(|name| !same_part(opened, package, name))
}

/// Whether a footnotes', endnotes', header's or footer's part of `package` is not the same
/// part as in `opened` (the package the model opened before).
pub(super) fn notes_or_stories_changed(opened: Option<&Package>, package: &Package) -> bool {
    let Some(opened) = opened else {
        return false;
    };
    let drawn = |name: &String| matches!(part_reason(name), Some("notes" | "headers-footers"));
    let (before, now) = (opened.part_names(), package.part_names());
    before
        .iter()
        .chain(&now)
        .filter(|n| drawn(n))
        .any(|name| !same_part(opened, package, name))
}

/// Whether part `name` is the same in both packages: the same part, or the same bytes (a
/// part neither has is the same; one that does not read is not).
fn same_part(a: &Package, b: &Package, name: &str) -> bool {
    match (a.part(name), b.part(name)) {
        (Some(x), Some(y)) if x.same(y) => true,
        (Some(_), Some(_)) => matches!((a.read(name), b.read(name)), (Ok(Some(x)), Ok(Some(y))) if x == y),
        (None, None) => true,
        _ => false,
    }
}
