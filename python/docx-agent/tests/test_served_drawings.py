"""Lane P2c14: ``word_drawings`` served by the Rust session, an inserted picture's bytes
asked of the host's blobs (``Session.edit(blobs=...)``); its replies as docx-agent's own
handler gives them (a session with its own clock is answered by the Python handler).
Made-up documents and pictures only."""

from __future__ import annotations

import datetime as dt
import io

import pytest

from docx_agent import Document
from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry

CLOCK = lambda: dt.datetime(2026, 11, 20, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731


def png(width: int = 40, height: int = 20) -> bytes:
    from PIL import Image

    out = io.BytesIO()
    Image.new("RGB", (width, height), (30, 140, 60)).save(out, format="PNG")
    return out.getvalue()


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

    def run(arguments: dict, *, counted: bool = True) -> object:
        before = dict(edits.served())
        result = toolbox.dispatch(session, "word_drawings", {"doc": doc, **arguments})
        if served and counted:
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
    # a blob that is not there: refused (the Python handler words the refusal)
    missing = run({"action": "insert_picture", "image": "b9", "at": ids["Lantern meadow by the harbour."],
                   "alt_text": "None"}, counted=False)
    assert not missing.ok and missing.error.code == "not_found"
    return out


def test_pictures_served_from_the_hosts_blobs(toolbox, report):
    assert _calls(toolbox, report, True) == _calls(toolbox, report, False)
