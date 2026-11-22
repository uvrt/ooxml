//! Owns: a source's rows as table text (docx-agent's `tools/table_data.py`, `Placed` and
//! `placed`): the columns picked by name in the order placed (default every column, in the
//! source's order), a header row above (the column names, or texts given, or none), a row
//! per source row; each value as [`super::label`] writes it, a blank as an empty cell and a
//! value no cell takes refused naming it; line breaks as the cell's lines, characters XML
//! cannot hold as U+FFFD; the block's provenance for a result (`data.from`); and the
//! refusal of a block bigger than a table or the room it has (`fits`), with the row range
//! that places fewer (`first_rows`).
//!
//! Must not: write into the document.

use super::{date_pattern, label, what, DatePattern, SourceRows};
use ooxml_base::pyfmt::repr_str;
use ooxml_numfmt::Locale;
use ooxml_tools::{Failure, Refusal, ToolError, TryCall, WayOut};
use serde_json::{json, Map, Value};

/// The most cells one call places.
pub(crate) const MAX_CELLS: usize = 5000;

/// A source's rows as table text: the header (if any), then a row per source row.
pub(crate) struct Placed<'r> {
    /// The rows.
    pub(crate) rows: &'r SourceRows,
    /// The source's columns picked, in the order placed.
    pub(crate) at: Vec<usize>,
    /// The header row's texts; empty: none.
    pub(crate) header: Vec<String>,
    locale: Locale,
    pattern: Option<DatePattern>,
}

impl Placed<'_> {
    /// The block's rows (the header's included) and columns.
    pub(crate) fn size(&self) -> (usize, usize) {
        (
            usize::from(!self.header.is_empty()) + self.rows.rows.len(),
            self.at.len(),
        )
    }

    /// The block's cells, row by row (the header first), each as its lines; a value no cell
    /// takes refused, naming it.
    pub(crate) fn lines(&self, retry: &TryCall) -> Result<Vec<Vec<Vec<String>>>, ToolError> {
        let mut out = Vec::with_capacity(self.size().0);
        if !self.header.is_empty() {
            out.push(self.header.iter().map(|t| lines(t)).collect());
        }
        for row in 0..self.rows.rows.len() {
            let mut line = Vec::with_capacity(self.at.len());
            for column in &self.at {
                line.push(lines(&self.text(row, *column, retry)?));
            }
            out.push(line);
        }
        Ok(out)
    }

    /// A value as cell text (`_text`): a blank is empty; anything else not a finite number,
    /// text, a logical value or a date refused, naming the cell.
    fn text(&self, row: usize, column: usize, retry: &TryCall) -> Result<String, ToolError> {
        let value = self.rows.value(row, column);
        if value.is_blank() {
            return Ok(String::new());
        }
        label(value, &self.locale, self.rows.type_of(column), self.pattern.as_ref()).ok_or_else(|| {
            ToolError::refusal(
                Refusal::InvalidArguments,
                format!(
                    "{} is {}: a cell takes text, a number, a logical value or a blank",
                    self.rows.where_is(row, column),
                    what(value)
                ),
                WayOut::call(retry.clone()),
            )
            .at("from")
        })
    }

    /// `data.from` for the block written with its first row at table row `top`.
    pub(crate) fn provenance(&self, top: usize) -> Map<String, Value> {
        let order: Vec<usize> = (0..self.rows.rows.len()).collect();
        let mut out = self.rows.describe();
        out.insert(
            "header_row".into(),
            if self.header.is_empty() {
                Value::Null
            } else {
                json!(top)
            },
        );
        out.insert("first_row".into(), json!(top + usize::from(!self.header.is_empty())));
        let columns: Vec<Value> = self
            .at
            .iter()
            .enumerate()
            .map(|(col, column)| {
                let mut entry = Map::new();
                entry.insert("col".into(), json!(col));
                entry.extend(self.rows.provenance(*column, &order));
                Value::Object(entry)
            })
            .collect();
        out.insert("columns".into(), Value::Array(columns));
        out
    }
}

