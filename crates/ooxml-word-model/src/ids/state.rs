//! Owns: the id state kept beside a document's bytes (docx-agent's round 38): the ledger as
//! one short string, `docx-ids.1.` and the url-safe base64 (no padding) of zlib-compressed
//! JSON, written and read as `docx_agent/edit/ids.py`'s `export_state` and `load_state` do,
//! so a state either side wrote opens on the other. The JSON is Python's
//! `json.dumps(record, separators=(",", ":"))` byte for byte (keys in its order, non-ASCII
//! escaped); the compressed bytes may differ from C zlib's, which neither reader minds.
//!
//! Must not: read a package (it is given the parts' roots) or keep anything between calls.

use crate::ids::ledger::{drawing_numbers, used_long_hex, IdPool, Ledger};
use crate::ids::sha256::Sha256;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use ooxml_xml::Element;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};

/// What every id state starts with: the format's name and number.
pub const STATE_PREFIX: &str = "docx-ids.1.";

/// The way out every refusal of a state ends with (docx-agent's words).
const WAY_OUT: &str = "Pass the state saved together with these bytes (Session.save gives both at one \
version; a save_document output carries its own, Output.state), or open the bytes without ids=: the ids \
are then read from the file alone, and one deleted after the last surviving id of its sequence may be given again";

/// Why a state was refused; its message says what to pass instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdStateError {
    /// Not a state docx-agent writes.
    #[error("ids= takes the string Document.id_state() gave (it starts 'docx-ids.1.'), not {0}. {WAY_OUT}")]
    NotAState(String),
    /// A state of a later form.
    #[error("this id state is of another form ('{0}') than this docx-agent reads (1): a later docx-agent wrote it -- upgrade docx-agent. {WAY_OUT}")]
    OtherForm(String),
    /// Not the whole string.
    #[error("this id state is damaged ({0}): it is not the whole string Document.id_state() gave. {WAY_OUT}")]
    Damaged(String),
    /// The state of other bytes.
    #[error("this id state is of other bytes: it was saved with a document whose ids' digest is {saved}, and these bytes' is {found} -- the state of another version of this document, or of another document. {WAY_OUT}")]
    OtherBytes {
        /// The digest the state names.
        saved: String,
        /// These bytes' digest.
        found: String,
    },
}

/// The digest of a document's ids (every paraId, textId and drawing id, and every drawing
/// number) that a state names its bytes by: 24 hex digits of SHA-256.
#[must_use]
pub fn footprint(used: &HashSet<u32>, drawings: &HashSet<u64>) -> String {
    let mut values: Vec<u32> = used.iter().copied().collect();
    values.sort_unstable();
    let mut numbers: Vec<u64> = drawings.iter().copied().collect();
    numbers.sort_unstable();
    let mut hash = Sha256::new();
    for value in values {
        hash.update(&value.to_be_bytes());
    }
    let joined: Vec<String> = numbers.iter().map(u64::to_string).collect();
    hash.update(format!("|{}", joined.join(",")).as_bytes());
    hash.finish().iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// What a state says beyond the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Versions {
    /// The version the state was taken at.
    pub version: i64,
    /// The highest version its history gave.
    pub last: i64,
}

/// The ledger of a document whose parts are `roots`, at `versions`, as one string. Read the
/// state first ([`Ledger::note`]). `describe` says who made a change, by its version.
#[must_use]
pub fn export_state(
    ledger: &Ledger,
    roots: &[&Element],
    versions: &Versions,
    describe: &dyn Fn(i64) -> Option<String>,
) -> String {
    let record = record_of(ledger, roots, versions, describe);
    pack(&record)
}

