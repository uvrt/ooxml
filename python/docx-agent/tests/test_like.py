"""Paragraphs like a given one (round 42, R47): ``word_format``'s ``like`` and
``word_insert_markdown``'s ``like`` -- the targets, or new list items, take the sample's
paragraph style, its list at its level (numbered on from it), its direct indents and its page
settings; not its run formatting.  Laid out (docx2svg), a new item is drawn as the sample's
items are: the same marker at the same place, its text at the same place.  Made-up template
and text only."""

from __future__ import annotations

import datetime as dt
import io
import zipfile

import pytest
from lxml import etree
from ooxml_edit.tools import Toolbox
from layout_reads import block_ids, effective

from docx_agent.tools import FORMAT, GROUPS, TOOLS
import layout_reads
import word_docs

W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
W = "{%s}" % W_NS
CLOCK = lambda: dt.datetime(2026, 11, 8, 12, 0, tzinfo=dt.timezone.utc)  # noqa: E731
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering"
NUMBERING_TYPE = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"


def _level(ilvl: int, fmt: str, text: str, left: int) -> str:
    return (f'<w:lvl w:ilvl="{ilvl}"><w:start w:val="1"/><w:numFmt w:val="{fmt}"/>'
            f'<w:lvlText w:val="{text}"/><w:lvlJc w:val="left"/>'
            f'<w:pPr><w:ind w:left="{left}" w:hanging="360"/></w:pPr></w:lvl>')


#: A three-level bullet definition (a dot, a dash, a ring) and a two-level numbered one (1., a)),
#: their lists 1 and 2.
NUMBERING = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<w:numbering xmlns:w="{W_NS}">'
             '<w:abstractNum w:abstractNumId="1"><w:nsid w:val="6B1A0001"/><w:multiLevelType w:val="multilevel"/>'
             + _level(0, "bullet", "•", 360) + _level(1, "bullet", "–", 720) + _level(2, "bullet", "◦", 1080)
             + '</w:abstractNum><w:abstractNum w:abstractNumId="2"><w:nsid w:val="6B1A0002"/>'
             '<w:multiLevelType w:val="multilevel"/>'
             + _level(0, "decimal", "%1.", 360) + _level(1, "lowerLetter", "%2)", 720) + "</w:abstractNum>"
             '<w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num>'
             '<w:num w:numId="2"><w:abstractNumId w:val="2"/></w:num></w:numbering>')
STYLES = "".join(
    f'<w:style w:type="paragraph" w:styleId="{style_id}"><w:name w:val="{name}"/>'
    f'<w:basedOn w:val="Normal"/><w:pPr>{ppr}</w:pPr></w:style>'
    for style_id, name, ppr in (
        ("ListBullet", "List Bullet", '<w:numPr><w:numId w:val="1"/></w:numPr>'),
        ("ListNumber", "List Number", '<w:numPr><w:numId w:val="2"/></w:numPr>'),
        ("PlanBody", "Plan Body", '<w:spacing w:after="120"/>')))


def template() -> bytes:
    """A made-up template: List Bullet numbers itself with a three-level bullet definition
    (a dot, a dash, a ring), List Number with a two-level numbered one (1., a)), and a body
    style, Plan Body."""
    source = zipfile.ZipFile(io.BytesIO(word_docs.blank()))
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            data = source.read(info.filename).decode("utf-8")
            if info.filename == "word/styles.xml":
                data = data.replace("</w:styles>", STYLES + "</w:styles>")
            elif info.filename == "word/_rels/document.xml.rels":
                data = data.replace("</Relationships>", f'<Relationship Id="rId90" Type="{REL}" '
                                    'Target="numbering.xml"/></Relationships>')
            elif info.filename == "[Content_Types].xml":
                data = data.replace("</Types>", f'<Override PartName="/word/numbering.xml" '
                                    f'ContentType="{NUMBERING_TYPE}"/></Types>')
            target.writestr(info, data)
        target.writestr("word/numbering.xml", NUMBERING)
    return out.getvalue()


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def run(box, session, name, **arguments):
    return box.dispatch(session, name, arguments).to_json()