/// The source's rows and the columns picked (`placed`); refusals name the argument.
///
/// # Errors
/// No column picked, one not in the source or picked twice, header texts that do not fit,
/// a date pattern that is none or for columns without a date.
pub(crate) fn placed<'r>(
    rows: &'r SourceRows,
    from_columns: Option<&[String]>,
    header: Option<&[String]>,
    date_format: Option<&str>,
    locale: &Locale,
    retry: &TryCall,
) -> Result<Placed<'r>, ToolError> {
    let invalid = |message: String, field: &str| {
        ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(retry.clone())).at(field)
    };
    let names: Vec<String> = from_columns.map_or_else(|| rows.columns.clone(), <[String]>::to_vec);
    if names.is_empty() {
        return Err(invalid(
            "from_columns names no column; leave it out for every column".to_owned(),
            "from_columns",
        ));
    }
    let at = names
        .iter()
        .enumerate()
        .map(|(i, name)| rows.column(name, &format!("from_columns[{i}]"), retry))
        .collect::<Result<Vec<usize>, ToolError>>()?;
    if (1..at.len()).any(|k| at[..k].contains(&at[k])) {
        return Err(invalid("a column is named twice".to_owned(), "from_columns"));
    }
    let header: Vec<String> = match header {
        None => at.iter().map(|c| rows.columns[*c].clone()).collect(),
        Some(given) if !given.is_empty() && given.len() != at.len() => {
            return Err(invalid(
                format!(
                    "{} header texts for {} columns; [] for no header row",
                    given.len(),
                    at.len()
                ),
                "header",
            ))
        }
        Some(given) => given.to_vec(),
    };
    let pattern = match date_format {
        None => None,
        Some(given) => {
            let pattern = date_pattern(given, locale, "date_format")?;
            let dated = at.iter().any(|column| {
                (0..rows.rows.len())
                    .any(|row| super::label::date_value(rows.value(row, *column), rows.type_of(*column)).is_some())
            });
            if !dated {
                return Err(no_dates(rows, &at, retry));
            }
            Some(pattern)
        }
    };
    Ok(Placed {
        rows,
        at,
        header,
        locale: locale.clone(),
        pattern,
    })
}

/// The refusal of a `date_format` for columns that hold no date (`no_dates`).
fn no_dates(rows: &SourceRows, columns: &[usize], retry: &TryCall) -> ToolError {
    let names = columns
        .iter()
        .take(5)
        .map(|c| repr_str(&rows.columns[*c]))
        .collect::<Vec<_>>()
        .join(", ");
    ToolError::refusal(
        Refusal::InvalidArguments,
        format!(
            "date_format is for dates, and no value of {}'s column{} {names} is one (a Python date, or a column \
             labelled date or datetime); leave date_format out",
            rows.address,
            if columns.len() > 1 { "s" } else { "" }
        ),
        WayOut::call(retry.clone()),
    )
    .at("date_format")
}

