"""Round 59's merged-rows probe document (``probes/merged_rows_probe.py``; Word's
observations in ``observations/merged-rows-word.json``): a table per case, three columns by
five rows, its first column merged down over rows 2-4 (``End``'s over rows 3-5), each
followed by a paragraph ``After <Key>.``; cell text ``<Key> r<i>c<j>``.

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

from probe_package import package as _package


TABLES = ["BelowLast", "BelowLastMerged", "BelowFirst", "Inside", "InsideAbove", "AboveFirst",
          "AboveFirstMerged", "End", "TrackedInside", "TrackedBelowLast"]


#: Copy and paste of a whole row (a second document): ``(key, row copied, row the insertion
#: selected when pasting)``.
PASTES = [("PasteInside", 3, 4), ("PasteFirst", 2, 3), ("PasteAfterGroup", 4, 5), ("PasteAboveFirst", 2, 2),
          ("PasteEnd", 5, 5)]


_paras = iter(range(1, 100000))


def _pid() -> str:
    return f"{0x51000000 + next(_paras):08X}"


def _merged(key: str) -> tuple[int, int]:
    """The rows (1-based, first and last) the first column is merged down over."""
    return (3, 5) if key == "End" else (2, 4)


def _cell(key: str, i: int, j: int) -> str:
    first, last = _merged(key)
    merge = ""
    text = f"{key} r{i}c{j}"
    if j == 1 and first <= i <= last:
        merge = '<w:vMerge w:val="restart"/>' if i == first else "<w:vMerge/>"
        if i != first:
            text = ""
    runs = f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>' if text else ""
    return (f'<w:tc><w:tcPr><w:tcW w:w="2400" w:type="dxa"/>{merge}</w:tcPr>'
            f'<w:p w14:paraId="{_pid()}" w14:textId="77777777"><w:pPr><w:spacing w:after="0"/></w:pPr>'
            f'{runs}</w:p></w:tc>')


def _table(key: str) -> str:
    rows = "".join(f'<w:tr w14:paraId="{_pid()}" w14:textId="77777777">'
                   + "".join(_cell(key, i, j) for j in (1, 2, 3)) + "</w:tr>" for i in range(1, 6))
    grid = "<w:tblGrid>" + '<w:gridCol w:w="2400"/>' * 3 + "</w:tblGrid>"
    borders = "".join(f'<w:{side} w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
                      for side in ("top", "left", "bottom", "right", "insideH", "insideV"))
    props = f'<w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblBorders>{borders}</w:tblBorders></w:tblPr>'
    return f"<w:tbl>{props}{grid}{rows}</w:tbl>"


def _paragraph(text: str) -> str:
    return f'<w:p w14:paraId="{_pid()}" w14:textId="77777777"><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>'


def document(keys: list[str] = TABLES) -> bytes:
    out = [_paragraph("Tables follow.")]
    for key in keys:
        out.append(_table(key))
        out.append(_paragraph(f"After {key}."))
    return _package("".join(out))
