//! Owns: a locale given from Python read as the one locale type ([`ooxml_numfmt::Locale`]):
//! ooxml-common's `Locale` (or any object with its five fields), or a BCP 47 tag alone,
//! read as `Locale(language=tag)` reads it (en-US's separators and date order).
//!
//! Must not: keep the Python object, or read the machine's settings.

use ooxml_numfmt::{Locale, EN_US};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyString;

/// `value` as a locale: a tag, or an object with `decimal_separator`,
/// `thousands_separator`, `date_order`, `language` and `list_separator`.
///
/// # Errors
/// `ValueError` for a tag or field the locale refuses; `AttributeError`/`TypeError` for an
/// object without those fields.
pub(crate) fn locale_of(value: &Bound<'_, PyAny>) -> PyResult<Locale> {
    let refused = |e: ooxml_numfmt::LocaleError| PyValueError::new_err(e.to_string());
    if value.is_instance_of::<PyString>() {
        let language = ooxml_numfmt::canonical_language(&value.extract::<String>()?).map_err(refused)?;
        return Ok(Locale {
            language: language.into(),
            ..EN_US
        });
    }
    let field = |name: &str| value.getattr(name);
    Locale::new(
        field("decimal_separator")?.extract()?,
        field("thousands_separator")?.extract()?,
        &field("date_order")?.extract::<String>()?,
        &field("language")?.extract::<String>()?,
        field("list_separator")?.extract()?,
    )
    .map_err(refused)
}
