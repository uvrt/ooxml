//! Owns: the images a call shows the model (ooxml-edit's `results.Image` and `Call.image`):
//! a PNG file a tool gives back made an image of the reply -- its descriptor (`label`,
//! `width`, `height`, `media_type`, each provider's visual `tokens`), its pixel size read
//! from its header -- and the providers' token counts (Claude's 28-pixel patches within its
//! long edge and token cap; OpenAI's 32-pixel patches times the model's multiplier).
//!
//! Must not: draw or decode an image, or send one (the adapters do).

use serde_json::{json, Value};

/// A PNG's width and height, read from its header; `None` for bytes that are not a PNG.
#[must_use]
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let signature = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    if png.get(..8)? != signature || png.get(12..16)? != b"IHDR" {
        return None;
    }
    let word = |at: usize| -> Option<u32> { Some(u32::from_be_bytes(png.get(at..at + 4)?.try_into().ok()?)) };
    Some((word(16)?, word(20)?))
}

fn ceil_div(a: u64, b: u64) -> u64 {
    a.div_ceil(b)
}

/// Claude's visual tokens at the high-resolution tier (long edge 2,576 px, 4,784 tokens): one
/// per 28-pixel patch, the image scaled down first as the API does.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn anthropic_tokens(width: u32, height: u32) -> u64 {
    let (max_edge, max_tokens) = (2576.0_f64, 4784_u64);
    let (mut w, mut h) = (f64::from(width), f64::from(height));
    let edge = w.max(h);
    if edge > max_edge {
        let scale = max_edge / edge;
        // Python's round(): half to even
        w = (w * scale).round_ties_even().max(1.0);
        h = (h * scale).round_ties_even().max(1.0);
    }
    let tokens = |w: f64, h: f64| ceil_div(w as u64, 28) * ceil_div(h as u64, 28);
    let mut found = tokens(w, h);
    while found > max_tokens {
        w = (w * 0.98).floor().max(1.0);
        h = (h * 0.98).floor().max(1.0);
        found = tokens(w, h);
    }
    found
}

/// OpenAI's patch tokens: 32-pixel patches times gpt-6-astra's multiplier (1.2), rounded up.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn openai_tokens(width: u32, height: u32) -> u64 {
    let patches = ceil_div(u64::from(width), 32) * ceil_div(u64::from(height), 32);
    (patches as f64 * 1.2).ceil() as u64
}

/// The descriptor of a PNG image labelled `label`, as the model reads it.
#[must_use]
pub fn descriptor(label: &str, width: u32, height: u32) -> Value {
    json!({"label": label, "width": width, "height": height, "media_type": "image/png",
           "tokens": {"anthropic": anthropic_tokens(width, height), "openai": openai_tokens(width, height)}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_costs_what_todays_results_say() {
        // the recorded render of an A4 page at 1000 px wide
        assert_eq!(anthropic_tokens(1000, 1415), 1836);
        assert_eq!(openai_tokens(1000, 1415), 1728);
    }
}
