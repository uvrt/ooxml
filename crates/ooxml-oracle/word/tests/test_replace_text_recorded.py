"""A tracked ``replace_text`` in a range, held to Word (``probes/replace_text_probe.py``,
``observations/replace-text-word.json``, Word 16.106 for Mac; round 60, R139): the tools' replace
of each case (a span of paragraphs, a table cell, a regular expression in one paragraph) records
what Word's own Find and Replace records, Word opens the tools' document, and its Accept All and
Reject All of it give the text replaced and the text as it was."""

from __future__ import annotations

from recorded import observed  # noqa: I001  (puts the probes on the path)

import replace_text_probe  # noqa: E402
from replace_text_cases import ACCEPTED, REJECTED  # noqa: E402

OBSERVED = observed("replace-text-word.json")
#: The observation's cases these tests compare with (index.py lists why the others are not).
READS = {"word-replace", "word-replace-untracked", "tools-opened", "tools-accept", "tools-reject"}

_TOOLS: dict = {}


def tools() -> dict[str, list[list[str]]]:
    """The tools' document's runs by key (made once)."""
    if not _TOOLS:
        _TOOLS.update(replace_text_probe.keyed(replace_text_probe.tools_document()))
    return _TOOLS


def test_word_opens_the_tools_replace_without_a_prompt():
    assert OBSERVED["tools-opened"]["outcome"] == "done", OBSERVED["tools-opened"]


def test_words_accept_all_gives_the_text_replaced_and_reject_all_the_text_before():
    for action, expected in (("accept", ACCEPTED), ("reject", REJECTED)):
        reviewed = OBSERVED[f"tools-{action}"]
        assert not reviewed["revisions"], action
        assert {key: reviewed["texts"][key] for key in expected} == expected, action


def test_a_replace_in_a_span_or_a_cell_records_what_words_find_and_replace_records():
    """Each match deleted and its replacement inserted after it, the text around kept as it
    was; outside the range nothing changes (Head, Cell, Tail)."""
    word = OBSERVED["word-replace"]["runs"]
    for key in ("Head", "Alpha", "Bravo", "Cell", "Other", "Tail"):
        assert tools()[key] == word[key], key


def test_a_regex_replace_records_the_match_deleted_and_its_expansion_inserted():
    """A deliberate difference. Word's wildcard replace, tracked, records its replacement piece by
    piece against the match (``11`` inserted, ``2`` deleted, ``2026`` inserted, ``026-11``
    deleted, ``/`` inserted), and those pieces accepted read ``112026/``, not the ``11/2026`` the
    same replace gives untracked: Word 16.106 for Mac records it wrongly. The tools record each
    match deleted whole and its expansion inserted whole, as Word records a replace of plain
    text: accepted, the text Word's untracked replace gives; rejected, the text before."""
    word, ours = OBSERVED["word-replace"]["runs"]["Charlie"], tools()["Charlie"]
    assert ours == [["text", "Charlie dates "], ["del", "2026-11"], ["ins", "11/2026"], ["text", " and "],
                    ["del", "2025-03"], ["ins", "03/2025"], ["text", " here."]]

    def view(runs: list[list[str]], left_out: str) -> str:
        return "".join(text for kind, text in runs if kind != left_out)

    untracked = OBSERVED["word-replace-untracked"]["texts"]
    assert untracked == ACCEPTED
    assert view(ours, "del") == untracked["Charlie"] != view(word, "del") == "Charlie dates 112026/ and 032025/ here."
    assert view(ours, "ins") == view(word, "ins") == REJECTED["Charlie"]