def _ok(box, session, name, **arguments) -> dict:
    result = run(box, session, name, **arguments)
    assert result["ok"], result
    return result


@pytest.fixture
def made(box):
    """The template with a made-up section: a heading, a body paragraph, a nested bullet
    list (each point followed by its examples, a level down, as Word's Tab makes them: the
    style's list at level 1), a numbered list, a body paragraph after.  The second example
    keeps with the next paragraph and has an indent of its own; the body paragraph is Plan Body and
    keeps its lines together."""
    session = box.session(clock=CLOCK)
    session.open(template(), "plan.docx")
    blocks = _ok(box, session, "word_insert_markdown", doc="d1", at="end", markdown=(
        "# River plan\n\nThe plan has three points.\n\n- First point\n- example one\n- example two\n"
        "- Second point\n\nSteps to take:\n\n1. Walk\n2. Ford\n\nThe plan ends here.\n"))["data"]["blocks"]
    ids = dict(zip(["heading", "body", "point1", "example1", "example2", "point2", "intro", "step1", "step2",
                    "end"], blocks))
    _ok(box, session, "word_lists", doc="d1", targets=[ids["example1"], ids["example2"]], action="level", level=1)
    _ok(box, session, "word_format", doc="d1", targets=[ids["example2"]], keep_with_next=True, indent_left=40)
    _ok(box, session, "word_format", doc="d1", targets=[ids["body"]], paragraph_style="Plan Body")
    _ok(box, session, "word_format", doc="d1", targets=[ids["body"]], keep_together=True)
    return session, ids


def _element(session, identifier: str):
    return word_docs.element(session, "d1", identifier)


def own(session, identifier: str, tag: str):
    found = _element(session, identifier).find(f"{W}pPr/{W}{tag}")
    return None if found is None else dict(found.attrib) or etree.tostring(found)


def numbering_of(session, identifier: str) -> tuple | None:
    node = _element(session, identifier).find(f"{W}pPr/{W}numPr")
    if node is None:
        return None
    return tuple(child.get(W + "val") for child in node)


def list_of(box, session, identifier: str) -> dict | None:
    """A paragraph's list item as ``word_inspect`` gives it: its ``num_id``, ``level`` and
    ``label`` (``None``: in no list)."""
    return _ok(box, session, "word_inspect", doc="d1", target=identifier)["data"]["blocks"][0].get("list")


def labels(box, session, ids: list[str]) -> list[str | None]:
    return [(list_of(box, session, i) or {}).get("label") for i in ids]


def style_of(session, identifier: str) -> str | None:
    """The style id a paragraph names."""
    found = _element(session, identifier).find(f"{W}pPr/{W}pStyle")
    return None if found is None else found.get(W + "val")


def drawn(box, session, identifier: str) -> tuple:
    """How a paragraph's first line is drawn: its marker (the list label) and where the
    marker and the text start (its effective indents)."""
    indent = effective(box, session, "d1", identifier)["indent"]
    return ((list_of(box, session, identifier) or {}).get("label"),
            (indent["left"] - indent["hanging"] + indent["first_line"], indent["left"]))


def nums(session) -> set[str]:
    """The numbering instances the document defines."""
    root = word_docs.tree(word_docs.saved(session, "d1"), "word/numbering.xml")
    return {num.get(W + "numId") for num in root.iter(W + "num")}


def with_strays(box, session, after: str, items: list[tuple[str, int]], style: str | None = None):
    """The document as it is now with ``items`` after ``after``, each in a list of its own
    (:func:`layout_reads.with_strays`), opened anew in a new session: ``(session, their ids)``."""
    changed, ids = layout_reads.with_strays(word_docs.saved(session, "d1"), after, items, style)
    fresh = box.session(clock=CLOCK)
    fresh.open(changed, "plan.docx")
    return fresh, ids


def inserted(box, session, text: str, after: str, style: str | None = None) -> str:
    """A plain paragraph of ``text`` after ``after`` (in ``style``): its id."""
    made = _ok(box, session, "word_insert_markdown", doc="d1", at=f"after:{after}", markdown=text)["data"]["blocks"][0]
    if style is not None:
        _ok(box, session, "word_format", doc="d1", targets=[made], paragraph_style=style)
    return made


