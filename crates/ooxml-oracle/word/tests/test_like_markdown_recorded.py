"""``word_insert_markdown`` ``like``, tracked, held to Word (``probes/like_markdown_probe.py``,
``observations/like-markdown-word.json``, Word 16.106 for Mac; round 60, R141): Word opens the
tools' items, gives them the settings it gives items a person adds to the sample's list (Enter
at the end of the list, list indent for an item a level down), numbers them on as it numbers
its own after Accept All, and its Reject All gives the lists as they were."""

from __future__ import annotations

from recorded import observed  # noqa: I001  (puts the probes on the path)

import like_markdown_probe  # noqa: E402
from like_markdown_cases import CASES, REJECTED  # noqa: E402

OBSERVED = observed("like-markdown-word.json")
#: The observation's cases these tests compare with (index.py lists why the others are not).
READS = {"word-added", "tools-opened", "word-labels", "tools-labels", "tools-reject"}

#: The cases the tools add to as Word does, and the one-level list where 0.1's rule differs.
ALIKE = ("count-on", "level-deeper")
ONE_LEVEL = "one-level"

_TOOLS: dict = {}


def tools() -> dict:
    """The tools' document's paragraphs by key and the warnings the calls gave (made once)."""
    if not _TOOLS:
        said: list[str] = []
        _TOOLS["paragraphs"] = like_markdown_probe.keyed(like_markdown_probe.tools_document(said))
        _TOOLS["said"] = said
    return _TOOLS


def _new(case: str) -> list[str]:
    return [text.split()[0] for text, _ in CASES[case]["new"]]


def _named(settings: dict) -> dict:
    """Settings with the style named by id: Word saves its Dutch interface's ids
    (``Lijstnummering``), the tools Word's own English ones (``ListNumber``)."""
    ids = {"Lijstnummering": "ListNumber", "Lijstopsomteken": "ListBullet", "Lijstalinea": "ListParagraph",
           "Lijstopsomteken2": "ListBullet2"}
    out = dict(settings)
    if "pStyle" in out:
        out["pStyle"] = {"val": ids.get(out["pStyle"]["val"], out["pStyle"]["val"])}
    return out


def test_word_opens_the_tools_items_without_a_prompt():
    assert OBSERVED["tools-opened"]["outcome"] == "done", OBSERVED["tools-opened"]


def test_new_items_have_the_settings_word_gives_items_added_to_the_list():
    word, ours = OBSERVED["word-added"]["paragraphs"], tools()["paragraphs"]
    for case in ALIKE:
        for key in [CASES[case]["sample"], *_new(case)]:
            assert _named(ours[key]["settings"]) == _named(word[key]["settings"]), (case, key)


def test_accept_all_numbers_the_tools_items_as_word_numbers_its_own():
    word, ours = OBSERVED["word-labels"], OBSERVED["tools-labels"]
    assert not ours["revisions"]
    for case in ALIKE:
        for key in [CASES[case]["sample"], *_new(case)]:
            assert ours["paragraphs"][key]["text"] == word["paragraphs"][key]["text"], (case, key)
    assert [ours["paragraphs"][k]["text"] for k in ("Walk", "Ford", "Cross", "Rest")] == [
        "1.\tWalk first.", "2.\tFord second.", "3.\tCross third.", "4.\tRest fourth."]


def test_reject_all_gives_the_lists_as_they_were():
    rejected = OBSERVED["tools-reject"]
    assert not rejected["revisions"]
    assert list(rejected["paragraphs"]) == REJECTED


def test_each_new_item_is_recorded_inserted_whole():
    """The tools record each new item's text and mark as inserted (Word's form for a paragraph
    inserted whole). Typing Enter at the end of the last item, Word records the inserted mark
    on that item instead and leaves the last new item's mark unmarked; Accept All and Reject
    All give the same in both forms (above)."""
    ours = tools()["paragraphs"]
    for case in CASES:
        for key in _new(case):
            assert ours[key]["runs"][0][0] == "ins" and ours[key]["mark"] == "ins", key


def test_a_level_below_a_one_level_list_is_written_without_like_and_said():
    """A deliberate difference (docx-agent 0.1's rule, round 42): Word's built-in List Bullet is
    a one-level list, so it has no level below the sample's. Word's list indent then indents the
    item in the same style (``ind`` 720, a ``num`` tab); the tools write the item as without
    like -- the Markdown's own List Bullet 2 -- and say so."""
    word, ours = OBSERVED["word-added"]["paragraphs"], tools()["paragraphs"]
    detail = CASES[ONE_LEVEL]["new"][1][0].split()[0]
    assert _named(word[detail]["settings"])["pStyle"] == {"val": "ListBullet"}
    assert "numPr" not in word[detail]["settings"] and word[detail]["settings"]["ind"] == {"left": "720"}
    assert _named(ours[detail]["settings"])["pStyle"] == {"val": "ListBullet2"}
    assert any("has no level 1 below" in w and "written as without like" in w for w in tools()["said"])
