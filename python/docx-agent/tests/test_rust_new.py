"""A new blank document made by the Rust core (``ooxml_core.new_document``, lane LC1)
against today's (``docx_agent.edit.blank.build``): the same entries in the same order, each
part the same bytes, the same entry headers -- for every page size, portrait and
landscape, a language and separators of another locale, a title to escape and no author."""
from __future__ import annotations

import io
import zipfile
from datetime import datetime, timezone

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "new_document"):
    pytest.skip("ooxml_core has no new_document", allow_module_level=True)

from docx_agent.edit import blank
from docx_agent.edit.properties import w3cdtf

CREATED = datetime(2026, 11, 21, 9, 30, 15, tzinfo=timezone.utc)


def _entries(data: bytes) -> list[tuple]:
    archive = zipfile.ZipFile(io.BytesIO(data))
    return [(i.filename, i.date_time, i.compress_type, i.create_system, i.external_attr, archive.read(i.filename))
            for i in archive.infolist()]


CASES = [
    {"page": page, "landscape": landscape, "language": "en-US", "separators": (".", ","),
     "title": None, "author": None}
    for page in blank.PAGE_SIZES for landscape in (False, True)
] + [
    {"page": "A4", "landscape": False, "language": "nl-NL", "separators": (",", ";"),
     "title": 'R&D <plan> "Q4"', "author": "Writer A"},
    {"page": "Letter", "landscape": False, "language": "fr-FR", "separators": (",", ";"),
     "title": "", "author": "Écrivain"},
]


@pytest.mark.parametrize("case", CASES, ids=lambda c: f"{c['page']}-{c['landscape']}-{c['language']}")
def test_the_rust_blank_document_is_todays(case):
    today = blank.build(page=case["page"], orientation="landscape" if case["landscape"] else "portrait",
                        language=case["language"], separators=case["separators"], title=case["title"],
                        author=case["author"], created=CREATED)
    mine = ooxml_core.new_document(page=(case["page"], case["landscape"]), language=case["language"],
                                   separators=case["separators"],
                                   properties=(case["title"], case["author"], w3cdtf(CREATED)))
    assert _entries(mine) == _entries(today)


def test_an_unknown_page_size_is_refused():
    with pytest.raises(ValueError, match="page size"):
        ooxml_core.new_document(page=("B5", False), language="en-US", separators=(".", ","),
                                properties=(None, None, w3cdtf(CREATED)))


def test_the_generated_data_is_current():
    import runpy
    from pathlib import Path

    script = Path(__file__).resolve().parents[3] / "crates" / "ooxml-tools-word" / "data" / "word_new.py"
    generator = runpy.run_path(str(script))
    assert generator["main"](["--check"]) == 0, f"run {script} to write it again"
