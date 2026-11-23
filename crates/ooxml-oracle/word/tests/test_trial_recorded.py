"""A template saved as a document, held to what Word opens (``probes/trial_probe.py``,
``observations/trial-word.json``, Word 16.106 for Mac): Word refuses a package whose main part's
content type is not its extension's -- a template's, or a macro-enabled one's, in a ``.docx`` --
and opens the one whose type is; ``save_document`` writes the kind its format names."""

from __future__ import annotations

import io
import re
import warnings
import zipfile

import pytest
from recorded import Opened, observed  # noqa: I001  (puts the probes on the path)

import probe_paths  # noqa: E402

OBSERVED = observed("trial-word.json")
#: The observation's cases these tests compare with (index.py lists why the others are not).
READS = {"template-as-docx", "template-saved-docx", "template-saved-dotx", "macro-type-as-docx"}
TEMPLATES = probe_paths.FIXTURES / "generated" / "templates"


def main_type(data: bytes) -> str:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        types = archive.read("[Content_Types].xml").decode()
        names = archive.namelist()
    found = re.search(r'PartName="/word/document\.xml" ContentType="([^"]*)"', types)
    assert found, names
    return found.group(1)


def saved(box, template: str, name: str, fmt: str) -> bytes:
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")          # a template opened: the tools say so
        document = Opened(box, (TEMPLATES / template).read_bytes(), template, kind="docx")
    document.call("save_document", name=name, format=fmt)
    return document.session.take_outputs()[-1].data


@pytest.mark.parametrize("fmt, case", [("docx", "template-saved-docx"), ("dotx", "template-saved-dotx")])
def test_a_template_is_saved_as_the_kind_word_opens(box, fmt, case):
    word = OBSERVED[case]
    assert word["outcome"] == "done"
    assert main_type(saved(box, "brand.dotx", f"report.{fmt}", fmt)) == word["main_content_type"]


def test_a_document_is_never_written_with_a_type_word_refuses(box):
    """``template-as-docx``, ``macro-type-as-docx``: Word refuses a template's or a macro-enabled
    document's type in a ``.docx``; the tools' ``.docx`` of a template is a document's, and of a
    macro-enabled template is refused (its macros cannot go in a ``.docx``)."""
    refused = {OBSERVED[case]["main_content_type"] for case in ("template-as-docx", "macro-type-as-docx")}
    assert {OBSERVED[case]["outcome"] for case in ("template-as-docx", "macro-type-as-docx")} == {"rejected"}
    data = saved(box, "brand.dotx", "report.docx", "docx")
    assert main_type(data) not in refused
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        document = Opened(box, (TEMPLATES / "brand-macros.dotm").read_bytes(), "brand-macros.dotm", kind="docx")
    assert document.refused("save_document", name="report.docx", format="docx")["code"] == "refused"
