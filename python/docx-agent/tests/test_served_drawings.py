"""Lane P2c14: ``word_drawings`` on the Rust host, an inserted picture's bytes from the
session's blobs: inserted, listed, resized and described; a blob that is not there refused.  Made-up documents and pictures only."""

from __future__ import annotations

import datetime as dt
import struct
import zlib

import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox

import word_docs


CLOCK = lambda: dt.datetime(2026, 11, 20, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731


def png(width: int = 40, height: int = 20) -> bytes:
    """A made-up PNG, written by hand (no imaging library on every leg)."""
    row = b"\x00" + bytes((30, 140, 60)) * width

    def chunk(kind: bytes, body: bytes) -> bytes:
        return (struct.pack(">I", len(body)) + kind + body
                + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(row * height))
            + chunk(b"IEND", b""))


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def _made(markdown: str) -> bytes:
    """A blank document with ``markdown`` inserted by the toolbox, as bytes (every paragraph
    with its paraId)."""
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(word_docs.blank(), "made.docx")
        word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown=markdown)
        return word_docs.saved(session, doc)


def _ids(session, doc) -> dict[str, str]:
    """Each paragraph's id by its text (the first of a text), from the package."""
    out: dict[str, str] = {}
    for block, text in word_docs.paragraphs(session, doc):
        out.setdefault(text, block)
    return out


def _report() -> bytes:
    return _made("# A made-up report\n\nLantern meadow by the harbour.\n\nRiver stone number one.\n")


def test_pictures_from_the_sessions_blobs(toolbox):
    session = toolbox.session(clock=CLOCK)
    doc = session.open(_report(), "report.docx")
    image = session.add_blob(png(), "logo.png")
    ids = _ids(session, doc)

    def run(arguments: dict) -> object:
        return toolbox.dispatch(session, "word_drawings", {"doc": doc, **arguments})

    first = run({"action": "insert_picture", "image": image, "find": "meadow", "width": 30,
                 "alt_text": "A made-up logo"})
    assert first.ok, first.error
    drawing = first.data["id"]
    second = run({"action": "insert_picture", "image": image, "at": f"after:{ids['River stone number one.']}",
                  "align": "center", "alt_text": "The logo again"})
    assert second.ok, second.error
    listed = run({"action": "list"})
    assert listed.ok and len(listed.data) == 2, listed.data
    assert run({"action": "resize", "target": drawing, "width": 60}).ok
    assert run({"action": "set", "target": drawing, "alt_text": "A made-up logo, larger"}).ok
    xml = word_docs.part(word_docs.saved(session, doc))
    assert 'descr="A made-up logo, larger"' in xml and 'descr="The logo again"' in xml
    assert any(name.startswith("word/media/") for name in
               __import__("zipfile").ZipFile(__import__("io").BytesIO(word_docs.saved(session, doc))).namelist())
    # a blob that is not there: refused, in docx-agent's words (lane P2e1)
    missing = run({"action": "insert_picture", "image": "b9", "at": ids["Lantern meadow by the harbour."],
                   "alt_text": "None"})
    assert not missing.ok and missing.error.code == "not_found"
