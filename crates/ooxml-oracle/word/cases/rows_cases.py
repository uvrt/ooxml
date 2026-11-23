"""The rows probe document (``probes/rows_probe.py``; Word's observations in
``observations/rows-word.json``): a table per case whose rows (or every row) were inserted
by this writer, the other writer (:data:`OTHER`) or nobody, each followed by ``After <Key>.``.

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

from probe_package import package as _package

#: The other writer of the probe's tracked records (``probes/tracking_probe.py``).
OTHER = "Other Writer"
DATE = 'w:date="2026-11-01T10:00:00Z"'


#: Case key -> what row 3 (or every row) of its table holds: (row author, per-cell text
#: author, per-cell mark author); ``None`` is no revision.
TABLES = ["Live", "LiveEmpty", "LiveClear", "Mine", "Other", "MineOtherText", "MineOtherMid", "OtherMineText",
          "MinePlain",
          "Two", "Typed", "TableMine", "TableMixed", "TableLive"]


_ids = iter(range(1000, 100000))


_paras = iter(range(1, 100000))


def _pid() -> str:
    return f"{0x40000000 + next(_paras):08X}"


def _record(tag: str, author: str | None) -> str:
    return f'<w:{tag} w:id="{next(_ids)}" w:author="{author}" {DATE}/>' if author else ""


def _cell(text: str, text_author: str | None, mark_author: str | None) -> str:
    run = f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>'
    if text_author:
        run = f'<w:ins w:id="{next(_ids)}" w:author="{text_author}" {DATE}>{run}</w:ins>'
    mark = _record("ins", mark_author)
    props = f"<w:pPr><w:rPr>{mark}</w:rPr></w:pPr>" if mark else ""
    return (f'<w:tc><w:tcPr><w:tcW w:w="2400" w:type="dxa"/></w:tcPr>'
            f'<w:p w14:paraId="{_pid()}" w14:textId="77777777">{props}{run}</w:p></w:tc>')


def _row(key: str, i: int, row_author=None, text_authors=(None, None), mark_authors=(None, None)) -> str:
    record = _record("ins", row_author)
    props = f"<w:trPr>{record}</w:trPr>" if record else ""
    cells = "".join(_cell(f"{key} r{i}c{j + 1}", text_authors[j], mark_authors[j]) for j in range(2))
    return f'<w:tr w14:paraId="{_pid()}" w14:textId="77777777">{props}{cells}</w:tr>'


def _table(rows: list[str]) -> str:
    grid = '<w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid>'
    props = '<w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
    return f"<w:tbl>{props}{grid}{''.join(rows)}</w:tbl>"


def _paragraph(text: str = "") -> str:
    run = f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>' if text else ""
    return f'<w:p w14:paraId="{_pid()}" w14:textId="77777777">{run}</w:p>'


def document(me: str) -> bytes:
    def inserted(key: str, i: int, row: str, texts: tuple, marks: tuple | None = None) -> str:
        return _row(key, i, row, texts, texts if marks is None else marks)

    plain = lambda key, i: _row(key, i)  # noqa: E731
    out = [_paragraph("Tables follow.")]
    for key in TABLES:
        if key == "MineOtherMid":
            rows = [plain(key, 1), inserted(key, 2, me, (OTHER, me), (me, me)), plain(key, 3)]
        elif key in ("Mine", "Other", "MineOtherText", "OtherMineText", "MinePlain", "Two"):
            author = OTHER if key.startswith("Other") else me
            texts = {"MineOtherText": (OTHER, me), "OtherMineText": (me, me), "MinePlain": (None, None)}.get(
                key, (author, author))
            marks = {"MineOtherText": (me, me), "OtherMineText": (OTHER, OTHER)}.get(key, texts)
            rows = [plain(key, 1), plain(key, 2), inserted(key, 3, author, texts, marks)]
        elif key == "TableMine":
            rows = [inserted(key, i, me, (me, me)) for i in (1, 2, 3)]
        elif key == "TableMixed":
            rows = [inserted(key, 1, me, (me, me)), inserted(key, 2, OTHER, (OTHER, OTHER)),
                    inserted(key, 3, me, (me, me))]
        elif key == "TableLive":
            out.append(_paragraph("Here"))
            continue
        else:
            rows = [plain(key, i) for i in (1, 2, 3)]
        out.append(_table(rows))
        out.append(_paragraph(f"After {key}."))
    out.append(_paragraph("Tail end."))
    return _package("".join(out))
