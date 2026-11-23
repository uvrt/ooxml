"""What the recorded-Word tests share: Word's observations, a probe document opened in a tool
session, the tools' calls on it and the parts they wrote, normalised as the probes normalised
Word's.

Must not: import pytest or docx-agent's library (only its tool host, ``docx_agent.tools``).
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
WORD = HERE.parent
sys.path.insert(0, str(WORD / "probes"))

import probe_paths  # noqa: E402  (the probes, their documents and docx-agent's test helpers on the path)
import word_docs  # noqa: E402

W = word_docs.W
_W = "{%s}" % W
#: The date the tests' tracked edits carry.
DATE = "2026-10-04T12:00:00Z"


def observed(name: str) -> Any:
    """An observation file of Word's (``observations/<name>``), read."""
    return json.loads((probe_paths.OBSERVATIONS / name).read_text(encoding="utf-8"))


def plain(xml: str) -> str:
    """A part or block as compared: no XML declaration, namespace declarations, paragraph and
    text ids, rsids or relationship ids; bookmarks' numbers and ``_Toc`` names aside; and no
    ``xml:space`` where the text neither starts nor ends with a space (Word drops it on saving)."""
    xml = re.sub(r"<\?xml[^>]*\?>\s*", "", xml)
    xml = re.sub(r' xmlns(:\w+)?="[^"]*"', "", xml)
    xml = re.sub(r' (w14:paraId|w14:textId|w:rsid\w*|r:id|mc:Ignorable)="[^"]*"', "", xml)
    xml = re.sub(r"_Toc\d+", "_Toc", xml)
    xml = re.sub(r'<w:t xml:space="preserve">([^<\s][^<]*[^<\s]|[^<\s])</w:t>', r"<w:t>\1</w:t>", xml)
    return re.sub(r'(bookmark(?:Start|End)) w:id="\d+"', r"\1", xml)


class Opened:
    """A document open in a tool session as ``d1``: the tools' calls on it and what they wrote."""

    def __init__(self, box: Any, data: bytes, name: str = "probe.docx", *, kind: str | None = None,
                 **session: Any) -> None:
        self.box = box
        self.session = box.session(**session)
        self.session.open(data, name, **({"kind": kind} if kind else {}))

    def call(self, tool: str, **arguments: Any) -> Any:
        """``tool``'s result on the document, which must be ok."""
        return word_docs.call(self.box, self.session, tool, doc="d1", **arguments)

    def refused(self, tool: str, **arguments: Any) -> dict[str, Any]:
        """``tool``'s refusal on the document, which must be one: its error."""
        result = self.box.dispatch(self.session, tool, {"doc": "d1", **arguments})
        assert not result.ok, (tool, arguments)
        return result.to_json()["error"]

    def track(self, author: str = "A") -> None:
        self.call("word_set_tracking", on=True, author=author)

    def data(self) -> bytes:
        return word_docs.saved(self.session, "d1")

    def part(self, name: str = "word/document.xml") -> str:
        return word_docs.part(self.data(), name)

    def names(self) -> set[str]:
        import io
        import zipfile

        with zipfile.ZipFile(io.BytesIO(self.data())) as archive:
            return set(archive.namelist())

    def paragraph_ids(self) -> list[str]:
        return word_docs.paragraph_ids(self.box, self.session, "d1")

    def blocks(self) -> list[tuple[str, str]]:
        return word_docs.blocks(self.box, self.session, "d1")

    def body(self) -> list[str]:
        """The body's blocks as written (the section's properties left out), each :func:`plain`."""
        from lxml import etree

        body = word_docs.tree(self.data()).find(_W + "body")
        return [plain(etree.tostring(child, encoding="unicode")) for child in body if child.tag != _W + "sectPr"]


def section_properties(xml: str) -> list[str]:
    """Each ``w:sectPr`` in ``xml`` (a body or its blocks), in order, a revision's old ones
    aside."""
    return re.findall(r"<w:sectPr>.*?</w:sectPr>(?!</w:sectPrChange)", plain(xml))
