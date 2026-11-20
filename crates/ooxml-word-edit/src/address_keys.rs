//! Owns: [`names_by_id`], whether an address names its blocks by their durable ids alone
//! (`p:1A2B3C4D`, `t:1A2B3C4D/c1,2`, `p:A@3..p:B@8`), so that an edit resolving it depends
//! only on those blocks (it reads them by key); any other form (a bookmark, a heading's
//! range, a hyperlink, a control, a story's id) is resolved by condition, and an edit
//! resolving one reads the whole document (design section 5).
//!
//! Must not: resolve anything; it reads the address's syntax only.

/// Whether `address` names its blocks by durable id alone: each end of a span is a
/// paragraph (`p:`, with an optional position `@5` or range `@4:11`), a row (`tr:`) or a
/// table (`t:`, with an optional row or cell below it, `/r1`, `/c1,2`) of the main story, by
/// its hex id.
#[must_use]
pub(crate) fn names_by_id(address: &str) -> bool {
    address.split("..").all(|end| end_by_id(end.trim()))
}

fn end_by_id(end: &str) -> bool {
    let (rest, table) = if let Some(rest) = end.strip_prefix("p:") {
        (rest, false)
    } else if let Some(rest) = end.strip_prefix("t:").or_else(|| end.strip_prefix("tr:")) {
        (rest, true)
    } else {
        return false;
    };
    let hex = rest.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(rest.len());
    if hex == 0 {
        return false;
    }
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    match rest.get(hex..).unwrap_or_default() {
        "" => true,
        below if table => below
            .strip_prefix('/')
            .is_some_and(|b| !b.is_empty() && b.bytes().all(|c| c.is_ascii_alphanumeric() || c == b',')),
        position => position.strip_prefix('@').is_some_and(|at| match at.split_once(':') {
            Some((a, b)) => digits(a) && digits(b),
            None => digits(at),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::names_by_id;

    #[test]
    fn ids_and_their_positions_spans_rows_and_cells_are_by_id() {
        for address in [
            "p:1A2B3C4D",
            "p:1A2B3C4D@3",
            "p:1A2B3C4D@4:11",
            "t:00AB12CD/c1,2",
            "tr:00AB12CD",
            "p:1A2B3C4D@0..p:5E6F7A8B@8",
            "t:00AB12CD",
            "t:00AB12CD/r2",
            "t:00AB12CD/r2c1",
            "p:1A2B3C4D..t:00AB12CD",
        ] {
            assert!(names_by_id(address), "{address}");
        }
    }

    #[test]
    fn names_resolved_by_condition_are_not() {
        for address in [
            "bm:intro",
            "intro",
            "h:p:1A2B3C4D",
            "p:1A2B3C4D..h:p:5E6F7A8B",
            "hl:3",
            "cc:1",
            "footnotes/p:1A2B3C4D",
            "p:",
            "p:1A2B3C4D@",
            "p:1A2B3C4D@4:",
            "t:00AB12CD/",
            "p:1A2B3C4D/r1",
            "p:XYZ",
            "s:1",
            "body",
        ] {
            assert!(!names_by_id(address), "{address}");
        }
    }
}