/// A cell's text as its lines (`_lines`): split at `\r\n`, `\r`, `\n` and `\v`; characters
/// XML cannot hold as U+FFFD.
fn lines(text: &str) -> Vec<String> {
    let clean = |line: &str| -> String {
        line.chars()
            .map(|c| match c {
                '\u{0}'..='\u{8}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => '\u{fffd}',
                other => other,
            })
            .collect()
    };
    let mut out = Vec::new();
    let mut rest = text;
    loop {
        match rest.find(['\r', '\n', '\u{b}']) {
            Some(k) => {
                out.push(clean(&rest[..k]));
                let skip = if rest[k..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[k + skip..];
            }
            None => {
                out.push(clean(rest));
                return out;
            }
        }
    }
}

/// Row positions from 0 as text from 1, consecutive ones as runs (`runs`): `1-4,7`.
pub(crate) fn runs(rows: &[usize]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut k = 0;
    while k < rows.len() {
        let start = rows[k];
        let mut end = start;
        while k + 1 < rows.len() && rows[k + 1] == end + 1 {
            k += 1;
            end = rows[k];
        }
        parts.push(if end == start {
            (start + 1).to_string()
        } else {
            format!("{}-{}", start + 1, end + 1)
        });
        k += 1;
    }
    parts.join(",")
}

/// The first `count` rows of a source as a row range (`first_rows`): `t:sales/#1:#9` of
/// `t:sales`, `d:x/#10:#14` of `d:x/#10:`, `d:x/#1` for one row; `None` for a source that is
/// no whole dataset or table nor a row range, or `count` under 1.
pub(crate) fn first_rows(address: &str, count: i64) -> Option<String> {
    if count < 1 {
        return None;
    }
    let address = address.trim();
    // `(dN/)?x:name`, the name without `/`, `[` or `]`; then `/#first:`, `/#first:#last`
    // or `/#first:last`
    let qualifier = address
        .split_once('/')
        .filter(|(d, _)| d.len() > 1 && d.starts_with('d') && d[1..].bytes().all(|c| c.is_ascii_digit()))
        .map_or(0, |(d, _)| d.len() + 1);
    let unqualified = &address[qualifier..];
    let (name_part, range) = match unqualified.split_once('/') {
        Some((name, range)) => (name, Some(range)),
        None => (unqualified, None),
    };
    let base = &address[..qualifier + name_part.len()];
    let mut chars = name_part.chars();
    let (Some(letter), Some(':')) = (chars.next(), chars.next()) else {
        return None;
    };
    let name = chars.as_str();
    if !letter.is_ascii_lowercase() || name.is_empty() || name.contains(['[', ']']) {
        return None;
    }
    let number = |t: &str| t.starts_with(|c: char| ('1'..='9').contains(&c)) && t.bytes().all(|c| c.is_ascii_digit());
    let first: i64 = match range {
        None => 1,
        Some(range) => {
            let (first, last) = range.strip_prefix('#')?.split_once(':')?;
            if !number(first) || !(last.is_empty() || number(last.strip_prefix('#').unwrap_or(last))) {
                return None;
            }
            first.parse().ok()?
        }
    };
    let last = first + count - 1;
    Some(if last == first {
        format!("{base}/#{first}")
    } else {
        format!("{base}/#{first}:#{last}")
    })
}

/// Refuse a block bigger than `max_rows` x `max_columns`, saying by how much (`fits`): a
/// table's limits (`room` `None`: `limit`), or the room an existing table has (`refused`).
///
/// # Errors
/// The block does not fit.
pub(crate) fn fits(
    block: &Placed<'_>,
    max_rows: usize,
    max_columns: usize,
    room: Option<&str>,
    retry: &TryCall,
) -> Result<(), ToolError> {
    let address = &block.rows.address;
    let (need_rows, need_columns) = block.size();
    let header = !block.header.is_empty();
    let error = |message: String, field: &str, options: Vec<String>| -> ToolError {
        let way_out = WayOut::names(options).unwrap_or_else(|| WayOut::call(retry.clone()));
        match room {
            Some(_) => ToolError::refusal(Refusal::Refused, message, way_out).at(field),
            None => ToolError::failure(Failure::Limit, message)
                .at(field)
                .with_way_out(way_out),
        }
    };
    if need_rows > max_rows {
        let fewer = first_rows(address, i64::try_from(max_rows).unwrap_or(i64::MAX) - i64::from(header));
        let fix = match room {
            Some(room) => format!("{room}, or insert_row first"),
            None => format!(
                "a table takes at most {}: filter the source or split it across tables",
                thousands(max_rows)
            ),
        } + &fewer
            .as_ref()
            .map_or_else(String::new, |f| format!(", or place fewer with {f}"));
        return Err(error(
            format!(
                "{address} gives {} rows{}; {fix} (nothing is cut)",
                thousands(need_rows),
                if header { " with the header" } else { "" }
            ),
            "from",
            fewer.into_iter().collect(),
        ));
    }
    if need_columns > max_columns {
        let fix = if room.is_some() {
            format!("the table has {max_columns}")
        } else {
            format!("a table takes at most {max_columns}")
        };
        return Err(error(
            format!("{address}: {need_columns} columns picked; {fix}: pick fewer in from_columns"),
            "from_columns",
            Vec::new(),
        ));
    }
    if need_rows * need_columns > MAX_CELLS {
        return Err(ToolError::failure(
            Failure::Limit,
            format!(
                "{address} gives {}x{need_columns} = {} cells; at most {} in one call: filter the source or pick \
                 fewer columns (nothing is cut)",
                thousands(need_rows),
                thousands(need_rows * need_columns),
                thousands(MAX_CELLS)
            ),
        )
        .at("from"));
    }
    Ok(())
}

/// Python's `f"{n:,}"`.
pub(crate) fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
