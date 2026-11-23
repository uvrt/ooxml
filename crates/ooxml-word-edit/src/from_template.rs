//! Owns: [`NewFromTemplate`], a new document from a template (a .dotx, .dotm, .docx or
//! .docm) as docx-agent's `Document.new(template=..., keep_content=False)` makes one
//! (`edit/authoring.py`, `from_template` and `_restyle`), Word's File > New from a template
//! without the template's content: the macros dropped (a macro-enabled template's VBA
//! project and what hangs off it); the body one empty paragraph before the last section;
//! the notes and comments nothing references any more, the comment parts and the people
//! no one is left of; the headers and footers only the dropped sections used; Convert to
//! compatibility mode 15 where the template is older; the core properties started again
//! (author, last modified by, revision 1, created and modified, the title where given);
//! `app.xml`'s template `Normal.dotm` (the template is not attached); the page size where
//! one is given. The main part's content type is the writer's ([`crate::serialise`] as a
//! `.docx`).
//!
//! Must not: attach the template, or change the template's own bytes (the command edits
//! the version opened from a copy of them).

use crate::core_properties::{write_core, CoreProperty};
use crate::doc::{Key, WordDoc};
use crate::names::{R, W, W14};
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::sink::EditSink;
use crate::template::UpgradeToModern;
use crate::{EditError, Result};
use ooxml_opc::CONTENT_TYPES_PART;
use ooxml_store::{Command, Txn};
use ooxml_word_model::parts::{relationships, REL_COMMENTS, REL_COMMENTS_EXTENDED, REL_COMMENTS_IDS};
use std::collections::BTreeSet;
use std::sync::Arc;

/// The relationships of what belongs to a VBA project.
const MACRO_RELATIONSHIPS: [&str; 3] = [
    "http://schemas.microsoft.com/office/2006/relationships/vbaProject",
    "http://schemas.microsoft.com/office/2006/relationships/keyMapCustomizations",
    "http://schemas.microsoft.com/office/2006/relationships/wordVbaData",
];
/// A VBA project's content type.
const VBA_TYPE: &str = "application/vnd.ms-office.vbaProject";
/// The comment parts, in the order docx-agent drops them (`extended`, `ids`, `extensible`,
/// `comments`).
const COMMENT_PARTS: [&str; 4] = [
    REL_COMMENTS_EXTENDED,
    REL_COMMENTS_IDS,
    "http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible",
    REL_COMMENTS,
];
const REL_HEADER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
const REL_FOOTER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer";
const REL_APP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
const APP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties";
const W15_PEOPLE: &str = "http://schemas.microsoft.com/office/word/2012/wordml";

/// A new document from the template the version holds, its content left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFromTemplate {
    /// The page size (twips, portrait), where one is given: every section's, turned as the
    /// section is.
    pub page: Option<(u32, u32)>,
    /// The title (`None`: the template's stays).
    pub title: Option<String>,
    /// The author and last modified by (`None`: none).
    pub author: Option<String>,
    /// When it was made (W3CDTF): created and modified.
    pub created: String,
}

impl Command<WordDoc> for NewFromTemplate {
    type Out = ();
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<()> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<()> {
        tx.read_everything();
        {
            let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
            let main = sink.docx()?.main.clone();
            drop_macros(&mut sink, &main)?;
            drop_body(&mut sink, &main)?;
            drop_notes(&mut sink)?;
            drop_comment_parts(&mut sink, &main)?;
            drop_unused_stories(&mut sink, &main)?;
        }
        UpgradeToModern.prepare(tx)?;
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        let author = self.author.as_deref();
        let mut values = vec![
            (CoreProperty::Author, author),
            (CoreProperty::LastModifiedBy, author),
            (CoreProperty::Revision, Some("1")),
            (CoreProperty::Created, Some(self.created.as_str())),
            (CoreProperty::Modified, Some(self.created.as_str())),
        ];
        if let Some(title) = &self.title {
            values.push((CoreProperty::Title, Some(title.as_str())));
        }
        write_core(&mut sink, &values)?;
        app_template(&mut sink, "Normal.dotm")?;
        if let Some(page) = self.page {
            restyle_page(&mut sink, page)?;
        }
        Ok(())
    }
}

/// The ids of the relationships of `part` whose type is one of `types`.
fn related_ids(sink: &EditSink<'_, '_>, part: &str, types: &[&str]) -> Result<Vec<String>> {
    Ok(relationships(sink.doc().package(), part)
        .map_err(crate::doc::ModelError::from)?
        .into_iter()
        .filter(|r| types.contains(&r.rel_type.as_str()))
        .map(|r| r.id)
        .collect())
}

