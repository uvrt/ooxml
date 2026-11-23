"""Every block a tool's output holds carries its durable id.

Word drops every paraId of a document holding one paragraph or table row without a valid,
unique ``w14:paraId``, or one drawing without its ``wp14`` ids, when it saves it
(crates/ooxml-oracle/word/observations/paraid-durability.json, factors), and the block ids
(``p:``, ``t:``) rest on paraIds.  So whatever a writing tool does, the document it leaves has
none: every ``w:p`` and ``w:tr`` of every story with a paraId below 0x80000000 that no other
block has, and a textId beside it; every ``wp:inline``/``wp:anchor`` with its anchorId and
editId.  A property over the writing tools, each called on a few fixtures as opened (some of
whose rows, paragraphs or drawings have no ids), and the made-up document of the factors.
All through the toolbox, on the Rust session.
"""

from __future__ import annotations

import io
import json
import re
import struct
import zipfile
import zlib
from typing import Any

import pytest

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs
from conftest import FIXTURE_DIR

W14 = "http://schemas.microsoft.com/office/word/2010/wordml"
WP14 = "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing"
_STORY = re.compile(r"word/(document|header\d+|footer\d+|footnotes|endnotes|comments)\.xml$")
_PLACEHOLDER = re.compile(r"\$([ptb])(-?\d+)")

#: The fixtures, as opened: rows, header paragraphs and drawings without ids among them.
FIXTURES = ["generated/toc-numbered.docx", "generated/lists-and-styles.docx",
            "generated/review-follow.docx", "generated/table-keeps.docx"]

#: One call per writing tool and action, with ``$pN`` (the body's Nth paragraph, ``-1`` the
#: last), ``$t1`` (its first table) and ``$b1`` (a picture) named in the document it runs on.
CALLS: list[tuple[str, dict[str, Any]]] = [
    ("word_set_text", {"target": "$p2", "text": "Written whole."}),
    ("word_insert_text", {"target": "$p2", "position": "end", "text": " (more)"}),
    ("word_delete", {"blocks": "$p3"}),
    ("word_insert_markdown", {"at": "end", "markdown": "A paragraph.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n"}),
    ("word_format", {"targets": ["$p2"], "bold": True, "color": "C00000"}),
    ("word_lists", {"targets": ["$p2", "$p3"], "action": "add", "kind": "number"}),
    ("word_style", {"targets": ["$p2"], "style": "Heading 2"}),
    ("word_comments", {"action": "add", "author": "Writer A", "items": [{"target": "$p2", "text": "A note."}]}),
    ("word_notes", {"action": "insert", "kind": "footnote", "target": "$p2", "text": "A footnote."}),
    ("word_fields", {"action": "update"}),
    ("word_fields", {"action": "insert_toc", "after": "$p1", "levels": "1-2"}),
    ("word_fields", {"action": "insert_caption", "after": "$p-1", "label": "Table", "text": ": a caption"}),
    ("word_sections", {"action": "insert_break", "after": "$p2", "start": "nextPage"}),
    ("word_sections", {"action": "set", "section": "s:body", "orientation": "landscape"}),
    ("word_headers_footers", {"section": "s:body", "which": "header", "action": "set", "text": "A header"}),
    ("word_headers_footers", {"section": "s:body", "which": "footer", "action": "set", "text": "Report",
                              "page_number": "page_x_of_y"}),
    ("word_move", {"blocks": "$p2", "after": "$p-1"}),
    ("word_drawings", {"action": "insert_picture", "image": "$b1", "at": "after:$p2", "width": 72}),
    ("word_drawings", {"action": "insert_text_box", "target": "$p2", "text": "A box", "width": 144, "height": 48}),
    ("word_edit_table", {"table": "$t1", "action": "insert_row", "index": 0}),
    ("word_edit_table", {"table": "$t1", "action": "insert_column", "index": 0}),
    ("word_edit_table", {"table": "$t1", "action": "set_cells", "cells": [{"row": 0, "col": 0, "text": "Set"}]}),
    ("word_edit_table", {"table": "$t1", "action": "delete_row", "index": 0}),
    ("word_format_table", {"table": "$t1", "scope": "table", "style": "Table Grid", "header_rows": 1}),
    ("word_links", {"action": "add_bookmark", "target": "$p2", "name": "here"}),
]


def png() -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + b"\x28\x78\xc8" * 8 for _ in range(6))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 8, 6, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def _p(label: str, para: str | None = None) -> str:
    ids = f' w14:paraId="{para}" w14:textId="77777777"' if para else ""
    return f'<w:p{ids}><w:r><w:t>{label}</w:t></w:r></w:p>'


