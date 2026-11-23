//! Owns: `edit_chart`'s `add` (docx-agent's `_add_chart`): the arguments checked
//! (`chart_add_plan`), then one command -- the chart inserted ([`InsertChart`], its part
//! and workbook made here, [`NewParts`]) and given its scale, minor gridlines and look
//! (`finish_add`: the style and data actions' own edits, each planned on the chart as the
//! step left it), so the chart and its look are one undo step and a refusal of any part
//! leaves nothing -- and the reply: the chart as every `edit_chart` result gives it, the
//! paragraph made for it, the ref named.
//!
//! Not here yet: a document whose defaults name no language (the session's locale
//! decides the new text's: refused as not supported yet).
//!
//! Must not: change the document but through the command.

use super::chart_add_plan::{add_plan, workbook, Finish, Look};
use super::chart_data::{ChartDataEdit, DataAction, Warnings};
use super::chart_keeper::{not_supported as chart_not_supported, refused as data_refused, sheets_error};
use super::chart_read::chart_json;
use super::chart_style::{add_scale, colour_given, text_look, ChartStyleEdit, StyleAction};
use super::{EditChart, ScaleArg};
use crate::structure::common::{call, outcome, policy, refused, Outcome};
use crate::Editing;
use ooxml_chart::data::chart_data;
use ooxml_chart::edit::create::NewChart;
use ooxml_chart::edit::SeriesFormat;
use ooxml_store::{Command, Txn};
use ooxml_tools::{ArgEnum, ToolError, ToolResult};
use ooxml_word_edit::{
    ChartInserted, ChartParts, EditError, EditGraphic, GraphicEdit, InsertChart, RevisionPolicy, WordDoc,
};
use ooxml_xml::Tree;
use serde_json::{json, Value};
use std::sync::Arc;

/// Word's value labels' size (`WORD_LOOK.data_labels`, hundredths of a point).
const WORD_LABEL_SIZE: u32 = 900;

/// A new chart's part and workbook, from its data.
#[derive(Debug, Clone)]
pub(super) struct NewParts(NewChart);

impl ChartParts for NewParts {
    fn parts(&self, part: &str, workbook_rel: &str) -> Result<(Tree, Arc<[u8]>), EditError> {
        let space = self.0.chart_space(part, workbook_rel, &ooxml_word_model::ids::sha256);
        let book = workbook(&self.0).map_err(|_| chart_not_supported("a new chart's workbook past its last column"))?;
        let bytes = book.write().map_err(sheets_error)?;
        Ok((Tree::from_root(Arc::new(space)), Arc::from(bytes)))
    }
}

/// The chart inserted and finished, one step.
struct AddChart {
    insert: InsertChart<NewParts>,
    finish: Finish,
    lang: String,
    policy: RevisionPolicy,
}