/// The JSON record [`export_state`] packs (key for key Python's).
#[must_use]
pub fn record_of(
    ledger: &Ledger,
    roots: &[&Element],
    versions: &Versions,
    describe: &dyn Fn(i64) -> Option<String>,
) -> Value {
    let empty = IdPool::default();
    let pool = ledger.pool.as_ref().unwrap_or(&empty);
    let used = used_long_hex(roots);
    let drawings = drawing_numbers(roots);
    let mut scratch = pool.clone();
    let seeds: Vec<(String, usize)> = pool
        .cursors()
        .iter()
        .filter(|(_, k)| **k > 0)
        .map(|(s, k)| (s.clone(), *k))
        .collect();
    let mut place: HashMap<u32, (i64, usize)> = HashMap::new();
    let mut covered: HashSet<u32> = HashSet::new();
    for (number, (seed, k)) in seeds.iter().enumerate() {
        let members: Vec<u32> = scratch.sequence(seed, *k).iter().take(*k).copied().collect();
        for (index, value) in members.iter().enumerate() {
            place.entry(*value).or_insert((number as i64, index));
        }
        covered.extend(members);
    }
    let extra: Vec<i64> = pool
        .values()
        .iter()
        .filter(|v| !used.contains(v) && !covered.contains(v))
        .map(|v| i64::from(*v))
        .collect();
    let gone = gone_groups(ledger, &place);
    let mut drawn: Vec<(u64, i64)> = ledger.drawings_gone.iter().map(|(v, s)| (*v, *s)).collect();
    drawn.sort_unstable();
    let (words, by) = who(ledger, describe);
    let mut replaced: Vec<(u32, &String)> = ledger
        .replaced
        .iter()
        .filter(|(v, _)| ledger.gone.contains_key(v))
        .map(|(v, n)| (*v, n))
        .collect();
    replaced.sort_unstable();
    let mut record = Map::new();
    record.insert("digest".into(), Value::from(footprint(&used, &drawings)));
    record.insert("version".into(), Value::from(versions.version));
    record.insert("last".into(), Value::from(versions.last));
    record.insert(
        "seeds".into(),
        json!(seeds.iter().map(|(s, k)| json!([s, k])).collect::<Vec<_>>()),
    );
    record.insert("extra".into(), json!(deltas(extra.into_iter())));
    record.insert("gone".into(), Value::Array(gone));
    record.insert(
        "drawings".into(),
        json!([
            deltas(drawn.iter().map(|d| i64::try_from(d.0).unwrap_or(i64::MAX))),
            steps(drawn.iter().map(|d| d.1))
        ]),
    );
    record.insert("who".into(), json!(words));
    record.insert(
        "by".into(),
        json!([
            deltas(by.iter().map(|b| b.0)),
            by.iter().map(|b| b.1).collect::<Vec<_>>()
        ]),
    );
    record.insert("highest".into(), Value::from(ledger.highest_drawing));
    let replaced: Map<String, Value> = replaced
        .into_iter()
        .map(|(v, n)| (format!("{v:08X}"), Value::from(n.as_str())))
        .collect();
    record.insert("replaced".into(), Value::Object(replaced));
    let aliases: Map<String, Value> = ledger
        .aliases
        .iter()
        .map(|(k, v)| (k.clone(), Value::from(v.as_str())))
        .collect();
    record.insert("aliases".into(), Value::Object(aliases));
    Value::Object(record)
}

/// Each deleted id: its seed's number (-1: none) and place, and the version that deleted
/// it; per seed in order of place, places and versions as differences.
fn gone_groups(ledger: &Ledger, place: &HashMap<u32, (i64, usize)>) -> Vec<Value> {
    let mut groups: BTreeMap<i64, Vec<(i64, i64)>> = BTreeMap::new();
    for (value, step) in &ledger.gone {
        let (number, index) = place
            .get(value)
            .map_or((-1, i64::from(*value)), |(n, i)| (*n, *i as i64));
        groups.entry(number).or_default().push((index, *step));
    }
    groups
        .into_iter()
        .map(|(number, mut entries)| {
            entries.sort_unstable();
            json!([
                number,
                deltas(entries.iter().map(|e| e.0)),
                steps(entries.iter().map(|e| e.1))
            ])
        })
        .collect()
}

