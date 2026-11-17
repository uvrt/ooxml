//! Owns: an Enhanced Metafile's header and record loop (MS-EMF 2.3), and which records are
//! drawn -- once a file's first EMF+ comment is read, its GDI records are not drawn, except
//! those after an `EmfPlusGetDC` until the next EMF+ record (Windows PowerPoint).
//!
//! The header's frame (`rclFrame`, hundredths of a millimetre) is what is mapped onto the
//! picture's box, converted to the reference device's pixels with `szlDevice /
//! szlMillimeters`, and **inclusive**: its device extent plus one pixel fills the box (the
//! 0.9975 Windows draws a 400 x 300 px frame at in a 400 x 300 pt box). `rclBounds` plays
//! no part.
//!
//! Must not: draw (the player does), or play EMF+ records (`plus` does).

mod objects;
mod records;
pub(crate) mod shapes;

use crate::bytes::Bytes;
use crate::error::{Halt, RecordError};
use crate::fonts::Faces;
use crate::player::Player;
use crate::plus::PlusPlayer;
use ooxml_base::MetafileLimits;

const EMF_SIGNATURE: u32 = 0x464D_4520;
const EMF_PLUS_IDENTIFIER: u32 = 0x2B46_4D45;

/// Records with nothing to draw: colour management, palettes, the brush origin, stretch
/// modes, font mapping hints, escapes.
const IGNORED: [u32; 24] = [
    13, 16, 21, 23, 28, 48, 50, 51, 52, 65, 98, 100, 101, 104, 105, 106, 109, 110, 111, 112, 113, 119, 120, 121,
];

/// Whether `data` starts with an EMF header.
pub fn is_emf(data: &[u8]) -> bool {
    let b = Bytes(data);
    data.len() >= 88 && b.u32(0) == Ok(1) && b.u32(4).is_ok_and(|size| size >= 88) && b.u32(40) == Ok(EMF_SIGNATURE)
}

/// The header's frame and reference device.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Header {
    bounds: [i32; 4],
    frame: [i32; 4],
    pub per_mm: (f64, f64),
}

impl Header {
    fn read(data: &[u8]) -> Option<Header> {
        let b = Bytes(data);
        let four = |at: usize| -> Option<[i32; 4]> {
            Some([
                b.i32(at).ok()?,
                b.i32(at + 4).ok()?,
                b.i32(at + 8).ok()?,
                b.i32(at + 12).ok()?,
            ])
        };
        let (dev_x, dev_y) = (b.i32(72).ok()?, b.i32(76).ok()?);
        let (mm_x, mm_y) = (b.i32(80).ok()?, b.i32(84).ok()?);
        let per_mm = if dev_x > 0 && dev_y > 0 && mm_x > 0 && mm_y > 0 {
            (f64::from(dev_x) / f64::from(mm_x), f64::from(dev_y) / f64::from(mm_y))
        } else {
            (96.0 / 25.4, 96.0 / 25.4)
        };
        Some(Header {
            bounds: four(8)?,
            frame: four(24)?,
            per_mm,
        })
    }

    /// The frame in device pixels, inclusive: `x, y, width, height`.
    pub fn canvas(&self) -> (f64, f64, f64, f64) {
        let [l, t, r, b] = self.frame.map(f64::from);
        if r > l && b > t {
            let (sx, sy) = (self.per_mm.0 / 100.0, self.per_mm.1 / 100.0);
            return (l * sx, t * sy, (r - l) * sx + 1.0, (b - t) * sy + 1.0);
        }
        let [l, t, r, b] = self.bounds.map(i64::from);
        if r >= l && b >= t {
            return (l as f64, t as f64, (r - l + 1) as f64, (b - t + 1) as f64);
        }
        (0.0, 0.0, 1.0, 1.0)
    }

