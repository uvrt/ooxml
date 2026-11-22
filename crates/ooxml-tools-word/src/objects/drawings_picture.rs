//! Owns: `word_drawings insert_picture` (docx-agent's `word_drawings` and `_insert_picture`):
//! the picture's bytes from the session's blob, checked as docx-agent checks the call
//! (an image blob, alt text, a float only with wrap, a width alone), a paragraph of its own
//! for `after:`/`before:` a block (aligned when the picture stays in the line), the picture
//! inserted in the line and floated when `wrap` is given, and the reply (its id and size,
//! the paragraph made). An SVG (its PNG fallback is drawn by a later step), a tint and a
//! name given with `ref` are left to the host, as is a call when the host hands over no
//! blobs.
//!
//! Must not: change the document but through one transaction of its commands, or change it
//! at all before every check the call can fail on has passed.

use super::drawings_args::{WordDrawings, Wrap};
use crate::blobs::blob;
use crate::review::{needed, Editing};
use crate::structure::common::{outcome, policy, refused, Outcome};
use ooxml_base::pyfmt::round2;
use ooxml_store::{Command, Txn};
use ooxml_tools::{ArgEnum, Refusal, ToolError, ToolResult, TryCall, WayOut};
use ooxml_word_edit::{
    picture_size, written_size, DrawingValues, EditError, FloatDrawing, FormatEdit, FormatPlan, InsertParagraph,
    InsertPicture, Property, RevisionPolicy, Setting, Value as SettingValue, WordDoc,
};
use serde_json::{json, Value};

fn not_supported(what: &str, retry: &TryCall) -> ToolResult<Value> {
    refused(EditError::Unsupported(what.to_owned()), retry.clone())
}

fn invalid(message: &str, field: &str, way_out: WayOut) -> ToolError {
    ToolError::refusal(Refusal::InvalidArguments, message.to_owned(), way_out).at(field)
}

/// Whether `data` is meant as an SVG (ooxml-edit's `looks_like_svg`): markup with an
/// `<svg` tag in its first 4 KiB.
fn looks_like_svg(data: &[u8]) -> bool {
    let head = data.get(..4096).unwrap_or(data);
    let start = head
        .iter()
        .position(|b| !matches!(b, 0xEF | 0xBB | 0xBF | b' ' | b'\t' | b'\r' | b'\n'))
        .unwrap_or(head.len());
    let head = head.get(start..).unwrap_or_default();
    head.starts_with(b"<")
        && head.windows(5).any(|w| {
            w.starts_with(b"<svg") && matches!(w.get(4), Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'>' | b'/'))
        })
}

/// A paragraph of the picture's own beside a block (`after:`/`before:` with no `find`).
fn own_paragraph(args: &WordDrawings) -> Option<(bool, String)> {
    let at = args.at.as_deref().filter(|_| args.find.is_none())?;
    let (side, block) = at.split_once(':')?;
    matches!(side, "after" | "before").then(|| (side == "after", block.to_owned()))
}

/// What a float takes (docx-agent's `_floating` for a picture): `None` without `wrap`,
/// refused when a place is given without it.
fn floating(args: &WordDrawings, align_taken: bool, list: &TryCall) -> Result<Option<DrawingValues>, ToolError> {
    let align = args.align.filter(|_| !align_taken);
    let Some(wrap) = args.wrap else {
        if args.x.is_some() || args.y.is_some() || align.is_some() || args.against.is_some() {
            let wraps = [Wrap::Square, Wrap::Tight, Wrap::TopAndBottom, Wrap::Front, Wrap::Behind];
            let options = WayOut::names(wraps.iter().map(|w| w.as_str())).unwrap_or_else(|| WayOut::call(list.clone()));
            return Err(invalid(
                "a picture floats with wrap: give wrap to place it",
                "wrap",
                options,
            ));
        }
        return Ok(None);
    };
    Ok(Some(DrawingValues {
        wrap: Some(wrap.as_str().to_owned()),
        horizontal_from: Some(args.against.map_or("column", |a| a.as_str()).to_owned()),
        x_align: align.map(|a| a.as_str().to_owned()),
        x: if align.is_some() { None } else { args.x },
        y: args.y,
        ..DrawingValues::default()
    }))
}

/// The bytes of the image blob `image` names, refused when it is not an image.
fn image_bytes(
    args: &WordDrawings,
    editing: &Editing<'_>,
    list: &TryCall,
) -> Result<Option<std::sync::Arc<[u8]>>, ToolError> {
    let Some(blobs) = editing.blobs else { return Ok(None) };
    let Some(handle) = args.image.as_deref() else {
        return Err(needed("image", "", list.clone()));
    };
    let found = blob(blobs, handle, list)?;
    if !found.mime.starts_with("image/") {
        let message = format!("{handle} is {}, not an image", found.mime);
        return Err(invalid(&message, "image", WayOut::call(list.clone())));
    }
    Ok(Some(found.data))
}