# -- word_format like ----------------------------------------------------------------------------


def test_targets_join_the_samples_list_at_its_level_numbered_on(box, made):
    session, ids = made
    added = inserted(box, session, "Third", ids["end"])
    later = inserted(box, session, "Fourth", added)
    result = run(box, session, "word_format", doc="d1", targets=[f"{added}..{later}"], like=ids["step2"])
    assert result["ok"], result
    assert result["data"]["like"]["set"] == {added: ["style"], later: ["style"]}
    assert style_of(session, added) == style_of(session, ids["step1"]) == "ListNumber"
    assert numbering_of(session, added) is None             # the style numbers it, as it does the sample
    assert labels(box, session, [ids["step1"], ids["step2"], added, later]) == ["1.", "2.", "3.", "4."]


def test_a_nested_sample_gives_its_level_and_its_own_numbering_only_where_it_has_it(box, made):
    session, ids = made
    # A separate list of its own, as a writer's new items came out: another definition.
    session, (stray,) = with_strays(box, session, ids["example2"], [("example three", 1)])
    stray_num = list_of(box, session, stray)["num_id"]
    assert stray_num != list_of(box, session, ids["example1"])["num_id"]
    result = run(box, session, "word_format", doc="d1", targets=[stray], like=ids["example1"])
    assert result["ok"], result
    assert numbering_of(session, stray) == numbering_of(session, ids["example1"]) == ("1", "1")
    assert list_of(box, session, stray)["num_id"] == list_of(box, session, ids["example1"])["num_id"]
    assert str(stray_num) not in nums(session)         # the list left unused is gone
    assert f"num:{stray_num}" in result["removed"]
    assert labels(box, session, [ids["example1"], stray]) == ["–", "–"]


def test_indents_and_page_settings_are_taken_whole_and_runs_stay(box, made):
    session, ids = made
    target = inserted(box, session, "example four", ids["point2"], style="Plan Body")
    _ok(box, session, "word_format", doc="d1", targets=[target], keep_lines=True, page_break_before=True, bold=True,
        alignment="center")
    result = run(box, session, "word_format", doc="d1", targets=target, like=ids["example2"])
    assert result["ok"], result
    assert result["data"]["like"]["set"][target] == ["style", "list", "indent", "keep_with_next", "keep_lines",
                                                     "page_break_before"]
    assert "run formatting" in result["data"]["like"]["not_taken"]
    assert own(session, target, "ind") == own(session, ids["example2"], "ind")
    assert own(session, target, "keepNext") is not None
    assert own(session, target, "keepLines") is None and own(session, target, "pageBreakBefore") is None
    assert own(session, target, "jc") is not None                             # alignment not taken
    assert _element(session, target).find(f"{W}r/{W}rPr/{W}b") is not None   # nor the runs' formatting


def test_explicit_settings_in_the_same_call_win(box, made):
    session, ids = made
    target = inserted(box, session, "Aside", ids["end"])
    result = run(box, session, "word_format", doc="d1", targets=target, like=ids["example2"],
                 keep_with_next=False, indent_left=10)
    assert result["ok"], result
    assert "keep_with_next" not in result["data"]["like"]["set"][target]
    assert own(session, target, "keepNext") == {f"{W}val": "0"} or own(session, target, "keepNext") is None
    assert own(session, target, "ind")[f"{W}left"] == "200"
    assert numbering_of(session, target) == ("1", "1")                         # the list is the sample's
    styled = run(box, session, "word_format", doc="d1", targets=target, like=ids["step1"],
                 paragraph_style="Plan Body")
    assert styled["ok"] and style_of(session, target) == "PlanBody"


