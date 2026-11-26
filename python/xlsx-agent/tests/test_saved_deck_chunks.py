"""pptx-agent's saves through the shared chunk writer (``ooxml_core.chunk_file``; the design
page "CyberAI as an Office Client", request 4).  Here, not in pptx-agent's suite: xlsx-agent's
test environment is the one that installs both pptx-agent and ooxml-core (tools/ci.py), and
pptx-agent itself does not depend on the Rust core.

Every deck fixture saved without an edit chunks by ZIP analysis, chunk for chunk as the tests'
own check (``zipchunks``) does -- but the one whose writer put a data descriptor on its first
entry, which the analysis leaves to simple chunking and the save keeps as it is; one shape
sends only the changed parts' chunks and the central directory."""

from __future__ import annotations

import sys
from pathlib import Path

import ooxml_core
import pytest

from ooxml_edit.tools import Toolbox
from pptx_agent import Document
from pptx_agent.tools import FORMAT, GROUPS, TOOLS

PPTX = Path(__file__).resolve().parents[2] / "pptx-agent" / "tests" / "fixtures"
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "ooxml-common" / "tests" / "support"))
from zipchunks import changed_entries, zip_chunks  # noqa: E402

DECKS = sorted(PPTX.glob("*.pptx"))
#: A deck whose first entry has a data descriptor and no sizes: not analysed as ZIP.
NOT_ANALYSED = {"real-basic-theme.pptx"}


def _chunks(chunks) -> list[tuple[int, int, bytes | None]]:
    return [(c.offset, c.size, None if c.random else c.signature) for c in chunks.chunks]


@pytest.mark.parametrize("path", DECKS, ids=lambda p: p.name)
def test_a_saved_deck_chunks_as_the_check_does(path):
    saved = Document.open(path.read_bytes()).to_bytes()
    chunks = ooxml_core.chunk_file(saved)
    assert chunks.zip_analysed is (path.name not in NOT_ANALYSED)
    assert _chunks(chunks) == [(c.start, c.size, c.signature) for c in zip_chunks(saved)]
    assert chunks.root == (b"", len(saved))


def test_one_shape_sends_only_its_parts():
    data = (PPTX / "sample.pptx").read_bytes()
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        session.open(data, "sample.pptx")
        assert box.dispatch(session, "ppt_add_shape", {"doc": "d1", "slide": "s:256", "items": [
            {"preset": "rect", "box": {"x": 20, "y": 20, "w": 150, "h": 60}, "text": "A"}]}).ok
        assert box.dispatch(session, "save_document", {"doc": "d1", "name": "out.pptx",
                                                       "format": "pptx"}).ok
        edited = session.outputs[-1].data
    chunks = ooxml_core.chunk_file(edited)
    delta = chunks.delta(ooxml_core.chunk_file(data).leaves())
    travel = {chunks.chunks[i].name or chunks.chunks[i].kind for i in delta.new}
    # the slide, docProps/app.xml (PowerPoint keeps its counts) and the central directory
    assert travel == {"ppt/slides/slide1.xml", "docProps/app.xml", "final"}
    assert travel == changed_entries(data, edited) | {"final"}
    assert len(delta.new) == 3 and len(delta.held) == len(chunks) - 3
