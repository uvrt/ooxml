//! A copy from another document and an edit of what it reads, interleaved (lane P4f9): the
//! copy prepared on one version, the edit committed before the copy commits; the copy's
//! version the same, byte for byte, as the two run serially in commit order make it.
// Tests may unwrap, panic and index (docs/architecture/README.md, section 2.5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{fixture, written, Monotonic, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::{Caller, Command, KeySet, StoreConfig, Txn};
use ooxml_word_edit::{
    BookmarkPolicy, CopyBlocks, CopyRange, CopySource, EditError, Editor, Key, ListAction, ListEdit, ListPolicy,
    RevisionPolicy, SetText, StylePolicy, UnmappedPolicy, WordDoc,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A command that, the first time it is prepared, waits after preparing until it is let go
/// (another commit made in between, deterministically).
struct Paused<'a, C> {
    command: &'a C,
    prepared: Sender<()>,
    go: Mutex<Receiver<()>>,
    first: AtomicBool,
}

impl<C: Command<WordDoc, Error = EditError>> Command<WordDoc> for Paused<'_, C> {
    type Out = C::Out;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        self.command.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<C::Out, EditError> {
        let out = self.command.prepare(tx)?;
        if self.first.swap(false, Ordering::SeqCst) {
            self.prepared.send(()).unwrap();
            self.go.lock().unwrap().recv().unwrap();
        }
        Ok(out)
    }
}

/// `last` prepared on `editor`'s head, `between` committed before `last` commits; on
/// `serial` (the same document), the two run in that order. Each version the same, byte for
/// byte, as the serial one.
fn interleaved<B, L>(shared: &Editor, serial: &Editor, between: &B, last: &L)
where
    B: Command<WordDoc, Error = EditError> + Sync,
    L: Command<WordDoc, Error = EditError> + Sync,
    L::Out: Send,
{
    let (prepared, wait) = channel();
    let (release, go) = channel();
    let paused = Paused {
        command: last,
        prepared,
        go: Mutex::new(go),
        first: AtomicBool::new(true),
    };
    let (first, second) = std::thread::scope(|scope| {
        let handle = scope.spawn(|| shared.store().run(&Caller::named("last"), &paused));
        wait.recv().unwrap();
        let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            shared.store().run(&Caller::anonymous(), between).unwrap()
        }));
        release.send(()).unwrap();
        (first.unwrap(), handle.join().unwrap().unwrap())
    });
    let serial_first = serial.store().run(&Caller::anonymous(), between).unwrap();
    assert_eq!(
        written(serial_first.version.doc()),
        written(first.version.doc()),
        "the edit between"
    );
    let replayed = serial.store().run(&Caller::anonymous(), last).unwrap();
    let body = |doc: &WordDoc| String::from_utf8(doc.tree("word/document.xml").unwrap().unwrap().write()).unwrap();
    assert_eq!(
        body(replayed.version.doc()),
        body(second.version.doc()),
        "the body, serial (left) and concurrent (right)"
    );
    assert_eq!(
        written(replayed.version.doc()),
        written(second.version.doc()),
        "the parts, serial and concurrent"
    );
}

/// A paragraph style of the fixture's sheet, added to it.
fn with_style(styles: &str, id: &str, name: &str) -> String {
    styles.replace(
        "</w:styles>",
        &format!(
            r#"<w:style w:type="paragraph" w:customStyle="1" w:styleId="{id}"><w:name w:val="{name}"/><w:basedOn w:val="Standaard"/></w:style></w:styles>"#
        ),
    )
}

/// `generated/lists-and-styles.docx` with its sheet made by `styles` and a body of
/// `paragraphs` (the fixture's section kept).
fn built(styles: impl Fn(&str) -> String, paragraphs: &str) -> Package {
    let package = Package::open(fixture("generated/lists-and-styles.docx"), &Limits::default()).unwrap();
    let text = |part: &str| String::from_utf8(package.read(part).unwrap().unwrap().to_vec()).unwrap();
    let sheet = styles(&text("word/styles.xml"));
    let document = text("word/document.xml");
    let start = document.find("<w:body>").unwrap() + "<w:body>".len();
    let end = document.find("<w:sectPr>").unwrap();
    let body = format!("{}{paragraphs}{}", &document[..start], &document[end..]);
    package
        .with_bytes("word/styles.xml", Arc::from(sheet.into_bytes()))
        .with_bytes("word/document.xml", Arc::from(body.into_bytes()))
}

/// The body of `editor`'s head.
fn body(editor: &Editor) -> String {
    let head = editor.store().snapshot();
    String::from_utf8(head.doc().tree("word/document.xml").unwrap().unwrap().write()).unwrap()
}

fn open(package: Package) -> Editor {
    Editor::open(
        package,
        StoreConfig::default(),
        Arc::new(Monotonic(Instant::now())),
        Arc::new(Unlimited),
    )
    .unwrap()
}

