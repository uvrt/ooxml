//! Owns: the one rule every Word reader reads markup compatibility by (ECMA-376 Part 3,
//! §10.2; phase 3 identity design §3): of an `mc:AlternateContent`, the first `mc:Choice`
//! the readers understand -- every namespace its `Requires` names is one they read
//! ([`UNDERSTOOD`]) -- else its `mc:Fallback` ([`chosen`]). The block index keys blocks
//! from the branch the layout draws and from no other ([`unchosen`]), so a text box's
//! paragraphs, which Word writes in the Choice and again in a VML Fallback (often with the
//! same paraIds), are keyed once.
//!
//! Must not: read a branch's content, or decide what a reader draws of it.

use crate::xmlread::Ns;
use ooxml_xml::{Element, Node};
use std::sync::Arc;

/// Markup compatibility.
pub const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// The namespaces a `Requires` may name for the Word readers to read its Choice: those the
/// readers read (WordprocessingML, transitional and strict; Word 2010's and 2012's
/// additions; Word 2010's drawing ids, shapes and groups -- the text boxes and groups Word
/// writes in a Choice with their VML in the Fallback). A Choice needing another (a chartex
/// chart `cx`, ink `aink`, a 3D model `am3d`, a canvas `wpc`...) is one Word may draw and
/// these readers cannot: its Fallback is read (SUPPORTED.md, known limits). On the corpus
/// every Choice in a story part requires `wps` or `wpg` (2026-11-27: 152 and 82, in 1,415
/// documents).
pub const UNDERSTOOD: &[Ns] = &[
    Ns::W,
    Ns::W14,
    Ns::only(crate::parts::W15),
    Ns::only(crate::ids::WP14),
    Ns::WPS,
    Ns::WPG,
];

/// Whether `element` is an `mc:AlternateContent`.
#[must_use]
pub fn is_alternate(element: &Element) -> bool {
    element.name().is(Some(MC), "AlternateContent")
}

/// The branch of the `mc:AlternateContent` `alternate` the Word readers read: its first
/// Choice they understand, else its Fallback; `None` where it has neither.
#[must_use]
pub fn chosen(alternate: &Element) -> Option<&Arc<Element>> {
    let mut fallback = None;
    for branch in alternate.child_elements() {
        let name = branch.name();
        if name.namespace() != Some(MC) {
            continue;
        }
        match name.local() {
            "Choice" if understood_here(alternate, branch) => return Some(branch),
            "Fallback" if fallback.is_none() => fallback = Some(branch),
            _ => {}
        }
    }
    fallback
}

/// The `Requires` of `alternate`'s first Choice where the readers read its Fallback (no
/// Choice they understand): what an edit inside that Fallback is refused naming, since Word
/// draws the Choice. `None` where they read a Choice, or there is none.
#[must_use]
pub fn not_understood(alternate: &Element) -> Option<&str> {
    chosen(alternate).filter(|b| b.name().local() == "Fallback")?;
    alternate
        .child_elements()
        .find(|b| b.name().is(Some(MC), "Choice"))
        .map(|choice| choice.attribute(None, "Requires").unwrap_or(""))
}

/// Whether `child`, a child of `parent`, is a branch of an `mc:AlternateContent` the Word
/// readers do not read (what a walk over a story's elements leaves out).
#[must_use]
pub fn unchosen(parent: &Element, child: &Arc<Element>) -> bool {
    is_alternate(parent)
        && child.name().namespace() == Some(MC)
        && !chosen(parent).is_some_and(|branch| Arc::ptr_eq(branch, child))
}

/// Whether a Choice whose `Requires` is `requires` is one the readers understand: it names
/// at least one prefix, and each names (through `namespace_of`, the namespace a prefix has
/// in the Choice) one of [`UNDERSTOOD`]. The rule a reader of another tree (the editor's
/// working tree) reads a Choice by too.
pub fn understood<S: AsRef<str>>(requires: Option<&str>, namespace_of: impl Fn(&str) -> Option<S>) -> bool {
    let Some(requires) = requires else {
        return false;
    };
    let mut prefixes = requires.split_whitespace().peekable();
    prefixes.peek().is_some()
        && prefixes.all(|prefix| {
            namespace_of(prefix).is_some_and(|uri| UNDERSTOOD.iter().any(|ns| ns.holds(Some(uri.as_ref()))))
        })
}