def test_already_alike_is_no_change_and_the_sample_among_its_targets_is_left(box, made):
    session, ids = made
    version = session.entry("d1").version
    result = run(box, session, "word_format", doc="d1", targets=[f"{ids['point1']}..{ids['point2']}"],
                 like=ids["point1"])
    assert result["ok"], result
    # The examples differ (their level, one's indent and keep); the points are alike.
    assert set(result["data"]["like"]["set"]) == {ids["example1"], ids["example2"]}
    assert result["data"]["like"]["alike"] == [ids["point1"], ids["point2"]]
    again = run(box, session, "word_format", doc="d1", targets=[ids["point2"], ids["example1"]], like=ids["point1"])
    assert again["ok"] and again["changed"] == []
    assert again["warnings"] == [f"every target already looks like {ids['point1']}: nothing changed"]
    assert session.entry("d1").version == version + 1                        # the no-op made no version


def test_a_heading_sample_makes_a_heading_and_says_the_outline_moved(box, made):
    session, ids = made
    result = run(box, session, "word_format", doc="d1", targets=ids["intro"], like=ids["heading"])
    assert result["ok"], result
    assert style_of(session, ids["intro"]) == style_of(session, ids["heading"])
    assert any("heading" in w.lower() for w in result["warnings"]), result["warnings"]


def test_a_sample_in_no_list_takes_the_target_out_of_its_list(box, made):
    session, ids = made
    result = run(box, session, "word_format", doc="d1", targets=ids["step2"], like=ids["body"])
    assert result["ok"], result
    assert list_of(box, session, ids["step2"]) is None and style_of(session, ids["step2"]) == "PlanBody"
    assert own(session, ids["step2"], "keepLines") is not None                 # the body's keep, taken


def test_a_table_cell_paragraph_as_the_sample(box, made):
    session, ids = made
    made_table = run(box, session, "word_insert_markdown", doc="d1", at=f"after:{ids['end']}",
                     markdown="| Item | Note |\n|---|---|\n| a | b |\n")
    table = made_table["data"]["blocks"][0]
    inspected = _ok(box, session, "word_inspect", doc="d1", target=table)["data"]["blocks"][0]
    cell = inspected["rows"][1][0]["blocks"][0]["id"]
    _ok(box, session, "word_format", doc="d1", targets=[cell], indent_left=6, keep_with_next=True)
    table = block_ids(box, session, "d1")[block_ids(box, session, "d1").index(ids["end"]) + 1]
    result = run(box, session, "word_format", doc="d1", targets=ids["intro"], like=f"{table}/c1,0")
    assert result["ok"], result
    assert f"like {table}/c1,0 read as its paragraph {cell}" in result["warnings"]
    assert style_of(session, ids["intro"]) == style_of(session, cell)
    assert own(session, ids["intro"], "ind") == own(session, cell, "ind")


def test_a_sample_of_several_paragraphs_or_none_is_refused_with_the_way_out(box, made):
    session, ids = made
    several = run(box, session, "word_format", doc="d1", targets=ids["intro"],
                  like=f"{ids['point1']}..{ids['point2']}")
    assert not several["ok"] and several["error"]["field"] == "like"
    assert ids["point1"] in several["error"]["valid_options"]
    assert f"like: {ids['point1']!r}" in several["error"]["message"]
    missing = run(box, session, "word_format", doc="d1", targets=ids["intro"], like="p:0BADBEEF")
    assert not missing["ok"] and missing["error"]["field"] == "like"
    ranged = run(box, session, "word_format", doc="d1", targets=ids["intro"], like=f"{ids['point1']}@0:3")
    assert ranged["ok"] and f"like {ids['point1']}@0:3 read as its paragraph {ids['point1']}" in ranged["warnings"]


def test_tracked_like_is_a_paragraph_property_change_rejected_back(box, made):
    session, ids = made
    target = inserted(box, session, "Tracked", ids["end"])
    assert run(box, session, "word_set_tracking", doc="d1", on=True, author="Lane")["ok"]
    result = run(box, session, "word_format", doc="d1", targets=target, like=ids["example1"])
    assert result["ok"], result
    assert _element(session, target).find(f"{W}pPr/{W}pPrChange") is not None
    assert numbering_of(session, target) == ("1", "1")
    listed = run(box, session, "word_changes", doc="d1", action="reject", all=True)
    assert listed["ok"], listed
    assert list_of(box, session, target) is None


