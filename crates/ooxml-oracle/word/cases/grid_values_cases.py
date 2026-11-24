"""The table-grid probe documents (``probes/grid_values_probe.py``; Word's observations in
``observations/grid-values-word.json``): one document per case, each a table of three grid
columns (2000 twips, fixed layout) and two rows. The first row is the case's; the second is
three plain cells ``R1`` ``R2`` ``R3`` that fix where each grid column is drawn. Case cells
hold ``K1``, ``K2``... in document order.

Cases (:data:`CASES`): ``w:gridSpan`` values ``0``, ``-1``, ``+2``, `` 2``, ``1_0`` (and
``2``, the control) on the first cell; ``w:gridBefore`` / ``w:gridAfter`` of ``-1`` (and
``1``, the control); cells inside ``w:sdt``/``w:sdtContent`` and ``w:customXml`` (and both
nested); a ``w:vMerge`` continuation with no start above it (in the first row, and under a
cell not merged).

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

from probe_package import package as _package

WIDTH = 2000


def _tc(text: str, *, span: str | None = None, vmerge: str | None = None, width: int = WIDTH) -> str:
    props = f'<w:tcW w:w="{width}" w:type="dxa"/>'
    if span is not None:
        props += f'<w:gridSpan w:val="{span}"/>'
    if vmerge is not None:
        props += "<w:vMerge/>" if vmerge == "continue" else f'<w:vMerge w:val="{vmerge}"/>'
    return (f"<w:tc><w:tcPr>{props}</w:tcPr><w:p><w:pPr><w:spacing w:after=\"0\"/></w:pPr>"
            f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p></w:tc>')


def _sdt(inner: str) -> str:
    return f"<w:sdt><w:sdtPr><w:id w:val=\"7001\"/></w:sdtPr><w:sdtContent>{inner}</w:sdtContent></w:sdt>"


def _custom(inner: str) -> str:
    return f'<w:customXml w:uri="urn:probe" w:element="cell">{inner}</w:customXml>'


def _row(cells: str, *, before: str | None = None, after: str | None = None) -> str:
    props = ""
    if before is not None:
        props += f'<w:gridBefore w:val="{before}"/>'
    if after is not None:
        props += f'<w:gridAfter w:val="{after}"/>'
    head = f"<w:trPr>{props}</w:trPr>" if props else ""
    return f"<w:tr>{head}{cells}</w:tr>"


REFERENCE = _row(_tc("R1") + _tc("R2") + _tc("R3"))


def _span(value: str) -> str:
    return _row(_tc("K1", span=value, width=2 * WIDTH) + _tc("K2"))


#: Each case: its first row, and the second row (the reference unless the case changes it).
CASES: dict[str, tuple[str, str]] = {
    "span-2": (_span("2"), REFERENCE),
    "span-0": (_span("0"), REFERENCE),
    "span-minus-1": (_span("-1"), REFERENCE),
    "span-plus-2": (_span("+2"), REFERENCE),
    "span-space-2": (_span(" 2"), REFERENCE),
    "span-1_0": (_span("1_0"), REFERENCE),
    "before-1": (_row(_tc("K1") + _tc("K2"), before="1"), REFERENCE),
    "before-minus-1": (_row(_tc("K1") + _tc("K2"), before="-1"), REFERENCE),
    "after-1": (_row(_tc("K1") + _tc("K2"), after="1"), REFERENCE),
    "after-minus-1": (_row(_tc("K1") + _tc("K2"), after="-1"), REFERENCE),
    "sdt-cell": (_row(_tc("K1") + _sdt(_tc("K2")) + _tc("K3")), REFERENCE),
    "customxml-cell": (_row(_tc("K1") + _custom(_tc("K2")) + _tc("K3")), REFERENCE),
    "nested-wrappers": (_row(_tc("K1") + _custom(_sdt(_tc("K2"))) + _tc("K3")), REFERENCE),
    "vmerge-first-row": (_row(_tc("K1") + _tc("K2", vmerge="continue") + _tc("K3")), REFERENCE),
    "vmerge-under-plain": (_row(_tc("K1") + _tc("K2") + _tc("K3")),
                           _row(_tc("R1") + _tc("R2", vmerge="continue") + _tc("R3"))),
}


def table(key: str) -> str:
    """The case's table, as written into its document."""
    first, second = CASES[key]
    grid = "<w:tblGrid>" + f'<w:gridCol w:w="{WIDTH}"/>' * 3 + "</w:tblGrid>"
    borders = "".join(f'<w:{side} w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
                      for side in ("top", "left", "bottom", "right", "insideH", "insideV"))
    props = (f'<w:tblPr><w:tblW w:w="{3 * WIDTH}" w:type="dxa"/><w:tblBorders>{borders}</w:tblBorders>'
             '<w:tblLayout w:type="fixed"/></w:tblPr>')
    return f"<w:tbl>{props}{grid}{first}{second}</w:tbl>"


def document(key: str) -> bytes:
    body = (f"<w:p><w:r><w:t>Case {key}.</w:t></w:r></w:p>{table(key)}"
            "<w:p><w:r><w:t>After.</w:t></w:r></w:p>")
    return _package(body)
