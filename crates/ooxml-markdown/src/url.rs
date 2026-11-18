//! Owns: link destinations normalised and validated as markdown-it does it
//! (`common/normalize_url.py` over `mdurl` and its `_punycode`): a URL parsed as Node's
//! `url.parse` parses it, an internationalised host name put in punycode (or back, for
//! an autolink's text), the rest percent-encoded (or decoded); `javascript:` and its kin
//! refused.
//!
//! Must not: fetch, resolve or otherwise touch what a URL names.

use crate::chars::is_py_space;

/// A URL in its parts (`mdurl`'s `URL`).
#[derive(Debug, Clone, Default)]
struct Url {
    protocol: Option<String>,
    slashes: bool,
    auth: Option<String>,
    port: Option<String>,
    hostname: Option<String>,
    hash: Option<String>,
    search: Option<String>,
    pathname: Option<String>,
}

const NON_HOST: &[char] = &[
    '%', '/', '?', ';', '#', '\'', '{', '}', '|', '\\', '^', '`', '<', '>', '"', ' ', '\r', '\n', '\t',
];

fn hostless(protocol: &str) -> bool {
    matches!(protocol, "javascript" | "javascript:")
}

fn slashed(protocol: &str) -> bool {
    matches!(
        protocol,
        "http" | "https" | "ftp" | "gopher" | "file" | "http:" | "https:" | "ftp:" | "gopher:" | "file:"
    )
}

fn host_part_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '_' | '-')
}

/// `mdurl.parse(url, slashes_denote_host=True)`.
fn parse(url: &str) -> Url {
    let mut out = Url::default();
    let mut rest: String = url.trim_matches(is_py_space).to_owned();
    let proto_len = rest
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-')))
        .filter(|(at, c)| *c == ':' && *at > 0)
        .map(|(at, _)| at + 1);
    let mut proto = String::new();
    if let Some(len) = proto_len {
        proto = rest.get(..len).unwrap_or_default().to_owned();
        out.protocol = Some(proto.clone());
        rest = rest.get(len..).unwrap_or_default().to_owned();
    }
    let lower_proto = proto.to_lowercase();
    let slashes = rest.starts_with("//");
    if slashes && !(!proto.is_empty() && hostless(&proto)) {
        rest = rest.get(2..).unwrap_or_default().to_owned();
        out.slashes = true;
    }
    if !hostless(&proto) && (slashes || (!proto.is_empty() && !slashed(&proto))) {
        rest = host(&mut out, rest);
    }
    if let Some(at) = rest.find('#') {
        out.hash = Some(rest.get(at..).unwrap_or_default().to_owned());
        rest.truncate(at);
    }
    if let Some(at) = rest.find('?') {
        out.search = Some(rest.get(at..).unwrap_or_default().to_owned());
        rest.truncate(at);
    }
    if !rest.is_empty() {
        out.pathname = Some(rest);
    }
    if slashed(&lower_proto) && out.hostname.as_deref().is_some_and(|h| !h.is_empty()) && out.pathname.is_none() {
        out.pathname = Some(String::new());
    }
    out
}

/// The authority of `rest` read into `out`; what follows it.
fn host(out: &mut Url, mut rest: String) -> String {
    let host_end = rest.find(['/', '?', '#']);
    let at_sign = match host_end {
        None => rest.rfind('@'),
        Some(end) => rest.get(..(end + 1).min(rest.len())).and_then(|r| r.rfind('@')),
    };
    if let Some(at) = at_sign {
        out.auth = Some(rest.get(..at).unwrap_or_default().to_owned());
        rest = rest.get(at + 1..).unwrap_or_default().to_owned();
    }
    let mut end = rest.find(NON_HOST).unwrap_or(rest.len());
    if end > 0 && rest.get(..end).is_some_and(|h| h.ends_with(':')) {
        end -= 1;
    }
    let host = rest.get(..end).unwrap_or_default().to_owned();
    rest = rest.get(end..).unwrap_or_default().to_owned();
    parse_host(out, &host);
    let mut hostname = out.hostname.clone().unwrap_or_default();
    let ipv6 = hostname.starts_with('[') && hostname.ends_with(']');
    if !ipv6 {
        let parts: Vec<String> = hostname.split('.').map(str::to_owned).collect();
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            let fits = |p: &str| p.chars().count() <= 63 && p.chars().all(host_part_char);
            if fits(part) {
                continue;
            }
            let masked: String = part.chars().map(|c| if u32::from(c) > 127 { 'x' } else { c }).collect();
            if fits(&masked) {
                continue;
            }
            let mut valid: Vec<String> = parts.get(..i).unwrap_or_default().to_vec();
            let mut not_host: Vec<String> = parts.get(i + 1..).unwrap_or_default().to_vec();
            let head: String = part.chars().take_while(|c| host_part_char(*c)).take(63).collect();
            let tail: String = part.chars().skip(head.chars().count()).collect();
            if !tail.contains('\n') {
                valid.push(head);
                not_host.insert(0, tail);
            }
            if !not_host.is_empty() {
                rest = not_host.join(".") + &rest;
            }
            hostname = valid.join(".");
            break;
        }
    }
    if hostname.chars().count() > 255 {
        hostname.clear();
    }
    if ipv6 {
        let inner: Vec<char> = hostname.chars().collect();
        hostname = inner
            .get(1..inner.len().saturating_sub(1))
            .unwrap_or_default()
            .iter()
            .collect();
    }
    out.hostname = Some(hostname);
    rest
}