/// Who made each change that deleted something, once per distinct wording: the words and
/// `(version, word index)` in order of version.
fn who(ledger: &Ledger, describe: &dyn Fn(i64) -> Option<String>) -> (Vec<String>, Vec<(i64, usize)>) {
    let mut versions: Vec<i64> = ledger
        .gone
        .values()
        .chain(ledger.drawings_gone.values())
        .copied()
        .collect();
    versions.sort_unstable();
    versions.dedup();
    let mut words: Vec<String> = Vec::new();
    let mut by = Vec::new();
    for step in versions {
        let text = match ledger.earlier.get(&step) {
            Some(text) => text.clone(),
            None => describe(step),
        };
        let Some(text) = text else { continue };
        let at = match words.iter().position(|w| *w == text) {
            Some(at) => at,
            None => {
                words.push(text);
                words.len() - 1
            }
        };
        by.push((step, at));
    }
    (words, by)
}

/// A record as `docx-ids.1.` and the url-safe base64 of its JSON compressed at level 9.
fn pack(record: &Value) -> String {
    let text = python_json(record);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(9));
    // Writing to a Vec cannot fail.
    let packed = encoder
        .write_all(text.as_bytes())
        .and_then(|()| encoder.finish())
        .unwrap_or_default();
    format!("{STATE_PREFIX}{}", base64_url(&packed))
}

/// An id state read back: what [`Ledger::restore`] takes.
#[derive(Debug, Clone)]
pub struct LoadedState {
    /// The versions it was taken at.
    pub versions: Versions,
    /// Every value the document had, and where generation got to per seed.
    pub pool: IdPool,
    /// Each block id gone, with the version that removed it.
    pub gone: HashMap<u32, i64>,
    /// Each drawing number gone, with its version.
    pub drawings_gone: HashMap<u64, i64>,
    /// Who made each of those changes.
    pub steps: HashMap<i64, Option<String>>,
    /// The highest drawing number.
    pub highest_drawing: u64,
    /// The renames' replacements.
    pub replaced: HashMap<u32, String>,
    /// The renames.
    pub aliases: BTreeMap<String, String>,
}

impl Ledger {
    /// Take back what a state saved beside the bytes held, before any state of this session
    /// is read (then [`Ledger::note`] the state the roots are).
    pub fn restore(&mut self, loaded: LoadedState) {
        self.pool = Some(loaded.pool);
        self.gone = loaded.gone;
        self.drawings_gone = loaded.drawings_gone;
        self.highest_drawing = self.highest_drawing.max(loaded.highest_drawing);
        self.replaced = loaded.replaced;
        self.earlier = loaded.steps;
        self.aliases.extend(loaded.aliases);
    }
}

/// Read `state` for the document whose parts are `roots`.
///
/// # Errors
/// An [`IdStateError`] when it is not a state, of another form, damaged, or of other bytes.
pub fn load_state(state: &str, roots: &[&Element]) -> Result<LoadedState, IdStateError> {
    if !state.starts_with("docx-ids.") {
        let shown: String = state.chars().take(24).collect();
        return Err(IdStateError::NotAState(python_repr(&shown)));
    }
    let Some(body) = state.strip_prefix(STATE_PREFIX) else {
        let form = state.split('.').nth(1).unwrap_or("");
        return Err(IdStateError::OtherForm(form.to_owned()));
    };
    let record = unpack(body)?;
    let used = used_long_hex(roots);
    let drawings = drawing_numbers(roots);
    let found = footprint(&used, &drawings);
    let saved = record.get("digest").and_then(Value::as_str).ok_or_else(damaged)?;
    if saved != found {
        return Err(IdStateError::OtherBytes {
            saved: saved.to_owned(),
            found,
        });
    }
    read_record(&record, used, &drawings).ok_or_else(damaged)
}

fn damaged() -> IdStateError {
    IdStateError::Damaged("ValueError".to_owned())
}

fn unpack(body: &str) -> Result<Value, IdStateError> {
    let bytes = base64_url_decode(body).ok_or_else(|| IdStateError::Damaged("Error".to_owned()))?;
    let mut text = Vec::new();
    ZlibDecoder::new(bytes.as_slice())
        .read_to_end(&mut text)
        .map_err(|_| IdStateError::Damaged("error".to_owned()))?;
    serde_json::from_slice(&text).map_err(|_| IdStateError::Damaged("JSONDecodeError".to_owned()))
}

