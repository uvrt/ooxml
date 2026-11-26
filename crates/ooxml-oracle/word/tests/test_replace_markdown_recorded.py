"""``word_insert_markdown`` at ``replace:``, tracked, held to Word (``probes/replace_markdown_probe.py``,
``observations/replace-markdown-word.json``, Word 16.106 for Mac; round 60, R141): Word opens the
tools' replacement, its Accept All gives the new paragraphs with the settings each kept of the
replaced one at its position and its Reject All the replaced paragraphs as they were, and each
paragraph is recorded in Word's own form for a paragraph deleted whole and one inserted whole."""

from __future__ import annotations

from recorded import observed  # noqa: I001  (puts the probes on the path)

import replace_markdown_probe  # noqa: E402
from replace_markdown_cases import ACCEPTED, KEPT, REJECTED  # noqa: E402

OBSERVED = observed("replace-markdown-word.json")
#: The observation's cases these tests compare with (index.py lists why the others are not).
READS = {"word-replace", "tools-opened", "tools-accept", "tools-reject"}

_TOOLS: dict = {}


def tools() -> dict[str, dict]:
    """The tools' document's paragraphs by key (made once)."""
    if not _TOOLS:
        _TOOLS.update(replace_markdown_probe.keyed(replace_markdown_probe.tools_document()))
    return _TOOLS


def test_word_opens_the_tools_replacement_without_a_prompt():
    assert OBSERVED["tools-opened"]["outcome"] == "done", OBSERVED["tools-opened"]


def test_accept_all_gives_the_new_paragraphs_with_the_settings_they_kept():
    accepted = OBSERVED["tools-accept"]
    assert not accepted["revisions"]
    paragraphs = accepted["paragraphs"]
    assert {key: entry["text"] for key, entry in paragraphs.items()} == ACCEPTED
    assert {key: paragraphs[key]["settings"] for key in KEPT} == KEPT


def test_reject_all_gives_the_replaced_paragraphs_as_they_were():
    rejected = OBSERVED["tools-reject"]
    assert not rejected["revisions"]
    paragraphs = rejected["paragraphs"]
    assert {key: entry["text"] for key, entry in paragraphs.items()} == REJECTED
    assert paragraphs["First"]["settings"] == ["keepNext", "pageBreakBefore", "ind"]
    assert paragraphs["Second"]["settings"] == ["keepLines", "jc"]


def test_each_paragraph_is_recorded_as_word_records_one_deleted_or_inserted_whole():
    """Word's own form for a paragraph deleted whole (``First``: its text deleted, its mark
    deleted) and for one inserted whole (``Newtwo``: its text inserted, its mark inserted); the
    tools record every replaced paragraph and every new one so.

    A deliberate difference: typed over the selection, Word keeps the last replaced paragraph's
    mark for the first new text (``Second``: ``Second body.`` deleted, ``Newone first.`` inserted
    in it) and gives the new paragraphs that paragraph's settings, leaving an empty paragraph
    after them; the tools keep each replaced paragraph's settings by kind and position
    (docx-agent 0.1's rule, round 42), so the replacement lays out as the text rewritten in
    place."""
    word, ours = OBSERVED["word-replace"]["paragraphs"], tools()
    deleted = {"runs": word["First"]["runs"], "mark": word["First"]["mark"]}
    assert deleted == {"runs": [["del", "First body."]], "mark": "del"}
    inserted = {"runs": word["Newtwo"]["runs"], "mark": word["Newtwo"]["mark"]}
    assert inserted == {"runs": [["ins", "Newtwo second."]], "mark": "ins"}
    for key, form in (("First", "del"), ("Second", "del"), ("Newone", "ins"), ("Newtwo", "ins")):
        text = REJECTED.get(key) or ACCEPTED[key]
        assert ours[key]["runs"] == [[form, text]], key
        assert ours[key]["mark"] == form, key
    assert ours["First"]["settings"] == word["First"]["settings"]
    assert ours["Newone"]["settings"] == KEPT["Newone"] and ours["Newtwo"]["settings"] == KEPT["Newtwo"]
