//! The exporter's JSON (spike-S/py/export.py), as read.  Every length is an integer: px over
//! `px_den`, layout-unit widths over `w_den`.

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Clone)]
pub struct Export {
    pub compat: Option<i64>,
    pub even_odd: bool,
    pub dpi: i64,
    pub px_den: i64,
    pub w_den: i64,
    pub lu: i64,
    pub fonts: Vec<FontIn>,
    pub formats: Vec<(i64, i64, i64, i64, Option<(i64, i64)>)>,
    pub sections: Vec<SectionIn>,
    pub items: Vec<ItemIn>,
    #[serde(default)]
    pub pages: Vec<PageIn>,
    #[serde(default)]
    pub info: Vec<(i64, i64, bool, String, bool)>,
    #[serde(default)]
    pub stopped: Option<i64>,
    /// Lane F: what a check's facts read of the document as a whole (docx-agent's
    /// `docx_agent.rust.check`: each section's body height, the default language...).
    #[serde(default)]
    pub constants: Option<serde_json::Value>,
}

#[derive(Deserialize, Clone)]
pub struct FontIn {
    pub face: String,
    pub bold: bool,
    pub italic: bool,
    pub upm: i64,
    pub metrics: Option<Vec<Option<i64>>>,
    pub adv: HashMap<String, i64>,
    pub kern: Vec<(String, String, i64)>,
    pub kern_known: bool,
}

#[derive(Deserialize, Clone)]
pub struct SectionIn {
    pub top: i64,
    pub top_neg: bool,
    pub header: i64,
    pub bottom: i64,
    pub bottom_neg: bool,
    pub footer: i64,
    pub title_page: bool,
    pub start_type: Option<String>,
    pub new_page: bool,
    pub page_number_start: Option<i64>,
    pub stories: Option<HashMap<String, (i64, i64)>>,
}

#[derive(Deserialize, Clone)]
pub struct PageIn {
    pub start: (usize, usize, Vec<usize>),
}

#[derive(Deserialize, Clone)]
#[serde(tag = "t")]
pub enum ItemIn {
    #[serde(rename = "p")]
    Para(ParaIn),
    #[serde(rename = "t")]
    Table(TableIn),
}

/// `[pitch, text_above, text_below, rule, line, natural]`.
pub type HeightIn = (i64, i64, i64, u8, i64, i64);

#[derive(Deserialize, Clone)]
pub struct ParaIn {
    pub block: usize,
    #[serde(default)]
    pub digest: Option<String>,
    /// Lane E: the block's id as docx-agent names it.
    #[serde(default)]
    pub key: Option<String>,
    /// Lane E: bookmark starts in the paragraph, `(name, text offset)`.
    #[serde(default)]
    pub bookmarks: Vec<(String, usize)>,
    pub section: usize,
    pub before: i64,
    pub after: i64,
    pub contextual: bool,
    pub style: i64,
    pub border: i64,
    pub top_border: i64,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub pbb: bool,
    pub widow: Option<bool>,
    pub section_start: bool,
    pub section_mark: bool,
    #[serde(default)]
    pub page_breaks: Vec<usize>,
    #[serde(default)]
    pub heights: Vec<HeightIn>,
    pub measure: MeasureIn,
    #[serde(rename = "ref", default)]
    pub reference: RefIn,
    /// Lane F: what a check's facts read of the block besides its layout (docx-agent's
    /// `docx_agent.rust.records`), as made; `None` where no check reads it.
    #[serde(default)]
    pub rec: Option<serde_json::Value>,
}

#[derive(Deserialize, Clone, Default)]
pub struct RefIn {
    pub widths: Vec<i64>,
    pub kinds: Vec<u8>,
    pub lines: Vec<(usize, usize, u8)>,
}

