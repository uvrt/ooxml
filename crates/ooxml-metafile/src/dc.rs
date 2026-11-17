//! Owns: GDI's device context and its objects -- pens, brushes, fonts, the slots kept for
//! objects never drawn with, the stock objects -- with the defaults Windows starts from.
//!
//! Must not: draw; the player does, from this state.

use ooxml_scene::{Matrix, Rgb};
use std::sync::Arc;

// Mapping modes (wingdi.h).
pub const MM_TEXT: u32 = 1;
pub const MM_ISOTROPIC: u32 = 7;
pub const MM_ANISOTROPIC: u32 = 8;

pub const ALTERNATE: u32 = 1;
pub const WINDING: u32 = 2;
pub const OPAQUE: u32 = 2;
pub const AD_COUNTERCLOCKWISE: u32 = 1;
pub const AD_CLOCKWISE: u32 = 2;

pub const RGN_AND: u32 = 1;
pub const RGN_OR: u32 = 2;
pub const RGN_DIFF: u32 = 4;
pub const RGN_COPY: u32 = 5;

// Pen style bits.
pub const PS_DASH: u32 = 1;
pub const PS_DOT: u32 = 2;
pub const PS_DASHDOT: u32 = 3;
pub const PS_DASHDOTDOT: u32 = 4;
pub const PS_NULL: u32 = 5;
pub const PS_INSIDEFRAME: u32 = 6;
pub const PS_USERSTYLE: u32 = 7;
pub const PS_ALTERNATE: u32 = 8;
pub const PS_ENDCAP_MASK: u32 = 0x0F00;
pub const PS_ENDCAP_SQUARE: u32 = 0x0100;
pub const PS_ENDCAP_FLAT: u32 = 0x0200;
pub const PS_JOIN_MASK: u32 = 0xF000;
pub const PS_JOIN_BEVEL: u32 = 0x1000;
pub const PS_JOIN_MITER: u32 = 0x2000;
pub const PS_GEOMETRIC: u32 = 0x0001_0000;

// Text alignment and options.
pub const TA_UPDATECP: u32 = 1;
pub const TA_RIGHT: u32 = 2;
pub const TA_CENTER: u32 = 6;
pub const TA_BOTTOM: u32 = 8;
pub const TA_BASELINE: u32 = 24;
pub const ETO_OPAQUE: u32 = 0x2;
pub const ETO_CLIPPED: u32 = 0x4;
pub const ETO_GLYPH_INDEX: u32 = 0x10;
pub const ETO_PDY: u32 = 0x2000;

/// Raster operations drawn: the source copied, or a rectangle filled.
pub const SRCCOPY: u32 = 0x00CC_0020;
pub const BLACKNESS: u32 = 0x0000_0042;
pub const WHITENESS: u32 = 0x00FF_0062;
pub const PATCOPY: u32 = 0x00F0_0021;
/// Raster operations whose result is the source wherever the destination is the page's
/// background: drawn as a copy (SRCCOPY, SRCPAINT, SRCINVERT, SRCAND, MERGECOPY).
pub const COPY_LIKE: [u32; 5] = [SRCCOPY, 0x00EE_0086, 0x0066_0046, 0x0088_00C6, 0x00C0_00CA];

pub const BLACK: Rgb = Rgb(0, 0, 0);
pub const WHITE: Rgb = Rgb(0xFF, 0xFF, 0xFF);

