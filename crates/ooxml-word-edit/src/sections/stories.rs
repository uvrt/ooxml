//! Owns: a section's headers and footers as docx-agent edits them -- [`SetStoryText`] (a
//! story of its own given text: its first paragraph rewritten and the rest deleted, or a new
//! story part in the Header or Footer style; a paragraph after for each further line),
//! [`LinkToPrevious`] (its own reference removed, its part released when nothing else uses
//! it) and [`UnlinkFromPrevious`] (a copy of the story it showed made its own, as Word
//! unlinks) -- a first-page story turning on `w:titlePg`, an even one the document's
//! `w:evenAndOddHeaders`, each said in a warning.
//!
//! Must not: write but through the sink.

use super::breaks::own_reference;
use super::{find_section, Found, SectionEdited};
use crate::doc::Key;
use crate::names::{R, W, W14, W16DU};
use crate::policy::RevisionPolicy;
use crate::review::space::Space;
use crate::review::xml::{insert_in_order, make};
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::text::write::{check_text, make_run};
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_base::Limits;
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::sections::sections;
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_word_model::parts::{related, REL_SETTINGS};
use ooxml_xml::QName;
use std::sync::Arc;

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

/// A section's story of `which` (`header`, `footer`) and `kind` given text.
#[derive(Debug, Clone)]
pub struct SetStoryText {
    /// The section.
    pub section: String,
    /// `header` or `footer`.
    pub which: String,
    /// `default`, `first` or `even`.
    pub kind: String,
    /// The text, `\n` between paragraphs.
    pub text: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// A section's own story of `which` and `kind` removed: it shows the previous section's.
#[derive(Debug, Clone)]
pub struct LinkToPrevious {
    /// The section.
    pub section: String,
    /// `header` or `footer`.
    pub which: String,
    /// `default`, `first` or `even`.
    pub kind: String,
}

/// A section given a story of its own of `which` and `kind`: a copy of the one it showed.
#[derive(Debug, Clone)]
pub struct UnlinkFromPrevious {
    /// The section.
    pub section: String,
    /// `header` or `footer`.
    pub which: String,
    /// `default`, `first` or `even`.
    pub kind: String,
}

/// What a story edit did: each step's outcome, as docx-agent's edits report them, and the
/// story's name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StoryEdited {
    /// The story (`header2`).
    pub story: String,
    /// Each step's outcome, in order.
    pub steps: Vec<SectionEdited>,
}

/// The part a reference names, through the main part's relationships.
fn reference_part(doc: &WordDoc, docx: &Docx, rid: &str) -> Option<String> {
    let rels = doc.package().relationships(&docx.main).ok()?;
    let found = rels.iter().find(|r| r.id == rid).and_then(|r| r.target_part.clone());
    found
}

/// The reference a section shows for `which`/`kind`: its own, else the last a section
/// before it states (`_inherited_reference`).
fn inherited(docx: &Docx, index: usize, which: &str, kind: &str) -> Option<String> {
    let all = sections(docx);
    let local = format!("{which}Reference");
    (0..=index).rev().find_map(|k| {
        all.get(k)?
            .properties
            .child_elements()
            .find(|n| n.name().is(Some(W), &local) && n.attribute(Some(W), "type").unwrap_or("default") == kind)
            .and_then(|n| n.attribute(Some(R), "id").map(str::to_owned))
    })
}

/// The section's `w:sectPr` made editable.
fn sect_work(sink: &EditSink<'_, '_>, found: &Found) -> Result<(String, Work)> {
    let docx = sink.docx()?;
    let main = docx.main.clone();
    let tree = sink.doc().tree(&main)?.ok_or_else(|| EditError::NoPart(main.clone()))?;
    let element = at_path(tree.root(), &found.section.path).ok_or_else(|| EditError::Path(main.clone()))?;
    Ok((main, Work::of(element, scope_above(tree.root(), &found.section.path))))
}

