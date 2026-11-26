//! Owns: the source model of a story -- paragraphs, runs, fields, bookmarks, tables, drawings'
//! anchors and extents, sections -- as docx2svg's `docx2svg.model` holds it after its parser:
//! what the file says, in the final view of tracked changes (Accept All), not yet resolved.
//! Blocks are shared (`Arc`) so that a version reopened with unchanged blocks reuses them.
//!
//! Must not: resolve styles or numbering (`crate::resolve`), or hold anything a layout
//! computes (lines, pages).

use crate::ids::BlockKey;
use crate::value::{Props, Text};
use std::sync::Arc;

/// A story's (a cell's, a text box's, a note's) blocks.
pub type Story = Arc<[Block]>;
/// A table row's cells.
pub type Cells = Arc<[Story]>;
/// A cell's regions of its table style, lowest first.
pub type Conditions = Arc<[Text]>;
/// A row's cells' regions.
pub type RowConditions = Arc<[Conditions]>;

/// A `w:rPr` as written: its character style and declared properties.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct RunProps {
    /// `w:rStyle`.
    pub style_id: Option<Text>,
    /// Every property it declares.
    pub declared: Props,
}

/// A `w:pPr` as written.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ParaProps {
    /// `w:pStyle`.
    pub style_id: Option<Text>,
    /// Every property it declares (not its `w:rPr` or `w:sectPr`).
    pub declared: Props,
    /// The paragraph mark's `w:rPr`.
    pub mark: Option<Arc<RunProps>>,
}

/// A field whose result layout computes (`PAGE`, `NUMPAGES`, `SECTIONPAGES`, `PAGEREF`,
/// `REF`, `SEQ`, `STYLEREF`): its instruction, keyword, and the result Word cached, run by
/// run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The instruction, stripped.
    pub instruction: Text,
    /// Its keyword, upper-cased.
    pub keyword: Text,
    /// The cached result as `(text, properties)` per piece.
    pub cached: Arc<[(Text, Option<Arc<RunProps>>)]>,
}

/// What an inline drawing's mark shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineObject {
    /// Nothing the model reads (a picture with no relationship, an embedded object).
    None,
    /// A picture, by relationship id.
    Picture(Text),
    /// A shape, group, chart or SmartArt.
    Graphic(Arc<Graphic>),
}

/// `wp:positionH` / `wp:positionV`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnchorPosition {
    /// `relativeFrom`.
    pub relative: Option<Text>,
    /// `wp:posOffset`, EMU.
    pub offset: Option<i64>,
    /// `wp:align`.
    pub align: Option<Text>,
}

/// A floating drawing (`wp:anchor`, or a VML shape read as one), as written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Anchor {
    /// Its extent, EMU.
    pub extent: (i64, i64),
    /// Its effect extent (left, top, right, bottom), EMU.
    pub effect: [i64; 4],
    /// Where it goes across.
    pub h: AnchorPosition,
    /// Where it goes down.
    pub v: AnchorPosition,
    /// `simplePos` when `simplePos="1"`.
    pub simple: Option<(i64, i64)>,
    /// The wrap element's local name.
    pub wrap: Option<Text>,
    /// Its `wrapText`.
    pub wrap_text: Option<Text>,
    /// `distT`, `distB`, `distL`, `distR`.
    pub distance: [i64; 4],
    /// `behindDoc`.
    pub behind: bool,
    /// `relativeHeight`.
    pub relative_height: i64,
    /// `allowOverlap`.
    pub allow_overlap: bool,
    /// `layoutInCell`.
    pub layout_in_cell: bool,
    /// `locked`.
    pub locked: bool,
    /// What it shows.
    pub graphic: Option<Arc<Graphic>>,
    /// `wp:docPr/@id`.
    pub identifier: Option<Text>,
    /// A tight or through wrap's polygon.
    pub polygon: Option<Arc<[(i64, i64)]>>,
    /// Read from VML.
    pub vml: bool,
}

/// A text body setting (`wps:bodyPr`): a number (`None` where not one) or a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyValue {
    /// An inset.
    Int(Option<i64>),
    /// A setting as written.
    Str(Text),
}

