//! Owns: big-endian reads at an offset of a table's bytes, through read-fonts' `FontData`,
//! each one bounds checked (`None` past the end).
//!
//! Must not: know any table's layout.

use read_fonts::FontData;

/// The `u16` at `offset`.
pub(crate) fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    FontData::new(data).read_at::<u16>(offset).ok()
}

/// The `i16` at `offset`.
pub(crate) fn i16_at(data: &[u8], offset: usize) -> Option<i16> {
    FontData::new(data).read_at::<i16>(offset).ok()
}

/// The `u32` at `offset`.
pub(crate) fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    FontData::new(data).read_at::<u32>(offset).ok()
}

/// `data[start .. start + len]`, cut short at the end of `data` (as a Python slice is).
pub(crate) fn slice(data: &[u8], start: usize, len: usize) -> &[u8] {
    let start = start.min(data.len());
    let end = start.saturating_add(len).min(data.len());
    data.get(start..end).unwrap_or_default()
}