def test_one_undo_step(box, made):
    session, ids = made
    before = word_docs.saved(session, "d1")
    target = ids["intro"]
    assert run(box, session, "word_format", doc="d1", targets=target, like=ids["point1"])["ok"]
    assert run(box, session, "undo", doc="d1")["ok"]
    assert word_docs.saved(session, "d1") == before


def test_drawn_as_the_samples_items_are(box, made):
    session, ids = made
    point = inserted(box, session, "Third point", ids["point2"])
    example = inserted(box, session, "example five", point)
    assert run(box, session, "word_format", doc="d1", targets=point, like=ids["point1"])["ok"]
    assert run(box, session, "word_format", doc="d1", targets=example, like=ids["example1"])["ok"]
    assert drawn(box, session, point) == drawn(box, session, ids["point1"])          # the marker and its place
    assert drawn(box, session, example) == drawn(box, session, ids["example1"])


# -- the way out of removing an item's own list --------------------------------------------------


def test_removing_a_stray_lists_numbering_offers_joining_the_list_keeping_the_markers(box, made):
    session, ids = made
    session, strays = with_strays(box, session, ids["point2"], [("Third point", 0), ("example six", 1)],
                                  style="ListBullet")
    refused = run(box, session, "word_lists", doc="d1", targets=strays, action="remove")
    assert not refused["ok"]
    message = refused["error"]["message"]
    assert "join an earlier list at their level" in message and f'"like": "{ids["example2"]}"' in message
    assert f"word_format like {ids['example2']}" in refused["error"]["valid_options"]
    # The call it names keeps the marker at the item's level.
    assert run(box, session, "word_format", doc="d1", targets=[strays[1]], like=ids["example2"])["ok"]
    assert labels(box, session, [strays[1]]) == ["–"]


# -- word_insert_markdown like -------------------------------------------------------------------


def test_new_nested_items_follow_the_sample_at_each_level(box, made):
    session, ids = made
    result = run(box, session, "word_insert_markdown", doc="d1", at=f"after:{ids['point2']}",
                 markdown="- Third point\n  - example seven\n  - example eight\n- Fourth point\n",
                 like=[ids["point1"], ids["example1"]])
    assert result["ok"], result
    new = result["data"]["blocks"]
    assert [style_of(session, i) for i in new] == ["ListBullet"] * 4
    assert [numbering_of(session, i) for i in new] == [None, ("1", "1"), ("1", "1"), None]
    assert labels(box, session, new) == ["•", "–", "–", "•"]
    assert {list_of(box, session, i)["num_id"] for i in new} == {list_of(box, session, ids["point1"])["num_id"]}
    assert drawn(box, session, new[1]) == drawn(box, session, ids["example1"])


def test_a_level_below_the_given_ones_is_the_list_a_level_deeper(box, made):
    session, ids = made
    result = run(box, session, "word_insert_markdown", doc="d1", at=f"after:{ids['point2']}",
                 markdown="- Third point\n  - example\n    - detail\n", like=ids["point1"])
    assert result["ok"], result
    new = result["data"]["blocks"]
    assert [list_of(box, session, i)["level"] for i in new] == [0, 1, 2]
    assert labels(box, session, new) == ["•", "–", "◦"]
    assert [style_of(session, i) for i in new] == ["ListBullet"] * 3        # Word's Tab: the style stays


def test_numbered_items_count_on_from_the_sample(box, made):
    session, ids = made
    result = run(box, session, "word_insert_markdown", doc="d1", at=f"after:{ids['step2']}",
                 markdown="1. Cross\n2. Rest\n", like=[ids["step1"]])
    assert result["ok"], result
    assert labels(box, session, [ids["step2"], *result["data"]["blocks"]]) == ["2.", "3.", "4."]