/// The VBA project and what belongs to it released; the project's `Default` content type
/// gone once no part of that type is left (docx-agent's `_drop_macros`).
fn drop_macros(sink: &mut EditSink<'_, '_>, main: &str) -> Result<()> {
    let ids = related_ids(sink, main, &MACRO_RELATIONSHIPS)?;
    if ids.is_empty() {
        return Ok(());
    }
    sink.release(main, &ids)?;
    let held = sink.doc().clone();
    let names = held.package().part_names();
    let mut space = Space::default();
    let Some(work) = space.work(&held, CONTENT_TYPES_PART)? else {
        return Ok(());
    };
    let root = work.root();
    let gone: Vec<_> = work
        .children(root)
        .iter()
        .copied()
        .filter(|&node| {
            let typed = work.get(node, None, "ContentType") == Some(VBA_TYPE);
            let extension = work
                .get(node, None, "Extension")
                .map(|e| format!(".{}", e.to_lowercase()));
            typed
                && extension.is_some_and(|ext| {
                    !names
                        .iter()
                        .any(|n| n.to_lowercase().ends_with(&ext) && held.package().has_part(n))
                })
        })
        .collect();
    for node in &gone {
        work.remove(*node);
    }
    if !gone.is_empty() {
        space.mark_dirty(CONTENT_TYPES_PART);
    }
    space.finish(sink)
}

/// The body but its last section, and one empty paragraph before it (docx-agent's
/// `_drop_body`).
fn drop_body(sink: &mut EditSink<'_, '_>, main: &str) -> Result<()> {
    let ids = sink.draw(main, 2)?;
    let held = sink.doc().clone();
    let mut space = Space::default();
    let work = space
        .work(&held, main)?
        .ok_or_else(|| EditError::NoPart(main.to_owned()))?;
    let root = work.root();
    let body = work
        .find(root, W, "body")
        .ok_or_else(|| EditError::Refused("the template has no body".to_owned()))?;
    let section = work.find(body, W, "sectPr");
    for child in work.children(body).to_vec() {
        if Some(child) != section {
            work.remove(child);
        }
    }
    let paragraph = work.new_element(work.element_name(body, W, "p", "w"));
    let (para, text) = (
        ids.first().cloned().unwrap_or_default(),
        ids.get(1).cloned().unwrap_or_default(),
    );
    work.set(paragraph, work.qname(W14, "paraId", "w14"), &para);
    work.set(paragraph, work.qname(W14, "textId", "w14"), &text);
    match section {
        Some(section) => work.add_previous(section, paragraph),
        None => work.append(body, paragraph),
    }
    space.mark_dirty(main);
    space.finish(sink)?;
    sink.ensure_w14(main, false, &[])
}

/// The footnotes and endnotes (but the separators) and every comment taken out of their
/// parts.
fn drop_notes(sink: &mut EditSink<'_, '_>) -> Result<()> {
    let held = sink.doc().clone();
    let docx = sink.docx()?;
    let mut space = Space::default();
    for part in &docx.parts {
        let name = part.part.to_string();
        let Some(work) = space.work(&held, &name)? else {
            continue;
        };
        let root = work.root();
        let gone: Vec<_> = work
            .children(root)
            .iter()
            .copied()
            .filter(|&node| {
                let note = work.is_any(node, W, &["footnote", "endnote"]) && work.get(node, Some(W), "type").is_none();
                note || work.is(node, W, "comment")
            })
            .collect();
        for node in &gone {
            work.remove(*node);
        }
        if !gone.is_empty() {
            space.mark_dirty(&name);
        }
    }
    space.finish_changed(sink)
}

/// Where the template had comments: the comment parts released, and the people no one is
/// left of dropped (docx-agent's `_drop_comment_part`, `_tidy_people`).
fn drop_comment_parts(sink: &mut EditSink<'_, '_>, main: &str) -> Result<()> {
    if related_ids(sink, main, &[REL_COMMENTS])?.is_empty() {
        return Ok(());
    }
    for kind in COMMENT_PARTS {
        // the first relationship of the kind, as docx-agent drops the part it finds
        if let Some(id) = related_ids(sink, main, &[kind])?.into_iter().next() {
            sink.release(main, &[id])?;
        }
    }
    let docx = sink.docx()?;
    let mut authors = BTreeSet::new();
    for part in &docx.parts {
        let root = part.root();
        if root.name().is(Some(W15_PEOPLE), "people") {
            for person in root.child_elements() {
                if let Some(author) = person.attribute(Some(W15_PEOPLE), "author") {
                    authors.insert(author.to_owned());
                }
            }
        }
    }
    for author in &authors {
        sink.people(None, Some(author))?;
    }
    Ok(())
}

