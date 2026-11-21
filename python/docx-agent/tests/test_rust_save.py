"""``save_document`` served by the Rust session (lane LC1) against today's: the file the
session writes is today's as Word reads it -- the same parts, each XML part the same
canonical XML (c14n), every other part the same bytes -- for a document as opened, after an
edit (``app.xml``'s statistics refreshed), as a template, cleaned, and as Markdown; the
reply's summary and data are today's but for the file's size: a part written again is
deflated by the Rust zip writer (zlib-rs), whose stream may be a few bytes from zlib's, and
Word reads both alike."""
from __future__ import annotations

import io
import zipfile
from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Session"):          # an ooxml-core before 0.9.0
    pytest.skip("ooxml_core has no Session", allow_module_level=True)

from lxml import etree
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from test_cleaning import bloated

HERE = Path(__file__).resolve().parent
#: A few fixtures, and a template saved as a .docx (its main part retyped on a .dotx save).
FIXTURES = sorted({*sorted((HERE / "fixtures").rglob("*.docx"))[:6],
                   *(HERE / "fixtures").rglob("template-saved-as-docx.docx")})


def _canonical(data: bytes) -> dict[str, bytes]:
    archive = zipfile.ZipFile(io.BytesIO(data))
    out = {}
    for name in archive.namelist():
        body = archive.read(name)
        if name.endswith((".xml", ".rels")):
            body = etree.tostring(etree.fromstring(body), method="c14n")
        out[name] = body
    return out


def _saves(data: bytes, rust: bool, calls: list[tuple[str, dict]]) -> list[tuple[object, bytes | None]]:
    from docx_agent.rust import reads

    before = reads.use_session(reads._session() if rust else None)
    try:
        with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
            session = box.session()
            d = session.open(data, "old.docx")
            out = []
            for tool, arguments in calls:
                result = box.dispatch(session, tool, {"doc": d, **arguments})
                saved = session.take_outputs()[-1].data if tool == "save_document" and result.ok else None
                out.append((result, saved))
            return out
    finally:
        reads.use_session(before)


CALLS = [
    ("save_document", {"name": "same.docx", "format": "docx"}),
    ("save_document", {"name": "same.dotx", "format": "dotx"}),
    ("save_document", {"name": "same.md", "format": "markdown"}),
    ("word_insert_markdown", {"at": "end", "markdown": "One more paragraph of words."}),
    ("save_document", {"name": "edited.docx", "format": "docx"}),
    ("save_document", {"name": "edited.docx", "format": "docx", "clean": True}),
]


def _compare(data: bytes) -> None:
    from docx_agent.rust import reads

    served = reads.served().get("save_document", 0)
    rust = _saves(data, True, CALLS)
    python = _saves(data, False, CALLS)
    for (tool, arguments), (mine, mine_bytes), (today, today_bytes) in zip(CALLS, rust, python):
        assert mine.ok == today.ok, (tool, arguments)
        if tool != "save_document" or not today.ok:
            continue
        unsized = lambda data: {k: v for k, v in data.items() if k not in ("size", "cleaned")}
        assert unsized(mine.data) == unsized(today.data), arguments
        assert mine.data["size"] == len(mine_bytes)
        if "cleaned" in today.data:
            # the files' sizes are the zip writer's; what was taken out, uncompressed, is the same
            strip = lambda cleaned: {k: v for k, v in cleaned.items() if k != "bytes"}
            assert strip(mine.data["cleaned"]) == strip(today.data["cleaned"]), arguments
            assert mine.data["cleaned"]["bytes"]["after"] == len(mine_bytes)
        else:
            assert mine.summary == today.summary, arguments
        if arguments["format"] == "markdown":
            assert mine_bytes == today_bytes
        else:
            assert _canonical(mine_bytes) == _canonical(today_bytes), arguments
    saves = sum(tool == "save_document" for tool, _ in CALLS)
    assert reads.served().get("save_document", 0) - served == saves, reads.fallbacks().get("save_document")


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: p.name)
def test_the_rust_save_is_todays_as_word_reads_it(path):
    _compare(path.read_bytes())


def test_a_cleaned_save_takes_out_what_todays_does():
    _compare(bloated())


def test_a_served_save_still_refuses_a_new_validation_problem(monkeypatch):
    """The save gate is the host's on the served path too: the session writes the file, and
    ``call.output`` refuses it when the version has a problem the document did not open
    with -- compared by the session, which holds the version saved -- and hands the
    application nothing."""
    from docx_agent.rust import owner, reads

    asked: list = []

    def problem(entry):
        asked.append(entry.version)
        return {"new": ["word/document.xml: bad"], "fixed": [], "baseline": 0}

    monkeypatch.setattr(owner, "session_validation", problem)
    before = reads.use_session(reads._session())
    fell_back = dict(reads.fallbacks())
    try:
        with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
            session = box.session()
            d = session.open(FIXTURES[0].read_bytes(), "old.docx")
            result = box.dispatch(session, "save_document", {"doc": d, "name": "a.docx", "format": "docx"})
            assert not result.ok and result.error.code == "refused", result
            assert result.error.valid_options == ["word/document.xml: bad"]
            assert session.take_outputs() == []
    finally:
        reads.use_session(before)
    assert asked, "the gate did not compare the version"
    assert reads.fallbacks() == fell_back, "the save was not asked of the session"
