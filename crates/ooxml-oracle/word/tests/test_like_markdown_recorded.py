"""``word_insert_markdown`` ``like``, tracked, held to Word (``probes/like_markdown_probe.py``,
``observations/like-markdown-word.json``, Word 16.106 for Mac; round 60, R141): Word opens the
tools' items, gives them the settings it gives items a person adds to the sample's list (Enter
at the end of the list, list indent for an item a level down), numbers them on as it numbers
its own after Accept All, and its Reject All gives the lists as they were. In a one-level list
(F-2c, user decision 2026-11-27: follow Word) an item a level down keeps the style and moves
as Word's list indent moves it (``word-indents``: the rule is
crates/ooxml-word-edit/src/format/list_indent.rs's)."""

from __future__ import annotations

from recorded import observed  # noqa: I001  (puts the probes on the path)

import like_markdown_probe  # noqa: E402
from like_markdown_cases import CASES, INDENT_NEW, INDENTS, REJECTED  # noqa: E402

OBSERVED = observed("like-markdown-word.json")
#: The observation's cases these tests compare with (index.py lists why the others are not).
READS = {"word-added", "tools-opened", "word-labels", "tools-labels", "tools-reject", "word-indents"}

#: The cases the tools add to as Word does (all of them).
ALIKE = tuple(CASES)

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
           "Lijstopsomteken2": "ListBullet2", "Lijstopsomteken3": "ListBullet3"}
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


def test_a_level_below_a_one_level_list_is_moved_as_word_moves_it_and_nothing_is_said():
    """Word's built-in List Bullet and List Number are one-level lists: Word's list indent keeps
    the item's style and moves it (``ind`` left, a ``num`` tab there, no ``numPr``). The tools
    do the same, and say nothing (docx-agent 0.1 wrote the Markdown's List Bullet 2 and warned)."""
    word, ours = OBSERVED["word-added"]["paragraphs"], tools()["paragraphs"]
    for case in ("one-level", "one-level-number", "one-level-indented"):
        for text, depth in CASES[case]["new"][1:]:
            key = text.split()[0]
            assert "numPr" not in word[key]["settings"] and word[key]["settings"]["ind"]["left"], (case, key)
            assert _named(ours[key]["settings"]) == _named(word[key]["settings"]), (case, key)
    assert not any("has no level" in w for w in tools()["said"]), tools()["said"]


def _indented(name: str) -> dict:
    case = INDENTS[name]
    said: list[str] = []
    return {"paragraphs": like_markdown_probe.keyed(like_markdown_probe.tools_indent_document(case, said)),
            "said": said}


def test_new_items_in_a_one_level_list_move_to_where_word_moves_them():
    """Each ``new`` case of ``word-indents`` (a document of its own: one-level lists alone, a
    default tab stop of 567, what follows the new items): the tools' items, written ``like``
    the list's first item, have the settings Word gives the items it types and list-indents,
    at every level."""
    for name, case in INDENTS.items():
        if case["mode"] != "new":
            continue
        word, ours = OBSERVED["word-indents"][name], _indented(name)
        for text, _ in INDENT_NEW:
            key = text.split()[0]
            assert _named(ours["paragraphs"][key]["settings"]) == _named(word[key]), (name, key)
        assert not ours["said"], (name, ours["said"])


def test_an_existing_item_moved_where_it_stands_is_known_and_not_followed():
    """Known, not followed (no tool moves an existing item of a one-level list): Word's list
    indent of an item where it stands stops at the paragraph before it (its first line), not at
    the one after it -- unlike new items, which stop at the paragraph after them."""
    lefts = {name: OBSERVED["word-indents"][name]["Alpha"]["ind"]["left"]
             for name in ("existing-alone", "existing-after-list", "existing-before-list")}
    assert lefts == {"existing-alone": "1068", "existing-after-list": "926", "existing-before-list": "1068"}
