"""A dataset's text written by reference (round 45, R65, R66): the host's side.

``word_insert_markdown``'s ``from`` names a table of rows (``d:notes``, ``d:notes/#3:#9``)
-- or one field of one row (``d:notes/#12/body``, ``d:notes/Doors/body``,
``t:checks/Alpha/note``): that value's text, written as the ``markdown`` argument is,
verbatim.  The datasets are the application's, held by the tool layer, so the host resolves
such a value (:func:`field_from`, :func:`text_field`) and hands the Rust session its text and
its record (:func:`as_fact`); the session writes it and keeps the record on what it made.  A
value that is not text is refused with the way out (a figure goes into text as
``{{d:...}}``) and the text fields of its row; an unknown dataset, row or field is refused as
a reference to it is, naming what there is (:func:`with_rows`).  A workbook cell's own
Markdown (its emphasis and list lines) is read again from its source
(:func:`reference_markdown`).
"""

from __future__ import annotations

import re
from typing import Any, Mapping

from ooxml_edit.tools import ToolError
from ooxml_edit.tools import textfacts
from ooxml_edit.tools.datasets import DATASET_PREFIX, row_range

#: Column types whose values are not text.
_NOT_TEXT = {"number", "percent", "date", "datetime", "time", "bool"}
#: A source address after its document handle: ``d2/t:x`` -> ``t:x``.
_HANDLE = re.compile(r"^d[0-9]+/")


# -- one field of one row --------------------------------------------------------------------


def names_one_value(address: Any) -> bool:
    """Whether a ``from`` names one field of one row rather than rows: a dataset address
    past its id that is no run of rows (``d:notes/#12/body``, ``d:notes/Doors/body``)."""
    if not isinstance(address, str) or not address.startswith(DATASET_PREFIX):
        return False
    body = textfacts.strip_format(address)
    rest = body[len(DATASET_PREFIX):]
    return "/" in rest and row_range(body) is None


def might_name_one_value(address: Any) -> bool:
    """Whether a ``from`` that is no dataset (``t:checks/Alpha/note``, ``d2/t:...``) has the
    parts of one value -- a table, a row and a column -- so that a refusal as rows is worth
    reading it as one value."""
    if not isinstance(address, str) or address.startswith(DATASET_PREFIX):
        return False
    _source, segments, _fmt = textfacts.split(address)
    return len(segments) >= 2


def _rows_and_cell(call: Any, address: str, field: str) -> tuple[Any, int, int]:
    """The rows a one-value address reads and the value's ``(row, column)``, as the
    dispatcher's reference resolution picks them (``Call.fact``)."""
    source, segments, _fmt = textfacts.split(address)
    selected = False
    if source.startswith(DATASET_PREFIX):
        rows = call.source(source, field=field)
    elif len(segments) >= 2:
        segment, quoted = segments[0]
        numbered = re.fullmatch(r"#[1-9][0-9]*", segment)
        raw = textfacts.quote(segment) if quoted or not numbered else segment
        rows = call.source(f"{source}/{raw}", field=field)
        segments, selected = segments[1:], True
    else:
        rows = call.source(source, field=field)
    row, column = textfacts.pick(rows, segments, address, field=field, locale=call.locale,
                                 row_selected=selected)
    return rows, row, column


def is_text(value: Any, kind: str | None) -> bool:
    return isinstance(value, str) and kind not in _NOT_TEXT


def text_field(call: Any, address: str, field: str) -> tuple[str, dict[str, Any]]:
    """The text of the one text field ``address`` names, and the record of it
    (``Call.fact``'s).  A value that is not text is refused, naming the row's text fields
    and the way to write a figure; a blank, an unknown dataset, row or field as a
    reference's are."""
    try:
        rows, row, column = _rows_and_cell(call, address, field)
    except ToolError as error:
        with_rows(call, address, field, error)
        raise
    value = rows.dataset.rows[row][column]
    kind = rows.type(column)
    shown = _HANDLE.sub("", textfacts.strip_format(address))
    if value is not None and value != "" and not is_text(value, kind):
        columns = list(rows.dataset.columns)
        texts = [c for k, c in enumerate(columns)
                 if is_text(rows.dataset.rows[row][k], rows.type(k))]
        what = kind or _kind(value)
        prefix = shown.rsplit("/", 1)[0]
        raise ToolError("invalid_arguments", f"{shown} is a {what} value ({value!r}), not text: Markdown comes "
                        f"from a text field" + (f" ({', '.join(texts[:6])})" if texts else "") +
                        f"; a figure goes into the markdown as {{{{{shown}}}}}", field=field,
                        valid_options=[f"{prefix}/{textfacts.quote(c)}" for c in texts[:10]]
                        + [f"markdown: '{{{{{shown}}}}}'"])
    text, record = call.fact(address, field=field)
    return value_markdown(rows, row, column) or text, record


