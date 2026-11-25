//! Owns: [`Planned`], an edit a handler plans in the edit's own transaction -- the command
//! and what the reply needs worked out on the version the edit is applied to, its reads
//! recorded there ([`ooxml_word_edit::anchor_in`], [`ooxml_word_edit::section_in`],
//! [`ooxml_word_edit::last_block_in`], [`ooxml_word_edit::read_matching`], the command's own)
//! -- and [`run_planned`], which runs one. A commit meanwhile that changes what the plan
//! read is a conflict: the plan is made again on the newer version, never applied as worked
//! out on the version before. A refusal the plan makes is the call's, nothing changed (a
//! read-only answer of the version it was made on).
//!
//! Must not: plan from the head outside the transaction; change anything but through the
//! command the plan makes.

use crate::review::Editing;
use ooxml_store::{Command, Txn};
use ooxml_tools::ToolError;
use ooxml_word_edit::{EditError, Edited, WordDoc};

/// An edit planned in its transaction: `.0` makes, on the transaction's version, the command
/// to apply and what the reply needs (or the call's refusal); the command is then checked
/// and prepared on that version, in the same transaction.
pub(crate) struct Planned<F>(pub(crate) F);

impl<F, C, P> Command<WordDoc> for Planned<F>
where
    F: Fn(&mut Txn<'_, WordDoc>) -> Result<(C, P), ToolError>,
    C: Command<WordDoc, Error = EditError>,
{
    /// The command's out and the plan's, or the plan's refusal.
    type Out = Result<(C::Out, P), ToolError>;
    type Error = EditError;

    /// Nothing to check before the plan: the plan, and the command's checks, are made in
    /// [`Command::prepare`] on the version the edit is applied to.
    fn validate(&self, _doc: &WordDoc) -> Result<(), EditError> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Self::Out, EditError> {
        let (command, plan) = match (self.0)(tx) {
            Ok(planned) => planned,
            Err(refusal) => return Ok(Err(refusal)),
        };
        command.validate(tx.doc())?;
        let out = command.prepare(tx)?;
        Ok(Ok((out, plan)))
    }
}

/// `plan` run as one edit through the call's editor ([`Planned`]): the edit, its out the
/// command's and the plan's, or the command's refusal.
///
/// # Errors
/// The plan's refusal (nothing changed).
#[allow(clippy::type_complexity)]
pub(crate) fn run_planned<F, C, P>(
    editing: &Editing<'_>,
    plan: F,
) -> Result<Result<Edited<(C::Out, P)>, EditError>, ToolError>
where
    F: Fn(&mut Txn<'_, WordDoc>) -> Result<(C, P), ToolError>,
    C: Command<WordDoc, Error = EditError>,
{
    match editing.editor.run(editing.caller, &Planned(plan)) {
        Ok(done) => done.transpose().map(Ok),
        Err(error) => Ok(Err(error)),
    }
}

#[cfg(test)]
pub(crate) mod held {
    //! A planned edit held after its first prepare while another edit commits (the forced
    //! interleaving of the edit core's `interleave` tests, on one thread: the other edit is
    //! committed from inside the first's prepare, which holds no lock), and what the tests
    //! compare it with: the same call made after that edit, one at a time.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::Planned;
    use crate::review::{CallEditor, Editing};
    use ooxml_base::{Cancel, Limits};
    use ooxml_opc::Package;
    use ooxml_store::{Caller, Clock, Command, StoreConfig, Txn};
    use ooxml_tools::{ToolError, ToolResult};
    use ooxml_word_edit::{
        EditError, Editor, InsertBeside, InsertMarkdown, MarkdownOptions, RevisionPolicy, Side, TextPlace, WordDoc,
        WriteMap,
    };
    use ooxml_word_model::address::Address;
    use ooxml_xml::{Element, Node};
    use serde_json::Value;
    use std::cell::Cell;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    use std::sync::Arc;
    use std::time::Duration;

    /// `command`, `meanwhile` run once after its first prepare.
    struct Meanwhile<C, M> {
        command: C,
        meanwhile: M,
        first: Cell<bool>,
    }

    impl<C: Command<WordDoc, Error = EditError>, M: Fn()> Command<WordDoc> for Meanwhile<C, M> {
        type Out = C::Out;
        type Error = EditError;

        fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
            self.command.validate(doc)
        }

        fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<C::Out, EditError> {
            let out = self.command.prepare(tx)?;
            if self.first.replace(false) {
                (self.meanwhile)();
            }
            Ok(out)
        }
    }

    struct Never;

    impl Cancel for Never {
        fn is_cancelled(&self) -> bool {
            false
        }
    }

    struct Still;

    impl Clock for Still {
        fn monotonic(&self) -> Duration {
            Duration::ZERO
        }
    }

    struct Unlimited;

    impl ooxml_derive::CacheBudget for Unlimited {
        fn reserve(&self, _bytes: usize) -> bool {
            true
        }
        fn release(&self, _bytes: usize) {}
        fn used(&self) -> usize {
            0
        }
    }

    /// An editor of the generated fixture `name` (`crates/ooxml-oracle/fixtures/generated`)
    /// after a first edit, which stamps every block (a paragraph put at the end).
    pub(crate) fn opened(name: &str) -> Editor {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ooxml-oracle/fixtures/generated/");
        let bytes: Arc<[u8]> = Arc::from(std::fs::read(format!("{path}{name}")).unwrap());
        let package = Package::open(bytes, &Limits::default()).unwrap();
        let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
        editor.run(&Caller::anonymous(), &markdown("end", "Stamped.")).unwrap();
        editor
    }

    /// `markdown` put at `at`.
    pub(crate) fn markdown(at: &str, markdown: &str) -> InsertMarkdown {
        InsertMarkdown {
            markdown: markdown.to_owned(),
            at: at.into(),
            style_map: WriteMap::default(),
            options: MarkdownOptions::default(),
            policy: RevisionPolicy::Untracked,
        }
    }

    /// `text` put at the start of `paragraph`: what follows in it moves on.
    pub(crate) fn before(paragraph: &str, text: &str) -> InsertBeside {
        InsertBeside {
            place: TextPlace::Range(Address::parse(&format!("{paragraph}@0"))),
            side: Side::Before,
            text: text.to_owned(),
            policy: RevisionPolicy::Untracked,
        }
    }

    /// A tool's arguments, as a docx call gives them.
    pub(crate) fn args<A: ooxml_tools::ToolArgs>(mut value: Value) -> A {
        value["doc"] = Value::from("d1");
        A::accept(value, Some("docx")).unwrap().0
    }

    /// Text put before "this year" in the paragraph that holds it, the only place it occurs:
    /// where it is moves on.
    pub(crate) fn shifted(editor: &Editor) -> InsertBeside {
        before(&paragraph(editor, "Revenue"), "So far, ")
    }

    /// The fixture every find test runs on.
    pub(crate) fn marked() -> Editor {
        opened("ids-and-markup.docx")
    }

    /// The id of the body's paragraph whose text starts with `start`.
    pub(crate) fn paragraph(editor: &Editor, start: &str) -> String {
        let docx = editor.store().snapshot().doc().docx().unwrap();
        let body = docx.body().unwrap();
        let k = body.texts().iter().position(|t| t.starts_with(start)).unwrap();
        body.ids.paragraphs().get(k).unwrap().id.clone()
    }

    /// A call's `editing` of `editor`, for `f`.
    pub(crate) fn calling<R>(editor: &Editor, f: impl FnOnce(&Editing<'_>) -> R) -> R {
        let call_editor = CallEditor::new(editor);
        let caller = Caller::named("last");
        let opened = editor.store().snapshot().doc().clone();
        let editing = Editing::new(&call_editor, &caller, &opened, "d1", &Never, "2026-11-25T10:00:00Z");
        f(&editing)
    }

    /// The plan run as one edit, held after its first prepare while `between` commits: the
    /// plan's refusal, if it made one in the end, and the conflicts the store counted.
    pub(crate) fn raced<F, C, P, B>(editor: &Editor, plan: F, between: &B) -> (Option<ToolError>, u64)
    where
        F: Fn(&mut Txn<'_, WordDoc>) -> Result<(C, P), ToolError>,
        C: Command<WordDoc, Error = EditError>,
        B: Command<WordDoc, Error = EditError>,
    {
        let before = editor.store().stats().conflicts;
        let held = Meanwhile {
            command: Planned(plan),
            meanwhile: || {
                editor.store().run(&Caller::named("between"), between).unwrap();
            },
            first: Cell::new(true),
        };
        let done = editor.run(&Caller::named("last"), &held).unwrap();
        (done.out.err(), editor.store().stats().conflicts - before)
    }

    /// A planned call raced against another edit, and the same call made after it.
    pub(crate) struct Compared {
        /// The body's [`shape`] after the race.
        pub(crate) raced: String,
        /// After the other edit, then the call.
        pub(crate) serial: String,
        /// The raced plan's refusal, where it made one.
        pub(crate) refused: Option<ToolError>,
        /// The conflicts the race counted.
        pub(crate) conflicts: u64,
        /// The serial call's reply.
        pub(crate) reply: ToolResult<Value>,
    }

    /// On two editors `open` makes alike: the call's plan raced against `between` (each made
    /// of its editor), and `between` committed, then the call made through its tool.
    pub(crate) fn compare<B>(
        open: impl Fn() -> Editor,
        between: impl Fn(&Editor) -> B,
        race: impl Fn(&Editor, &B) -> (Option<ToolError>, u64),
        call: impl Fn(&Editing<'_>) -> ToolResult<Value>,
    ) -> Compared
    where
        B: Command<WordDoc, Error = EditError>,
    {
        let shared = open();
        let (refused, conflicts) = race(&shared, &between(&shared));
        let alone = open();
        alone.store().run(&Caller::named("between"), &between(&alone)).unwrap();
        let reply = calling(&alone, call);
        Compared {
            raced: shape(&shared),
            serial: shape(&alone),
            refused,
            conflicts,
            reply,
        }
    }

    impl Compared {
        /// Both made the call's edit, the race after a conflict, and the body is the same.
        pub(crate) fn assert_same_edit(&self) {
            assert!(self.refused.is_none(), "{:?}", self.refused);
            assert!(self.conflicts >= 1, "the edit meanwhile is a conflict");
            assert!(
                matches!(self.reply.outcome, ooxml_tools::Outcome::Done(_)),
                "{:?}",
                self.reply.to_json()
            );
            assert_eq!(self.raced, self.serial);
        }

        /// Both refused the call alike, the race after a conflict, and the body is the same.
        pub(crate) fn assert_same_refusal(&self) {
            assert!(self.conflicts >= 1, "the edit meanwhile is a conflict");
            let refused = self.refused.as_ref().expect("the race refused");
            match &self.reply.outcome {
                ooxml_tools::Outcome::Failed(error) => assert_eq!(refused, error),
                _ => panic!("the serial call refused: {:?}", self.reply.to_json()),
            }
            assert_eq!(self.raced, self.serial);
        }
    }

    /// The body as text, the marks a placed edit makes (a control, a note's reference, a
    /// field, a drawing, a bookmark, a link, a comment's range, a section break) and the
    /// formatting the tests set (bold, italic, alignment, style, list, keep with next), ids left out: what
    /// two runs that made the same edits share.
    pub(crate) fn shape(editor: &Editor) -> String {
        let docx = editor.store().snapshot().doc().docx().unwrap();
        let mut out = String::new();
        walk(docx.body().unwrap().root(), &mut out);
        out
    }

    fn walk(element: &Element, out: &mut String) {
        let local = element.name().local();
        match local {
            "t" | "instrText" | "delText" => out.push_str(&element.text_content()),
            "p" => out.push('\n'),
            "b" | "i" => out.push_str(&format!("[{local}]")),
            "jc" | "pStyle" => out.push_str(&format!(
                "[{local}={}]",
                element.attribute(Some(W), "val").unwrap_or("")
            )),
            "numPr" | "keepNext" => out.push_str(&format!("[{local}]")),
            "sdt" | "footnoteReference" | "endnoteReference" | "fldChar" | "fldSimple" | "drawing"
            | "bookmarkStart" | "hyperlink" | "commentRangeStart" | "commentReference" | "sectPr" | "tbl" => {
                out.push_str(&format!("[{local}]"));
            }
            _ => {}
        }
        for child in element.children().iter() {
            if let Node::Element(child) = child {
                walk(child, out);
            }
        }
    }
}
