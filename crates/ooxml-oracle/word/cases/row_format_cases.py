"""Round 58's row-format probe document (``probes/row_format_probe.py``; Word's
observations in ``observations/row-format-word.json``): a two-row table per case of cell and
run formatting, each followed by ``After <Key>.``.

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

from probe_package import package as _package

TABLES = ["Below", "Above", "Blank", "Marked", "Mixed", "Empty", "EmptyMarked", "Tracked"]


RUN = ('<w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:cs="Arial"/>{bold}<w:color w:val="1F4E79"/>'
       '<w:sz w:val="16"/><w:szCs w:val="16"/>')


MARK_OTHER = ('<w:rFonts w:ascii="Courier New" w:hAnsi="Courier New" w:cs="Courier New"/>'
              '<w:color w:val="C00000"/><w:sz w:val="28"/><w:szCs w:val="28"/>')


_paras = iter(range(1, 100000))


def _pid() -> str:
    return f"{0x50000000 + next(_paras):08X}"


def _cell(key: str, i: int, j: int) -> str:
    bold = "<w:b/><w:bCs/>" if j == 2 else ""
    run_props = RUN.format(bold=bold)
    mark = ""
    if key == "Marked":
        mark = MARK_OTHER
    text = [f"{key}", f"text r{i}", f"{10 + i}"][j]
    runs = f'<w:r><w:rPr>{run_props}</w:rPr><w:t xml:space="preserve">{text}</w:t></w:r>'
    if key == "Mixed" and j == 0:
        runs = ('<w:r><w:rPr><w:b/><w:color w:val="C00000"/></w:rPr><w:t xml:space="preserve">Lead</w:t></w:r>'
                f'<w:r><w:t xml:space="preserve"> {text}</w:t></w:r>')
    if key in ("Empty", "EmptyMarked") and i == 2 and j == 1:
        runs = ""
        mark = RUN.format(bold="") if key == "EmptyMarked" else ""
    jc = '<w:jc w:val="center"/>' if j == 2 else ""
    mark = f"<w:rPr>{mark}</w:rPr>" if mark else ""
    return (f'<w:tc><w:tcPr><w:tcW w:w="2400" w:type="dxa"/><w:shd w:val="clear" w:color="auto" '
            f'w:fill="DEEAF6"/><w:vAlign w:val="center"/></w:tcPr>'
            f'<w:p w14:paraId="{_pid()}" w14:textId="77777777"><w:pPr><w:spacing w:after="0"/>{jc}{mark}</w:pPr>'
            f'{runs}</w:p></w:tc>')


def _table(key: str) -> str:
    rows = "".join(f'<w:tr w14:paraId="{_pid()}" w14:textId="77777777">'
                   + "".join(_cell(key, i, j) for j in range(3)) + "</w:tr>" for i in (1, 2))
    grid = "<w:tblGrid>" + '<w:gridCol w:w="2400"/>' * 3 + "</w:tblGrid>"
    props = '<w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
    return f"<w:tbl>{props}{grid}{rows}</w:tbl>"


def _paragraph(text: str) -> str:
    return f'<w:p w14:paraId="{_pid()}" w14:textId="77777777"><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>'


def document() -> bytes:
    out = [_paragraph("Tables follow.")]
    for key in TABLES:
        out.append(_table(key))
        out.append(_paragraph(f"After {key}."))
    return _package("".join(out))