/// A call checked as docx-agent checks it: what it makes.
struct Plan {
    /// The paragraph of the picture's own.
    paragraph: Option<InsertParagraph>,
    /// Whether that paragraph takes `align` (the picture stays in the line).
    align_taken: bool,
    /// The position, without a paragraph of its own.
    at: Option<String>,
    data: std::sync::Arc<[u8]>,
    alt_text: String,
    float: Option<DrawingValues>,
    size: (f64, f64),
}

/// The call checked in docx-agent's order (`Ok(Err(reply))`: a reply ending the call before
/// anything is changed).
fn checked(
    args: &WordDrawings,
    editing: &Editing<'_>,
    list: &TryCall,
) -> Result<Result<Plan, ToolResult<Value>>, ToolError> {
    let not_supported_blobs = || not_supported("word_drawings insert_picture without the session's blobs", list);
    if args.reference.is_some() {
        return Ok(Err(not_supported("word_drawings ref", list)));
    }
    let own = own_paragraph(args);
    let paragraph = own.as_ref().map(|(after, block)| InsertParagraph {
        anchor: block.clone(),
        after: *after,
        text: String::new(),
        style: None,
        policy: policy(editing),
    });
    // docx-agent makes the paragraph first: its refusal comes before the others
    if let Some(command) = &paragraph {
        if let Err(error) = command.validate(editing.editor.store().snapshot().doc()) {
            return Ok(Err(refused(error, list.clone())));
        }
    }
    let align_taken = own.is_some() && args.wrap.is_none() && args.align.is_some();
    let Some(data) = image_bytes(args, editing, list)? else {
        return Ok(Err(not_supported_blobs()));
    };
    let alt_text = args
        .alt_text
        .clone()
        .ok_or_else(|| needed("alt_text", " for a picture", list.clone()))?;
    let float = floating(args, align_taken, list)?;
    if args.height.is_some() {
        let message = "a picture keeps its aspect ratio: give width";
        return Err(invalid(message, "height", WayOut::call(list.clone())));
    }
    let at = match &paragraph {
        Some(_) => None,
        None => Some(super::drawings::position(args, editing, list)?),
    };
    let svg = looks_like_svg(&data);
    if args.color.is_some() && !svg {
        let message = "only an SVG picture can be tinted: this one is a bitmap; leave color out, or give an SVG";
        return Err(invalid(message, "color", WayOut::call(list.clone())));
    }
    if svg {
        return Ok(Err(not_supported("word_drawings insert_picture of an SVG", list)));
    }
    let size = match picture_size(&data, args.width) {
        Ok(size) => size,
        Err(error) => return Ok(Err(refused(error, list.clone()))),
    };
    Ok(Ok(Plan {
        paragraph,
        align_taken,
        at,
        data,
        alt_text,
        float,
        size,
    }))
}

/// `insert_picture`: every check first, then the paragraph of its own (aligned), the
/// picture, its float -- one transaction, so one step, and nothing changed where a later
/// part refuses.
pub(super) fn insert_picture(
    args: &WordDrawings,
    editing: &Editing<'_>,
    list: &TryCall,
) -> Result<ToolResult<Value>, ToolError> {
    let plan = match checked(args, editing, list)? {
        Ok(plan) => plan,
        Err(reply) => return Ok(reply),
    };
    let command = Picture {
        args,
        plan,
        policy: policy(editing),
    };
    Ok(match editing.editor.run(editing.caller, &command) {
        Ok(done) => {
            let (edits, data) = done.out;
            let summary = format!(
                "Inserted picture {}",
                data.get("id").and_then(Value::as_str).unwrap_or("")
            );
            outcome(&edits, summary, Some(data))
        }
        Err(error) => refused(error, list.clone()),
    })
}

/// A checked call's commands, applied in order in one transaction: their outcomes and the
/// reply's data.
struct Picture<'a> {
    args: &'a WordDrawings,
    plan: Plan,
    policy: RevisionPolicy,
}

impl Command<WordDoc> for Picture<'_> {
    type Out = (Vec<Outcome>, Value);
    type Error = EditError;

    fn validate(&self, _doc: &WordDoc) -> Result<(), EditError> {
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Self::Out, EditError> {
        let plan = &self.plan;
        let mut edits: Vec<Outcome> = Vec::new();
        let mut own = None;
        if let Some(command) = &plan.paragraph {
            let done = step(tx, command)?;
            let id = done.id.clone();
            edits.push(Outcome {
                id: Some(id.clone()),
                changed: true,
                created: vec![id.clone()],
                removed: Vec::new(),
                renamed: done.renamed,
                warnings: Vec::new(),
            });
            if plan.align_taken {
                self.align(tx, &id, &mut edits)?;
            }
            own = Some(id);
        }
        let command = InsertPicture {
            at: own
                .as_ref()
                .map_or_else(|| plan.at.clone().unwrap_or_default(), |p| format!("{p}@0")),
            image: std::sync::Arc::clone(&plan.data),
            width: self.args.width,
            alt_text: Some(plan.alt_text.clone()),
            policy: self.policy.clone(),
        };
        let inserted = step(tx, &command)?;
        let id = inserted.id.clone();
        edits.push(inserted.into());
        if let Some(values) = &plan.float {
            let command = FloatDrawing {
                drawing: id.clone(),
                values: values.clone(),
                policy: self.policy.clone(),
            };
            edits.push(step(tx, &command)?.into());
        }
        let (width, height) = written_size(plan.size);
        let mut data = serde_json::Map::new();
        data.insert("id".into(), json!(id));
        data.insert("size".into(), json!([round2(width), round2(height)]));
        if let Some(paragraph) = own {
            data.insert("paragraph".into(), json!(paragraph));
        }
        Ok((edits, Value::Object(data)))
    }
}