fn parse_host(out: &mut Url, host: &str) {
    let mut host = host;
    if let Some(colon) = host.rfind(':') {
        let digits = host.get(colon + 1..).unwrap_or_default();
        if digits.chars().all(|c| c.is_ascii_digit()) {
            if !digits.is_empty() {
                out.port = Some(digits.to_owned());
            }
            host = host.get(..colon).unwrap_or_default();
        }
    }
    if !host.is_empty() {
        out.hostname = Some(host.to_owned());
    }
}

/// `mdurl.format`.
fn format(url: &Url) -> String {
    let mut out = String::new();
    out.push_str(url.protocol.as_deref().unwrap_or(""));
    if url.slashes {
        out.push_str("//");
    }
    if let Some(auth) = url.auth.as_deref().filter(|a| !a.is_empty()) {
        out.push_str(auth);
        out.push('@');
    }
    match url.hostname.as_deref() {
        Some(h) if h.contains(':') => {
            out.push('[');
            out.push_str(h);
            out.push(']');
        }
        Some(h) => out.push_str(h),
        None => {}
    }
    if let Some(port) = url.port.as_deref().filter(|p| !p.is_empty()) {
        out.push(':');
        out.push_str(port);
    }
    for part in [&url.pathname, &url.search, &url.hash] {
        out.push_str(part.as_deref().unwrap_or(""));
    }
    out
}