/// Whether every namespace the Choice `choice` of `alternate` requires is one the readers
/// read ([`understood`]).
fn understood_here(alternate: &Element, choice: &Element) -> bool {
    understood(choice.attribute(None, "Requires"), |prefix| {
        namespace_of(prefix, alternate, choice)
    })
}

/// The namespace `prefix` names in `choice`: as the Choice or its AlternateContent declares
/// it, else as the Choice's content uses it (Word declares every prefix once, on the part's
/// root, and a Choice uses the prefixes it requires). `None`: not found, so not understood.
fn namespace_of<'a>(prefix: &str, alternate: &'a Element, choice: &'a Element) -> Option<&'a str> {
    let declared = |element: &'a Element| {
        element
            .namespaces()
            .iter()
            .find(|d| d.prefix.as_deref() == Some(prefix))
            .map(|d| &*d.uri)
    };
    declared(choice)
        .or_else(|| declared(alternate))
        .or_else(|| used(prefix, choice))
}

/// The namespace an element or attribute below `element` names with `prefix`.
fn used<'a>(prefix: &str, element: &'a Element) -> Option<&'a str> {
    for node in element.children() {
        let Node::Element(child) = node else { continue };
        let name = child.name();
        if name.prefix() == Some(prefix) {
            return name.namespace();
        }
        if let Some(found) = child
            .attributes()
            .iter()
            .find(|a| a.name.prefix() == Some(prefix))
            .and_then(|a| a.name.namespace())
        {
            return Some(found);
        }
        if let Some(found) = used(prefix, child) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{chosen, unchosen};
    use ooxml_xml::Limits;

    const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex""#;

    fn alternate(content: &str) -> ooxml_xml::Tree {
        let xml = format!(r#"<mc:AlternateContent {NS}>{content}</mc:AlternateContent>"#);
        ooxml_xml::parse(xml.as_bytes(), &Limits::default()).unwrap()
    }

    fn chosen_local(content: &str) -> Option<String> {
        let tree = alternate(content);
        chosen(tree.root()).map(|b| format!("{}:{}", b.name().local(), b.child_elements().count()))
    }

    #[test]
    fn an_understood_choice_is_read() {
        let got = chosen_local(
            r#"<mc:Choice Requires="wps"><w:drawing/></mc:Choice><mc:Fallback><w:pict/><w:pict/></mc:Fallback>"#,
        );
        assert_eq!(got.as_deref(), Some("Choice:1"));
    }

    #[test]
    fn a_choice_the_readers_do_not_read_falls_back() {
        let got = chosen_local(
            r#"<mc:Choice Requires="cx"><w:drawing/></mc:Choice><mc:Fallback><w:pict/><w:pict/></mc:Fallback>"#,
        );
        assert_eq!(got.as_deref(), Some("Fallback:2"));
        let unknown = chosen_local(r#"<mc:Choice Requires="zz"><w:drawing/></mc:Choice><mc:Fallback/>"#);
        assert_eq!(
            unknown.as_deref(),
            Some("Fallback:0"),
            "an undeclared prefix is not understood"
        );
        let none = chosen_local(r#"<mc:Choice><w:drawing/></mc:Choice><mc:Fallback/>"#);
        assert_eq!(
            none.as_deref(),
            Some("Fallback:0"),
            "a Choice requiring nothing named is not read"
        );
    }

    #[test]
    fn every_namespace_required_must_be_understood_and_the_first_such_choice_wins() {
        let got = chosen_local(
            r#"<mc:Choice Requires="wps cx"><w:drawing/></mc:Choice><mc:Choice Requires="w wps"><w:drawing/><w:drawing/></mc:Choice><mc:Fallback/>"#,
        );
        assert_eq!(got.as_deref(), Some("Choice:2"));
    }

    #[test]
    fn a_prefix_is_found_where_the_content_uses_it() {
        let xml = r#"<w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wps:wsp xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"/></w:drawing></mc:Choice><mc:Fallback/></mc:AlternateContent></w:r>"#;
        let tree = ooxml_xml::parse(xml.as_bytes(), &Limits::default()).unwrap();
        let alternate = tree.root().child_elements().next().unwrap();
        let branches: Vec<_> = alternate.child_elements().collect();
        assert_eq!(chosen(alternate).map(|b| b.name().local()), Some("Choice"));
        assert!(!unchosen(alternate, branches[0]));
        assert!(unchosen(alternate, branches[1]));
    }
}