impl Picture<'_> {
    /// The paragraph of the picture's own aligned (docx-agent's `format_paragraph`).
    fn align(&self, tx: &mut Txn<'_, WordDoc>, id: &str, edits: &mut Vec<Outcome>) -> Result<(), EditError> {
        let value = self.args.align.map_or("left", |a| a.as_str());
        let command = FormatEdit {
            targets: vec![id.to_owned()],
            plan: FormatPlan {
                paragraph: vec![Setting {
                    property: Property::Alignment,
                    value: Some(SettingValue::Text(value.to_owned())),
                }],
                ..FormatPlan::default()
            },
            cleared: false,
            policy: self.policy.clone(),
        };
        for done in step(tx, &command)?.steps {
            edits.push(Outcome {
                id: Some(done.id),
                changed: done.changed,
                created: Vec::new(),
                removed: done.removed,
                renamed: done.renamed,
                warnings: done.warnings,
            });
        }
        Ok(())
    }
}

/// `command` checked and made in `tx`, one part of the call's transaction.
fn step<C: Command<WordDoc, Error = EditError>>(tx: &mut Txn<'_, WordDoc>, command: &C) -> Result<C::Out, EditError> {
    command.validate(tx.doc())?;
    command.prepare(tx)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use ooxml_base::Limits;
    use ooxml_derive::CacheBudget;
    use ooxml_opc::Package;
    use ooxml_store::{Caller, Clock, StoreConfig};
    use ooxml_tools::ToolArgs;
    use ooxml_word_edit::Editor;
    use std::sync::Arc;
    use std::time::Duration;

    const FIXTURE: &[u8] =
        include_bytes!("../../../../crates/ooxml-oracle/fixtures/generated/charts/chart-places.docx");

    struct Still;

    impl Clock for Still {
        fn monotonic(&self) -> Duration {
            Duration::ZERO
        }
    }

    struct Unlimited;

    impl CacheBudget for Unlimited {
        fn reserve(&self, _bytes: usize) -> bool {
            true
        }
        fn release(&self, _bytes: usize) {}
        fn used(&self) -> usize {
            0
        }
    }

    /// A part refusing after an earlier one was made (the picture after its own paragraph):
    /// the call's one transaction changes nothing; where all apply, one step undo takes
    /// back whole.
    #[test]
    fn a_later_part_refusing_leaves_nothing_and_a_call_is_one_step() {
        let package = Package::open(Arc::from(FIXTURE), &Limits::default()).unwrap();
        let editor = Editor::open(package, StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
        let before = editor.store().snapshot();
        let (args, _) = WordDrawings::accept(
            json!({"doc": "d1", "action": "insert_picture", "image": "b1", "at": "after:p@body/0",
                   "alt_text": "x", "wrap": "square"}),
            Some("docx"),
        )
        .unwrap();
        let plan = |data: &[u8]| Plan {
            paragraph: Some(InsertParagraph {
                anchor: "p@body/0".to_owned(),
                after: true,
                text: String::new(),
                style: None,
                policy: RevisionPolicy::Untracked,
            }),
            align_taken: false,
            at: None,
            data: Arc::from(data),
            alt_text: "x".to_owned(),
            float: Some(DrawingValues {
                wrap: Some("square".to_owned()),
                ..DrawingValues::default()
            }),
            size: (72.0, 36.0),
        };
        let caller = Caller::anonymous();
        let refused = Picture {
            args: &args,
            plan: plan(b"words"),
            policy: RevisionPolicy::Untracked,
        };
        assert!(editor.run(&caller, &refused).is_err());
        assert_eq!(editor.store().snapshot().seq(), before.seq(), "nothing committed");
        assert!(editor.store().steps().is_empty());
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&96u32.to_be_bytes());
        png.extend_from_slice(&48u32.to_be_bytes());
        png.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        let made = Picture {
            args: &args,
            plan: plan(&png),
            policy: RevisionPolicy::Untracked,
        };
        let done = editor.run(&caller, &made).unwrap();
        assert_eq!(done.out.0.len(), 3, "the paragraph, the picture, its float");
        assert_eq!(editor.store().steps().len(), 1, "one step");
        assert_eq!(done.version.number(), 1, "one state number");
        editor.store().undo().unwrap();
        let back = editor.store().snapshot();
        let main = |v: &ooxml_store::Snapshot<WordDoc>| v.doc().package().read("word/document.xml").unwrap();
        assert_eq!(main(&back), main(&before));
    }
}
