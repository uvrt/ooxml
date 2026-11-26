"""The replace_text probe document (``probes/replace_text_probe.py``; Word's observations in
``observations/replace-text-word.json``): paragraphs named by their first word, a one-row table,
and the three replaces made in a range -- a span of paragraphs, a table cell and one paragraph by
a regular expression -- each as the tools take it and as Word's Find and Replace makes it.

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

from probe_package import package as _package

#: The tracked edits' author, on the tools' side (Word's is its user, normalised away).
AUTHOR = "Probe Writer"

#: The body's paragraphs before and after the table, by key (the first word).
BEFORE = ["Head 2026 stays outside.", "Alpha the year 2026 begins.", "Bravo 2026 and 2026 again."]
AFTER = ["Charlie dates 2026-11 and 2025-03 here.", "Tail 2026 stays too."]
#: The table's one row: two cells.
CELLS = ["Cell 2026 left", "Other 2026 right"]

#: Each case: the tools' ``replace_text`` arguments (``{span}``, ``{cell}``, ``{Charlie}`` the
#: range), and Word's Find and Replace of the same (``range`` an AppleScript text range of ``d``,
#: ``find``/``replace`` in Word's syntax, ``wildcards`` for a pattern).
CASES = {
    "span": {
        "tools": {"find": "2026", "replace": "2027", "expect": "all", "range": "{span}"},
        "word": {"range": "span", "find": "2026", "replace": "2027", "wildcards": False},
    },
    "cell": {
        "tools": {"find": "2026", "replace": "2028", "expect": "all", "range": "{cell}"},
        "word": {"range": "cell", "find": "2026", "replace": "2028", "wildcards": False},
    },
    "regex": {
        "tools": {"find": r"(\d{4})-(\d{2})", "replace": r"\2/\1", "expect": "all", "regex": True,
                  "range": "{Charlie}"},
        "word": {"range": "Charlie", "find": "([0-9]{4})-([0-9]{2})", "replace": r"\2/\1", "wildcards": True},
    },
}

#: The text each paragraph and cell holds once every case is replaced (Accept All), and before
#: (Reject All), by key.
ACCEPTED = {
    "Head": "Head 2026 stays outside.", "Alpha": "Alpha the year 2027 begins.",
    "Bravo": "Bravo 2027 and 2027 again.", "Cell": "Cell 2026 left", "Other": "Other 2028 right",
    "Charlie": "Charlie dates 11/2026 and 03/2025 here.", "Tail": "Tail 2026 stays too.",
}
REJECTED = {key: text for key, text in
            zip(["Head", "Alpha", "Bravo", "Cell", "Other", "Charlie", "Tail"], BEFORE + CELLS + AFTER)}


def _paragraph(k: int, text: str) -> str:
    return (f'<w:p w14:paraId="{0x50000000 + k:08X}" w14:textId="77777777">'
            f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>')


def document() -> bytes:
    """The probe document: :data:`BEFORE`, the table of :data:`CELLS`, :data:`AFTER`."""
    k = iter(range(1, 100))
    cells = "".join(f'<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr>{_paragraph(next(k), text)}</w:tc>'
                    for text in CELLS)
    table = ('<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
             '<w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>'
             f'<w:tr w14:paraId="{0x50000000 + next(k):08X}" w14:textId="77777777">{cells}</w:tr></w:tbl>')
    body = "".join(_paragraph(next(k), text) for text in BEFORE) + table
    body += "".join(_paragraph(next(k), text) for text in AFTER)
    return _package(body)
