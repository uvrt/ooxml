//! Owns: the GDI player both metafile readers drive -- the device context, its saved-state
//! stack and object table, the logical-to-device map, the caps on the work a file may ask
//! for, the open path bracket, and the scene it draws (resources, the drawing grouped by
//! clip) -- with the reference's bookkeeping: what was skipped, how much was drawn, and
//! whether a cap stopped it.
//!
//! The scene's user space is the metafile's device space: the reference device's pixels
//! for an EMF, the placeable header's units for a WMF. A call's logical coordinates are
//! written as they are, with the logical-to-device matrix as the element's transform, so an
//! ellipse under a rotation is still an ellipse and a pen widens anisotropically under an
//! anisotropic map, as Windows draws it.
//!
//! Must not: read records (the EMF and WMF readers do), or know EMF+'s state (its player
//! draws through this one).

mod clip;
pub(crate) mod image;
pub(crate) mod paint;
mod path;
pub(crate) mod text;

use crate::dc::{self, Dc, Object};
use crate::error::Halt;
use crate::fonts::Faces;
use ooxml_base::MetafileLimits;
use ooxml_scene::svg::{write_def, write_node, CharCount};
use ooxml_scene::{
    Bounds, Content, Def, Figure, Group, Matrix, Node, Origin, ResourceId, ResourceKind, Root, Scene, StrokeAttr,
};
use std::collections::{BTreeMap, HashMap};

pub(crate) use paint::PatternKey;

/// The prefix every id is written with while the drawing is played (its length counts
/// toward the output cap); the drawing's own prefix replaces it when it is finished.
const PLAYING_PREFIX: &str = "emf-";

/// The fixed mapping modes' millimetres per logical unit (their y axis points up).
fn fixed_mm(mode: u32) -> Option<f64> {
    match mode {
        2 => Some(0.1),
        3 => Some(0.01),
        4 => Some(0.254),
        5 => Some(0.0254),
        6 => Some(25.4 / 1440.0),
        _ => None,
    }
}

/// Where a drawn element goes: inside the device context's clip, or inside the one given
/// (`None`: no clip) -- EMF+ keeps its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ClipChoice {
    Dc,
    Given(Option<ResourceId>),
}

/// `LINETO`s drawn one by one and kept as one path while their transform and stroke stay
/// the same.
#[derive(Debug, Clone)]
struct Run {
    key: String,
    transform: Option<Matrix>,
    stroke: Vec<StrokeAttr>,
    figures: Vec<Figure>,
}

/// The device context and the scene it draws.
pub(crate) struct Player<'a> {
    pub dc: Dc,
    stack: Vec<Dc>,
    pub objects: HashMap<u32, Object>,
    device_per_mm: (f64, f64),
    /// The device rectangle the drawing shows: `x, y, width, height`.
    pub canvas: (f64, f64, f64, f64),
    pub faces: &'a Faces,
    pub limits: MetafileLimits,
    defs: Vec<Def>,
    body: Vec<Node>,
    open_clip: Option<(ResourceId, Vec<Node>)>,
    size: usize,
    pub drawn: u64,
    points: i64,
    pub pixels: i64,
    pub skipped: BTreeMap<String, u64>,
    pub truncated: bool,
    /// The open path bracket, in device coordinates.
    pub path: Option<Vec<Figure>>,
    /// A closed bracket waiting to be used.
    pub path_done: Option<Vec<Figure>>,
    pub path_widened: bool,
    ids: u32,
    run: Option<Run>,
    pub patterns: Vec<(PatternKey, ResourceId)>,
    serials: u64,
}

