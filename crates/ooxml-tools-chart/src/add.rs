//! Owns: `edit_chart`'s `add` (docx-agent's `_add_chart`): the arguments checked
//! (`add_plan`), then one edit through the host's writer ([`GraphicWriter::insert`]) --
//! the chart inserted, its part and workbook made here ([`NewParts`]), and given its scale,
//! minor gridlines and look (`finish_add`: the style and data actions' own edits, each
//! planned on the chart as the step left it, [`Finishing`]), so the chart and its look are
//! one undo step and a refusal of any part leaves nothing -- and the reply: the chart as
//! every `edit_chart` result gives it, the block made for it (a document's paragraph), the
//! ref named.
//!
//! The new text is in the writer's language ([`GraphicWriter::language`]).
//!
//! Must not: change the document but through the writer.

use crate::add_plan::{add_plan, workbook, Finish, Look};
use crate::data::{ChartDataEdit, DataAction};
use crate::edit_chart::read_call;
use crate::host::{FinishGraphic, GraphicWriter, NewGraphic};
use crate::keeper::{not_supported as chart_not_supported, refused as data_refused, sheets_error};
use crate::read::chart_json;
use crate::style::{add_scale, colour_given, text_look, ChartStyleEdit, StyleAction};
use crate::{EditChart, ScaleArg, Warnings};
use ooxml_chart::data::chart_data;
use ooxml_chart::edit::create::NewChart;
use ooxml_chart::edit::{GraphicEdit, GraphicError, GraphicParts, SeriesFormat};
use ooxml_opc::Package;
use ooxml_tools::{ArgEnum, ToolError, ToolResult};
use ooxml_xml::Tree;
use serde_json::{json, Value};
use std::sync::Arc;

/// Word's value labels' size (`WORD_LOOK.data_labels`, hundredths of a point).
const WORD_LABEL_SIZE: u32 = 900;

/// A new chart's part and workbook, from its data.
#[derive(Debug, Clone)]
pub(crate) struct NewParts(NewChart);

impl NewGraphic for NewParts {
    fn parts(&self, part: &str, workbook_rel: &str) -> Result<(Tree, Arc<[u8]>), GraphicError> {
        let space = self.0.chart_space(part, workbook_rel, &ooxml_base::sha256::sha256);
        let book = workbook(&self.0).map_err(|_| chart_not_supported("a new chart's workbook past its last column"))?;
        let bytes = book.write().map_err(sheets_error)?;
        Ok((Tree::from_root(Arc::new(space)), Arc::from(bytes)))
    }
}

/// What finishes a new chart (`finish_add`): its steps, each planned on the chart as the
/// step before left it, in the language its text is written in.
struct Finishing {
    finish: Finish,
    lang: String,
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

impl Finishing {
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

    /// `step`'s edit, planned on the chart in `part` as `package` holds it.
    fn planned(&self, step: Step<'_>, package: &Package, part: &str, address: &str) -> Result<Edit, GraphicError> {
        let refuse = |e: ooxml_chart::edit::DataError| data_refused(&e.0);
        let look = match step {
            Step::Gridlines(axis, major, minor) => {
                return Ok(Edit::Style(StyleAction::Gridlines { axis, major, minor }))
            }
            Step::Scale(scale) => {
                let tree = package
                    .tree(part)?
                    .ok_or_else(|| GraphicError::NoPart(part.to_owned()))?;
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
                lang: self.lang.clone(),
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
                lang: self.lang.clone(),
            }),
        })
    }
}

impl FinishGraphic for Finishing {
    fn next(
        &self,
        done: usize,
        package: &Package,
        chart: &str,
        frame: &str,
    ) -> Option<Result<ChartEdit, GraphicError>> {
        let step = self.steps().get(done).copied()?;
        let address = frame.to_owned();
        Some(self.planned(step, package, chart, frame).map(|edit| match edit {
            Edit::Style(action) => ChartEdit::Style(ChartStyleEdit { address, action }),
            Edit::Data(action) => ChartEdit::Data(ChartDataEdit { address, action }),
        }))
    }
}

/// A style or data edit of a chart, as one of `finish_add`'s steps makes it.
#[derive(Debug, Clone)]
pub enum ChartEdit {
    /// A style action's.
    Style(ChartStyleEdit),
    /// A data action's.
    Data(ChartDataEdit),
}

impl GraphicEdit for ChartEdit {
    type Out = Warnings;

    fn edit(&self, package: &Package, part: &str) -> Result<GraphicParts<Warnings>, GraphicError> {
        match self {
            ChartEdit::Style(edit) => edit.edit(package, part),
            ChartEdit::Data(edit) => edit.edit(package, part),
        }
    }
}

/// `add`: a new chart placed after the target (a document's: inline, in a paragraph of its
/// own, or at a position in one), with the look its arguments give.
///
/// # Errors
/// The plan's refusal, the writer's, or the edit's.
pub(crate) fn add(args: &EditChart, writer: &impl GraphicWriter) -> Result<ToolResult<Value>, ToolError> {
    let lang = writer.language()?;
    let read = read_call(args);
    let plan = add_plan(args, lang.clone(), &read)?;
    let finishing = Finishing {
        finish: plan.finish,
        lang,
    };
    let inserted = writer.insert(&args.target, &read, &NewParts(plan.chart), plan.size, &finishing)?;
    let id = inserted.frame;
    let mut data = match inserted.root.map(|root| (chart_data(&root), root)) {
        Some((Ok(read), root)) => chart_json(&id, &root, &read),
        _ => serde_json::Map::new(),
    };
    if let Some(paragraph) = &inserted.block {
        data.insert("paragraph".into(), json!(paragraph));
    }
    let kind = args.chart_type.map_or("", ArgEnum::as_str);
    let summary = format!(
        "Inserted a {kind} chart {id} in {}",
        inserted.block.as_deref().unwrap_or(&args.target)
    );
    let mut result = inserted.result;
    if let ooxml_tools::Outcome::Done(done) = &mut result.outcome {
        done.summary = summary;
        done.data = Some(Value::Object(data));
        if let Some(name) = args.ref_name.as_ref().filter(|n| !n.is_empty()) {
            done.refs.push((name.clone(), id));
        }
    }
    Ok(result)
}