/// What a drawing shows, as the file says it (DrawingML); how it is painted is read from
/// its [`Graphic::source`] by its painter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graphic {
    /// `picture`, `shape`, `group`, `chart`, `diagram` or `other`.
    pub kind: Text,
    /// `a:xfrm`'s offset, EMU.
    pub offset: (i64, i64),
    /// `a:xfrm`'s extent, EMU.
    pub extent: (i64, i64),
    /// Rotation, 60000ths of a degree.
    pub rotation: i64,
    /// `flipH`.
    pub flip_h: bool,
    /// `flipV`.
    pub flip_v: bool,
    /// The picture's, chart's or diagram's relationship id.
    pub relationship: Option<Text>,
    /// A text box's blocks.
    pub text: Arc<[Block]>,
    /// `wps:bodyPr`'s settings by name.
    pub body: Arc<[(Text, BodyValue)]>,
    /// A group's members.
    pub children: Arc<[Arc<Graphic>]>,
    /// A group's `a:chOff`.
    pub child_offset: (i64, i64),
    /// A group's `a:chExt`.
    pub child_extent: (i64, i64),
    /// What is there and not drawn.
    pub unsupported: Arc<[Text]>,
    /// `a:graphicData/@uri` for `other`.
    pub uri: Option<Text>,
    /// The structural path of its content.
    pub path: Text,
    /// The element it was read from, for its painter to read how it is painted: the
    /// DrawingML element (`pic:pic`, `wps:wsp`, `wpg:wgp`, `wpg:grpSp`: its geometry, fill,
    /// outline, effects, style references and crop), or for a VML preset shape or text path
    /// the run's object it was read from (`w:pict`, `w:object` or
    /// `mc:AlternateContent`: its fill, stroke and text path); `None` for a chart, a diagram, a VML picture or anything else.
    pub source: Option<ooxml_xml::Element>,
}

/// One run as the layout reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Run {
    /// Its drawn text.
    pub text: Text,
    /// Its `w:rPr`.
    pub properties: Option<Arc<RunProps>>,
    /// Tabs, breaks, note references, drawings: `(index into the text, mark)`, in order.
    pub breaks: Arc<[(usize, Text)]>,
    /// Its structural path from its paragraph (`w:hyperlink[1]/w:r[2]`).
    pub path: Text,
    /// For each inline drawing's mark, what it shows.
    pub drawings: Arc<[InlineObject]>,
    /// Its floating drawings (`anchor:<k>` marks).
    pub anchors: Arc<[Anchor]>,
    /// The computed field it carries.
    pub field: Option<Arc<Field>>,
    /// A table of contents' link.
    pub toc_link: bool,
    /// It draws a note's number: a reference's or a note's own, put in by the layout's
    /// numbering of notes (never by the reader).
    pub note_number: bool,
}

/// A bookmark start or end in a paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    /// A start (else an end).
    pub start: bool,
    /// `w:id`.
    pub id: Option<Text>,
    /// `w:name`.
    pub name: Option<Text>,
    /// The run it stands before.
    pub run: usize,
}

/// A paragraph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paragraph {
    /// Its runs.
    pub runs: Arc<[Run]>,
    /// Its `w:pPr`.
    pub properties: Option<Arc<ParaProps>>,
    /// The innermost table's `w:tblStyle` (`""` none named), `None` outside tables.
    pub table_style_id: Option<Text>,
    /// The table style's regions that apply to its cell, in order (band positions unnamed).
    pub table_conditions: Arc<[Text]>,
    /// Its block's key in its story (`None`: a paragraph no story's index holds -- a text
    /// box's, one the layout makes); a joined paragraph's is its first's.
    pub key: Option<BlockKey>,
    /// Where no story's index holds it, the path it is named by: a text box's (its steps
    /// below its drawing, until RD1 keys text-box blocks), one the layout makes, one of a
    /// part no index holds. `None` with a key.
    pub unkeyed: Option<Text>,
    /// `w14:paraId`.
    pub para_id: Option<Text>,
    /// Where each joined paragraph's runs begin (the final view of a deleted mark).
    pub joins: Arc<[usize]>,
    /// Its mark is deleted and there was no paragraph to join.
    pub mark_deleted: bool,
    /// The section it ends.
    pub section: Option<Arc<SectionProps>>,
    /// Its bookmarks.
    pub bookmarks: Arc<[Bookmark]>,
}

