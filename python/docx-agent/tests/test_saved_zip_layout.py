"""A saved document answers as a native Word client would on a co-authoring host (the
design page "CyberAI as an Office Client", requests 1, 2, 3 and 6): every fixture saved
without an edit is its bytes; an untouched entry is copied byte for byte, a data descriptor
too, and a save adds no layout problem MS-FSSHTTPD 2.4.1's chunking meets (data
descriptors, needless ZIP64) and keeps the input's entry order; one paragraph changes only
the parts its ``Result.changed_parts`` names, and ``docProps/app.xml``
where its measured counts changed (Word writes them on save too) -- never where they did
not."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

from conftest import FIXTURE_DIR
from ooxml_edit.tools import Toolbox
from docx_agent.tools import FORMAT, GROUPS, TOOLS

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "ooxml-common" / "tests" / "support"))
from zipchunks import (changed_entries, entry_records, input_order,  # noqa: E402
                       new_problems, with_data_descriptors)

APP = "docProps/app.xml"


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _save(box, session, name: str) -> bytes:
    saved = box.dispatch(session, "save_document", {"doc": "d1", "name": name, "format": "docx"})
    assert saved.ok, saved.to_json()
    assert saved.changed_parts == []
    return session.outputs[-1].data


def test_a_save_chunks_as_office_chunks(box, reading_path):
    data = reading_path.read_bytes()
    session = box.session()
    session.open(data, reading_path.name)
    saved = _save(box, session, "same.docx")
    assert new_problems(data, saved) == []
    assert saved == data         # directory entries too: the Rust package copies them


@pytest.mark.parametrize("name", ["wordto/sample-1page", "generated/lists-and-styles"])
def test_one_paragraph_changes_the_parts_it_names(box, name):
    data = (FIXTURE_DIR / f"{name}.docx").read_bytes()
    session = box.session()
    session.open(data, "d.docx")
    made = box.dispatch(session, "word_insert_markdown", {"doc": "d1", "at": "end",
                                                          "markdown": "A new paragraph."})
    assert made.ok, made.to_json()
    assert made.changed_parts == [{"doc": "d1", "part": "word/document.xml"}]
    assert "changed_parts" not in made.to_json()
    edited = _save(box, session, "edited.docx")
    assert new_problems(data, edited) == []
    assert input_order(edited)[:len(input_order(data))] == input_order(data)
    assert changed_entries(data, edited) <= {"word/document.xml", APP}


def test_app_xml_stays_while_its_counts_do(box):
    data = (FIXTURE_DIR / "wordto" / "sample-1page.docx").read_bytes()
    session = box.session()
    session.open(data, "d.docx")
    assert box.dispatch(session, "replace_text", {"doc": "d1", "find": "the", "replace": "teh"}).ok
    first = _save(box, session, "one.docx")
    # the same words, lines and characters: the counts Word would write are the same
    again = box.dispatch(session, "replace_text", {"doc": "d1", "find": "teh", "replace": "thx"})
    assert again.ok and again.changed_parts == [{"doc": "d1", "part": "word/document.xml"}]
    second = _save(box, session, "two.docx")
    assert changed_entries(first, second) == {"word/document.xml"}


def test_an_entry_with_a_data_descriptor_is_copied_as_it_is(box):
    """A made-up document whose every entry has a data descriptor: saved without an edit it
    is its bytes; after one paragraph every entry the edit did not change keeps its record,
    descriptor and all."""
    data = with_data_descriptors((FIXTURE_DIR / "wordto" / "sample-1page.docx").read_bytes())
    session = box.session()
    session.open(data, "d.docx")
    assert _save(box, session, "same.docx") == data
    made = box.dispatch(session, "word_insert_markdown", {"doc": "d1", "at": "end",
                                                          "markdown": "A new paragraph."})
    assert made.ok, made.to_json()
    before, after = entry_records(data), entry_records(_save(box, session, "edited.docx"))
    changed = {name for name in before if after.get(name) != before[name]}
    assert changed <= {"word/document.xml", APP} and "word/document.xml" in changed