    /// The frame's size at 96 pixels to the inch.
    pub fn natural_size(&self) -> (f64, f64) {
        let [l, t, r, b] = self.frame.map(f64::from);
        if r > l && b > t {
            return ((r - l) / 2540.0 * 96.0, (b - t) / 2540.0 * 96.0);
        }
        let (_, _, w, h) = self.canvas();
        (w, h)
    }
}

/// One playback's result.
pub(crate) struct Played<'a> {
    pub player: Player<'a>,
    pub header: Header,
    pub emf_plus: bool,
}

/// Plays EMF bytes: `None` when they are not an EMF (or are larger than the cap). Unless
/// `gdi_only`, a file's EMF+ records are what is drawn.
pub(crate) fn play<'a>(
    data: &[u8],
    faces: &'a Faces,
    limits: MetafileLimits,
    gdi_only: bool,
    nesting: u32,
) -> Option<Played<'a>> {
    if !is_emf(data) || data.len() > limits.max_input_bytes {
        return None;
    }
    let header = Header::read(data)?;
    let player = Player::new(faces, limits, header.per_mm, header.canvas());
    let mut reader = Reader {
        p: player,
        emf_plus: false,
        gdi_only,
        nesting,
        plus: None,
    };
    let _ = reader.run(data);
    Some(Played {
        player: reader.p,
        header,
        emf_plus: reader.emf_plus,
    })
}

/// The record reader: routes each record to its handler.
pub(crate) struct Reader<'a> {
    pub p: Player<'a>,
    pub emf_plus: bool,
    gdi_only: bool,
    nesting: u32,
    plus: Option<PlusPlayer>,
}

impl Reader<'_> {
    fn run(&mut self, data: &[u8]) -> Result<(), Halt> {
        let b = Bytes(data);
        let mut offset = 0usize;
        for _ in 0..self.p.limits.max_records {
            if offset + 8 > data.len() {
                return Ok(());
            }
            let (Ok(kind), Ok(size)) = (b.u32(offset), b.u32(offset + 4)) else {
                return Ok(());
            };
            let size = size as usize;
            if size < 8
                || !size.is_multiple_of(4)
                || size > self.p.limits.max_record_bytes
                || offset + size > data.len()
            {
                self.p
                    .skip("a record whose size runs past the file (the rest is not read)");
                return Ok(());
            }
            if kind == 14 {
                return Ok(());
            }
            let record = data.get(offset..offset + size).unwrap_or(&[]);
            offset += size;
            if kind != 1 {
                self.record(kind, record)?;
            }
        }
        self.p.truncated = true;
        Ok(())
    }

    fn record(&mut self, kind: u32, rec: &[u8]) -> Result<(), Halt> {
        if self.plus.as_ref().is_some_and(|plus| !plus.get_dc) && kind != 70 {
            return Ok(()); // an EMF+ file's GDI records: not drawn (Windows)
        }
        if IGNORED.contains(&kind) {
            return Ok(());
        }
        let name = format!("EMR_{}", records::name(kind));
        match self.handle(kind, Bytes(rec)) {
            Some(Ok(true)) => Ok(()),
            Some(Ok(false)) | None => {
                self.p.skip(name);
                Ok(())
            }
            Some(Err(RecordError::Halt(h))) => Err(h),
            Some(Err(RecordError::Read(_))) => {
                self.p.skip(format!("{name} (unreadable)"));
                Ok(())
            }
        }
    }

    /// `EMR_COMMENT`: an EMF+ comment is played by the EMF+ player.
    fn comment(&mut self, rec: Bytes<'_>) -> crate::error::Played {
        if rec.len() >= 16 && rec.u32(12)? == EMF_PLUS_IDENTIFIER {
            self.emf_plus = true;
            if self.gdi_only {
                return Ok(true);
            }
            let plus = self.plus.get_or_insert_with(|| PlusPlayer::new(&self.p, self.nesting));
            let size = rec.u32(8)? as usize;
            let end = 12 + size.min(rec.len() - 12);
            plus.play(&mut self.p, rec.slice(16, end))?;
        }
        Ok(true)
    }
}
