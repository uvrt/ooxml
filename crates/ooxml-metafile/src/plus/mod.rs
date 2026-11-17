//! Owns: the EMF+ player ([MS-EMFPLUS]) -- the GDI+ graphics state (world and page
//! transforms, the container's frame, the clip, the save stack and containers, the 64-slot
//! object table, the header's resolution), the record loop of one `EMR_COMMENT`, and the
//! dispatch to each record's handler. It draws through the GDI [`Player`], so both write one
//! scene whose user space is the reference device's pixels.
//!
//! Windows PowerPoint is followed (round 7's measurements, listed in ooxml-common's
//! `metafile.emfplus`): EMF+ device pixels are the EMF's; a pen's width is centred on the
//! outline and scaled by the world transform; angles turn clockwise; the page transform's
//! scale is honoured with its unit from the flags' high byte; a `BeginContainer` maps its
//! rectangles only with a page unit; saves and containers restore transform and clip.
//!
//! Must not: read GDI records (the EMF reader does).

mod clip;
pub(crate) mod image;
pub(crate) mod objects;
mod paint;
mod reader;
mod records;
mod shade;
mod stroke;
mod text;

use crate::bytes::Bytes;
use crate::error::{Halt, RecordError};
use crate::player::{ClipChoice, Player};
use objects::Object;
use ooxml_scene::{Figure, Matrix, Node, ResourceId};
use std::collections::HashMap;

/// Records with nothing for a drawing to change: hints and qualities, and the comment.
const NO_OP: [u16; 6] = [0x4003, 0x401E, 0x401F, 0x4020, 0x4022, 0x4024];
const HEADER: u16 = 0x4001;
const GET_DC: u16 = 0x4004;

// Units.
pub(crate) const U_WORLD: u32 = 0;
pub(crate) const U_DISPLAY: u32 = 1;
pub(crate) const U_PIXEL: u32 = 2;
const U_POINT: u32 = 3;
const U_INCH: u32 = 4;
const U_DOCUMENT: u32 = 5;
const U_MM: u32 = 6;

/// A clip in device coordinates: its clip path, and -- when it is one plain union of
/// figures -- those figures and their rule, for a later combination.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Clip {
    pub id: ResourceId,
    pub parts: Option<Vec<(Vec<Figure>, bool)>>,
}

/// The graphics state.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct State {
    pub world: Matrix,
    pub page_scale: f64,
    pub page_unit: u32,
    /// The container's frame: world-and-page space to device space.
    pub base: Matrix,
    pub clip: Option<Clip>,
    pub base_clip: Option<Clip>,
    pub interpolation: u32,
    pub compositing: u32,
    pub origin: (f64, f64),
}

impl Default for State {
    fn default() -> Self {
        State {
            world: Matrix::IDENTITY,
            page_scale: 1.0,
            page_unit: U_PIXEL,
            base: Matrix::IDENTITY,
            clip: None,
            base_clip: None,
            interpolation: 0,
            compositing: 0,
            origin: (0.0, 0.0),
        }
    }
}

/// A save's key: a `Save` record's index, or a container's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SaveKey {
    Save(u32),
    Container(u32),
}

/// The EMF+ player's own state, kept across the comments of one file.
pub(crate) struct PlusPlayer {
    pub state: State,
    pub objects: HashMap<u32, Object>,
    /// Saves in the order they were made (a key is replaced in place).
    pub saves: Vec<(SaveKey, u64, State)>,
    pub containers: Vec<(u32, State)>,
    pub order: u64,
    /// Continued objects: slot -> (type, total size, bytes so far).
    pub partial: HashMap<u32, (u32, usize, Vec<u8>)>,
    pub dpi: (f64, f64),
    /// The last record was `EmfPlusGetDC`: the GDI records after it are drawn.
    pub get_dc: bool,
    pub nesting: u32,
    pub gradient_pixels: u64,
}

impl PlusPlayer {
    pub fn new(_player: &Player<'_>, nesting: u32) -> PlusPlayer {
        PlusPlayer {
            state: State::default(),
            objects: HashMap::new(),
            saves: Vec::new(),
            containers: Vec::new(),
            order: 0,
            partial: HashMap::new(),
            dpi: (96.0, 96.0),
            get_dc: false,
            nesting,
            gradient_pixels: 0,
        }
    }

