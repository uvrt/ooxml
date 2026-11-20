//! Owns: cell positions and their A1 spelling -- a column's letters and number, a cell
//! ([`Cell`]), a rectangle of cells ([`Area`]), and a chart formula's one range on one sheet
//! ([`SheetRange`], [`parse_formula`]) -- read as ooxml-edit's `sheets/workbook.py` reads
//! them, so a reference it accepts is accepted here and one it refuses is refused.
//!
//! Must not: look into a workbook; a reference names cells, it never reads them.
//!
//! Digits are ASCII digits: Python's `\d` also takes other scripts' decimal digits, which no
//! application writes in a reference.

use ooxml_xml::view::py_space;
use thiserror::Error;

/// The sheet's last row (Excel's grid).
pub const MAX_ROW: u32 = 1_048_576;
/// The sheet's last column, `XFD`.
pub const MAX_COLUMN: u32 = 16_384;

/// Why a reference was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AddressError {
    /// A column number off the grid.
    #[error("column {0} is outside the sheet")]
    ColumnOutside(u32),
    /// Text that is not a column's letters.
    #[error("{0:?} is not a column's letters (one to three of A to Z, as A, AA or XFD)")]
    NotColumnLetters(String),
    /// Letters past `XFD`.
    #[error("{0:?} is past the sheet's last column, XFD")]
    PastLastColumn(String),
    /// Text that is not a cell reference.
    #[error("{0:?} is not a cell reference")]
    NotACell(String),
    /// Text that is not a range.
    #[error("{0:?} is not a range")]
    NotARange(String),
}

/// `1` -> `A`, `27` -> `AA`.
///
/// # Errors
/// [`AddressError::ColumnOutside`] for a number off the grid.
pub fn column_letters(index: u32) -> Result<String, AddressError> {
    if !(1..=MAX_COLUMN).contains(&index) {
        return Err(AddressError::ColumnOutside(index));
    }
    let mut letters = Vec::new();
    let mut rest = index;
    while rest > 0 {
        let remainder = (rest - 1) % 26;
        rest = (rest - 1) / 26;
        letters.push(char::from(b'A' + remainder as u8));
    }
    Ok(letters.into_iter().rev().collect())
}

/// `A` -> 1, `AA` -> 27, `XFD` -> 16384, case aside: one to three ASCII letters on the grid.
///
/// # Errors
/// [`AddressError::NotColumnLetters`] or [`AddressError::PastLastColumn`].
pub fn column_index(letters: &str) -> Result<u32, AddressError> {
    let count = letters.chars().count();
    if !(1..=3).contains(&count) || !letters.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(AddressError::NotColumnLetters(letters.to_owned()));
    }
    let value = letters.bytes().fold(0u32, |value, b| {
        value * 26 + u32::from(b.to_ascii_uppercase() - b'A' + 1)
    });
    if value > MAX_COLUMN {
        return Err(AddressError::PastLastColumn(letters.to_owned()));
    }
    Ok(value)
}

/// A cell reference's parts as written: `$`, letters, `$`, digits.
#[derive(Clone, Copy)]
struct CellParts<'a> {
    /// The whole reference.
    text: &'a str,
    column_absolute: bool,
    letters: &'a str,
    row_absolute: bool,
    digits: &'a str,
}

/// The longest prefix of `text` that is `\$?[A-Za-z]{1,3}\$?\d{1,7}`, and what follows it.
fn cell_prefix(text: &str) -> Option<(CellParts<'_>, &str)> {
    let (column_absolute, rest) = match text.strip_prefix('$') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let letters_end = rest.bytes().take(3).take_while(u8::is_ascii_alphabetic).count();
    let (letters, rest) = rest.split_at_checked(letters_end)?;
    let (row_absolute, rest) = match rest.strip_prefix('$') {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    let digits_end = rest.bytes().take(7).take_while(u8::is_ascii_digit).count();
    let (digits, rest) = rest.split_at_checked(digits_end)?;
    if letters.is_empty() || digits.is_empty() {
        return None;
    }
    let parts = CellParts {
        text: text.get(..text.len() - rest.len())?,
        column_absolute,
        letters,
        row_absolute,
        digits,
    };
    Some((parts, rest))
}

/// A cell position, 1-based: `Cell { row: 2, column: 1 }` is `A2`. Ordered by row, then
/// column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cell {
    /// The row.
    pub row: u32,
    /// The column.
    pub column: u32,
}

