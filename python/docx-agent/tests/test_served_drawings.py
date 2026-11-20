"""Lane P2c14: ``word_drawings`` served by the Rust session, an inserted picture's bytes
asked of the host's blobs (``CallOptions(blobs=...)``); its replies as docx-agent's own
handler gives them (a session with its own clock is answered by the Python handler).
Made-up documents and pictures only."""

from __future__ import annotations

import datetime as dt
import struct
import zlib

import pytest

from docx_agent import Document
from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry

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


@pytest.fixture(scope="module")
def report() -> bytes:
    doc = Document.new()
    doc.insert_markdown("# A made-up report\n\nLantern meadow by the harbour.\n\nRiver stone number one.\n")
    return doc.to_bytes()


def _calls(toolbox, data: bytes, served: bool) -> list:
    session = toolbox.session() if served else toolbox.session(clock=CLOCK)
    doc = session.open(data, "report.docx")
    if served and not isinstance(session.entry(doc), OwnedEntry):
        pytest.skip("no Rust session to own the document")
    image = session.add_blob(png(), "logo.png")
    ids = {p.text: p.id for p in session.entry(doc).document.paragraphs()}
    out = []

    def run(arguments: dict) -> object:
        before = dict(edits.served())
        result = toolbox.dispatch(session, "word_drawings", {"doc": doc, **arguments})
        if served:
            assert edits.served().get("word_drawings", 0) == before.get("word_drawings", 0) + 1, edits.fallbacks()
        out.append((result.ok, result.summary, result.data, sorted(result.changed), result.warnings))
        return result

    first = run({"action": "insert_picture", "image": image, "find": "meadow", "width": 30,
                 "alt_text": "A made-up logo"})
    assert first.ok, first.error
    run({"action": "insert_picture", "image": image, "at": f"after:{ids['River stone number one.']}",
         "align": "center", "alt_text": "The logo again"})
    run({"action": "list"})
    drawing = first.data["id"] if isinstance(first.data, dict) else None
    if drawing is not None:
        run({"action": "resize", "target": drawing, "width": 60})
        run({"action": "set", "target": drawing, "alt_text": "A made-up logo, larger"})
    # a blob that is not there: refused by the session, in docx-agent's words (lane P2e1)
    missing = run({"action": "insert_picture", "image": "b9", "at": ids["Lantern meadow by the harbour."],
                   "alt_text": "None"})
    assert not missing.ok and missing.error.code == "not_found"
    return out


def test_pictures_served_from_the_hosts_blobs(toolbox, report):
    assert _calls(toolbox, report, True) == _calls(toolbox, report, False)