impl Paragraph {
    /// Its text.
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| &*r.text).collect()
    }

    /// Its path where no story's index holds it (a text box's, one the layout makes): what
    /// names it then. A paragraph an index holds is named by its key.
    pub fn unkeyed_path(&self) -> Option<&Text> {
        self.unkeyed.as_ref()
    }

    /// Its path: named through the version's names `names` where a story's index holds it,
    /// else its own.
    #[must_use]
    pub fn path_in(&self, names: &crate::docx::VersionNames) -> Option<String> {
        match self.key {
            Some(key) => names.path(crate::ids::Anchor::whole(key)),
            None => self.unkeyed.as_deref().map(str::to_owned),
        }
    }

    /// Whether `other` lays out as this paragraph does wherever it stands: everything the
    /// same but its name (its key or path) and its bookmarks. A layout names a paragraph's lines by its block
    /// and reads its path from the version's body; its bookmarks are read from the body by
    /// the fields that look them up. The model gives a paragraph it kept by its content and
    /// moved the same parts (`Arc`s), so there this is a few pointer compares.
    pub fn lays_out_as(&self, other: &Paragraph) -> bool {
        // Every field named, so that a new one is weighed here.
        let Paragraph {
            runs,
            properties,
            table_style_id,
            table_conditions,
            key: _,
            unkeyed: _,
            para_id,
            joins,
            mark_deleted,
            section,
            bookmarks: _,
        } = self;
        *runs == other.runs
            && *properties == other.properties
            && *table_style_id == other.table_style_id
            && *table_conditions == other.table_conditions
            && *para_id == other.para_id
            && *joins == other.joins
            && *mark_deleted == other.mark_deleted
            && *section == other.section
    }
}

/// `w:tblLook`: which of the table style's regions the table turns on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Look {
    /// The header row.
    pub first_row: bool,
    /// The total row.
    pub last_row: bool,
    /// The first column.
    pub first_column: bool,
    /// The last column.
    pub last_column: bool,
    /// No row bands.
    pub no_h_band: bool,
    /// No column bands.
    pub no_v_band: bool,
}

/// Each cell's place in its table as written, by the final view's rows and cells: its
/// row's place among the table's rows and its own among the row's cells.
pub type CellPlaces = Arc<[Arc<[(u32, u32)]>]>;

/// A table: rows of cells of blocks, and what its geometry is read from, unresolved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    /// `w:tblStyle`, `None` where it names none.
    pub style_id: Option<Text>,
    /// rows -> cells -> blocks.
    pub rows: Arc<[Cells]>,
    /// Its block's key in its story (`None`: a table no story's index holds); each part of
    /// a table the final view parts has the table's.
    pub key: Option<BlockKey>,
    /// Where no story's index holds it, the path it is named by (a text box's table, its
    /// steps below its drawing until RD1; one of a part no index holds). `None` with a key.
    pub unkeyed: Option<Text>,
    /// Where no story's index holds it, the path its cells are named below: the path it was
    /// read at (a text box's top-level table is named below its drawing's paragraph once
    /// laid out, its cells as they were read).
    pub unkeyed_cells: Option<Text>,
    /// `w:tblGrid`'s widths, twips.
    pub grid: Arc<[i64]>,
    /// `w:tblPr` as declared.
    pub properties: Props,
    /// Each row's `w:trPr`.
    pub row_properties: Arc<[Props]>,
    /// Each cell's `w:tcPr` (revisions accepted).
    pub cell_properties: Arc<[Arc<[Props]>]>,
    /// `w:tblLook`.
    pub look: Look,
    /// Each cell's regions.
    pub conditions: Arc<[RowConditions]>,
    /// Each cell's place in the table as written: its row among the
    /// table's rows and its place among the row's cells, each counted through row- and
    /// cell-level wrappers (what [`crate::ids::Sub::Cell`] names; the final view's rows and
    /// cells, deleted ones dropped, lose it).
    pub cell_places: CellPlaces,
    /// The final view moved content across its edge.
    pub joined: bool,
}

impl Table {
    /// Its path where no story's index holds it (a text box's): what names it then. A table
    /// an index holds is named by its key.
    pub fn unkeyed_path(&self) -> Option<&Text> {
        self.unkeyed.as_ref()
    }