impl<'a> Player<'a> {
    pub fn new(
        faces: &'a Faces,
        limits: MetafileLimits,
        device_per_mm: (f64, f64),
        canvas: (f64, f64, f64, f64),
    ) -> Player<'a> {
        Player {
            dc: Dc::default(),
            stack: Vec::new(),
            objects: HashMap::new(),
            device_per_mm,
            canvas,
            faces,
            limits,
            defs: Vec::new(),
            body: Vec::new(),
            open_clip: None,
            size: 0,
            drawn: 0,
            points: 0,
            pixels: 0,
            skipped: BTreeMap::new(),
            truncated: false,
            path: None,
            path_done: None,
            path_widened: false,
            ids: 0,
            run: None,
            patterns: Vec::new(),
            serials: 0,
        }
    }

    // ---- bookkeeping ------------------------------------------------------------------

    /// Names something not drawn (or drawn approximately), counted.
    pub fn skip(&mut self, name: impl Into<String>) {
        *self.skipped.entry(name.into()).or_insert(0) += 1;
    }

    /// A fresh id of `kind`, unique in this drawing.
    pub fn new_id(&mut self, kind: ResourceKind) -> ResourceId {
        self.ids += 1;
        ResourceId { kind, n: self.ids }
    }

    /// A fresh serial for an object compared by identity (a pattern brush's bitmap).
    pub fn serial(&mut self) -> u64 {
        self.serials += 1;
        self.serials
    }

    /// Counts `n` points against the cap; stops the playback past it.
    pub fn count_points(&mut self, n: i64) -> Result<(), Halt> {
        self.points += n;
        if self.points > i64::try_from(self.limits.max_points).unwrap_or(i64::MAX) {
            self.truncated = true;
            return Err(Halt);
        }
        Ok(())
    }

    fn charge(&mut self, chars: usize) -> Result<(), Halt> {
        self.size += chars;
        if self.size > self.limits.max_output_chars {
            self.truncated = true;
            return Err(Halt);
        }
        Ok(())
    }

    /// Appends `node` to the drawing inside the clip `clip` chooses, counting it as drawn or
    /// not.
    pub fn write(&mut self, node: Node, drawn: bool, clip: ClipChoice) -> Result<(), Halt> {
        self.write_all(vec![node], drawn, clip)
    }

    /// Appends `nodes` as one drawn element (the reference writes them as one string).
    pub fn write_all(&mut self, nodes: Vec<Node>, drawn: bool, clip: ClipChoice) -> Result<(), Halt> {
        let mut count = CharCount::default();
        nodes.iter().for_each(|n| write_node(&mut count, n, PLAYING_PREFIX));
        self.charge(count.0)?;
        let target = match clip {
            ClipChoice::Dc => self.dc.clip,
            ClipChoice::Given(c) => c,
        };
        if target != self.open_clip.as_ref().map(|(id, _)| *id) {
            self.close_clip_group();
            if let Some(id) = target {
                self.open_clip = Some((id, Vec::new()));
            }
        }
        match &mut self.open_clip {
            Some((_, children)) => children.extend(nodes),
            None => self.body.extend(nodes),
        }
        if drawn {
            self.drawn += 1;
        }
        Ok(())
    }

    fn close_clip_group(&mut self) {
        if let Some((id, children)) = self.open_clip.take() {
            self.body.push(Node::Group(Group {
                clip: Some(id),
                children,
                ..Group::default()
            }));
        }
    }

    /// Adds a resource.
    pub fn define(&mut self, def: Def) -> Result<(), Halt> {
        let mut count = CharCount::default();
        write_def(&mut count, &def, PLAYING_PREFIX);
        self.charge(count.0)?;
        self.defs.push(def);
        Ok(())
    }

    /// Draws the pending `LINETO`s.
    pub fn flush(&mut self) -> Result<(), Halt> {
        if let Some(run) = self.run.take() {
            let node = Node::Path(ooxml_scene::Path {
                data: ooxml_scene::PathData::new(run.figures),
                transform: run.transform,
                fill: Some(ooxml_scene::Fill::of(ooxml_scene::Paint::None)),
                stroke: run.stroke,
                ..ooxml_scene::Path::default()
            });
            self.write(node, true, ClipChoice::Dc)?;
        }
        Ok(())
    }

    /// The finished drawing: its root shows the canvas, stretched over `width` by `height`.
    pub fn finish(mut self, width: f64, height: f64, id_prefix: String) -> Result<(Scene, u64), Halt> {
        self.flush()?;
        self.close_clip_group();
        let (x, y, w, h) = self.canvas;
        let scene = Scene {
            root: Root {
                width,
                height,
                view_box: Bounds {
                    x,
                    y,
                    width: w,
                    height: h,
                },
                stretch: true,
                origin: Some(Origin::Metafile),
            },
            content: Content {
                id_prefix,
                defs: self.defs,
                body: self.body,
            },
        };
        Ok((scene, self.drawn))
    }

    // ---- mapping ----------------------------------------------------------------------

    fn page_matrix(&self) -> Matrix {
        let dc = &self.dc;
        let (wox, woy) = dc.window_org;
        let (vox, voy) = dc.viewport_org;
        let mode = dc.map_mode;
        let (sx, sy) = if mode == dc::MM_ANISOTROPIC || mode == dc::MM_ISOTROPIC {
            let (wex, wey) = dc.window_ext;
            let (vex, vey) = dc.viewport_ext;
            let mut sx = if wex != 0.0 { vex / wex } else { 1.0 };
            let mut sy = if wey != 0.0 { vey / wey } else { 1.0 };
            if mode == dc::MM_ISOTROPIC {
                let mag = if sy.abs() < sx.abs() { sy.abs() } else { sx.abs() };
                sx = mag.copysign(sx);
                sy = mag.copysign(sy);
            }
            (sx, sy)
        } else if let Some(unit) = fixed_mm(mode) {
            (unit * self.device_per_mm.0, -unit * self.device_per_mm.1)
        } else {
            (1.0, 1.0)
        };
        Matrix::new(sx, 0.0, 0.0, sy, vox - wox * sx, voy - woy * sy)
    }

    /// Logical to device: the world transform, then the page's map.
    pub fn matrix(&self) -> Matrix {
        self.dc.world.then(&self.page_matrix())
    }

    /// `SETWORLDTRANSFORM`; `false` for a matrix that is not finite.
    pub fn set_world(&mut self, m: Matrix) -> bool {
        if !m.is_finite() {
            return false;
        }
        self.dc.world = m;
        true
    }

    /// `MODIFYWORLDTRANSFORM`; `false` for a matrix that is not finite or an unknown mode.
    pub fn modify_world(&mut self, m: Matrix, mode: u32) -> bool {
        if !m.is_finite() {
            return false;
        }
        match mode {
            1 => self.dc.world = Matrix::IDENTITY,
            2 => self.dc.world = m.then(&self.dc.world),
            3 => self.dc.world = self.dc.world.then(&m),
            // MWT_SET is in MS-EMF, but Windows' player leaves the transform as it is
            // (measured: a rectangle after MWT_SET translate (20, 20) at (0, 0)).
            4 => {}
            _ => return false,
        }
        true
    }

    // ---- state stack and objects --------------------------------------------------------

    pub fn save_dc(&mut self) {
        if self.stack.len() >= self.limits.max_save_depth {
            self.skip("SAVEDC beyond the depth cap");
            return;
        }
        self.stack.push(self.dc.clone());
    }

    pub fn restore_dc(&mut self, which: i64) -> Result<(), Halt> {
        let len = i64::try_from(self.stack.len()).unwrap_or(i64::MAX);
        let depth = if which < 0 { len + which } else { which - 1 };
        if depth < 0 || depth >= len {
            return Ok(());
        }
        self.flush()?;
        let depth = usize::try_from(depth).unwrap_or(0);
        if let Some(dc) = self.stack.get(depth) {
            self.dc = dc.clone();
        }
        self.stack.truncate(depth);
        Ok(())
    }

    pub fn create(&mut self, index: u32, obj: Object) {
        if index < self.limits.max_objects {
            self.objects.insert(index, obj);
        }
    }

    /// The object a selection names: a stock object with the high bit, else a slot.
    pub fn object(&self, index: u32) -> Option<Object> {
        if index & 0x8000_0000 != 0 {
            dc::stock(index & 0x7FFF_FFFF)
        } else {
            self.objects.get(&index).cloned()
        }
    }

    pub fn select(&mut self, index: u32) -> Result<(), Halt> {
        match self.object(index) {
            Some(Object::Pen(pen)) => {
                self.flush()?;
                self.dc.pen = pen;
            }
            Some(Object::Brush(brush)) => self.dc.brush = brush,
            Some(Object::Font(font)) => self.dc.font = font,
            _ => {}
        }
        Ok(())
    }

    pub fn delete(&mut self, index: u32) {
        self.objects.remove(&index);
    }

    /// Whether drawing is visible (`R2_NOP` draws nothing).
    pub fn visible(&self) -> bool {
        self.dc.rop2 != 11
    }
}

/// A transform attribute's value: `None` for the identity.
pub(crate) fn transform_of(m: &Matrix) -> Option<Matrix> {
    (!m.is_identity()).then_some(*m)
}