fn read_record(record: &Value, used: HashSet<u32>, drawings: &HashSet<u64>) -> Option<LoadedState> {
    let int = |v: &Value| v.as_i64();
    let versions = Versions {
        version: record.get("version").and_then(int)?,
        last: record.get("last").and_then(int)?,
    };
    let mut pool = IdPool::of(used);
    let mut members: Vec<Vec<u32>> = Vec::new();
    for seed in record.get("seeds")?.as_array()? {
        let name = seed.get(0)?.as_str()?;
        let k = usize::try_from(seed.get(1)?.as_i64()?).ok()?;
        let values: Vec<u32> = pool.sequence(name, k).iter().take(k).copied().collect();
        pool.extend(values.iter().copied());
        pool.set_cursor(name, k);
        members.push(values);
    }
    let empty = Value::Array(Vec::new());
    pool.extend(
        sums(record.get("extra").unwrap_or(&empty))?
            .into_iter()
            .map(u32::try_from)
            .collect::<Result<Vec<_>, _>>()
            .ok()?,
    );
    let mut gone = HashMap::new();
    for group in record.get("gone").unwrap_or(&empty).as_array()? {
        let number = group.get(0)?.as_i64()?;
        let places = sums(group.get(1)?)?;
        let versions = sums(group.get(2)?)?;
        if places.len() != versions.len() {
            return None;
        }
        for (index, step) in places.into_iter().zip(versions) {
            let value = if number >= 0 {
                *members
                    .get(usize::try_from(number).ok()?)?
                    .get(usize::try_from(index).ok()?)?
            } else {
                u32::try_from(index).ok()?
            };
            gone.insert(value, step);
        }
    }
    let drawn = record.get("drawings").cloned().unwrap_or(json!([[], []]));
    let (values, steps_) = (sums(drawn.get(0)?)?, sums(drawn.get(1)?)?);
    if values.len() != steps_.len() {
        return None;
    }
    let drawings_gone: HashMap<u64, i64> = values
        .into_iter()
        .zip(steps_)
        .map(|(v, s)| u64::try_from(v).ok().map(|v| (v, s)))
        .collect::<Option<_>>()?;
    let steps = earlier_steps(record, &gone, &drawings_gone)?;
    pool.extend(gone.keys().copied());
    let highest = record.get("highest").and_then(Value::as_u64).unwrap_or(0);
    Some(LoadedState {
        versions,
        pool,
        gone,
        drawings_gone,
        steps,
        highest_drawing: highest.max(drawings.iter().copied().max().unwrap_or(0)),
        replaced: string_map(record.get("replaced"))?
            .into_iter()
            .map(|(k, v)| u32::from_str_radix(&k, 16).ok().map(|k| (k, v)))
            .collect::<Option<_>>()?,
        aliases: string_map(record.get("aliases"))?.into_iter().collect(),
    })
}

fn earlier_steps(
    record: &Value,
    gone: &HashMap<u32, i64>,
    drawings_gone: &HashMap<u64, i64>,
) -> Option<HashMap<i64, Option<String>>> {
    let words: Vec<String> = match record.get("who") {
        Some(w) => w
            .as_array()?
            .iter()
            .map(|x| x.as_str().map(str::to_owned))
            .collect::<Option<_>>()?,
        None => Vec::new(),
    };
    let by = record.get("by").cloned().unwrap_or(json!([[], []]));
    let versions = sums(by.get(0)?)?;
    let said = by.get(1)?.as_array()?;
    if versions.len() != said.len() {
        return None;
    }
    let mut known = HashMap::new();
    for (step, word) in versions.into_iter().zip(said) {
        let word = words.get(usize::try_from(word.as_i64()?).ok()?)?;
        known.insert(step, word.clone());
    }
    Some(
        gone.values()
            .chain(drawings_gone.values())
            .map(|step| (*step, known.get(step).cloned()))
            .collect(),
    )
}