#[derive(Deserialize, Clone)]
pub struct MeasureIn {
    pub runs: Vec<RunIn>,
    pub mark_fmt: usize,
    #[serde(default)]
    pub label: Option<LabelIn>,
    pub label_height_fmts: Vec<usize>,
    pub geometry: GeometryIn,
    pub rule: u8,
    pub line: i64,
    pub columns: bool,
    /// Lane F: a digest of what the paragraph draws besides its characters' faces and
    /// widths (alignment, borders, shading): with each run's `draw`, what a line's digest
    /// is made of (`since_last_check`).
    #[serde(default)]
    pub draw: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct RunIn {
    pub text: String,
    pub vanish: bool,
    #[serde(default)]
    pub fmts: Vec<usize>,
    #[serde(default)]
    pub tracking: i64,
    #[serde(default)]
    pub scale: i64,
    #[serde(default)]
    pub kern: i64,
    #[serde(default)]
    pub caps: bool,
    #[serde(default)]
    pub hyphen_fmt: usize,
    pub marks: Vec<(usize, String)>,
    /// Lane E: a computed field's run, `(keyword, argument)`: its text is the field's value.
    #[serde(default)]
    pub field: Option<(String, Option<String>)>,
    /// Lane E: superscript or subscript (left out of the smallest text size).
    #[serde(default)]
    pub va: bool,
    /// Lane F: a digest of how the run's characters are drawn (colour, underline...).
    #[serde(default)]
    pub draw: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct LabelIn {
    pub chars: Vec<(String, usize)>,
    pub align: String,
    pub suffix: String,
    pub space_fmt: usize,
}

#[derive(Deserialize, Clone)]
pub struct GeometryIn {
    pub first_start: i64,
    pub start: i64,
    pub right: i64,
    pub tabs: Vec<(i64, String)>,
    pub default_tab: i64,
    pub hanging_stop: Option<i64>,
    pub squeeze: bool,
    pub tab_origin: i64,
}

#[derive(Deserialize, Clone)]
pub struct TableIn {
    pub block: usize,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    pub section: usize,
    pub section_start: bool,
    pub bottom_border: i64,
    pub previous_style: i64,
    pub default_style: i64,
    pub first_style: i64,
    pub rows: Vec<RowIn>,
    /// Lane F: as a paragraph's `rec`.
    #[serde(default)]
    pub rec: Option<serde_json::Value>,
    /// Lane F: a digest of the table's borders, shading and widths (`since_last_check`).
    #[serde(default)]
    pub draw: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct RowIn {
    pub top_border: i64,
    #[serde(default)]
    pub height: i64,
    pub header: bool,
    pub cant_split: bool,
    pub keeps_next: bool,
    pub tr_height: Option<(i64, Option<String>)>,
    pub cells: Vec<CellIn>,
}

#[derive(Deserialize, Clone)]
pub struct CellIn {
    pub mt: i64,
    pub mb: i64,
    pub merge: Option<String>,
    pub paragraphs: Vec<CellParaIn>,
    #[serde(default)]
    pub after: i64,
    #[serde(default)]
    pub content: i64,
    /// `[above, pitch, below, line number, paragraph index]`.
    #[serde(default)]
    pub stack: Vec<(i64, i64, i64, usize, usize)>,
}

#[derive(Deserialize, Clone)]
pub struct CellParaIn {
    pub block: usize,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub bookmarks: Vec<(String, usize)>,
    pub before: i64,
    pub after: i64,
    pub contextual: bool,
    pub style: i64,
    pub effective_style: i64,
    pub top_border: i64,
    pub border: i64,
    pub hidden: bool,
    pub widow: Option<bool>,
    pub keep_next: bool,
    #[serde(default)]
    pub heights: Vec<HeightIn>,
    pub measure: MeasureIn,
    #[serde(rename = "ref", default)]
    pub reference: RefIn,
}

pub fn load(path: &str) -> Export {
    let data = std::fs::read(path).expect("read export");
    serde_json::from_slice(&data).expect("parse export")
}
