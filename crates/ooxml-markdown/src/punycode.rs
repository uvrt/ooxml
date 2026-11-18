//! Owns: punycode (RFC 3492) as Python's `punycode` codec writes and reads it, for host
//! names (`markdown_it._punycode`).
//!
//! Must not: map whole domains (that is [`crate::url`]).

const DIGITS: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";

fn threshold(j: usize, bias: usize) -> usize {
    (36 * (j + 1)).saturating_sub(bias).clamp(1, 26)
}

fn adapt(delta: usize, first: bool, count: usize) -> usize {
    let mut delta = if first { delta / 700 } else { delta / 2 };
    delta += delta / count.max(1);
    let mut divisions = 0;
    while delta > 455 {
        delta /= 35;
        divisions += 36;
    }
    divisions + 36 * delta / (delta + 38)
}

/// A label's punycode (its ASCII characters, `-`, the rest coded); `None` on overflow.
pub(crate) fn encode(label: &str) -> Option<String> {
    let chars: Vec<char> = label.chars().collect();
    let base: String = chars.iter().filter(|c| u32::from(**c) < 128).collect();
    let mut extended: Vec<char> = chars.iter().copied().filter(|c| u32::from(*c) >= 128).collect();
    extended.sort_unstable();
    extended.dedup();
    let mut deltas: Vec<usize> = Vec::new();
    let mut old_char: usize = 0x80;
    let mut old_index: isize = -1;
    for c in extended {
        let code = c as usize;
        let current = chars.iter().filter(|x| **x < c).count();
        let mut delta = (current + 1).checked_mul(code - old_char)?;
        let mut index: isize = -1;
        for x in &chars {
            if *x < c {
                index += 1;
            } else if *x == c {
                index += 1;
                delta += usize::try_from(index - old_index).ok()?;
                deltas.push(delta - 1);
                old_index = index;
                delta = 0;
            }
        }
        old_char = code;
    }
    let mut out = String::new();
    let mut bias = 72;
    for (points, delta) in deltas.iter().enumerate() {
        let mut n = *delta;
        let mut j = 0;
        loop {
            let t = threshold(j, bias);
            if n < t {
                out.push(char::from(*DIGITS.get(n)?));
                break;
            }
            out.push(char::from(*DIGITS.get(t + (n - t) % (36 - t))?));
            n = (n - t) / (36 - t);
            j += 1;
        }
        bias = adapt(*delta, points == 0, base.chars().count() + points + 1);
    }
    Some(if base.is_empty() { out } else { format!("{base}-{out}") })
}

/// A label read back from punycode; `None` where Python's codec raises.
pub(crate) fn decode(text: &str) -> Option<String> {
    if !text.is_ascii() {
        return None;
    }
    let (base, extended) = match text.rfind('-') {
        Some(at) => (text.get(..at)?, text.get(at + 1..)?),
        None => ("", text),
    };
    let extended: Vec<u8> = extended.to_ascii_uppercase().into_bytes();
    let mut out: Vec<char> = base.chars().collect();
    let (mut code, mut pos, mut bias, mut at): (usize, isize, usize, usize) = (0x80, -1, 72, 0);
    while at < extended.len() {
        let (next, delta) = number(&extended, at, bias)?;
        pos += isize::try_from(delta).ok()? + 1;
        let length = isize::try_from(out.len()).ok()? + 1;
        code += usize::try_from(pos / length).ok()?;
        if code > 0x10FFFF {
            return None;
        }
        pos %= length;
        let ch = char::from_u32(u32::try_from(code).ok()?)?;
        out.insert(usize::try_from(pos).ok()?, ch);
        bias = adapt(delta, at == 0, out.len());
        at = next;
    }
    Some(out.into_iter().collect())
}

/// One generalised variable-length integer from `at`: the place after it and its value.
fn number(extended: &[u8], mut at: usize, bias: usize) -> Option<(usize, usize)> {
    let (mut result, mut w, mut j) = (0usize, 1usize, 0usize);
    loop {
        let byte = *extended.get(at)?;
        at += 1;
        let digit = match byte {
            b'A'..=b'Z' => usize::from(byte - b'A'),
            b'0'..=b'9' => usize::from(byte - b'0') + 26,
            _ => return None,
        };
        let t = threshold(j, bias);
        result = result.checked_add(digit.checked_mul(w)?)?;
        if digit < t {
            return Some((at, result));
        }
        w = w.checked_mul(36 - t)?;
        j += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_labels() {
        assert_eq!(encode("bücher").as_deref(), Some("bcher-kva"));
        assert_eq!(encode("münchen").as_deref(), Some("mnchen-3ya"));
        assert_eq!(decode("bcher-kva").as_deref(), Some("bücher"));
        assert_eq!(encode("日本").as_deref(), Some("wgv71a"));
        assert_eq!(decode("wgv71a").as_deref(), Some("日本"));
    }
}