/// `mdurl.encode` with its default exclusions: what is not a letter, digit or one of
/// `;/?:@&=+$,-_.!~*'()#` percent-encoded as UTF-8, `%XX` kept.
fn encode(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while let Some(&ch) = chars.get(i) {
        if ch == '%'
            && i + 2 < chars.len()
            && chars
                .get(i + 1..i + 3)
                .is_some_and(|h| h.iter().all(char::is_ascii_hexdigit))
        {
            out.extend(chars.get(i..i + 3).unwrap_or_default());
            i += 3;
            continue;
        }
        if ch.is_ascii_alphanumeric() || ";/?:@&=+$,-_.!~*'()#".contains(ch) {
            out.push(ch);
        } else {
            let mut buffer = [0u8; 4];
            for byte in ch.encode_utf8(&mut buffer).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
        i += 1;
    }
    out
}

/// `mdurl.decode` keeping `exclude`'s characters encoded: runs of `%XX` decoded as UTF-8,
/// bad sequences as U+FFFD.
fn decode(text: &str, exclude: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let byte_at = |k: usize| -> Option<u8> {
        let ok = chars.get(k) == Some(&'%')
            && chars
                .get(k + 1..k + 3)
                .is_some_and(|h| h.iter().all(char::is_ascii_hexdigit));
        ok.then(|| {
            u8::from_str_radix(
                &chars.get(k + 1..k + 3).unwrap_or_default().iter().collect::<String>(),
                16,
            )
            .ok()
        })
        .flatten()
    };
    while i < chars.len() {
        let mut run = Vec::new();
        while let Some(b) = byte_at(i + run.len() * 3) {
            run.push(b);
        }
        if run.is_empty() {
            out.extend(chars.get(i));
            i += 1;
            continue;
        }
        i += run.len() * 3;
        decode_run(&run, exclude, &mut out);
    }
    out
}

/// One run of percent-encoded bytes (`repl_func_with_cache`).
fn decode_run(run: &[u8], exclude: &str, out: &mut String) {
    let mut k = 0;
    while let Some(&b1) = run.get(k) {
        if b1 < 0x80 {
            let ch = char::from(b1);
            if exclude.contains(ch) {
                out.push_str(&format!("%{b1:02X}"));
            } else {
                out.push(ch);
            }
            k += 1;
            continue;
        }
        let need = if b1 & 0xE0 == 0xC0 {
            2
        } else if b1 & 0xF0 == 0xE0 {
            3
        } else if b1 & 0xF8 == 0xF0 {
            4
        } else {
            0
        };
        if need > 0 {
            if let Some(bytes) = run
                .get(k..k + need)
                .filter(|b| b.iter().skip(1).all(|x| x & 0xC0 == 0x80))
            {
                match std::str::from_utf8(bytes) {
                    Ok(s) => out.push_str(s),
                    Err(_) => out.extend(std::iter::repeat_n('\u{fffd}', need)),
                }
                k += need;
                continue;
            }
        }
        out.push('\u{fffd}');
        k += 1;
    }
}

/// `_punycode.map_domain`: an address's domain (after an `@`, the part after the first
/// one only) mapped label by label.
fn map_domain(text: &str, map: impl Fn(&str) -> Option<String>) -> Option<String> {
    let parts: Vec<&str> = text.split('@').collect();
    let (prefix, domain) = match parts.as_slice() {
        [local, domain, ..] => (format!("{local}@"), *domain),
        _ => (String::new(), text),
    };
    let labels: Vec<&str> = domain.split(['.', '\u{3002}', '\u{ff0e}', '\u{ff61}']).collect();
    let mapped: Option<Vec<String>> = labels.iter().map(|l| map(l)).collect();
    Some(prefix + &mapped?.join("."))
}

/// Normalise a link's destination (`normalizeLink`).
pub(crate) fn normalize_link(url: &str) -> String {
    let mut parsed = parse(url);
    let recode = parsed
        .protocol
        .as_deref()
        .is_none_or(|p| matches!(p, "http:" | "https:" | "mailto:"));
    if let Some(host) = parsed.hostname.clone().filter(|h| !h.is_empty() && recode) {
        let ascii = map_domain(&host, |label| {
            if label.chars().any(|c| u32::from(c) > 0x7E) {
                crate::punycode::encode(label).map(|p| format!("xn--{p}"))
            } else {
                Some(label.to_owned())
            }
        });
        if let Some(ascii) = ascii {
            parsed.hostname = Some(ascii);
        }
    }
    if parsed
        .hostname
        .as_deref()
        .is_none_or(|h| h.is_empty() || !h.contains(':'))
    {
        return encode(&format(&parsed));
    }
    let coded = |part: &Option<String>| part.as_deref().filter(|p| !p.is_empty()).map(encode).or(part.clone());
    let url = Url {
        protocol: coded(&parsed.protocol),
        auth: coded(&parsed.auth),
        hostname: parsed.hostname.as_deref().map(encode),
        pathname: coded(&parsed.pathname),
        search: coded(&parsed.search),
        hash: coded(&parsed.hash),
        ..parsed
    };
    format(&url)
}

/// Normalise an autolink's text (`normalizeLinkText`).
pub(crate) fn normalize_link_text(url: &str) -> String {
    let mut parsed = parse(url);
    let recode = parsed
        .protocol
        .as_deref()
        .is_none_or(|p| matches!(p, "http:" | "https:" | "mailto:"));
    if let Some(host) = parsed.hostname.clone().filter(|h| !h.is_empty() && recode) {
        let unicode = map_domain(&host, |label| match label.strip_prefix("xn--") {
            Some(code) => crate::punycode::decode(&code.to_lowercase()),
            None => Some(label.to_owned()),
        });
        if let Some(unicode) = unicode {
            parsed.hostname = Some(unicode);
        }
    }
    decode(&format(&parsed), ";/?:@&=+$,#%")
}

/// Whether a normalised destination may be linked (`validateLink`): not `javascript:`,
/// `vbscript:`, `file:` or `data:` other than an image's.
pub(crate) fn validate_link(url: &str) -> bool {
    let url = url.trim_matches(is_py_space).to_lowercase();
    let bad = ["vbscript:", "javascript:", "file:", "data:"]
        .iter()
        .any(|p| url.starts_with(p));
    !bad || [
        "data:image/gif;",
        "data:image/png;",
        "data:image/jpeg;",
        "data:image/webp;",
    ]
    .iter()
    .any(|p| url.starts_with(p))
}
