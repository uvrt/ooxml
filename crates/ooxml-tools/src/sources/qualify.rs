//! A source's document (ooxml-edit's `sources.qualify`, ported exactly): `d2/t:sales`
//! names it; `t:sales` alone is the one open document whose format takes `t:` addresses.
//! The prefixes each format resolves to rows ([`Prefixes`]) and the open documents' kinds
//! are the host's.

use crate::datasets::DATASET_PREFIX;
use crate::{Refusal, ToolError, TryCall, WayOut};
use ooxml_base::pyfmt::repr_str;
use std::collections::BTreeMap;

/// Whether `text` is `d` and digits (`re.fullmatch(r"d[0-9]+", text)`): a document handle.
#[must_use]
pub fn is_handle(text: &str) -> bool {
    text.strip_prefix('d')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// `d2/t:x` as `("d2", "t:x")` (`QUALIFIED`, `^(d[0-9]+)/(.+)$`); `None` without a handle.
#[must_use]
pub fn qualified(text: &str) -> Option<(&str, &str)> {
    let (handle, rest) = text.split_once('/')?;
    if !is_handle(handle) {
        return None;
    }
    // `.+$`: no newline, but one final one after it
    let kept = rest.strip_suffix('\n').unwrap_or(rest);
    (!kept.is_empty() && !kept.contains('\n')).then_some((handle, kept))
}

/// How handles are ordered (`_doc_number`): by their number (`d2` before `d10`), anything
/// else after.
#[must_use]
pub fn doc_order(handle: &str) -> (u64, &str) {
    let number = handle
        .get(1..)
        .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|d| d.parse().ok())
        .unwrap_or(1 << 30);
    (number, handle)
}

/// The address prefixes each kind of document resolves to rows (`sources.prefixes`): by
/// kind, longest first (equal lengths as the format gives them); `d:` never (the session's
/// datasets).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prefixes(BTreeMap<String, Vec<String>>);

impl Prefixes {
    /// Each kind's prefixes, as its format gives them.
    #[must_use]
    pub fn new<K: Into<String>, P: Into<String>>(kinds: impl IntoIterator<Item = (K, Vec<P>)>) -> Prefixes {
        Prefixes(
            kinds
                .into_iter()
                .map(|(kind, found)| {
                    let mut found: Vec<String> = found
                        .into_iter()
                        .map(Into::into)
                        .filter(|p| !p.is_empty() && !p.starts_with(DATASET_PREFIX))
                        .collect();
                    found.sort_by_key(|p| std::cmp::Reverse(p.chars().count()));
                    (kind.into(), found)
                })
                .collect(),
        )
    }

    /// The prefixes of `kind`, longest first.
    #[must_use]
    pub fn of(&self, kind: &str) -> &[String] {
        self.0.get(kind).map_or(&[], Vec::as_slice)
    }

    /// Each kind and its prefixes, kinds in alphabetical order.
    pub fn kinds(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.0.iter().map(|(k, p)| (k.as_str(), p.as_slice()))
    }

    /// Every prefix any kind takes, sorted, each once.
    #[must_use]
    pub fn known(&self) -> Vec<String> {
        let mut found: Vec<String> = self.0.values().flatten().cloned().collect();
        found.sort();
        found.dedup();
        found
    }

    /// The prefix `address` starts with: the kinds in alphabetical order, each's longest
    /// first.
    #[must_use]
    pub fn prefix_of(&self, address: &str) -> Option<&str> {
        self.0
            .values()
            .flatten()
            .find(|p| address.starts_with(p.as_str()))
            .map(String::as_str)
    }
}

/// A way out naming `options`, else the call as given.
fn options_or(options: Vec<String>, retry: &TryCall) -> WayOut {
    WayOut::names(options).unwrap_or_else(|| WayOut::call(retry.clone()))
}

/// `(doc, address)` of a source that is not a dataset: the document `value` names
/// (`d2/t:sales`), or the one open document whose format takes its prefix. `documents`:
/// the open documents' handles and kinds.
///
/// # Errors
/// `invalid_arguments` when no format takes the address, or the document named takes no
/// such address; `not_found` when no open document is named so, or none takes it;
/// `ambiguous` when several do.
pub fn qualify(
    value: &str,
    documents: &[(String, String)],
    taken: &Prefixes,
    field: Option<&str>,
    retry: &TryCall,
) -> Result<(String, String), ToolError> {
    let at = |error: ToolError| match field {
        Some(field) => error.at(field),
        None => error,
    };
    let (doc, address) = qualified(value).map_or((None, value), |(doc, address)| (Some(doc), address));
    let Some(prefix) = taken.prefix_of(address) else {
        let known = taken.known();
        let listed = if known.is_empty() {
            String::new()
        } else {
            let shown: Vec<String> = known.iter().map(|p| format!("{p}...")).collect();
            format!(" ({})", shown.join(", "))
        };
        let options = std::iter::once(format!("{DATASET_PREFIX}<id>")).chain(known);
        return Err(at(ToolError::refusal(
            Refusal::InvalidArguments,
            format!(
                "{} is not a source: give a dataset (d:<id>) or an address of an open document{listed}",
                repr_str(value)
            ),
            options_or(options.collect(), retry),
        )));
    };
    let mut candidates: Vec<&str> = documents
        .iter()
        .filter(|(_, kind)| taken.of(kind).iter().any(|p| p == prefix))
        .map(|(handle, _)| handle.as_str())
        .collect();
    candidates.sort_by_key(|h| doc_order(h));
    let named = || -> Vec<String> { candidates.iter().map(|h| format!("{h}/{address}")).collect() };
    if let Some(doc) = doc {
        let Some((_, kind)) = documents.iter().find(|(handle, _)| handle == doc) else {
            return Err(at(ToolError::refusal(
                Refusal::NotFound,
                format!("no open document {}", repr_str(doc)),
                options_or(named(), retry),
            )));
        };
        if !candidates.contains(&doc) {
            return Err(at(ToolError::refusal(
                Refusal::InvalidArguments,
                format!("{doc} ({kind}) takes no {prefix} addresses"),
                options_or(named(), retry),
            )));
        }
        return Ok((doc.to_owned(), address.to_owned()));
    }
    match candidates.as_slice() {
        [] => Err(at(ToolError::refusal(
            Refusal::NotFound,
            format!("no open document takes {prefix} addresses: open one first"),
            WayOut::call(retry.clone()),
        ))),
        [one] => Ok(((*one).to_owned(), address.to_owned())),
        many => Err(at(ToolError::refusal(
            Refusal::Ambiguous,
            format!("{} open documents take {prefix} addresses: name one", many.len()),
            options_or(named(), retry),
        ))),
    }
}
