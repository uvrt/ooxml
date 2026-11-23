"""A saved deck answers as a native PowerPoint client would on a co-authoring host (the
design page "CyberAI as an Office Client", requests 1, 2, 3 and 6): every fixture saved
without an edit is its input's bytes (directory entries aside: not parts); an untouched entry
is copied byte for byte, a data descriptor too, and a save adds no layout problem MS-FSSHTTPD
2.4.1's chunking meets (data descriptors, needless ZIP64) and keeps the input's entry order;
one edit changes only the parts it names in ``Result.changed_parts``, plus what a save
derives from them."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

from ooxml_edit.tools import Toolbox
from pptx_agent import Document
from pptx_agent.tools import FORMAT, GROUPS, TOOLS

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "ooxml-common" / "tests" / "support"))
from zipchunks import (changed_chunks, changed_entries, entry_records,  # noqa: E402
                       has_directories, input_order, new_problems, with_data_descriptors,
                       zip_chunks)

FIXTURES = sorted((Path(__file__).parent / "fixtures").glob("*.pptx"))


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: p.name)
def test_a_save_chunks_as_office_chunks(path):
    data = path.read_bytes()
    deck = Document.open(data)
    saved = deck.to_bytes()
    assert new_problems(data, saved) == []
    assert input_order(saved) == input_order(data)
    if not has_directories(data):
        assert saved == data
    deck.slides[0].add_textbox(100000, 100000, 2000000, 500000, "Added")
    edited = deck.to_bytes()
    assert new_problems(data, edited) == []
    assert input_order(edited)[:len(input_order(data))] == input_order(data)


def test_an_entry_with_a_data_descriptor_is_copied_as_it_is():
    """A made-up deck whose every entry has a data descriptor (as some writers make one):
    saved without an edit it is its bytes; after one edit every entry the edit did not
    change keeps its record, descriptor and all."""
    data = with_data_descriptors((Path(__file__).parent / "fixtures" / "sample.pptx").read_bytes())
    assert Document.open(data).to_bytes() == data
    deck = Document.open(data)
    deck.slides[0].add_textbox(100000, 100000, 2000000, 500000, "Added")
    before, after = entry_records(data), entry_records(deck.to_bytes())
    changed = {name for name in before if after.get(name) != before[name]}
    assert changed == {"ppt/slides/slide1.xml", "docProps/app.xml"}


def test_one_shape_changes_its_slide_and_what_a_save_derives():
    data = (Path(__file__).parent / "fixtures" / "sample.pptx").read_bytes()
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        session.open(data, "sample.pptx")
        made = box.dispatch(session, "ppt_add_shape", {"doc": "d1", "slide": "s:256", "items": [
            {"preset": "rect", "box": {"x": 20, "y": 20, "w": 150, "h": 60}, "text": "A"}]})
        assert made.ok
        assert made.changed_parts == [{"doc": "d1", "part": "ppt/slides/slide1.xml"}]
        assert "changed_parts" not in made.to_json()
        read = box.dispatch(session, "ppt_read_slides", {"doc": "d1"})
        assert read.changed_parts == []
        saved = box.dispatch(session, "save_document", {"doc": "d1", "name": "out.pptx",
                                                        "format": "pptx"})
        assert saved.ok and saved.changed_parts == []
        edited = session.outputs[-1].data
    # the slide, docProps/app.xml (its word and paragraph counts, as PowerPoint keeps them)
    # and the central directory; every other entry keeps its chunk
    assert changed_chunks(data, edited) == (3, len(zip_chunks(data)))
    assert changed_entries(data, edited) == {"ppt/slides/slide1.xml", "docProps/app.xml"}