/// The `w:sectPr` written back.
fn write_sect(sink: &mut EditSink<'_, '_>, main: &str, found: &Found, work: &Work) -> Result<()> {
    let new = work.build().ok_or_else(|| EditError::Path(main.to_owned()))?;
    sink.replace(main, &found.section.path, new, vec![Key::Part(Arc::from(main))])
}

/// A reference of `which`/`kind` to the relationship `rid`, first in the `w:sectPr` in
/// Word's order (`_reference`).
fn add_reference(work: &mut Work, sect: Id, which: &str, kind: &str, rid: &str) {
    let node = make(work, &format!("{which}Reference"), &[("type", kind)]);
    let name = if work.has_prefix_for(sect, R) {
        work.qname(R, "id", "r")
    } else {
        work.declare(node, "ns1", R);
        Arc::new(QName::new(Some("ns1"), "id", Some(R)))
    };
    work.set(node, name, rid);
    work.insert(sect, 0, node);
    super::breaks::order_references(work, sect);
}

/// What showing a `kind` story needs, turned on (`_needs`): `w:titlePg` for a first page,
/// the document's `w:evenAndOddHeaders` for even pages; what was turned on, said.
fn needs(sink: &mut EditSink<'_, '_>, work: &mut Work, sect: Id, kind: &str) -> Result<Vec<String>> {
    let shared = sink.doc().shared_handle();
    let orders = shared.facts.orders();
    let on = |work: &Work, node: Option<Id>| {
        node.is_some_and(|n| {
            !matches!(
                work.get(n, Some(W), "val").map(str::to_lowercase).as_deref(),
                Some("0" | "false" | "off")
            )
        })
    };
    if kind == "first" && !on(work, work.find(sect, W, "titlePg")) {
        for node in work.children(sect).to_vec() {
            if work.is(node, W, "titlePg") {
                work.remove(node);
            }
        }
        let node = make(work, "titlePg", &[]);
        insert_in_order(work, sect, node, orders);
        return Ok(vec![
            "the section's first page now shows its first-page header and footer (w:titlePg)".to_owned(),
        ]);
    }
    if kind != "even" {
        return Ok(Vec::new());
    }
    let docx = sink.docx()?;
    let Some(settings) = related(sink.doc().package(), &docx.main, REL_SETTINGS)? else {
        return Err(EditError::Unsupported("a document without a settings part".to_owned()));
    };
    let held = sink.doc().clone();
    let mut space = Space::default();
    let Some(settings_work) = space.work(&held, &settings)? else {
        return Ok(Vec::new());
    };
    let root = settings_work.root();
    if on(settings_work, settings_work.find(root, W, "evenAndOddHeaders")) {
        return Ok(Vec::new());
    }
    for node in settings_work.children(root).to_vec() {
        if settings_work.is(node, W, "evenAndOddHeaders") {
            settings_work.remove(node);
        }
    }
    let node = make(settings_work, "evenAndOddHeaders", &[]);
    insert_in_order(settings_work, root, node, orders);
    space.mark_dirty(&settings);
    space.finish(sink)?;
    Ok(vec![
        "w:evenAndOddHeaders is document-wide: every section's even pages now show their even header and footer"
            .to_owned(),
    ])
}