/// One of `finish_add`'s steps, in its order: the scale, the gridlines, then the look.
#[derive(Clone, Copy)]
enum Step<'a> {
    Scale(&'a ScaleArg),
    Gridlines(&'static str, bool, bool),
    Look(&'a Look),
}

/// One of `finish_add`'s edits, planned on the chart as it stands.
enum Edit {
    Style(StyleAction),
    Data(DataAction),
}

impl AddChart {
    /// `finish_add`'s steps, each where given.
    fn steps(&self) -> Vec<Step<'_>> {
        let scale = self.finish.scale.as_ref().map(Step::Scale);
        let gridlines = self
            .finish
            .gridlines
            .map(|(a, major, minor)| Step::Gridlines(a, major, minor));
        scale
            .into_iter()
            .chain(gridlines)
            .chain(self.finish.look.iter().map(Step::Look))
            .collect()
    }

    /// `step`'s edit, planned on the chart in `part` as `doc` holds it.
    fn planned(&self, step: Step<'_>, doc: &WordDoc, part: &str, address: &str) -> Result<Edit, EditError> {
        let refuse = |e: ooxml_chart::edit::DataError| data_refused(&e.0);
        let look = match step {
            Step::Gridlines(axis, major, minor) => {
                return Ok(Edit::Style(StyleAction::Gridlines { axis, major, minor }))
            }
            Step::Scale(scale) => {
                let docx = doc.docx()?;
                let tree = docx
                    .package
                    .tree(part)?
                    .ok_or_else(|| EditError::NoPart(part.to_owned()))?;
                let data =
                    chart_data(tree.root()).map_err(|_| chart_not_supported("a new chart the model cannot read"))?;
                return add_scale(address, tree.root(), &data, scale, self.finish.axis)
                    .map(Edit::Style)
                    .map_err(refuse);
            }
            Step::Look(look) => look,
        };
        Ok(match look {
            Look::DataLabels => Edit::Data(DataAction::DataLabels {
                show: true,
                series: None,
                number_format: None,
                size: Some(WORD_LABEL_SIZE),
                lang: Some(self.lang.clone()),
            }),
            Look::Series { series, key, colour } => {
                let given = colour_given(Some(colour));
                let mut look = SeriesFormat::default();
                match *key {
                    "line" => look.line = given,
                    "marker_fill" => look.marker_fill = given,
                    _ => look.fill = given,
                }
                Edit::Style(StyleAction::Format {
                    series: *series,
                    point: None,
                    look,
                })
            }
            Look::Slice { point, colour } => Edit::Style(StyleAction::Format {
                series: 0,
                point: Some(*point),
                look: SeriesFormat {
                    fill: colour_given(Some(colour)),
                    ..SeriesFormat::default()
                },
            }),
            Look::Text { style, part } => Edit::Style(StyleAction::Text {
                part: part.map(ArgEnum::as_str),
                look: text_look(Some(style)).map_err(refuse)?,
                lang: Some(self.lang.clone()),
            }),
        })
    }

    /// One edit of the chart `drawing` run in `tx`: its warnings.
    fn run<E: GraphicEdit<Out = Warnings>>(
        &self,
        tx: &mut Txn<'_, WordDoc>,
        drawing: &str,
        edit: E,
    ) -> Result<Vec<String>, EditError> {
        let command = EditGraphic {
            drawing: drawing.to_owned(),
            edit,
            policy: self.policy.clone(),
        };
        command.validate(tx.doc())?;
        let done = command.prepare(tx)?;
        Ok(done.warnings.into_iter().chain(done.out).collect())
    }
}

impl Command<WordDoc> for AddChart {
    type Out = (ChartInserted, Vec<String>);
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<(), EditError> {
        self.insert.validate(doc)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<Self::Out, EditError> {
        let inserted = self.insert.insert(tx)?;
        let drawing = inserted.drawing.id.clone();
        let mut warnings = Vec::new();
        for step in self.steps() {
            let address = drawing.clone();
            warnings.extend(match self.planned(step, tx.doc(), &inserted.part, &drawing)? {
                Edit::Style(action) => self.run(tx, &drawing, ChartStyleEdit { address, action })?,
                Edit::Data(action) => self.run(tx, &drawing, ChartDataEdit { address, action })?,
            });
        }
        Ok((inserted, warnings))
    }
}

/// `add`: a new inline chart in a paragraph of its own after the target (or at a position
/// in one), with the look its arguments give.
pub(super) fn add(
    args: &EditChart,
    editing: &Editing<'_>,
    lang: Option<String>,
) -> Result<ToolResult<Value>, ToolError> {
    let read = call(
        "edit_chart",
        &[
            ("doc", json!(args.doc)),
            ("target", json!(args.target)),
            ("action", json!("read")),
        ],
    );
    let Some(lang) = lang else {
        return Err(ToolError::not_supported(
            "edit_chart add: a document whose defaults name no language (the session's locale decides)",
            ooxml_tools::WayOut::call(read),
        ));
    };
    let plan = add_plan(args, lang.clone(), &read)?;
    let command = AddChart {
        insert: InsertChart {
            at: args.target.clone(),
            chart: NewParts(plan.chart),
            size: plan.size,
            policy: policy(editing),
        },
        finish: plan.finish,
        lang,
        policy: policy(editing),
    };
    let done = match editing.editor.run(editing.caller, &command) {
        Ok(done) => done,
        Err(error) => return Ok(refused(error, read)),
    };
    let (inserted, warnings) = done.out;
    let id = inserted.drawing.id.clone();
    let after = done
        .version
        .doc()
        .docx()
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let tree = after
        .package
        .tree(&inserted.part)
        .map_err(|e| ToolError::internal(e.to_string(), "word:model"))?;
    let mut data = match tree.map(|t| (chart_data(t.root()), Arc::clone(t.root()))) {
        Some((Ok(read), root)) => chart_json(&id, &root, &read),
        _ => serde_json::Map::new(),
    };
    let paragraph = inserted.drawing.created.iter().find(|c| c.starts_with("p:")).cloned();
    if let Some(paragraph) = &paragraph {
        data.insert("paragraph".into(), json!(paragraph));
    }
    let kind = args.chart_type.map_or("", ArgEnum::as_str);
    let summary = format!(
        "Inserted a {kind} chart {id} in {}",
        paragraph.as_deref().unwrap_or(&args.target)
    );
    let edit: Outcome = inserted.drawing.into();
    let mut result = outcome(&[edit], summary, Some(Value::Object(data)));
    // one warning per edit of the chart made untracked, as docx-agent warns each
    result.warnings.extend(warnings);
    if let ooxml_tools::Outcome::Done(done) = &mut result.outcome {
        if let Some(name) = args.ref_name.as_ref().filter(|n| !n.is_empty()) {
            done.refs.push((name.clone(), id));
        }
    }
    Ok(result)
}