def value_markdown(rows: Any, row: int, column: int) -> str | None:
    """Round 48: the Markdown a source holds for one text value -- a workbook cell's bold
    and italic runs and its list lines (ooxml-edit's ``Dataset.markdown``) -- ``None`` when
    the value is its text."""
    found = getattr(rows.dataset, "markdown_at", None)
    return found(row, column) if found is not None else None


def reference_markdown(call: Any, fact: Any) -> str | None:
    """Round 48: the Markdown of the value a reference wrote (``fact``, the dispatcher's),
    read again from its source; ``None`` when it is its text or cannot be read again."""
    address = fact.address
    doc = fact.record.get("doc")
    if not address.startswith(DATASET_PREFIX) and isinstance(doc, str) and _HANDLE.match(doc + "/"):
        address = f"{doc}/{address}"
    try:
        rows, row, column = _rows_and_cell(call, address, fact.field)
    except ToolError:
        return None
    if not is_text(rows.dataset.rows[row][column], rows.type(column)):
        return None
    return value_markdown(rows, row, column)


def _kind(value: Any) -> str:
    if isinstance(value, bool):
        return "logical"
    if isinstance(value, (int, float)):
        return "number"
    return "date" if hasattr(value, "isoformat") else type(value).__name__


def with_rows(call: Any, address: str, field: str, error: ToolError) -> None:
    """A refusal of a row or column a dataset lacks given the way out (rule 2): whole
    addresses -- the first and last rows with the same field, the row's fields -- in place
    of none or of bare column names."""
    if error.code != "not_found" or not address.startswith(DATASET_PREFIX):
        return
    source, segments, _fmt = textfacts.split(address)
    if len(segments) != 2:
        return
    try:
        dataset = call.source(source, field=field).dataset
    except ToolError:
        return
    row, column = textfacts.quote(segments[0][0]) if segments[0][1] else segments[0][0], segments[1][0]
    count = len(dataset.rows)
    if not error.valid_options and re.fullmatch(r"#[1-9][0-9]*", row) and count:
        error.valid_options = list(dict.fromkeys([f"{source}/#1/{textfacts.quote(column)}",
                                                  f"{source}/#{count}/{textfacts.quote(column)}"]))
    elif error.valid_options and set(error.valid_options) <= set(dataset.columns):
        error.valid_options = [f"{source}/{row}/{textfacts.quote(c)}" for c in error.valid_options]


# -- a value resolved by its handler -----------------------------------------------------


def as_fact(address: str, field: str, text: str, record: Mapping[str, Any]) -> Any:
    """A :class:`ooxml_edit.tools.textfacts.Fact` for a value a handler resolved itself
    (``from``), as the dispatcher makes one for a reference."""
    shown = _HANDLE.sub("", textfacts.strip_format(address))
    return textfacts.Fact(field, 0, len(text), text, shown, dict(record))


def field_from(call: Any, source: str, from_columns: list[str] | None, header: list[str] | None,
               date_format: str | None) -> Any:
    """Round 45, R65: ``from`` naming one field of one row -- its text's fact
    (:class:`~ooxml_edit.tools.textfacts.Fact`, its text the Markdown), else ``None`` (rows:
    a table).  A source that is no dataset is read as rows first, and as one value only when
    rows refuse it."""
    from .table_data import placed

    if not names_one_value(source):
        if not might_name_one_value(source):
            return None
        try:
            placed(call, source, from_columns, header, date_format)
            return None
        except ToolError as error:
            try:
                text, record = text_field(call, source, "from")
            except ToolError:
                raise error from None
    else:
        text, record = text_field(call, source, "from")
    for name, value in (("from_columns", from_columns), ("header", header), ("date_format", date_format)):
        if value is not None:
            raise ToolError("invalid_arguments", f"{name} is for a table of rows; from {source} is one text "
                            "field, written as Markdown: leave it out", field=name)
    return as_fact(source, "from", text, record)
