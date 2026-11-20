//! Owns: a new part's content type declared as ooxml-edit's package declares one when it
//! adds a part (`opc.py`, `add_part` and `declare_content_type`): nothing where the
//! extension's `Default` already gives the type (a stale `Override` for the name removed),
//! the name's `Override` changed where it has one (any second one removed), a `Default`
//! after the last one for an extension that has none, else an `Override` at the end; a new
//! element indented like its siblings, a removed one leaving no indentation behind.
//!
//! Must not: choose a part's type, or touch any part but `[Content_Types].xml`.

use crate::doc::ModelError;
use crate::names::TYPES;
use crate::new_part::{edit_part, match_tail};
use crate::work::{Id, Work};
use ooxml_opc::name::normalize;
use ooxml_opc::{Package, CONTENT_TYPES_PART};
use ooxml_xml::{Element, QName};
use std::sync::Arc;

/// What declaring a type does to `[Content_Types].xml`.
enum Declaring {
    /// Nothing: the part's type is already the one wanted.
    Nothing,
    /// The part's `Override`s (by their place among the `Override`s) removed: its
    /// extension's `Default` gives the type.
    Remove(Vec<usize>),
    /// The first of the part's `Override`s given the type, the others removed.
    Change(Vec<usize>),
    /// A `Default` for the extension, after the last `Default`.
    Default(String),
    /// An `Override` at the end.
    Override,
}

/// `package` with `part`'s content type declared as `content_type`; `None` where it
/// already resolves to it.
///
/// # Errors
/// When the package has no `[Content_Types].xml`, or it is not XML.
pub(crate) fn declare(package: &Package, part: &str, content_type: &str) -> Result<Option<Package>, ModelError> {
    let part = normalize(part);
    let tree = package
        .tree(CONTENT_TYPES_PART)?
        .ok_or_else(|| ModelError::NoPart(CONTENT_TYPES_PART.to_owned()))?;
    let declaring = what(tree.root(), &part, content_type);
    if matches!(declaring, Declaring::Nothing) {
        return Ok(None);
    }
    edit_part(package, CONTENT_TYPES_PART, |work| {
        let root = work.root();
        let children = work.children(root).to_vec();
        let pick = |work: &Work, local: &str| -> Vec<Id> {
            children.iter().copied().filter(|c| work.is(*c, TYPES, local)).collect()
        };
        let (defaults, overrides) = (pick(work, "Default"), pick(work, "Override"));
        let nth = |k: &usize| overrides.get(*k).copied();
        let set = |work: &mut Work, node: Id, local: &str, value: &str| {
            work.set(node, Arc::new(QName::new(None, local, None)), value);
        };
        let make = |work: &mut Work, local: &str| work.new_element(work.element_name(root, TYPES, local, ""));
        let node = match &declaring {
            Declaring::Nothing => return,
            Declaring::Remove(named) => {
                named
                    .iter()
                    .filter_map(nth)
                    .for_each(|n| remove_keeping_layout(work, n));
                return;
            }
            Declaring::Change(named) => {
                let mut found = named.iter().filter_map(nth);
                if let Some(first) = found.next() {
                    set(work, first, "ContentType", content_type);
                }
                found.for_each(|n| remove_keeping_layout(work, n));
                return;
            }
            Declaring::Default(extension) => {
                let node = make(work, "Default");
                match defaults.last() {
                    Some(&last) => work.add_next(last, node),
                    None => work.insert(root, 0, node),
                }
                set(work, node, "Extension", extension);
                node
            }
            Declaring::Override => {
                let node = make(work, "Override");
                work.append(root, node);
                set(work, node, "PartName", &format!("/{part}"));
                node
            }
        };
        set(work, node, "ContentType", content_type);
        match_tail(work, node);
    })
    .map(Some)
}

/// What declaring `part` (normalised) as `content_type` does to the content types `root`.
fn what(root: &Element, part: &str, content_type: &str) -> Declaring {
    // `path.rsplit(".", 1)[-1].lower() if "." in path else ""`
    let extension = part.rsplit_once('.').map(|(_, e)| e.to_lowercase());
    let of = |local: &str| -> Vec<&Element> {
        root.child_elements()
            .filter(|e| e.name().is(Some(TYPES), local))
            .map(|e| &**e)
            .collect()
    };
    let (defaults, overrides) = (of("Default"), of("Override"));
    let attribute = |e: &Element, name: &str| e.attribute(None, name).map(str::to_owned);
    let of_extension = |e: &&&Element| {
        extension.as_deref() == Some(attribute(e, "Extension").unwrap_or_default().to_lowercase().as_str())
    };
    let named: Vec<usize> = overrides
        .iter()
        .enumerate()
        .filter(|(_, e)| normalize(&attribute(e, "PartName").unwrap_or_default()) == part)
        .map(|(k, _)| k)
        .collect();
    // `_default_type` reads the first `Default` of the extension; `content_type` the last
    let first_default = defaults.iter().find(of_extension);
    if first_default.and_then(|e| attribute(e, "ContentType")).as_deref() == Some(content_type) {
        return if named.is_empty() {
            Declaring::Nothing
        } else {
            Declaring::Remove(named)
        };
    }
    let current = match named.first().and_then(|k| overrides.get(*k)) {
        Some(e) => attribute(e, "ContentType"),
        None => defaults
            .iter()
            .rev()
            .find(of_extension)
            .and_then(|e| attribute(e, "ContentType")),
    };
    if current.as_deref() == Some(content_type) {
        return Declaring::Nothing;
    }
    match (named.is_empty(), extension) {
        (false, _) => Declaring::Change(named),
        (true, Some(extension)) if !extension.is_empty() && first_default.is_none() => Declaring::Default(extension),
        _ => Declaring::Override,
    }
}

/// ooxml-edit's `_remove_keeping_layout`: the last child's tail (the parent's closing
/// indentation) handed to the one before; the node's own tail gone with it.
fn remove_keeping_layout(work: &mut Work, node: Id) {
    let tail = work.tail(node).to_owned();
    if let (None, Some(previous)) = (work.next(node), work.previous(node)) {
        work.set_tail(previous, &tail);
    }
    work.set_tail(node, "");
    work.remove(node);
}

#[cfg(test)]
mod tests {
    use super::{what, Declaring};
    use ooxml_base::Limits;

    const TYPES: &str = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="PNG" ContentType="image/png"/><Override PartName="/word/media/a.png" ContentType="image/x"/></Types>"#;

    fn declaring(part: &str, content_type: &str) -> Declaring {
        let tree = ooxml_xml::parse(TYPES.as_bytes(), &Limits::default().xml).unwrap();
        what(tree.root(), part, content_type)
    }

    #[test]
    fn declared_as_ooxml_edit_declares() {
        assert!(matches!(declaring("word/media/b.png", "image/png"), Declaring::Nothing));
        assert!(matches!(declaring("word/media/a.png", "image/png"), Declaring::Remove(k) if k == [0]));
        assert!(matches!(declaring("word/media/a.png", "image/y"), Declaring::Change(k) if k == [0]));
        assert!(matches!(declaring("word/media/a.png", "image/x"), Declaring::Nothing));
        assert!(matches!(declaring("word/media/c.emf", "image/x-emf"), Declaring::Default(x) if x == "emf"));
        assert!(matches!(
            declaring("word/charts/chart1.xml", "application/chart"),
            Declaring::Override
        ));
        assert!(matches!(
            declaring("word/bin", "application/octet-stream"),
            Declaring::Override
        ));
    }
}
