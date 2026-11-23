"""Round 42: ``word_insert_markdown`` at ``replace:p:A..p:B`` gives the new paragraphs the
replaced ones' paragraph settings -- style where Markdown names none, keeps, page breaks,
indents, list membership and level -- matched by kind and position, Markdown's word winning,
and says what was kept.  Before, the new paragraphs had the style map's defaults only.
Made-up documents on a blank package, through the toolbox."""

from __future__ import annotations

import re

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

MD = """# Plan

Intro text.

First body.

Second body.

- one
- two
    - two a
- three

After.
"""


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def call(box, session, tool, **arguments):
    return word_docs.call(box, session, tool, doc="d1", **arguments)


def made(box):
    """The report: its third paragraph kept with the next, its lines together, a page break
    before and indented; its fourth in Report Body, lines together; the list items kept with
    the next."""
    session = box.session()
    session.open(word_docs.blank(), "keeps.docx")
    call(box, session, "word_insert_markdown", at="end", markdown=MD)
    call(box, session, "word_styles", action="add", name="Report Body", kind="paragraph", based_on="Normal",
         space_after=9)
    call(box, session, "word_styles", action="add", name="Chapter Head", kind="paragraph", based_on="heading 2",
         color="1F4E79")
    ps = word_docs.paragraph_ids(box, session, "d1")
    call(box, session, "word_format", targets=[ps[2]], keep_with_next=True, keep_lines=True, page_break_before=True,
         indent_left=18)
    call(box, session, "word_format", targets=[ps[3]], paragraph_style="Report Body", keep_lines=True)
    call(box, session, "word_format", targets=ps[4:8], keep_with_next=True)
    return session


_ID = re.compile(r"<!-- ((?:p|t):[0-9A-Fa-f]+)[^>]*-->")


def ids(box, session):
    """Each block's id by its Markdown text: a block's comment on a line of its own names the
    lines after it, up to the next line naming a block; a list item's comment ends its line."""
    out: dict[str, str] = {}
    lines = word_docs.read(box, session, "d1").split("\n")
    for k, line in enumerate(lines):
        found = _ID.search(line)
        if found is None:
            continue
        if found.start() == 0:
            following = []
            for after in lines[k + 1:]:
                if _ID.search(after):
                    break
                following.append(after)
            out.setdefault("\n".join(following).strip(), found.group(1))
        else:
            out.setdefault(line[:found.start()].strip(), found.group(1))
    return out


def replace(box, session, markdown, at, **arguments):
    """``word_insert_markdown`` at ``replace:<at>``: its result."""
    return call(box, session, "word_insert_markdown", markdown=markdown, at=f"replace:{at}", **arguments)


def _paragraph(session, text):
    return next(p for p in word_docs.tree(word_docs.saved(session, "d1")).iter(W + "p")
                if word_docs.text_of(p) == text)


def has(session, text, tag):
    return _paragraph(session, text).find(f"{W}pPr/{W}{tag}") is not None


def style_name(session, text) -> str:
    """The paragraph's style's name (Normal where it names none)."""
    found = _paragraph(session, text).find(f"{W}pPr/{W}pStyle")
    styles = word_docs.tree(word_docs.saved(session, "d1"), "word/styles.xml")
    style_id = found.get(W + "val") if found is not None else None
    for style in styles.iter(W + "style"):
        if style.get(W + "type") == "paragraph" and (style.get(W + "styleId") == style_id
                                                     or style_id is None and style.get(W + "default") == "1"):
            return style.find(W + "name").get(W + "val")
    raise AssertionError(f"no style {style_id}")


def list_of(session, text) -> tuple[str | None, int | None]:
    """The paragraph's own list membership: (numId, level)."""
    num = _paragraph(session, text).find(f"{W}pPr/{W}numPr")
    if num is None:
        return None, None
    level = num.find(W + "ilvl")
    return num.find(W + "numId").get(W + "val"), int(level.get(W + "val")) if level is not None else 0


def test_body_paragraphs_keep_their_style_keeps_page_break_and_indent(box):
    session = made(box)
    old = ids(box, session)
    edit = replace(box, session, "New first.\n\nNew second.", f"{old['First body.']}..{old['Second body.']}")
    for tag in ("keepNext", "keepLines", "pageBreakBefore", "ind"):
        assert has(session, "New first.", tag), tag
    assert style_name(session, "New second.") == "Report Body" and has(session, "New second.", "keepLines")
    assert not has(session, "New second.", "pageBreakBefore")
    now = ids(box, session)
    assert edit.data["kept"] == [
        {"block": now["New first."], "from": old["First body."],
         "settings": ["keep_with_next", "keep_lines", "page_break_before", "indent"]},
        {"block": now["New second."], "from": old["Second body."], "settings": ["style Report Body", "keep_lines"]}]


def test_list_items_join_the_replaced_list_at_their_levels(box):
    session = made(box)
    old = ids(box, session)
    nested_before = list_of(session, "two a")
    replace(box, session, "- uno\n- dos\n    - dos a\n- tres\n- cuatro", f"{old['- one']}..{old['- three']}")
    items = ("uno", "dos", "dos a", "tres", "cuatro")
    assert [style_name(session, t) for t in items] == ["List Bullet", "List Bullet", "List Bullet 2", "List Bullet",
                                                       "List Bullet"]
    # The nested item is in the very list (w:num) the replaced one was in, at its level.
    assert list_of(session, "dos a") == nested_before
    assert all(has(session, t, "keepNext") for t in items)
    # The instance the Markdown list would have made is gone: nothing names it.
    data = word_docs.saved(session, "d1")
    nums = {n.get(W + "numId") for n in word_docs.tree(data, "word/numbering.xml").iter(W + "num")}
    named = {n.get(W + "val") for n in word_docs.tree(data).iter(W + "numId")}
    named |= {n.get(W + "val") for n in word_docs.tree(data, "word/styles.xml").iter(W + "numId")}
    assert nums <= named


