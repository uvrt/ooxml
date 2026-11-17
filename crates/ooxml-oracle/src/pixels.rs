//! Owns: a PNG written as its pixels' fingerprint (ooxml-common's `tests/goldens.py`,
//! `png_fingerprint` and `stable`): its size, depth, colour type, other chunks' tags, and the
//! SHA-256 of its inflated image data -- the same whichever zlib deflated it -- and the
//! SHA-256 that needs (FIPS 180-4).
//!
//! Must not: be used by any library; it is a test instrument.

use flate2::read::ZlibDecoder;
use ooxml_render::image::base64_decode;
use std::io::Read;

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

/// The SHA-256 of `data`, in hexadecimal.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut message = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());
    for block in message.chunks(64) {
        let mut w: Vec<u32> = block
            .chunks(4)
            .map(|b| b.iter().fold(0u32, |acc, byte| (acc << 8) | u32::from(*byte)))
            .collect();
        for i in 16..64 {
            let at = |k: usize| w.get(k).copied().unwrap_or(0);
            let s0 = at(i - 15).rotate_right(7) ^ at(i - 15).rotate_right(18) ^ (at(i - 15) >> 3);
            let s1 = at(i - 2).rotate_right(17) ^ at(i - 2).rotate_right(19) ^ (at(i - 2) >> 10);
            w.push(at(i - 16).wrapping_add(s0).wrapping_add(at(i - 7)).wrapping_add(s1));
        }
        let mut v = h;
        for (k, word) in K.iter().zip(&w) {
            let [a, b, c, d, e, f, g, hh] = v;
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(*k)
                .wrapping_add(*word);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            v = [
                t1.wrapping_add(s0.wrapping_add(maj)),
                a,
                b,
                c,
                d.wrapping_add(t1),
                e,
                f,
                g,
            ];
        }
        for (a, b) in h.iter_mut().zip(v) {
            *a = a.wrapping_add(b);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

fn be32(data: &[u8], at: usize) -> u32 {
    data.get(at..at + 4)
        .map_or(0, |b| b.iter().fold(0u32, |acc, byte| (acc << 8) | u32::from(*byte)))
}

fn short(hex: &str) -> String {
    hex.chars().take(16).collect()
}

/// A PNG as `WxH:depth:colour[:TAG...]:pixels`.
pub fn png_fingerprint(data: &[u8]) -> String {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return format!("not-png:{}", short(&sha256_hex(data)));
    }
    let (mut offset, mut idat, mut head) = (8usize, Vec::new(), String::new());
    while offset + 8 <= data.len() {
        let length = usize::try_from(be32(data, offset)).unwrap_or(usize::MAX);
        let tag = data.get(offset + 4..offset + 8).unwrap_or_default();
        let end = offset.saturating_add(8).saturating_add(length).min(data.len());
        let body = data.get((offset + 8).min(end)..end).unwrap_or_default();
        match tag {
            b"IHDR" => {
                let (w, h) = (be32(body, 0), be32(body, 4));
                head = format!(
                    "{w}x{h}:{}:{}",
                    body.get(8).copied().unwrap_or(0),
                    body.get(9).copied().unwrap_or(0)
                );
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => {}
            other => {
                head.push(':');
                head.push_str(&String::from_utf8_lossy(other));
            }
        }
        offset = offset.saturating_add(12).saturating_add(length);
    }
    let mut pixels = Vec::new();
    let fingerprint = match ZlibDecoder::new(idat.as_slice()).read_to_end(&mut pixels) {
        Ok(_) => short(&sha256_hex(&pixels)),
        Err(_) => "undecodable".into(),
    };
    format!("{head}:{fingerprint}")
}

/// `svg` with each embedded PNG written as [`png_fingerprint`].
pub fn stable(svg: &str) -> String {
    const MARK: &str = "data:image/png;base64,";
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(at) = rest.find(MARK) {
        let (before, tail) = rest.split_at(at);
        out.push_str(before);
        let after = tail.get(MARK.len()..).unwrap_or_default();
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='))
            .unwrap_or(after.len());
        if end == 0 {
            out.push_str(MARK);
            rest = after;
            continue;
        }
        let (encoded, remainder) = after.split_at(end);
        let bytes = base64_decode(encoded).unwrap_or_default();
        out.push_str("data:image/png;pixels,");
        out.push_str(&png_fingerprint(&bytes));
        rest = remainder;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn sha256_is_the_standard_one() {
        assert_eq!(
            super::sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