impl Cell {
    /// The reference, `A2`.
    ///
    /// # Errors
    /// [`AddressError::ColumnOutside`] for a column off the grid.
    pub fn reference(&self) -> Result<String, AddressError> {
        Ok(format!("{}{}", column_letters(self.column)?, self.row))
    }

    /// A reference such as `B2` or `$B$2`, surrounding whitespace aside.
    ///
    /// # Errors
    /// [`AddressError::NotACell`], or [`AddressError::PastLastColumn`] naming `text`.
    pub fn parse(text: &str) -> Result<Cell, AddressError> {
        let not_a_cell = || AddressError::NotACell(text.to_owned());
        let (parts, rest) = cell_prefix(text.trim_matches(py_space)).ok_or_else(not_a_cell)?;
        if !rest.is_empty() {
            return Err(not_a_cell());
        }
        Cell::from_parts(&parts, text)
    }

    fn from_parts(parts: &CellParts<'_>, text: &str) -> Result<Cell, AddressError> {
        let column = column_index(parts.letters).map_err(|_| AddressError::PastLastColumn(text.to_owned()))?;
        let row = parts
            .digits
            .parse()
            .map_err(|_| AddressError::NotACell(text.to_owned()))?;
        Ok(Cell { row, column })
    }
}

/// A rectangle of cells, both corners included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Area {
    /// The top left cell.
    pub first: Cell,
    /// The bottom right cell.
    pub last: Cell,
}

impl Area {
    /// The reference, `A1:D4`, or `B2` for one cell.
    ///
    /// # Errors
    /// [`AddressError::ColumnOutside`] for a column off the grid.
    pub fn reference(&self) -> Result<String, AddressError> {
        if self.first == self.last {
            return self.first.reference();
        }
        Ok(format!("{}:{}", self.first.reference()?, self.last.reference()?))
    }

    /// How many rows.
    pub fn rows(&self) -> u32 {
        self.last.row - self.first.row + 1
    }

    /// How many columns.
    pub fn columns(&self) -> u32 {
        self.last.column - self.first.column + 1
    }

    /// Whether `cell` is inside.
    pub fn contains(&self, cell: Cell) -> bool {
        (self.first.row..=self.last.row).contains(&cell.row)
            && (self.first.column..=self.last.column).contains(&cell.column)
    }

    /// Every cell, row by row.
    pub fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        (self.first.row..=self.last.row)
            .flat_map(move |row| (self.first.column..=self.last.column).map(move |column| Cell { row, column }))
    }

    /// `A1:D4` or `B2`, surrounding whitespace aside; the corners in either order.
    ///
    /// # Errors
    /// [`AddressError::NotARange`], or [`AddressError::PastLastColumn`] for a corner past
    /// `XFD`.
    pub fn parse(text: &str) -> Result<Area, AddressError> {
        let not_a_range = || AddressError::NotARange(text.to_owned());
        let stripped = text.trim_matches(py_space);
        let (head, rest) = cell_prefix(stripped).ok_or_else(not_a_range)?;
        let first = Cell::from_parts(&head, head.text)?;
        let last = match rest.strip_prefix(':') {
            None if rest.is_empty() => first,
            None => return Err(not_a_range()),
            Some(tail) => match cell_prefix(tail) {
                Some((parts, "")) => Cell::from_parts(&parts, tail)?,
                _ => return Err(not_a_range()),
            },
        };
        Ok(Area::spanning(first, last))
    }

    /// The rectangle with `a` and `b` at two of its corners.
    pub fn spanning(a: Cell, b: Cell) -> Area {
        Area {
            first: Cell {
                row: a.row.min(b.row),
                column: a.column.min(b.column),
            },
            last: Cell {
                row: a.row.max(b.row),
                column: a.column.max(b.column),
            },
        }
    }
}

/// A chart formula such as `Sheet1!$B$2:$B$4`: one rectangle on one named sheet, with its
/// spelling kept (the sheet name's quoting, which coordinates were absolute).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SheetRange {
    /// The sheet's name, unquoted: `My data`.
    pub sheet: String,
    /// The cells.
    pub area: Area,
    /// The sheet as written, quotes included: `'My data'`.
    pub sheet_text: String,
    /// `[column absolute, row absolute]` for the first and the last corner.
    pub absolute: [[bool; 2]; 2],
}