def test_a_flat_list_replacing_nested_items_stays_at_their_level(box):
    session = made(box)
    old = ids(box, session)
    level = list_of(session, "two a")[1]
    replace(box, session, "- deep one\n- deep two", old["- two a"])
    for text in ("deep one", "deep two"):
        assert list_of(session, text)[1] == level and style_name(session, text) == "List Bullet 2"
    call(box, session, "word_set_tracking", on=True, author="Writer")
    now = ids(box, session)
    replace(box, session, "Tracked one.\n\n- tracked item", f"{now['Second body.']}..{now['- one']}")
    assert style_name(session, "Tracked one.") == "Report Body"
    assert has(session, "tracked item", "keepNext") and style_name(session, "tracked item") == "List Bullet"
    assert word_docs.validation(box, session, "d1")["new"] == []


def test_markdown_wins_on_kind_level_start_and_named_styles(box):
    session = made(box)
    old = ids(box, session)
    # A numbered list replacing bullets: Markdown says numbered, nothing kept for it.
    edit = replace(box, session, "1. first\n2. second", f"{old['- one']}..{old['- two']}")
    assert style_name(session, "first") == "List Number" and edit.data.get("kept", []) == []
    session = made(box)
    old = ids(box, session)
    # The style map names the paragraph style: Markdown's style, the keeps kept.
    edit = replace(box, session, "Mapped.", old["First body."], style_map=[{"element": "paragraph", "style": "Report Body"}])
    assert style_name(session, "Mapped.") == "Report Body" and has(session, "Mapped.", "keepNext")
    assert not has(session, "Mapped.", "ind")
    assert edit.data["kept"][0]["settings"] == ["keep_with_next", "keep_lines", "page_break_before"]


def test_headings_keep_a_same_level_heading_style(box):
    for markdown, style in (("## Renamed", "Chapter Head"), ("# Top", "heading 1")):
        session = made(box)
        plan = ids(box, session)["# Plan"]
        call(box, session, "word_format", targets=[plan], paragraph_style="Chapter Head")   # an outline level 2 style
        replace(box, session, markdown, plan)                     # another level: Markdown's
        assert style_name(session, markdown.lstrip("# ")) == style


def test_more_new_paragraphs_take_the_last_donor_without_a_second_page_break(box):
    session = made(box)
    old = ids(box, session)
    replace(box, session, "A.\n\nB.\n\nC.", old["First body."])
    assert has(session, "A.", "pageBreakBefore")
    assert not has(session, "B.", "pageBreakBefore") and not has(session, "C.", "pageBreakBefore")
    assert all(has(session, t, "keepLines") for t in ("A.", "B.", "C."))


def test_tracked_replacement_keeps_settings_and_stays_valid(box):
    session = made(box)
    old = ids(box, session)
    call(box, session, "word_set_tracking", on=True, author="Writer")
    replace(box, session, "Tracked one.\n\n- tracked item", f"{old['Second body.']}..{old['- one']}")
    assert style_name(session, "Tracked one.") == "Report Body"
    assert has(session, "tracked item", "keepNext") and style_name(session, "tracked item") == "List Bullet"
    assert word_docs.validation(box, session, "d1")["new"] == []


def test_the_tool_says_what_was_kept(box):
    session = made(box)
    old = ids(box, session)
    result = box.dispatch(session, "word_insert_markdown",
                          {"doc": "d1", "markdown": "- uno\n- dos", "at": f"replace:{old['- one']}..{old['- two']}"})
    assert result.ok, result.to_json()
    kept = result.data["kept"]
    assert [k["block"] for k in kept] == result.data["blocks"]
    assert kept[0]["settings"] == ["style List Bullet", "keep_with_next", "list level 0"]
    assert "kept the replaced paragraphs' settings" in result.summary
    # Inserting elsewhere keeps nothing and says nothing.
    result = box.dispatch(session, "word_insert_markdown", {"doc": "d1", "markdown": "Tail.", "at": "end"})
    assert result.ok and "kept" not in (result.data or {})


def test_the_replacement_lays_out_as_the_text_rewritten_in_place(box):
    """The layout check: replacing paragraphs with Markdown lays out as rewriting their text
    in place (which keeps every setting) does."""
    replaced = made(box)
    old = ids(box, replaced)
    replace(box, replaced, "New first.\n\nNew second.\n\n- uno\n- dos\n    - dos a\n- tres",
            f"{old['First body.']}..{old['- three']}")
    hand = made(box)
    old = ids(box, hand)
    for before, after in (("First body.", "New first."), ("Second body.", "New second."), ("- one", "uno"),
                          ("- two", "dos"), ("- two a", "dos a"), ("- three", "tres")):
        call(box, hand, "word_set_text", target=old[before], text=after)
    assert _laid_out(box, replaced) == _laid_out(box, hand)


def _laid_out(box, session) -> list:
    """The pages, drawn."""
    from pixels import all_pixels

    count = call(box, session, "check", include=["reflow"]).data["reflow"]["pages"]
    drawn = call(box, session, "render", pages=list(range(1, count + 1)), width=400)
    return all_pixels(image.data for image in drawn.images)


@pytest.mark.parametrize("at", ["end", "after:{p}", "before:{p}"])
def test_other_places_are_written_as_before(box, at):
    session = made(box)
    p = ids(box, session)["First body."]
    edit = call(box, session, "word_insert_markdown", markdown="Placed.", at=at.format(p=p))
    assert "kept" not in (edit.data or {}) and style_name(session, "Placed.") == "Normal"
