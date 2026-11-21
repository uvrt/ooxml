//! Owns: what `edit_chart`'s `series` and `category` name (ooxml-edit's `resolve_series`,
//! `resolve_category`, `charts/actions.py`): a name first -- whitespace collapsed, then
//! full-width forms and case ignored too -- and a position from 0 only when nothing has that
//! name; a name two share, a name nothing has and a position past the end refused, with the
//! names there are.
//!
//! The loose comparison folds as Python 3.12 does (`chart_fold`: NFKC, then `casefold`,
//! Unicode 15.0); a text holding a character newer than that is left to the host
//! ([`Named::Unported`]), as is a name of digits outside ASCII (Python's `isdigit` and
//! `int` disagree on some).
//!
//! Must not: change the chart.

use super::chart_fold::loose;
use ooxml_base::pyfmt::repr_str;
use ooxml_chart::data::{ChartData, Point};
use ooxml_tools::{Failure, Refusal, ToolError, TryCall, WayOut};
use ooxml_xml::view::py_flat;

/// What a `series` or `category` names.
#[derive(Debug)]
pub(crate) enum Named {
    /// Its position.
    At(usize),
    /// Refused, as ooxml-edit refuses it.
    Refused(ToolError),
    /// Left to the host (a character newer than Python's Unicode data).
    Unported(&'static str),
}

/// The series names a call is matched against: each one's name as Python's `str`, `""` for
/// none.
pub(crate) fn series_names(data: &ChartData) -> Vec<String> {
    data.series
        .iter()
        .map(|s| s.name.as_ref().map(Point::text).unwrap_or_default())
        .collect()
}

/// The category labels a call is matched against (`category_labels`): ISO dates for dates
/// kept as serials, else each label as Python's `str` (a blank `"None"`).
pub(crate) fn category_names(data: &ChartData) -> Vec<String> {
    let text = |c: &Option<Point>| c.as_ref().map_or_else(|| "None".to_owned(), Point::text);
    match &data.category_dates {
        None => data.categories.iter().map(text).collect(),
        Some(dates) => dates
            .iter()
            .zip(&data.categories)
            .map(|(day, c)| day.clone().unwrap_or_else(|| text(c)))
            .collect(),
    }
}

/// How ooxml-edit refuses a name: a refusal, or `label_not_found` (a failure).
#[derive(Debug, Clone, Copy)]
enum Code {
    Refusal(Refusal),
    Label,
}

/// What `which` names among `names` (`_resolve`): `what` is `series` or `category`,
/// `action` the call's, `hint` what a refusal adds (a date's spelling); a refusal offers the
/// names (its first 50), else `read` (the chart read).
pub(crate) fn resolve(
    names: &[String],
    which: Option<&str>,
    (what, action, hint): (&str, &str, &str),
    read: &TryCall,
) -> Named {
    let refuse = |code: Code, message: String| {
        let way_out = WayOut::names(names.iter().take(50).map(String::as_str))
            .map_or_else(|| WayOut::call(read.clone()), |w| w.and_call(read.clone()));
        let error = match code {
            Code::Refusal(refusal) => ToolError::refusal(refusal, message, way_out),
            Code::Label => ToolError::failure(Failure::LabelNotFound, message).with_way_out(way_out),
        };
        Named::Refused(error.at(what))
    };
    let Some(which) = which else {
        return refuse(
            Code::Refusal(Refusal::InvalidArguments),
            format!("{action} needs {what}: {}", choices(names)),
        );
    };
    let exact = py_flat(which);
    let hits: Vec<usize> = names
        .iter()
        .enumerate()
        .filter(|(_, name)| !name.is_empty() && py_flat(name) == exact)
        .map(|(k, _)| k)
        .collect();
    if let Some(named) = decided(&hits, which, what, &refuse) {
        return named;
    }
    // the loose pass: NFKC and case folded, as Python 3.12 folds
    let unported = || Named::Unported("edit_chart: a series or category named with characters newer than Unicode 15.0");
    let Some(wanted) = loose(&exact) else { return unported() };
    let mut hits = Vec::new();
    for (k, name) in names.iter().enumerate().filter(|(_, name)| !name.is_empty()) {
        match loose(&py_flat(name)) {
            Some(folded) if folded == wanted => hits.push(k),
            Some(_) => {}
            None => return unported(),
        }
    }
    if let Some(named) = decided(&hits, which, what, &refuse) {
        return named;
    }
    if !which.is_ascii() && which.chars().all(char::is_numeric) {
        return Named::Unported("edit_chart: a series or category named with digits outside ASCII");
    }
    if !which.is_empty() && which.bytes().all(|b| b.is_ascii_digit()) {
        return match which.parse::<usize>() {
            Ok(position) if position < names.len() => Named::At(position),
            _ => refuse(
                Code::Refusal(Refusal::NotFound),
                format!("no {what} {which}; there are {}{hint}", names.len()),
            ),
        };
    }
    refuse(
        Code::Label,
        format!("no {what} {}{hint}; give {}", repr_str(which), choices(names)),
    )
}

/// One hit is the answer; several are refused; none goes on.
fn decided(hits: &[usize], which: &str, what: &str, refuse: &dyn Fn(Code, String) -> Named) -> Option<Named> {
    match hits {
        [] => None,
        [one] => Some(Named::At(*one)),
        _ => {
            let plural = if what == "series" { "series" } else { "categories" };
            let listed: Vec<String> = hits.iter().map(ToString::to_string).collect();
            Some(refuse(
                Code::Label,
                format!(
                    "{} names {} {plural} ({}); give the position instead",
                    repr_str(which),
                    hits.len(),
                    listed.join(", ")
                ),
            ))
        }
    }
}

/// What a refusal offers (`choices`): the names, a long list's first 12 and how many more,
/// and the positions.
fn choices(names: &[String]) -> String {
    if names.is_empty() {
        return "the chart has none".to_owned();
    }
    let shown: Vec<String> = names.iter().take(12).map(|n| repr_str(n)).collect();
    let listed = if names.len() > 12 {
        format!("{} and {} more", shown.join(", "), names.len() - 12)
    } else {
        match shown.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
            Some((last, _)) => last.clone(),
            None => String::new(),
        }
    };
    if names.len() == 1 {
        format!("{listed} (or 0)")
    } else {
        format!("{listed} (or a number from 0 to {})", names.len() - 1)
    }
}