impl SheetRange {
    /// The formula's text, spelled as it was read.
    ///
    /// # Errors
    /// [`AddressError::ColumnOutside`] for a column off the grid.
    pub fn formula(&self) -> Result<String, AddressError> {
        let spell = |cell: Cell, [column, row]: [bool; 2]| -> Result<String, AddressError> {
            let dollar = |on: bool| if on { "$" } else { "" };
            Ok(format!(
                "{}{}{}{}",
                dollar(column),
                column_letters(cell.column)?,
                dollar(row),
                cell.row
            ))
        };
        let [first_flags, last_flags] = self.absolute;
        let first = spell(self.area.first, first_flags)?;
        if self.area.first == self.area.last {
            return Ok(format!("{}!{first}", self.sheet_text));
        }
        Ok(format!(
            "{}!{first}:{}",
            self.sheet_text,
            spell(self.area.last, last_flags)?
        ))
    }

    /// Whether the range is one row or one column.
    pub fn is_line(&self) -> bool {
        self.area.rows() == 1 || self.area.columns() == 1
    }

    /// How many cells along its longer side.
    pub fn length(&self) -> u32 {
        self.area.rows().max(self.area.columns())
    }

    /// The `index`-th cell of a one-row or one-column range (down a column when it is one
    /// column); `None` for a rectangle or an index past its end.
    pub fn cell(&self, index: u32) -> Option<Cell> {
        if !self.is_line() || index >= self.length() {
            return None;
        }
        let first = self.area.first;
        Some(if self.area.columns() == 1 {
            Cell {
                row: first.row + index,
                ..first
            }
        } else {
            Cell {
                column: first.column + index,
                ..first
            }
        })
    }
}

/// The end of a sheet name at the start of `text`: quoted (`'it''s'`, quotes doubled
/// inside) or bare (no quote, `!`, `:`, parenthesis, whitespace or comma).
fn sheet_name_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices().peekable();
    if text.starts_with('\'') {
        chars.next();
        let mut inside = 0;
        while let Some((at, c)) = chars.next() {
            if c != '\'' {
                inside += 1;
            } else if chars.peek().is_some_and(|&(_, next)| next == '\'') {
                chars.next();
                inside += 1;
            } else {
                return (inside > 0).then_some(at + 1);
            }
        }
        return None;
    }
    let bare = |c: char| !matches!(c, '\'' | '!' | ':' | '(' | ')' | ',') && !py_space(c);
    let end = text.find(|c: char| !bare(c)).unwrap_or(text.len());
    (end > 0).then_some(end)
}

/// A chart's `c:f` as one rectangle on one sheet: `None` for anything else (several areas,
/// a defined name, another workbook, corners the wrong way round), which a reader then takes
/// from the chart's cache only.
///
/// # Errors
/// [`AddressError::PastLastColumn`] for a corner past `XFD`, which ooxml-edit refuses
/// rather than reads as no range.
pub fn parse_formula(text: &str) -> Result<Option<SheetRange>, AddressError> {
    let text = text.trim_start_matches(py_space);
    let Some(end) = sheet_name_end(text) else {
        return Ok(None);
    };
    let (sheet_text, rest) = text.split_at(end);
    let Some((head, rest)) = rest.strip_prefix('!').and_then(cell_prefix) else {
        return Ok(None);
    };
    let (tail, rest) = match rest.strip_prefix(':') {
        None => (None, rest),
        Some(after) => match cell_prefix(after) {
            Some((tail, rest)) => (Some(tail), rest),
            None => return Ok(None),
        },
    };
    if !rest.trim_start_matches(py_space).is_empty() || sheet_text.starts_with('[') {
        return Ok(None);
    }
    let tail = tail.unwrap_or(head);
    let (first, last) = (Cell::from_parts(&head, head.text)?, Cell::from_parts(&tail, tail.text)?);
    if first.row > last.row || first.column > last.column {
        return Ok(None);
    }
    let sheet = match sheet_text.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Some(quoted) => quoted.replace("''", "'"),
        None => sheet_text.to_owned(),
    };
    Ok(Some(SheetRange {
        sheet,
        area: Area { first, last },
        sheet_text: sheet_text.to_owned(),
        absolute: [
            [head.column_absolute, head.row_absolute],
            [tail.column_absolute, tail.row_absolute],
        ],
    }))
}