/// A story part made from `root` (its paragraphs and rows stamped with fresh ids, `w14`
/// declared), related from the main part: its name and relationship id.
fn new_story_part(sink: &mut EditSink<'_, '_>, which: &str, mut work: Work) -> Result<(String, String)> {
    let docx = sink.docx()?;
    let main = docx.main.clone();
    let package = sink.doc().package().clone();
    let part = crate::new_part::unused_name(
        &package,
        &format!("word/{which}1.xml"),
        &format!("word/{which}{{n}}.xml"),
    );
    let root = work.root();
    for node in work.iter(root, W, &["p", "tr"]) {
        let drawn = sink.draw(&part, 2)?;
        let (Some(para), Some(text)) = (drawn.first(), drawn.get(1)) else {
            return Err(EditError::Path(part));
        };
        for (local, value) in [("paraId", para), ("textId", text)] {
            work.set(node, Arc::new(QName::new(Some("w14"), local, Some(W14))), value);
        }
    }
    let built = work.build().ok_or_else(|| EditError::Path(part.clone()))?;
    let head = crate::sink::ensured_head(&built, false, &[]);
    let built = match head {
        Some((head, _)) => Arc::new(ooxml_xml::Element::from_head(Arc::new(head), built.children().clone())),
        None => built,
    };
    let content_type = format!("application/vnd.openxmlformats-officedocument.wordprocessingml.{which}+xml");
    let made = crate::new_part::add_part(
        &package,
        &main,
        &part,
        (*built).clone(),
        &content_type,
        &format!("{REL}{which}"),
    )?;
    let rels = ooxml_opc::name::rels_for(&main);
    for written in [part.as_str(), ooxml_opc::CONTENT_TYPES_PART, rels.as_str()] {
        if let Some(tree) = made.tree(written)? {
            sink.put(written, tree)?;
        }
    }
    let rid = made
        .relationships(&main)?
        .iter()
        .find(|r| r.target_part.as_deref() == Some(part.as_str()))
        .map(|r| r.id.clone())
        .ok_or_else(|| EditError::Path(part.clone()))?;
    Ok((part, rid))
}

/// A story part's root holding one paragraph in `style`, with `text` in lines
/// (`_new_story_part`).
fn story_root(which: &str, style: &str, text: &str) -> Result<Work> {
    let tag = if which == "header" { "hdr" } else { "ftr" };
    let data = format!(
        r#"<w:{tag} xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/>"#
    );
    let tree =
        ooxml_xml::parse(data.as_bytes(), &Limits::default().xml).map_err(|e| EditError::Refused(e.to_string()))?;
    let mut work = Work::of(tree.root(), Vec::new());
    let root = work.root();
    let mut paragraph = make(&mut work, "p", &[]);
    let properties = make(&mut work, "pPr", &[]);
    let named = make(&mut work, "pStyle", &[("val", style)]);
    work.append(properties, named);
    work.append(paragraph, properties);
    if !text.is_empty() {
        for (k, line) in text.split('\n').enumerate() {
            if k > 0 {
                work.append(root, paragraph);
                paragraph = work.deep_copy(paragraph);
                for child in work.children(paragraph).to_vec() {
                    if !work.is(child, W, "pPr") {
                        work.remove(child);
                    }
                }
            }
            let run = make_run(&mut work, line, None);
            work.append(paragraph, run);
        }
    }
    work.append(root, paragraph);
    Ok(work)
}

/// A story of its own for the section (docx-agent's `_add_story`).
fn add_story(sink: &mut EditSink<'_, '_>, section: &str, which: &str, kind: &str, text: &str) -> Result<SectionEdited> {
    check_text(text)?;
    let docx = sink.docx()?;
    let found = find_section(sink.doc(), &docx, section)?;
    let (_, probe) = sect_work(sink, &found)?;
    if own_reference(&probe, probe.root(), which, kind).is_some() {
        return Err(EditError::Refused(format!(
            "{section} has its own {kind} {which} already"
        )));
    }
    crate::builtin::resolve(&docx, which, "paragraph")?;
    let renamed = stamp_document(sink, &docx)?;
    let style = crate::builtin::ensure(sink, which, "paragraph")?;
    let mut root = story_root(which, &style, text)?;
    let mut dated = false;
    if let (RevisionPolicy::Tracked(tracking), false) = (sink.policy().clone(), text.is_empty()) {
        let top = root.root();
        let first = root.children(top).first().copied();
        if let Some(first) = first {
            let ((), _, d) = sink.stamped(tracking, |stamp| {
                crate::marks::insert_paragraph_content(&mut root, first, stamp);
                Ok(())
            })?;
            dated = d;
        }
    }
    let (part, rid) = new_story_part(sink, which, root)?;
    let (main, mut work) = sect_work(sink, &found)?;
    let sect = work.root();
    add_reference(&mut work, sect, which, kind, &rid);
    let warnings = needs(sink, &mut work, sect, kind)?;
    write_sect(sink, &main, &found, &work)?;
    if dated {
        sink.ensure_w14(&part, false, &[("w16du", W16DU)])?;
    }
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        sink.people(Some(&tracking.author), Some(&tracking.author))?;
    }
    let docx = sink.docx()?;
    let story = docx.part(&part).map(|p| p.story.clone()).unwrap_or_default();
    let paragraph = docx
        .part(&part)
        .and_then(|p| p.ids.paragraphs().first().map(|p| p.id.clone()))
        .unwrap_or_default();
    Ok(SectionEdited {
        id: Some(story.clone()),
        changed: true,
        created: vec![story, paragraph],
        renamed,
        removed: Vec::new(),
        warnings,
    })
}

