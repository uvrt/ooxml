"""Word's side of the suite: the documents and PDFs Word makes of a tool output, through the
oracle driver (``word/oracle.py``: the machine-wide lock, the content-hash cache), and Word's
version.  Each answer is cached by the input's bytes, so a second run costs no Word time.

Must not: import pytest, or read a document (``answers.py`` does).
"""

from __future__ import annotations

import plistlib
from typing import Any

import oracle

#: Word updates every field of the body, then of each section's headers and footers, then
#: each table of contents (its page numbers from Word's own layout), and saves.
UPDATE_FIELDS = oracle.word_script([
    "repeat with i from 1 to (count of fields of d)",
    "  try",
    "    update field (field i of d)",
    "  end try",
    "end repeat",
    "repeat with s from 1 to (count of sections of d)",
    "  repeat with k in {header footer primary, header footer first page, header footer even pages}",
    "    repeat with which in {true, false}",
    "      try",
    "        if which then",
    "          set hf to get header (section s of d) index k",
    "        else",
    "          set hf to get footer (section s of d) index k",
    "        end if",
    "        repeat with i from 1 to (count of fields of text object of hf)",
    "          update field (field i of text object of hf)",
    "        end repeat",
    "      end try",
    "    end repeat",
    "  end repeat",
    "end repeat",
    "repeat with i from 1 to (count of tables of contents of d)",
    "  try",
    "    update (table of contents i of d)",
    "  end try",
    "end repeat",
])
ACCEPT_ALL = oracle.review_script("accept")
REJECT_ALL = oracle.review_script("reject")


def word_version() -> str | None:
    """The installed Word's version (its bundle's), recorded with every observation."""
    try:
        with open(oracle.WORD_APP / "Contents" / "Info.plist", "rb") as handle:
            return plistlib.load(handle).get("CFBundleShortVersionString")
    except OSError:
        return None


def _bytes(outcome: Any) -> bytes | None:
    return outcome.path.read_bytes() if outcome else None


class WordSide:
    """One Word session's answers about tool outputs."""

    def __init__(self, session: Any) -> None:
        self.session = session

    def export(self, data: bytes, name: str) -> Any:
        """Word's PDF (its final view where there is markup): an outcome, ``rejected`` when
        Word blocked on a prompt."""
        return self.session.export_pdf(data, name=name)

    def resave(self, data: bytes, name: str) -> bytes | None:
        return _bytes(self.session.resave(data, name=name))

    def reviewed(self, data: bytes, action: str, name: str) -> bytes | None:
        """Word's Accept All (``action`` "accept") or Reject All ("reject"), saved."""
        script = ACCEPT_ALL if action == "accept" else REJECT_ALL
        return _bytes(self.session.run_script(script, data, name=name, tag=action))

    def fields_updated(self, data: bytes, name: str) -> bytes | None:
        """The document after Word updated every field, saved."""
        return _bytes(self.session.run_script(UPDATE_FIELDS, data, name=name, tag="fields"))

    @staticmethod
    def pdf_pages(pdf: Any) -> list[str]:
        """Each page's text, whitespace collapsed."""
        return [" ".join(page.split()) for page in oracle.pdf_pages(pdf)]