    /// Its path: named through the version's names `names` where a story's index holds it,
    /// else its own.
    #[must_use]
    pub fn path_in(&self, names: &crate::docx::VersionNames) -> Option<String> {
        match self.key {
            Some(key) => names.path(crate::ids::Anchor::whole(key)),
            None => self.unkeyed.as_deref().map(str::to_owned),
        }
    }

    /// The path of the final view's cell `cell` of row `row`, where no story's index holds
    /// the table: its own path and the cell's place as written (`w:tr[r]/w:tc[c]`).
    #[must_use]
    pub fn unkeyed_cell_path(&self, row: usize, cell: usize) -> Option<String> {
        let table = self.unkeyed_cells.as_ref()?;
        let &(r, c) = self.cell_places.get(row)?.get(cell)?;
        Some(cell_path(table, r, c))
    }
}

/// The path of the cell at `(row, cell)` (its place as written) of the table at `table`.
#[must_use]
pub fn cell_path(table: &str, row: u32, cell: u32) -> String {
    format!("{table}/w:tr[{}]/w:tc[{}]", u64::from(row) + 1, u64::from(cell) + 1)
}

/// A block of a story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A paragraph.
    Paragraph(Arc<Paragraph>),
    /// A table.
    Table(Arc<Table>),
}

/// `w:pgSz`, twips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSize {
    /// Width.
    pub width: i64,
    /// Height.
    pub height: i64,
    /// `w:orient`.
    pub orientation: Option<Text>,
}

/// `w:pgMar`, twips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageMargins {
    /// Top.
    pub top: i64,
    /// Right.
    pub right: i64,
    /// Bottom.
    pub bottom: i64,
    /// Left.
    pub left: i64,
    /// Header distance from the page edge.
    pub header: i64,
    /// Footer distance from the page edge.
    pub footer: i64,
    /// Gutter.
    pub gutter: i64,
}

impl Default for PageMargins {
    fn default() -> Self {
        PageMargins {
            top: 1440,
            right: 1440,
            bottom: 1440,
            left: 1440,
            header: 720,
            footer: 720,
            gutter: 0,
        }
    }
}

/// How a section numbers its notes (`w:footnotePr`, `w:endnotePr`), as stated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteNumbering {
    /// `w:numFmt`.
    pub format: Option<Text>,
    /// `w:numStart`.
    pub start: Option<i64>,
    /// `w:numRestart`.
    pub restart: Option<Text>,
    /// `w:pos` (footnotes).
    pub position: Option<Text>,
}

/// `w:lnNumType` as Word reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineNumbering {
    /// `countBy` (1 unstated).
    pub count_by: i64,
    /// `start` (0 unstated).
    pub start: i64,
    /// `distance`, twips; `None` for automatic.
    pub distance: Option<i64>,
    /// `restart` (`newPage` unstated).
    pub restart: Text,
}

/// A section's properties (`w:sectPr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionProps {
    /// `w:pgSz`.
    pub page_size: PageSize,
    /// `w:pgMar`.
    pub margins: PageMargins,
    /// `w:type`.
    pub start_type: Option<Text>,
    /// The number of text columns.
    pub column_count: i64,
    /// The space between columns.
    pub column_space: i64,
    /// Unequal columns' `(width, space after)`.
    pub columns: Arc<[(i64, i64)]>,
    /// `w:cols/@w:sep`.
    pub column_separator: bool,
    /// `(type, relationship id)` of each header reference.
    pub headers: Arc<[(Text, Text)]>,
    /// The same for footers.
    pub footers: Arc<[(Text, Text)]>,
    /// `w:titlePg`.
    pub title_page: bool,
    /// `w:pgNumType/@w:start`.
    pub page_number_start: Option<i64>,
    /// `w:pgNumType/@w:fmt`.
    pub page_number_format: Option<Text>,
    /// `w:vAlign`.
    pub vertical_alignment: Option<Text>,
    /// `w:endnotePr`.
    pub endnotes: NoteNumbering,
    /// `w:footnotePr`.
    pub footnotes: NoteNumbering,
    /// `w:lnNumType`.
    pub line_numbering: Option<LineNumbering>,
}

/// A section: its properties and the top-level paragraphs it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Its properties.
    pub properties: Arc<SectionProps>,
    /// Its first paragraph, an index into the story's top-level paragraphs.
    pub first_paragraph: usize,
    /// One past its last.
    pub last_paragraph: usize,
}
