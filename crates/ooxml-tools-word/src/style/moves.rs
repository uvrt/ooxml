//! Owns: what a change of headings' levels did to the outline, as docx-agent's
//! `edit/outline.py` `moves` says it: one line per heading whose level changed, became or
//! stopped being a heading -- where the blocks it gained or lost now go.
//!
//! Must not: read the document but through the headings it is given.

use ooxml_base::pyfmt::repr_str;
use ooxml_word_model::docx::outline::Heading;
use ooxml_word_model::docx::text::{paragraph_text, View};

/// The lines, at most `limit` and one saying how many more.
#[must_use]
pub(crate) fn moves(before: &[Heading], after: &[Heading], renamed: &[(String, String)], limit: usize) -> Vec<String> {
    let rename = |id: &str| {
        renamed
            .iter()
            .find(|(old, _)| old == id)
            .map_or_else(|| id.to_owned(), |(_, new)| new.clone())
    };
    let before: Vec<Heading> = before
        .iter()
        .map(|h| Heading {
            id: rename(&h.id),
            last: rename(&h.last),
            ..h.clone()
        })
        .collect();
    let was = |id: &str| before.iter().find(|h| h.id == id);
    let now = |id: &str| after.iter().find(|h| h.id == id);
    let mut changed: Vec<&Heading> = after
        .iter()
        .filter(|h| was(&h.id).is_none_or(|w| w.level != h.level))
        .collect();
    changed.extend(before.iter().filter(|h| now(&h.id).is_none()));
    let mut out = Vec::new();
    for heading in &changed {
        if out.len() >= limit {
            out.push(format!(
                "and {} more heading(s) changed level: describe shows the outline",
                changed.len() - limit
            ));
            break;
        }
        out.push(line(heading, was(&heading.id), now(&heading.id), &before, after));
    }
    out
}

fn line(
    heading: &Heading,
    old: Option<&Heading>,
    new: Option<&Heading>,
    before: &[Heading],
    after: &[Heading],
) -> String {
    let Some(new) = new else {
        let under = owner(after, heading.start, None);
        return if heading.blocks > 1 {
            format!(
                "{} is no longer a heading (was level {}): its {} block(s) now go under {}",
                label(Some(heading)),
                heading.level,
                heading.blocks - 1,
                label(under)
            )
        } else {
            format!(
                "{} is no longer a heading (was level {})",
                label(Some(heading)),
                heading.level
            )
        };
    };
    let Some(old) = old else {
        let taken = new.blocks - 1;
        return if taken > 0 {
            format!(
                "{} is now a level {} heading: it takes the {taken} block(s) after it, to {}, from under {}",
                label(Some(new)),
                new.level,
                new.last,
                label(owner(before, new.start, None))
            )
        } else {
            format!(
                "{} is now a level {} heading, with nothing under it",
                label(Some(new)),
                new.level
            )
        };
    };
    let verb = if new.level > old.level { "demoted" } else { "promoted" };
    let head = format!("{} {verb} {} -> {}", label(Some(new)), old.level, new.level);
    if new.end < old.end {
        let lost: Vec<&Heading> = after
            .iter()
            .filter(|h| new.end <= h.start && h.start < old.end && h.level <= new.level)
            .collect();
        let names = names(&lost).unwrap_or_else(|| format!("{} block(s)", old.end - new.end));
        format!(
            "{head}: it now ends at {}; {names} after it now go under {}",
            new.last,
            label(owner(after, new.end, Some(new.start)))
        )
    } else if new.end > old.end {
        let within = |h: &&Heading| old.end <= h.start && h.start < new.end;
        let mut gained: Vec<&Heading> = after
            .iter()
            .filter(within)
            .filter(|h| h.level == new.level + 1)
            .collect();
        if gained.is_empty() {
            gained = after.iter().filter(within).collect();
        }
        let names = names(&gained).unwrap_or_else(|| format!("{} block(s)", new.end - old.end));
        format!(
            "{head}: it now runs to {}, taking {names} from under {}",
            new.last,
            label(owner(before, old.end, Some(old.start)))
        )
    } else {
        let parent_was = parent(before, old);
        let parent_now = parent(after, new);
        let same = parent_was.map(|h| &h.id) == parent_now.map(|h| &h.id);
        let was = if same {
            String::new()
        } else {
            format!(" (was {})", label(parent_was))
        };
        format!(
            "{head}: its blocks stay its own; it is now under {}{was}",
            label(parent_now)
        )
    }
}

fn names(found: &[&Heading]) -> Option<String> {
    if found.is_empty() {
        return None;
    }
    let mut out = found
        .iter()
        .take(3)
        .map(|h| label(Some(h)))
        .collect::<Vec<_>>()
        .join(", ");
    if found.len() > 3 {
        out.push_str(&format!(" and {} more", found.len() - 3));
    }
    Some(out)
}

/// `p:A 'Results'`, or the start of the document for none.
fn label(heading: Option<&Heading>) -> String {
    let Some(heading) = heading else {
        return "no heading (the document's start)".to_owned();
    };
    let text = paragraph_text(&heading.element, View::Current)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return heading.id.clone();
    }
    format!(
        "{} {}",
        heading.id,
        repr_str(&text.chars().take(40).collect::<String>())
    )
}

/// The deepest heading whose range holds block `at` (one starting before `before`).
fn owner(found: &[Heading], at: usize, before: Option<usize>) -> Option<&Heading> {
    found
        .iter()
        .filter(|h| h.start <= at && at < h.end && before.is_none_or(|b| h.start < b))
        .max_by_key(|h| h.start)
}

fn parent<'a>(found: &'a [Heading], heading: &Heading) -> Option<&'a Heading> {
    found
        .iter()
        .filter(|h| h.start < heading.start && heading.start < h.end && h.level < heading.level)
        .max_by_key(|h| h.start)
}