fn string_map(value: Option<&Value>) -> Option<Vec<(String, String)>> {
    match value {
        None => Some(Vec::new()),
        Some(v) => v
            .as_object()?
            .iter()
            .map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
            .collect(),
    }
}

/// Sorted values as their differences.
fn deltas(values: impl Iterator<Item = i64>) -> Vec<i64> {
    let mut sorted: Vec<i64> = values.collect();
    sorted.sort_unstable();
    steps(sorted.into_iter())
}

/// Values in the order given as their differences.
fn steps(values: impl Iterator<Item = i64>) -> Vec<i64> {
    let mut previous = 0;
    values
        .map(|v| {
            let d = v - previous;
            previous = v;
            d
        })
        .collect()
}

/// Running sums of a JSON array of whole numbers.
fn sums(value: &Value) -> Option<Vec<i64>> {
    let mut running = 0_i64;
    value
        .as_array()?
        .iter()
        .map(|d| {
            running = running.checked_add(d.as_i64()?)?;
            Some(running)
        })
        .collect()
}

/// `json.dumps(value, separators=(",", ":"))`: compact, non-ASCII as `\uXXXX`.
#[must_use]
pub fn python_json(value: &Value) -> String {
    let text = value.to_string();
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut units = [0_u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

/// Python's `repr` of a short string, as a refusal shows it.
fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// URL-safe base64 without padding.
fn base64_url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk.first(), chunk.get(1), chunk.get(2)].map(|b| b.copied().unwrap_or(0));
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..=chunk.len() {
            let index = (n >> (18 - 6 * k)) & 63;
            if let Some(c) = ALPHABET.get(index as usize) {
                out.push(char::from(*c));
            }
        }
    }
    out
}

/// URL-safe base64, padding optional; `None` where it is not base64.
fn base64_url_decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0_u32;
    let mut bits = 0;
    for c in text.bytes() {
        let value = ALPHABET.iter().position(|a| *a == c)?;
        buffer = (buffer << 6) | u32::try_from(value).ok()?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xFF).ok()?);
        }
    }
    (bits < 6).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for data in [&b""[..], b"a", b"ab", b"abc", b"abcd", &[0xff, 0xfe, 0x00, 0x10]] {
            assert_eq!(base64_url_decode(&base64_url(data)).as_deref(), Some(data));
        }
        assert_eq!(base64_url(b"\xfb\xff"), "-_8");
    }

    #[test]
    fn python_json_escapes_non_ascii() {
        assert_eq!(
            python_json(&json!({"a": "\u{e9}\u{1f600}", "b": [1, 2]})),
            "{\"a\":\"\\u00e9\\ud83d\\ude00\",\"b\":[1,2]}"
        );
    }

    #[test]
    fn a_state_round_trips() {
        let mut ledger = Ledger::default();
        ledger.note(&[], 0);
        let made = ledger.pool(&[], 0).generate("word/document.xml", 3);
        let gone = u32::from_str_radix(&made[1], 16).unwrap();
        ledger.gone.insert(gone, 4);
        ledger.aliases.insert("p:1".into(), "p:2".into());
        let versions = Versions { version: 5, last: 6 };
        let state = export_state(&ledger, &[], &versions, &|v| Some(format!("change {v}")));
        let loaded = load_state(&state, &[]).unwrap();
        assert_eq!(loaded.versions, versions);
        assert_eq!(loaded.gone.get(&gone), Some(&4));
        assert_eq!(loaded.steps.get(&4), Some(&Some("change 4".to_owned())));
        assert!(made
            .iter()
            .all(|m| loaded.pool.contains(u32::from_str_radix(m, 16).unwrap())));
        assert_eq!(loaded.aliases.get("p:1").map(String::as_str), Some("p:2"));
        assert!(matches!(load_state("nope", &[]), Err(IdStateError::NotAState(_))));
        assert!(matches!(
            load_state("docx-ids.2.x", &[]),
            Err(IdStateError::OtherForm(_))
        ));
        assert!(matches!(
            load_state("docx-ids.1.!!", &[]),
            Err(IdStateError::Damaged(_))
        ));
    }
}