    /// The EMF+ records of one comment's data (after its identifier).
    pub fn play(&mut self, p: &mut Player<'_>, data: &[u8]) -> Result<(), Halt> {
        let b = Bytes(data);
        let mut offset = 0usize;
        while offset + 12 <= data.len() {
            let (Ok(kind), Ok(flags), Ok(size), Ok(data_size)) =
                (b.u16(offset), b.u16(offset + 2), b.u32(offset + 4), b.u32(offset + 8))
            else {
                return Ok(());
            };
            let size = size as usize;
            if size < 12 || offset.saturating_add(size) > data.len() {
                p.skip("an EMF+ record whose size runs past its comment");
                return Ok(());
            }
            let body = b.slice(offset + 12, offset + 12 + (data_size as usize).min(size - 12));
            offset += size;
            self.get_dc = kind == GET_DC;
            Plus { pp: self, p }.record(kind, flags, body)?;
        }
        Ok(())
    }
}

/// The EMF+ player and the GDI player it draws through, for one record.
pub(crate) struct Plus<'r, 'a> {
    pub pp: &'r mut PlusPlayer,
    pub p: &'r mut Player<'a>,
}

impl Plus<'_, '_> {
    fn record(&mut self, kind: u16, flags: u16, data: &[u8]) -> Result<(), Halt> {
        if NO_OP.contains(&kind) || kind == HEADER || kind == 0x4002 || kind == GET_DC {
            if kind == HEADER && data.len() >= 16 {
                let b = Bytes(data);
                if let (Ok(dx), Ok(dy)) = (b.u32(8), b.u32(12)) {
                    if (1..100_000).contains(&dx) && (1..100_000).contains(&dy) {
                        self.pp.dpi = (f64::from(dx), f64::from(dy));
                    }
                }
            }
            return Ok(());
        }
        let name = format!("EmfPlus{}", records::name(kind));
        if !records::handled(kind) {
            self.p.skip(name);
            return Ok(());
        }
        let result = self
            .p
            .flush()
            .map_err(RecordError::from)
            .and_then(|()| self.handle(kind, flags, data));
        match result {
            Ok(true) => Ok(()),
            Ok(false) => {
                self.p.skip(name);
                Ok(())
            }
            Err(RecordError::Halt(h)) => Err(h),
            Err(RecordError::Read(_)) => {
                self.p.skip(format!("{name} (unreadable)"));
                Ok(())
            }
        }
    }

    /// World to device: the world transform, the page scale, the container's frame.
    pub fn matrix(&self) -> Matrix {
        let s = &self.pp.state;
        let page = Matrix::scale(
            s.page_scale * self.unit(s.page_unit, 0),
            s.page_scale * self.unit(s.page_unit, 1),
        );
        s.world.then(&page).then(&s.base)
    }

    /// Device pixels per unit (`UnitWorld` and `UnitDisplay` as pixels).
    pub fn unit(&self, unit: u32, axis: usize) -> f64 {
        let dpi = if axis == 0 { self.pp.dpi.0 } else { self.pp.dpi.1 };
        match unit {
            U_POINT => dpi / 72.0,
            U_INCH => dpi,
            U_DOCUMENT => dpi / 300.0,
            U_MM => dpi / 25.4,
            _ => 1.0,
        }
    }

    /// The brush a record names: an ARGB colour with the `S` flag, else an object.
    pub fn brush(&self, flags: u16, value: u32) -> Option<objects::Brush> {
        if flags & 0x8000 != 0 {
            return Some(objects::Brush::Solid(value));
        }
        if value >= 64 {
            return None;
        }
        match self.pp.objects.get(&(value & 0xFF)) {
            Some(Object::Brush(b)) => Some(b.clone()),
            _ => None,
        }
    }

    pub fn pen(&self, index: u32) -> Option<objects::Pen> {
        match self.pp.objects.get(&(index & 0xFF)) {
            Some(Object::Pen(p)) => Some((**p).clone()),
            _ => None,
        }
    }

    /// Draws `node` inside the EMF+ clip.
    pub fn write(&mut self, node: Node) -> Result<(), Halt> {
        self.write_all(vec![node])
    }

    /// Draws `nodes` inside the EMF+ clip, as one drawn element.
    pub fn write_all(&mut self, nodes: Vec<Node>) -> Result<(), Halt> {
        let clip = self.pp.state.clip.as_ref().map(|c| c.id);
        self.p.write_all(nodes, true, ClipChoice::Given(clip))
    }
}
