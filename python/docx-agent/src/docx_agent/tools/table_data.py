"""Tables from data (X1), the host's side: what a source names, read with ``call.source``
(:mod:`ooxml_edit.tools.sources`).

A source is a dataset (``d:<id>``) or a table of another open document (``t:<name>``); the
Rust session writes its rows (``word_insert_markdown``'s and ``word_edit_table``'s
``from``).  The host reads the rows only to tell rows from one value (:func:`placed`, for
:func:`.field_text.field_from`), and words two refusals of the format's:

- ``d:`` is also how a Word document names a drawing (``d:3``); a ``from`` only ever names a
  dataset (whose id starts with a letter or ``_``), and :func:`source_hint` says so when a
  drawing's id is given;
- a Word document's own tables give no rows (round 56, R120): a ``from`` naming one
  (``t:60D6847F``, ``d1/t:60D6847F``) is told it is a table of the document being edited
  (:func:`own_source`).

Both read the version of the document the session owns (:mod:`docx_agent.rust.hooks`).
"""

from __future__ import annotations

import re
from typing import Any

from ooxml_edit.tools import ToolError
from ooxml_edit.tools import sources

from ..rust import hooks

_DRAWING = re.compile(r"^d:[0-9]+(#[0-9]+)?$")


def placed(call: Any, source: str, from_columns: list[str] | None,
           header: list[str] | None, date_format: str | None = None) -> None:
    """Read the source's rows and the columns picked as a table from them takes them; refused
    (naming the argument) where they are not rows to place.  ``date_format`` must be a
    pattern (:func:`ooxml_edit.tools.sources.date_pattern`) and one of the columns picked must
    hold a date."""
    rows = call.source(source, field="from")
    names = from_columns if from_columns is not None else list(rows.dataset.columns)
    if not names:
        raise ToolError("invalid_arguments", "from_columns names no column; leave it out for "
                        "every column", field="from_columns")
    at = [rows.column(name, field=f"from_columns[{i}]") for i, name in enumerate(names)]
    if len(set(at)) != len(at):
        raise ToolError("invalid_arguments", "a column is named twice", field="from_columns")
    if header and len(header) != len(at):
        raise ToolError("invalid_arguments", f"{len(header)} header texts for {len(at)} columns; "
                        "[] for no header row", field="header")
    if date_format is not None:
        sources.date_pattern(date_format, call.locale)
        count = len(rows.dataset.rows)
        if not any(rows.date(row, column) is not None for column in at for row in range(count)):
            raise sources.no_dates(rows, at)


def source_hint(document: Any, value: str) -> str | None:
    """The docx format's ``source_hint``: a ``from`` given a drawing's id (``d:3``, the
    form Word documents name drawings by) is told it is a drawing, not a dataset."""
    if not isinstance(value, str) or not _DRAWING.match(value):
        return None
    if not hooks.has_drawing(document, value):
        return (f"{value} is a drawing's id (none here), not a dataset: a source is a dataset "
                "d:<id>, its id starting with a letter, or a table t:<name>")
    return (f"{value} is a drawing in this document, not a dataset: a source is a dataset "
            "d:<id>, its id starting with a letter, or a table t:<name>")


#: Round 56 (R120): the kinds of a Word document's blocks, as a refusal names them.
_KINDS = {"table": "a table", "paragraph": "a paragraph", "content control": "a content control"}


def own_source(document: Any, address: str) -> str | None:
    """The docx format's ``own_source`` (round 56, R120): what a source value names in a Word
    document, which gives no rows -- ``"a table"`` for one of its tables (``t:60D6847F``),
    ``"a paragraph"``, ``"a content control"``; ``"part of a table"`` for a table's id with
    more after it (``t:60D6847F/#2``).  ``None`` when it names none of its blocks."""
    if not isinstance(address, str) or not address.startswith(("t:", "p:", "p@", "cc:", "cc@")):
        return None
    base, slash, _rest = address.partition("/")
    for identifier in ((address, base) if slash else (address,)):
        what = _KINDS.get(hooks.block_kind(document, identifier) or "")
        if what is not None:
            return what if identifier == address else f"part of {what}"
    return None
