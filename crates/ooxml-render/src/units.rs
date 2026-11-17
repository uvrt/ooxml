//! Owns: the painter's units -- EMU to CSS pixels at 96 dpi (or a caller's dpi) and back, as ooxml-common's
//! `units` computes them (the same operations in the same order, so the same bits) -- and
//! Python's rounding of a channel to a byte.
//!
//! Must not: round a length.

/// EMU an inch.
pub const EMU_PER_INCH: f64 = 914_400.0;
/// Pixels an inch.
pub const DPI: f64 = 96.0;

/// `emu` in px at 96 dpi: `(emu / 914400) * 96`.
pub fn emu_to_px(emu: f64) -> f64 {
    emu / EMU_PER_INCH * DPI
}

/// `emu` in px at `dpi` pixels an inch: `(emu / 914400) * dpi` (ooxml-common's
/// `emu_to_px(emu, dpi)`; at 96 the same bits as [`emu_to_px`]).
pub fn emu_to_px_at(emu: f64, dpi: f64) -> f64 {
    emu / EMU_PER_INCH * dpi
}

/// Pixels a point at `dpi`: `dpi / 72` (at 96, the bits of ooxml-common's `PX_PER_PT`).
pub fn px_per_pt(dpi: f64) -> f64 {
    dpi / 72.0
}

/// `px` in EMU at 96 dpi: `(px / 96) * 914400`.
pub fn px_to_emu(px: f64) -> f64 {
    px / DPI * EMU_PER_INCH
}

/// `max(0, min(255, round(value)))`: Python's rounding, half to even.
pub fn byte(value: f64) -> u8 {
    let rounded = value.round_ties_even().clamp(0.0, 255.0);
    // In 0..=255 after the clamp.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let out = rounded as u8;
    out
}