def factors() -> bytes:
    """The factors' document: every block stamped but a table row (the probe's ``row``)."""
    body = (_p("O1 a paragraph", "1A2B3C01") + _p("T1 another", "1A2B3C02")
            + '<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="2400"/>'
            + '</w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="2400" w:type="dxa"/></w:tcPr>'
            + _p("X1 in a row without a paraId", "1A2B3C05") + "</w:tc></w:tr></w:tbl>"
            + _p("S2 the last", "1A2B3C03"))
    return word_docs.with_body(body)


def problems(data: bytes) -> list[str]:
    """What in ``data``'s stories Word would drop every paraId for (empty when nothing)."""
    out: list[str] = []
    seen: dict[str, str] = {}
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        for name in package.namelist():
            if not _STORY.match(name):
                continue
            root = word_docs.tree(data, name)
            for element in root.iter():
                tag = element.tag
                if tag in (f"{{{word_docs.W}}}p", f"{{{word_docs.W}}}tr"):
                    kind = tag.rsplit("}", 1)[1]
                    para = element.get(f"{{{W14}}}paraId")
                    if para is None or element.get(f"{{{W14}}}textId") is None:
                        out.append(f"{name}: a w:{kind} without its ids")
                    elif not re.fullmatch(r"[0-9A-F]{8}", para) or int(para, 16) >= 0x80000000:
                        out.append(f"{name}: a w:{kind} with paraId {para}, not one Word keeps")
                    elif para in seen:
                        out.append(f"{name}: paraId {para} repeated (first in {seen[para]})")
                    else:
                        seen[para] = name
                elif tag.endswith("}inline") or tag.endswith("}anchor"):
                    if element.get(f"{{{WP14}}}anchorId") is None or element.get(f"{{{WP14}}}editId") is None:
                        out.append(f"{name}: a drawing without wp14:anchorId/editId")
    return out


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _resolve(value: Any, ids: dict[str, list[str]]) -> Any:
    if isinstance(value, str):
        def one(match: re.Match) -> str:
            n = int(match.group(2))
            return ids[match.group(1)][n - 1 if n > 0 else n]
        return _PLACEHOLDER.sub(one, value)
    if isinstance(value, list):
        return [_resolve(v, ids) for v in value]
    if isinstance(value, dict):
        return {k: _resolve(v, ids) for k, v in value.items()}
    return value


def written(box: Any, data: bytes, tool: str, arguments: dict[str, Any]) -> bytes | None:
    """``data`` after the call, ``None`` when the tools refused it or it named what the
    document lacks."""
    session = box.session()
    session.open(data, "probe.docx")
    blob = session.add_blob(png(), "blue.png")
    if _PLACEHOLDER.search(json.dumps(arguments)):
        listed = [b for b, _ in word_docs.blocks(box, session, "d1")]
        ids = {"p": [b for b in listed if b.startswith(("p:", "p@"))],
               "t": [b for b in listed if b.startswith("t")], "b": [blob]}
        try:
            arguments = _resolve(arguments, ids)
        except IndexError:
            return None
    result = box.dispatch(session, tool, {"doc": "d1", **arguments})
    if not result.ok:
        return None
    return session.entry("d1").snapshot()


@pytest.mark.parametrize("fixture", FIXTURES)
def test_every_writing_tool_leaves_every_block_stamped(box, fixture):
    data = (FIXTURE_DIR / fixture).read_bytes()
    ran, found = 0, {}
    for tool, arguments in CALLS:
        out = written(box, data, tool, arguments)
        if out is None or out == data:
            continue
        ran += 1
        wrong = problems(out)
        if wrong:
            found[f"{tool} {arguments.get('action', '')}".strip()] = wrong[:3]
    assert not found, found
    assert ran >= len(CALLS) // 2, f"only {ran} of {len(CALLS)} calls wrote on {fixture}"


def test_a_field_update_stamps_the_row_it_did_not_write():
    """The Word suite's e4/toc-update: the fixture's row has no paraId, the update writes
    the TOC's results only, and its output must still hold none without one."""
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        data = (FIXTURE_DIR / "generated/toc-numbered.docx").read_bytes()
        assert problems(data), "the fixture has blocks without ids: the case needs one"
        out = written(box, data, "word_fields", {"action": "update"})
        assert out is not None and out != data
        assert problems(out) == []
        made = written(box, factors(), "word_fields", {"action": "update"})
        # nothing to update: the factors' document is left as it is, its row unstamped
        assert made is None or made == factors() or problems(made) == []


def test_the_factors_row_is_stamped_by_any_write():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        data = factors()
        assert problems(data) == ["word/document.xml: a w:tr without its ids"]
        out = written(box, data, "word_insert_markdown", {"at": "end", "markdown": "More.\n"})
        assert out is not None and problems(out) == []