def test_a_style_map_naming_a_paragraph_is_read_as_like(box, made):
    session, ids = made
    result = run(box, session, "word_insert_markdown", doc="d1", at=f"after:{ids['point2']}",
                 markdown="- Third point\n  - example nine\n",
                 style_map=[{"element": "bullet2", "style": ids["example1"]}])
    assert result["ok"], result
    assert any("read as like" in w for w in result["warnings"])
    new = result["data"]["blocks"]
    assert numbering_of(session, new[1]) == ("1", "1") and labels(box, session, new) == ["•", "–"]


def test_an_empty_level_is_written_without_like(box, made):
    session, ids = made
    result = run(box, session, "word_insert_markdown", doc="d1", at=f"after:{ids['end']}",
                 markdown="- top\n  - nested\n", like=["", ids["example1"]])
    assert result["ok"], result
    new = result["data"]["blocks"]
    assert numbering_of(session, new[1]) == ("1", "1")
    assert style_of(session, new[0]) == "ListBullet" and numbering_of(session, new[0]) is None


def test_a_sample_in_no_list_and_another_kind_are_said(box, made):
    session, ids = made
    plain = run(box, session, "word_insert_markdown", doc="d1", at=f"after:{ids['end']}",
                markdown="- one\n", like=ids["body"])
    assert plain["ok"] and any("is in no list" in w for w in plain["warnings"])
    assert list_of(box, session, plain["data"]["blocks"][0]) is None
    other = run(box, session, "word_insert_markdown", doc="d1", at=f"after:{ids['end']}",
                markdown="1. one\n", like=ids["point1"])
    assert other["ok"] and any("are numbers and" in w for w in other["warnings"])


def test_replacing_the_sample_itself_keeps_its_look(box, made):
    session, ids = made
    result = run(box, session, "word_insert_markdown", doc="d1",
                 at=f"replace:{ids['example1']}..{ids['example2']}",
                 markdown="- example ten\n- example eleven\n", like=ids["example2"])
    assert result["ok"], result
    new = result["data"]["blocks"]
    assert [numbering_of(session, i) for i in new] == [("1", "1")] * 2
    assert own(session, new[0], "keepNext") is not None
    assert labels(box, session, [ids["point1"], *new, ids["point2"]]) == ["•", "–", "–", "•"]
    assert word_docs.validation(box, session, "d1")["new"] == []


def test_a_copied_sample_extended_follows_the_copy(box, made):
    session, ids = made
    copied = run(box, session, "word_copy_from", doc="d1", source_doc="d1",
                 range=f"{ids['point1']}..{ids['point2']}", at=f"after:{ids['end']}")
    assert copied["ok"], copied
    copy = copied["data"]["blocks"]
    last_point, first_example = copy[-1], copy[1]
    result = run(box, session, "word_insert_markdown", doc="d1", at=f"after:{last_point}",
                 markdown="- Third point\n  - example twelve\n", like=[copy[0], first_example])
    assert result["ok"], result
    new = result["data"]["blocks"]
    # Each level as the copy's item of that level is (the copy's own lists, as it was copied).
    assert [numbering_of(session, i) for i in new] == [numbering_of(session, copy[0]),
                                                        numbering_of(session, first_example)]
    assert [list_of(box, session, i)["num_id"] for i in new] == [list_of(box, session, copy[0])["num_id"],
                                                                 list_of(box, session, first_example)["num_id"]]
    assert [list_of(box, session, i)["level"] for i in new] == [0, 1]
    assert drawn(box, session, new[0])[0] == drawn(box, session, copy[0])[0]
    assert drawn(box, session, new[1])[1] == drawn(box, session, first_example)[1]


def test_like_wins_over_the_replaced_items_kept_settings(box, made):
    session, ids = made
    # The replaced item is a nested example keeping with the next; like names a top-level point.
    result = run(box, session, "word_insert_markdown", doc="d1", at=f"replace:{ids['example2']}",
                 markdown="- Second example, now a point\n", like=ids["point1"])
    assert result["ok"], result
    new = result["data"]["blocks"][0]
    assert numbering_of(session, new) is None and list_of(box, session, new)["level"] == 0
    assert own(session, new, "keepNext") is None and own(session, new, "ind") is None
    assert "kept" not in result["data"]