/// A paragraph with id `id`, style `style` (none: the default) and text `text` (none: blank).
fn paragraph(id: &str, style: Option<&str>, text: Option<&str>) -> String {
    let properties = style.map_or(String::new(), |s| format!(r#"<w:pPr><w:pStyle w:val="{s}"/></w:pPr>"#));
    let run = text.map_or(String::new(), |t| {
        format!(r#"<w:r><w:t xml:space="preserve">{t}</w:t></w:r>"#)
    });
    format!(r#"<w:p w14:paraId="{id}" w14:textId="77777777">{properties}{run}</w:p>"#)
}

/// docx-agent's importer (`unmapped="body"`, `_body_style`): a source style the destination
/// lacks becomes the destination's body style, the style most of its plain paragraphs with
/// text use. A blank paragraph given text in between tips the count (the default style
/// two to one, where the copy was prepared one to one and the first counted, `Plain Two`,
/// won), so the copy, which read which style the body's paragraphs use, is prepared again.
#[test]
fn a_copy_reads_the_body_style_a_paragraph_given_text_changes() {
    let destination = || {
        built(
            |s| with_style(s, "Plain2", "Plain Two"),
            &[
                paragraph("5A000101", Some("Plain2"), Some("One")),
                paragraph("5A000102", None, None),
                paragraph("5A000103", None, Some("Two")),
            ]
            .concat(),
        )
    };
    let source = open(built(
        |s| with_style(s, "SourceOnly", "Source Only"),
        &paragraph("5B000001", Some("SourceOnly"), Some("Copied")),
    ));
    let copy = CopyBlocks {
        source: CopySource::Other(Box::new(source.store().snapshot().doc().clone())),
        range: CopyRange::Blocks("p:5B000001".to_owned()),
        at: "end".into(),
        policy: RevisionPolicy::Untracked,
        styles: StylePolicy::UseDestination,
        style_map: Vec::new(),
        unmapped: UnmappedPolicy::Body,
        lists: ListPolicy::Separate,
        bookmarks: BookmarkPolicy::Rename,
    };
    let set = SetText {
        target: ooxml_word_model::address::Address::from("p:5A000102".to_owned()),
        text: "Given text".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    // the body style the copy gives: `Plain Two` before the edit, the default after it
    let styled = |editor: &Editor| body(editor).matches(r#"<w:pStyle w:val="Plain2"/>"#).count();
    let (before, after) = (open(destination()), open(destination()));
    before.store().run(&Caller::anonymous(), &copy).unwrap();
    after.store().run(&Caller::anonymous(), &set).unwrap();
    after.store().run(&Caller::anonymous(), &copy).unwrap();
    assert_eq!(
        (styled(&before), styled(&after)),
        (2, 1),
        "the edit changes the body style"
    );
    interleaved(&open(destination()), &open(destination()), &set, &copy);
}

/// A list item of list `num_id`, level 0, with id `id` and text `text`.
fn item(id: &str, num_id: u32, text: &str) -> String {
    format!(
        r#"<w:p w14:paraId="{id}" w14:textId="77777777"><w:pPr><w:pStyle w:val="Lijstalinea"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="{num_id}"/></w:numPr></w:pPr><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#
    )
}

/// docx-agent's importer (`lists="continue"`, `_continued`): a copied list item joins the
/// list of the last numbered paragraph before the place when its first level counts as the
/// copied one's. The items before the place taken out of their list in between (the list
/// kept: an item after the place still uses it) leave none before it, so the copy, which
/// read the lists of the paragraphs before the place, is prepared again and starts a list
/// of its own.
#[test]
fn a_copy_reads_the_list_before_the_place_an_edit_takes_away() {
    let destination = || {
        open(built(
            str::to_owned,
            &[
                item("5A000201", 1, "First"),
                item("5A000202", 1, "Second"),
                paragraph("5A000203", None, Some("Between")),
                item("5A000204", 2, "Restarted"),
                item("5A000205", 1, "Later"),
            ]
            .concat(),
        ))
    };
    let copy = CopyBlocks {
        source: CopySource::Own,
        range: CopyRange::Blocks("p:5A000204".to_owned()),
        at: "after:p:5A000203".into(),
        policy: RevisionPolicy::Untracked,
        styles: StylePolicy::UseDestination,
        style_map: Vec::new(),
        unmapped: UnmappedPolicy::Import,
        lists: ListPolicy::Continue,
        bookmarks: BookmarkPolicy::Rename,
    };
    let remove = ListEdit {
        targets: (["p:5A000201", "p:5A000202"].map(str::to_owned).to_vec())
            .iter()
            .map(|t| ooxml_word_model::address::Address::parse(t))
            .collect(),
        action: ListAction::Remove,
        policy: RevisionPolicy::Untracked,
    };
    // the list the copy goes into: the first one before the edit, a new one after it
    let listed = |editor: &Editor, made: &str| {
        let body = body(editor);
        let at = body
            .find(&format!("w14:paraId=\"{}\"", made.trim_start_matches("p:")))
            .unwrap();
        let copy = &body[at..at + body[at..].find("</w:p>").unwrap()];
        copy.split("<w:numId w:val=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_owned()
    };
    let (before, after) = (destination(), destination());
    let made = before.store().run(&Caller::anonymous(), &copy).unwrap().out.blocks[0].clone();
    after.store().run(&Caller::anonymous(), &remove).unwrap();
    let made_after = after.store().run(&Caller::anonymous(), &copy).unwrap().out.blocks[0].clone();
    let numbers = (listed(&before, &made), listed(&after, &made_after));
    assert!(
        numbers.0 == "1" && numbers.1 != "1",
        "the edit changes the list it joins: {numbers:?}"
    );
    interleaved(&destination(), &destination(), &remove, &copy);
}

/// Two writers that each give text to a blank `Plain Two` paragraph, prepared on one version
/// and committed one after the other: each adds a use of the style, a write of the body's
/// style uses, which are derived (recomputed where it lands), so neither conflicts with the
/// other. Both uses are recorded: each step writes the style uses, and a copy run after
/// them maps to `Plain Two`, which only both uses together make the body style (one use
/// ties with the default style's, counted first).
#[test]
fn two_writers_adding_uses_of_one_style_commit_without_a_conflict() {
    let destination = || {
        open(built(
            |s| with_style(s, "Plain2", "Plain Two"),
            &[
                paragraph("5A000301", None, Some("One")),
                paragraph("5A000302", Some("Plain2"), None),
                paragraph("5A000303", Some("Plain2"), None),
            ]
            .concat(),
        ))
    };
    let given = |id: &str| SetText {
        target: ooxml_word_model::address::Address::from(format!("p:{id}")),
        text: "Given text".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    let (first, second) = (given("5A000302"), given("5A000303"));
    let shared = destination();
    interleaved(&shared, &destination(), &first, &second);
    let stats = shared.store().stats();
    assert_eq!(
        (stats.conflicts, stats.exclusive),
        (0, 0),
        "a style use added is no conflict"
    );
    let uses = Key::StyleUse(ooxml_opc::PartName::from("word/document.xml"));
    let steps = shared.store().steps();
    assert_eq!(steps.len(), 2);
    assert!(
        steps.iter().all(|step| step.writes.contains(&uses)),
        "each step writes the style uses"
    );
    let source = open(built(
        |s| with_style(s, "SourceOnly", "Source Only"),
        &paragraph("5B000301", Some("SourceOnly"), Some("Copied")),
    ));
    let copy = CopyBlocks {
        source: CopySource::Other(Box::new(source.store().snapshot().doc().clone())),
        range: CopyRange::Blocks("p:5B000301".to_owned()),
        at: "end".into(),
        policy: RevisionPolicy::Untracked,
        styles: StylePolicy::UseDestination,
        style_map: Vec::new(),
        unmapped: UnmappedPolicy::Body,
        lists: ListPolicy::Separate,
        bookmarks: BookmarkPolicy::Rename,
    };
    let styled = |editor: &Editor| body(editor).matches(r#"<w:pStyle w:val="Plain2"/>"#).count();
    let one = destination();
    one.store().run(&Caller::anonymous(), &first).unwrap();
    one.store().run(&Caller::anonymous(), &copy).unwrap();
    assert_eq!(styled(&one), 2, "one use: the default style is the body style");
    shared.store().run(&Caller::anonymous(), &copy).unwrap();
    assert_eq!(styled(&shared), 3, "both uses: `Plain Two` is the body style");
}

/// The earlier of two steps that each add a use of one style undoes past the later one: the
/// style uses both write are derived, which entangles neither with the other (the later step
/// did not read them).
#[test]
fn a_step_adding_a_style_use_undoes_past_another() {
    let editor = open(built(
        |s| with_style(s, "Plain2", "Plain Two"),
        &[
            paragraph("5A000401", Some("Plain2"), None),
            paragraph("5A000402", Some("Plain2"), None),
        ]
        .concat(),
    ));
    let given = |id: &str| SetText {
        target: ooxml_word_model::address::Address::from(format!("p:{id}")),
        text: "Given text".to_owned(),
        policy: RevisionPolicy::Untracked,
    };
    editor.store().run(&Caller::anonymous(), &given("5A000401")).unwrap();
    editor.store().run(&Caller::anonymous(), &given("5A000402")).unwrap();
    let earlier = editor.store().steps()[0].step;
    editor.store().undo_step(earlier, &KeySet::new()).unwrap();
    assert_eq!(body(&editor).matches("Given text").count(), 1, "the later step kept");
}
