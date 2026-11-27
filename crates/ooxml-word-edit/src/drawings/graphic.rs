//! Owns: an edit of the parts behind a drawing's graphic ([`EditGraphic`]): the chart a `d:`
//! id names (the first drawing or group member with that id, docx-agent's
//! `Document.chart`), its part found from the drawing, and what a [`GraphicEdit`]
//! (`ooxml_chart::edit`'s) makes of the version the edit is prepared on -- parts put whole,
//! as trees or as bytes (a chart part, its embedded workbook) -- written through the sink,
//! with the parts it read recorded, and the chart's entry in the document's records kept as
//! the edit says ([`RecordChange`]: replaced or dropped, in the edit's step). Word tracks no chart data: tracked, the edit is applied untracked, and the
//! result says so as docx-agent's warning does (its category, `UntrackedChartEdit`, first).
//!
//! Must not: know what a chart holds or what its record says (the [`GraphicEdit`] does), or
//! write but through the sink.

use crate::doc::Key;
use crate::policy::RevisionPolicy;
use crate::provenance::store_record;
use crate::sink::EditSink;
use crate::{EditError, Result, WordDoc};
use ooxml_chart::edit::{GraphicEdit, PartWrite, RecordChange};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::drawings::graphics;
use ooxml_word_model::docx::graphics::chart_part;
use ooxml_word_model::docx::Docx;

/// The kind of a chart's entry in the document's records.
const CHART: &str = "chart";

/// A chart's parts edited ([`GraphicEdit`]) as the drawing `drawing` names it.
#[derive(Debug, Clone)]
pub struct EditGraphic<E> {
    /// The drawing's id (`d:3`, or a group member's `d:3/5`).
    pub drawing: String,
    /// The edit.
    pub edit: E,
    /// The call's revision policy (tracked: applied untracked, said in the warnings).
    pub policy: RevisionPolicy,
}

/// What [`EditGraphic`] did.
#[derive(Debug, Clone)]
pub struct GraphicEdited<T> {
    /// The edit's outcome.
    pub out: T,
    /// Whether a part was written.
    pub changed: bool,
    /// What the result says (the edit applied untracked).
    pub warnings: Vec<String>,
}

impl<E> EditGraphic<E> {
    /// The chart part of the drawing.
    fn part(&self, docx: &Docx) -> Result<String> {
        let drawings = graphics(docx);
        let Some(drawing) = drawings.iter().find(|d| d.id == self.drawing) else {
            return Err(EditError::Refused(format!("no drawing {}", self.drawing)));
        };
        if drawing.kind != "chart" {
            return Err(EditError::Refused(format!(
                "{} is a {}, not a chart",
                self.drawing, drawing.kind
            )));
        }
        chart_part(docx, &drawing.part, &drawing.element)
            .ok_or_else(|| EditError::Unsupported("edit_chart: a chart whose part is missing".to_owned()))
    }
}

impl<E: GraphicEdit> Command<WordDoc> for EditGraphic<E> {
    type Out = GraphicEdited<E::Out>;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.part(&*doc.docx()?).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<GraphicEdited<E::Out>> {
        let docx = tx.doc().docx()?;
        let part = self.part(&docx)?;
        let made = self.edit.edit(&docx.package, &part)?;
        let mut sink = EditSink::new(tx, RevisionPolicy::Untracked);
        for read in std::iter::once(&part).chain(&made.read) {
            sink.read(Key::part(read.as_str()));
        }
        let changed = !made.writes.is_empty();
        for write in made.writes {
            match write {
                PartWrite::Tree(name, tree) => sink.put(&name, tree)?,
                PartWrite::Bytes(name, bytes) => sink.put_bytes(&name, bytes)?,
            }
        }
        match &made.record {
            RecordChange::Kept => {}
            RecordChange::Stored(text) => store_record(&mut sink, CHART, &self.drawing, Some(text))?,
            RecordChange::Dropped => store_record(&mut sink, CHART, &self.drawing, None)?,
        }
        let tracked = matches!(self.policy, RevisionPolicy::Tracked(_));
        let warnings = (changed && tracked)
            .then(|| {
                format!(
                    "UntrackedChartEdit: {}: Word does not track a chart's data or a diagram's text: the edit is applied untracked",
                    self.drawing
                )
            })
            .into_iter()
            .collect();
        Ok(GraphicEdited {
            out: made.out,
            changed,
            warnings,
        })
    }
}