/// The headers and footers no section references any more released.
fn drop_unused_stories(sink: &mut EditSink<'_, '_>, main: &str) -> Result<()> {
    let Some(tree) = sink.doc().tree(main)? else {
        return Ok(());
    };
    let mut used = BTreeSet::new();
    let mut stack = vec![Arc::clone(tree.root())];
    while let Some(element) = stack.pop() {
        for attribute in element.attributes() {
            if attribute.name.namespace() == Some(R) {
                used.insert(attribute.value.to_string());
            }
        }
        stack.extend(element.child_elements().cloned());
    }
    let stale: Vec<String> = related_ids(sink, main, &[REL_HEADER, REL_FOOTER])?
        .into_iter()
        .filter(|id| !used.contains(id))
        .collect();
    sink.release(main, &stale)
}

/// `app.xml`'s `Template` set to `name` (first, where it names none), where there is an
/// `app.xml`.
fn app_template(sink: &mut EditSink<'_, '_>, name: &str) -> Result<()> {
    let package = sink.doc().package().clone();
    let found = relationships(&package, "")
        .map_err(crate::doc::ModelError::from)?
        .into_iter()
        .find(|r| r.rel_type == REL_APP)
        .map(|r| r.target.trim_start_matches('/').to_owned())
        .filter(|p| package.has_part(p));
    let Some(part) = found else {
        return new_app(sink, &package);
    };
    sink.read(Key::Part(Arc::from(part.as_str())));
    let held = sink.doc().clone();
    let mut space = Space::default();
    let Some(work) = space.work(&held, &part)? else {
        return Ok(());
    };
    let root = work.root();
    let node = match work.find(root, APP, "Template") {
        Some(node) => node,
        None => {
            let node = work.new_element(work.element_name(root, APP, "Template", "ns0"));
            work.insert(root, 0, node);
            node
        }
    };
    if work.text(node) != name {
        work.set_text(node, name);
        space.mark_dirty(&part);
    }
    space.finish_changed(sink)
}

/// A new document's `app.xml` (docx-agent's `app_xml()`: Word's element order, one page,
/// nothing counted, the template `Normal.dotm`).
const NEW_APP: &str = concat!(
    r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" "#,
    r#"xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">"#,
    "<Template>Normal.dotm</Template><TotalTime>0</TotalTime><Pages>1</Pages><Words>0</Words>",
    "<Characters>0</Characters><Application>docx-agent</Application><DocSecurity>0</DocSecurity>",
    "<Lines>0</Lines><Paragraphs>0</Paragraphs><ScaleCrop>false</ScaleCrop><Company></Company>",
    "<LinksUpToDate>false</LinksUpToDate><CharactersWithSpaces>0</CharactersWithSpaces>",
    "<SharedDoc>false</SharedDoc><HyperlinksChanged>false</HyperlinksChanged></Properties>"
);
const APP_TYPE: &str = "application/vnd.openxmlformats-officedocument.extended-properties+xml";

/// `docProps/app.xml` made where the template has none ([`NEW_APP`]), with its relationship
/// from the package and its content type.
fn new_app(sink: &mut EditSink<'_, '_>, package: &ooxml_opc::Package) -> Result<()> {
    let part = "docProps/app.xml";
    for name in [part, CONTENT_TYPES_PART, ooxml_opc::name::rels_for("").as_str()] {
        sink.read(Key::Part(Arc::from(name)));
    }
    let tree = ooxml_xml::parse(NEW_APP.as_bytes(), &ooxml_base::Limits::default().xml)
        .map_err(|e| EditError::Refused(e.to_string()))?;
    let root = Arc::unwrap_or_clone(Arc::clone(tree.root()));
    let made = crate::new_part::add_part(package, "", part, root, APP_TYPE, REL_APP)?;
    for name in [part, CONTENT_TYPES_PART, ooxml_opc::name::rels_for("").as_str()] {
        if let Some(tree) = made.tree(name).map_err(crate::doc::ModelError::from)? {
            sink.put(name, tree)?;
        }
    }
    Ok(())
}

/// Every section's page `(width, height)` (portrait), turned as the section is
/// (docx-agent's `_restyle` with a page and no orientation).
fn restyle_page(sink: &mut EditSink<'_, '_>, (width, height): (u32, u32)) -> Result<()> {
    let main = sink.docx()?.main.clone();
    let held = sink.doc().clone();
    let mut space = Space::default();
    let work = space
        .work(&held, &main)?
        .ok_or_else(|| EditError::NoPart(main.clone()))?;
    let root = work.root();
    for section in work.iter(root, W, &["sectPr"]) {
        let Some(size) = work.find(section, W, "pgSz") else {
            continue;
        };
        let landscape = work.get(size, Some(W), "orient") == Some("landscape");
        let (w, h) = if landscape { (height, width) } else { (width, height) };
        work.set(size, work.qname(W, "w", "w"), &w.to_string());
        work.set(size, work.qname(W, "h", "w"), &h.to_string());
        if landscape {
            work.set(size, work.qname(W, "orient", "w"), "landscape");
        } else if work.get(size, Some(W), "orient").is_some() {
            work.remove_attr(size, Some(W), "orient");
        }
    }
    space.mark_dirty(&main);
    space.finish(sink)
}