/// A `COLORREF` (`0x00bbggrr`).
pub fn colorref(value: u32) -> Rgb {
    Rgb(
        (value & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        ((value >> 16) & 0xFF) as u8,
    )
}

/// A pen.
#[derive(Debug, Clone, PartialEq)]
pub struct Pen {
    pub style: u32,
    pub width: f64,
    pub color: Rgb,
    pub geometric: bool,
    /// `CREATEPEN`'s pen, as opposed to `EXTCREATEPEN`'s.
    pub plain: bool,
    pub user_style: Vec<u32>,
    pub null: bool,
}

impl Pen {
    /// A cosmetic solid pen of a colour.
    pub fn solid(color: Rgb) -> Pen {
        Pen {
            style: 0,
            width: 0.0,
            color,
            geometric: false,
            plain: true,
            user_style: Vec::new(),
            null: false,
        }
    }

    /// The style's kind (its low four bits).
    pub fn kind(&self) -> u32 {
        self.style & 0x0F
    }
}

/// A pattern brush's bitmap: PNG bytes and size, with a serial that tells two bitmaps apart
/// (the drawing keeps one pattern per bitmap and transform).
#[derive(Debug, PartialEq)]
pub struct BrushImage {
    pub serial: u64,
    pub png: Vec<u8>,
    pub width: i64,
    pub height: i64,
}

/// A brush.
#[derive(Debug, Clone, PartialEq)]
pub enum Brush {
    Solid(Rgb),
    Null,
    Hatch { color: Rgb, hatch: u32 },
    Pattern(Arc<BrushImage>),
}

/// A logical font.
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    pub height: f64,
    pub escapement: f64,
    pub weight: i64,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub face: String,
    pub pitch_family: u8,
}

impl Font {
    /// The stock font: Arial at 16 pixels.
    pub fn stock() -> Font {
        Font {
            height: -16.0,
            escapement: 0.0,
            weight: 400,
            italic: false,
            underline: false,
            strike: false,
            face: "Arial".to_owned(),
            pitch_family: 0,
        }
    }
}

/// What an object slot holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Pen(Pen),
    Brush(Brush),
    Font(Font),
    /// A palette, colour space, region or pattern the player keeps a slot for.
    Slot,
}

/// A stock object (`GetStockObject`'s number, without the high bit).
pub fn stock(index: u32) -> Option<Object> {
    let gray = |v: u8| Object::Brush(Brush::Solid(Rgb(v, v, v)));
    Some(match index {
        0 | 18 => gray(0xFF),
        1 => gray(0xC0),
        2 => gray(0x80),
        3 => gray(0x40),
        4 => gray(0),
        5 => Object::Brush(Brush::Null),
        6 => Object::Pen(Pen::solid(WHITE)),
        7 | 19 => Object::Pen(Pen::solid(BLACK)),
        8 => Object::Pen(null_pen()),
        10..=14 | 16 | 17 => Object::Font(Font::stock()),
        _ => return None,
    })
}

/// The stock null pen.
pub fn null_pen() -> Pen {
    Pen {
        style: PS_NULL,
        null: true,
        ..Pen::solid(BLACK)
    }
}

/// The device context.
#[derive(Debug, Clone, PartialEq)]
pub struct Dc {
    pub pen: Pen,
    pub brush: Brush,
    pub font: Font,
    pub text_color: Rgb,
    pub bk_color: Rgb,
    pub bk_mode: u32,
    pub fill_mode: u32,
    pub text_align: u32,
    pub rop2: u32,
    pub map_mode: u32,
    pub window_org: (f64, f64),
    pub window_ext: (f64, f64),
    pub viewport_org: (f64, f64),
    pub viewport_ext: (f64, f64),
    pub world: Matrix,
    pub clip: Option<ooxml_scene::ResourceId>,
    pub position: (f64, f64),
    pub miter_limit: f64,
    pub arc_direction: u32,
}

impl Default for Dc {
    fn default() -> Self {
        Dc {
            pen: Pen::solid(BLACK),
            brush: Brush::Solid(WHITE),
            font: Font::stock(),
            text_color: BLACK,
            bk_color: WHITE,
            bk_mode: OPAQUE,
            fill_mode: ALTERNATE,
            text_align: 0,
            rop2: 13,
            map_mode: MM_TEXT,
            window_org: (0.0, 0.0),
            window_ext: (1.0, 1.0),
            viewport_org: (0.0, 0.0),
            viewport_ext: (1.0, 1.0),
            world: Matrix::IDENTITY,
            clip: None,
            position: (0.0, 0.0),
            miter_limit: 10.0,
            arc_direction: AD_COUNTERCLOCKWISE,
        }
    }
}