impl Command<WordDoc> for SetStoryText {
    type Out = StoryEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        find_section(doc, &*doc.docx()?, &self.section).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<StoryEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl SetStoryText {
    /// The edit, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<StoryEdited> {
        sink.read_everything();
        let text = self.text.replace("\r\n", "\n");
        let lines: Vec<&str> = text.split('\n').collect();
        let docx = sink.docx()?;
        let found = find_section(sink.doc(), &docx, &self.section)?;
        let (_, probe) = sect_work(&*sink, &found)?;
        let own = own_reference(&probe, probe.root(), &self.which, &self.kind)
            .and_then(|n| probe.get(n, Some(R), "id").map(str::to_owned))
            .and_then(|rid| reference_part(sink.doc(), &docx, &rid));
        let mut steps = Vec::new();
        let first_line = lines.first().copied().unwrap_or("");
        let story = match own {
            Some(part) => {
                let story = docx.part(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
                let name = story.story.clone();
                let ids: Vec<String> = story.ids.paragraphs().iter().map(|p| p.id.clone()).collect();
                let first = ids
                    .first()
                    .cloned()
                    .ok_or_else(|| EditError::Refused(format!("{name} has no paragraph")))?;
                let done = crate::set_text::write_text(&mut *sink, &first, first_line)?;
                steps.push(SectionEdited {
                    id: Some(done.id.clone()),
                    changed: done.changed,
                    renamed: done.renamed.clone(),
                    ..SectionEdited::default()
                });
                let mut renamed = done.renamed;
                for extra in ids.iter().skip(1) {
                    let target = crate::ranges::renamed_id(&renamed, extra);
                    let gone = crate::delete_write::delete(&mut *sink, &target)?;
                    renamed.extend(gone.renamed.clone());
                    steps.push(SectionEdited {
                        id: gone.id,
                        changed: true,
                        removed: gone.removed,
                        renamed: gone.renamed,
                        ..SectionEdited::default()
                    });
                }
                name
            }
            None => {
                let made = add_story(&mut *sink, &self.section, &self.which, &self.kind, first_line)?;
                let name = made.id.clone().unwrap_or_default();
                steps.push(made);
                name
            }
        };
        let docx = sink.docx()?;
        let mut last = docx
            .story(&story)
            .and_then(|s| s.ids.paragraphs().first().map(|p| p.id.clone()))
            .ok_or_else(|| EditError::Refused(format!("{story} has no paragraph")))?;
        for line in lines.iter().skip(1) {
            let done = crate::paragraphs::insert_paragraph(&mut *sink, &last, (true, line, None))?;
            steps.push(SectionEdited {
                id: Some(done.id.clone()),
                changed: true,
                created: vec![done.id.clone()],
                renamed: done.renamed,
                ..SectionEdited::default()
            });
            last = done.id;
        }
        Ok(StoryEdited { story, steps })
    }
}

impl Command<WordDoc> for LinkToPrevious {
    type Out = SectionEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        find_section(doc, &*doc.docx()?, &self.section).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<SectionEdited> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl LinkToPrevious {
    /// The edit, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<SectionEdited> {
        sink.read_everything();
        let docx = sink.docx()?;
        let found = find_section(sink.doc(), &docx, &self.section)?;
        let canonical = found.section.id.clone();
        let (_, probe) = sect_work(&*sink, &found)?;
        let Some(rid) = own_reference(&probe, probe.root(), &self.which, &self.kind)
            .and_then(|n| probe.get(n, Some(R), "id").map(str::to_owned))
        else {
            return Ok(SectionEdited {
                id: Some(canonical),
                ..SectionEdited::default()
            });
        };
        let story = reference_part(sink.doc(), &docx, &rid).and_then(|p| docx.part(&p).map(|s| (p, s.story.clone())));
        let renamed = stamp_document(&mut *sink, &docx)?;
        let (main, mut work) = sect_work(&*sink, &found)?;
        let sect = work.root();
        if let Some(node) = own_reference(&work, sect, &self.which, &self.kind) {
            work.remove(node);
        }
        write_sect(&mut *sink, &main, &found, &work)?;
        sink.release(&main, std::slice::from_ref(&rid))?;
        let removed = story
            .filter(|(part, _)| !sink.doc().package().has_part(part))
            .map(|(_, name)| vec![name])
            .unwrap_or_default();
        Ok(SectionEdited {
            id: Some(canonical),
            changed: true,
            renamed,
            removed,
            ..SectionEdited::default()
        })
    }
}

impl Command<WordDoc> for UnlinkFromPrevious {
    type Out = SectionEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        find_section(doc, &*doc.docx()?, &self.section).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<SectionEdited> {
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl UnlinkFromPrevious {
    /// The edit, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<SectionEdited> {
        sink.read_everything();
        let docx = sink.docx()?;
        let found = find_section(sink.doc(), &docx, &self.section)?;
        let (_, probe) = sect_work(&*sink, &found)?;
        if let Some(rid) = own_reference(&probe, probe.root(), &self.which, &self.kind)
            .and_then(|n| probe.get(n, Some(R), "id").map(str::to_owned))
        {
            let story = reference_part(sink.doc(), &docx, &rid)
                .and_then(|p| docx.part(&p).map(|s| s.story.clone()))
                .unwrap_or_default();
            return Ok(SectionEdited {
                id: Some(story),
                ..SectionEdited::default()
            });
        }
        let source = inherited(&docx, found.index, &self.which, &self.kind)
            .and_then(|rid| reference_part(sink.doc(), &docx, &rid))
            .filter(|p| sink.doc().package().has_part(p));
        let Some(source) = source else {
            return add_story(&mut *sink, &self.section, &self.which, &self.kind, "");
        };
        let renamed = stamp_document(&mut *sink, &docx)?;
        let tree = sink
            .doc()
            .tree(&source)?
            .ok_or_else(|| EditError::NoPart(source.clone()))?;
        let copy = Work::of(tree.root(), Vec::new());
        let (part, rid) = new_story_part(&mut *sink, &self.which, copy)?;
        let source_rels = ooxml_opc::name::rels_for(&source);
        if let Some(rels) = sink.doc().package().tree(&source_rels)? {
            sink.put(&ooxml_opc::name::rels_for(&part), rels)?;
        }
        let (main, mut work) = sect_work(&*sink, &found)?;
        let sect = work.root();
        add_reference(&mut work, sect, &self.which, &self.kind, &rid);
        write_sect(&mut *sink, &main, &found, &work)?;
        let docx = sink.docx()?;
        let story = docx.part(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
        let mut created = vec![story.story.clone()];
        created.extend(story.ids.paragraphs().iter().map(|p| p.id.clone()));
        Ok(SectionEdited {
            id: Some(story.story.clone()),
            changed: true,
            created,
            renamed,
            ..SectionEdited::default()
        })
    }
}
