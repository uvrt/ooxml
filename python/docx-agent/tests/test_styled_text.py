"""Round 42: ``word_set_text`` writes a paragraph's style and its inline emphasis in the
template's character styles in one call (``paragraph_style``; ``**strong**``, ``*emphasis*``,
``[text]{Character style}``), and text without markup is written as before.  One test (at
least) per tool design rule it touches.  Made-up documents, a blank document's package.
(How the markup is read went with the Python view: the tool's replies and the runs it
writes say it here.)"""

from __future__ import annotations

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs
from kept_records import text_facts
from pixels import all_pixels
from test_tables_from_data import GRID, _session, ds_put  # noqa: E402
from test_tables_from_data import call as dispatch  # noqa: E402

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

MD = """# Results

The mean was **4.0/5** over the year.

Second paragraph here.

| Name | Value |
|---|---|
| left | right |
"""


def made() -> bytes:
    """A blank document with :data:`MD` and two styles of its own, ``Report Body`` (a paragraph
    style) and ``Score Mark`` (a character style)."""
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        session = toolbox.session()
        doc = session.open(word_docs.blank(), "styled.docx")
        word_docs.call(toolbox, session, "word_insert_markdown", doc=doc, at="end", markdown=MD)
        word_docs.call(toolbox, session, "word_styles", doc=doc, action="add", name="Report Body",
                       kind="paragraph", based_on="Normal", space_after=9)
        word_docs.call(toolbox, session, "word_styles", doc=doc, action="add", name="Score Mark",
                       kind="character", bold=True, color="1F4E79")
        return word_docs.saved(session, doc)


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


@pytest.fixture(scope="module")
def made_once() -> bytes:
    return made()


@pytest.fixture
def opened(box, made_once):
    session = box.session()
    return session, session.open(made_once, "styled.docx")


def call(box, session, doc, **arguments):
    return box.dispatch(session, "word_set_text", {"doc": doc, **arguments})


def ok(box, session, doc, **arguments):
    result = call(box, session, doc, **arguments)
    assert result.ok, result.error and result.error.to_json()
    return result


def element(session, doc, identifier):
    return word_docs.element(session, doc, identifier)


def runs(session, doc, identifier):
    """(text, style id, bold, italic) per text run."""
    out = []
    for run in element(session, doc, identifier).iter(W + "r"):
        found = "".join(t.text or "" for t in run.iter(W + "t"))
        if not found:
            continue
        properties = run.find(W + "rPr")
        style = properties.find(W + "rStyle") if properties is not None else None
        out.append((found, style.get(W + "val") if style is not None else None,
                    properties is not None and properties.find(W + "b") is not None,
                    properties is not None and properties.find(W + "i") is not None))
    return out


def body(session, doc) -> list[str]:
    """The ids of the paragraphs that have text."""
    return [block for block, found in word_docs.paragraphs(session, doc) if found]


def text(session, doc, identifier) -> str:
    return word_docs.text_of(element(session, doc, identifier))


def range_text(session, doc, address: str) -> str:
    """The characters of ``p:A@start:end``."""
    paragraph, _, span = address.partition("@")
    start, end = (int(n) for n in span.split(":"))
    return text(session, doc, paragraph)[start:end]


def styles(session, doc) -> dict[str, tuple[str, str]]:
    """Each style's id: its type and name."""
    root = word_docs.tree(word_docs.saved(session, doc), "word/styles.xml")
    return {style.get(W + "styleId"): (style.get(W + "type"), style.find(W + "name").get(W + "val"))
            for style in root.iter(W + "style")}


def style_id(session, doc, name, kind) -> str | None:
    return next((sid for sid, (found, named) in styles(session, doc).items() if (found, named) == (kind, name)),
                None)


def style_name(session, doc, identifier) -> str:
    """A paragraph's style's name (``Normal`` without one)."""
    found = element(session, doc, identifier).find(f"{W}pPr/{W}pStyle")
    return "Normal" if found is None else styles(session, doc)[found.get(W + "val")][1]


# -- writing ---------------------------------------------------------------------------------------


def test_style_and_emphasis_in_one_call_with_the_runs_said(box, opened):
    session, doc = opened
    target = body(session, doc)[1]
    result = ok(box, session, doc, items=[{"target": target, "paragraph_style": "Report Body",
                                           "text": "The mean was **2.0/5**, below *3.0/5* in [March]{Score Mark}."}])
    assert style_name(session, doc, target) == "Report Body"
    assert text(session, doc, target) == "The mean was 2.0/5, below 3.0/5 in March."
    named = {sid: name for sid, (_, name) in styles(session, doc).items()}
    assert [(t, named.get(s)) for t, s, _, _ in runs(session, doc, target) if s] == [
        ("2.0/5", "Strong"), ("3.0/5", "Emphasis"), ("March", "Score Mark")]
    assert result.data["styled"] == [
        {"range": f"{target}@13:18", "text": "2.0/5", "style": "Strong"},
        {"range": f"{target}@26:31", "text": "3.0/5", "style": "Emphasis"},
        {"range": f"{target}@35:40", "text": "March", "style": "Score Mark"}]
    # The ranges said are the characters styled.
    for entry in result.data["styled"]:
        assert range_text(session, doc, entry["range"]) == entry["text"]
    assert "styled" in result.summary
    # One undo step for the style, the text and the emphasis.
    ok_undo = box.dispatch(session, "undo", {"doc": doc})
    assert ok_undo.ok and text(session, doc, target) == "The mean was 4.0/5 over the year."
    assert style_name(session, doc, target) == "Normal"


def test_markup_states_the_emphasis_old_emphasis_goes_other_formatting_stays(box, opened):
    session, doc = opened
    target = body(session, doc)[1]
    # The old text has Strong on 4.0/5; give the rest a colour (direct formatting not emphasis).
    ok_format = box.dispatch(session, "word_format", {"doc": doc, "targets": [f"{target}@0:12"],
                                                      "color": "C00000", "bold": True})
    assert ok_format.ok, ok_format.error and ok_format.error.to_json()
    ok(box, session, doc, target=target, text="The mean was 4.0/5 over the *year*.")
    look = runs(session, doc, target)
    # 4.0/5 is unmarked now: no Strong; "The mean was" keeps its colour but not its bold.
    assert all(style is None for found, style, _, _ in look if "4.0/5" in found)
    assert not any(bold for _, _, bold, _ in look)
    assert any(c.get(W + "val") == "C00000" for c in element(session, doc, target).iter(W + "color"))
    assert [t for t, s, _, _ in look if s] == ["year"]


def test_plain_text_keeps_its_formatting_as_before(box, opened):
    session, doc = opened
    target = body(session, doc)[1]
    result = ok(box, session, doc, target=target, text="The mean was 4.0/5 over the decade.")
    assert result.data is None
    assert [(t, s) for t, s, _, _ in runs(session, doc, target) if s] == [("4.0/5", "Strong")]
    # A lone asterisk or an underscore is text, written as before.
    ok(box, session, doc, target=target, text="The mean was 4.0/5 * 2 over file_name.")
    assert text(session, doc, target) == "The mean was 4.0/5 * 2 over file_name."
    assert [(t, s) for t, s, _, _ in runs(session, doc, target) if s] == [("4.0/5", "Strong")]


def test_rule_1_a_style_beside_items_is_each_items_and_an_id_names_a_style(box, opened):
    session, doc = opened
    first, second = body(session, doc)[1], body(session, doc)[2]
    ok(box, session, doc, paragraph_style="Report Body",
       items=[{"target": first, "text": "One."}, {"target": second, "text": "Two.", "paragraph_style": ""}])
    assert {style_name(session, doc, first), style_name(session, doc, second)} == {"Report Body"}
    found = style_id(session, doc, "Report Body", "paragraph")
    heading = body(session, doc)[0]
    ok(box, session, doc, target=heading, text="Results", paragraph_style=found)
    assert style_name(session, doc, heading) == "Report Body"


def test_rule_2_an_unknown_style_is_refused_with_the_ones_there_are(box, opened):
    session, doc = opened
    target = body(session, doc)[1]
    before = text(session, doc, target)
    refused = call(box, session, doc, items=[{"target": target, "text": "x", "paragraph_style": "Made Up Style"}])
    assert not refused.ok and refused.error.code == "not_found"
    assert refused.error.field == "items[0].paragraph_style" and "Report Body" in refused.error.valid_options
    refused = call(box, session, doc, items=[{"target": target, "text": "**x** in [y]{No Such Style}"}])
    assert not refused.ok and refused.error.field == "items[0].text"
    assert "Score Mark" in refused.error.valid_options and "\\[" in refused.error.message
    refused = call(box, session, doc, target=target, text="[y]{Report Body}")
    assert not refused.ok and "paragraph style" in refused.error.message
    # Refused whole, nothing written; an earlier good item included.
    refused = call(box, session, doc, items=[{"target": target, "text": "Good."},
                                             {"target": target, "text": "x", "paragraph_style": "Nope"}])
    assert not refused.ok and text(session, doc, target) == before


def test_rule_3_the_same_again_changes_nothing(box, opened):
    session, doc = opened
    target = body(session, doc)[1]
    arguments = {"items": [{"target": target, "paragraph_style": "Report Body",
                            "text": "The mean was **2.0/5** in [March]{Score Mark}."}]}
    ok(box, session, doc, **arguments)
    again = ok(box, session, doc, **arguments)
    assert again.changed == [] and any("already in Report Body" in w for w in again.warnings)
    assert again.data["styled"]            # what the runs are, said again
    same_style = ok(box, session, doc, target=target, text=text(session, doc, target),
                    paragraph_style="Report Body")
    assert same_style.changed == []


def test_rule_4_the_runs_listed_are_capped_and_the_rest_counted(box, opened):
    session, doc = opened
    target = body(session, doc)[1]
    written = " ".join(f"**w{k}**" for k in range(45))
    result = ok(box, session, doc, target=target, text=written)
    assert len(result.data["styled"]) == 40 and result.data["left_out"] == 5
    assert any("5 more styled run(s)" in w for w in result.warnings)


def test_rule_6_the_template_owns_the_styles_one_it_lacks_added_as_word_writes_it(box):
    session = box.session()
    doc = session.open(word_docs.with_body('<w:p w14:paraId="2A000001" w14:textId="77777777"><w:r><w:t>Plain one.'
                                           "</w:t></w:r></w:p>"), "bare.docx")
    assert style_id(session, doc, "Strong", "character") is None
    target = body(session, doc)[0]
    result = ok(box, session, doc, target=target, text="Plain **one**.")
    assert "style added: Strong, as Word writes it" in result.warnings
    strong = style_id(session, doc, "Strong", "character")
    assert strong is not None and [(t, s) for t, s, _, _ in runs(session, doc, target) if s] == [("one", strong)]
    # No warning when the style is there, or for a role the text does not use.
    again = ok(box, session, doc, target=target, text="Plain **two**.")
    assert not any("style added" in w for w in again.warnings)


def test_lines_and_cells_take_the_style_and_the_emphasis(box, opened):
    session, doc = opened
    target = body(session, doc)[2]
    result = ok(box, session, doc, target=target, paragraph_style="Report Body", text="One **a**\nTwo *b*")
    new = result.created[0]
    assert style_name(session, doc, new) == "Report Body" == style_name(session, doc, target)
    assert [r["range"] for r in result.data["styled"]] == [f"{target}@4:5", f"{new}@4:5"]
    table = next(block for block, _ in word_docs.blocks(box, session, doc) if block.startswith("t:"))
    result = ok(box, session, doc, target=f"{table}/c1,1", text="**big** value\nsecond *line*")
    assert word_docs.table_texts(session, doc, table)[1][1] == "big value\nsecond line"
    assert [r["text"] for r in result.data["styled"]] == ["big", "line"]


def test_marks_beside_a_span_are_direct(box, opened):
    session, doc = opened
    target = body(session, doc)[1]
    result = ok(box, session, doc, target=target, text="A [**b**]{Score Mark} c.")
    assert result.data["styled"] == [{"range": f"{target}@2:3", "text": "b", "style": "Score Mark", "bold": True}]
    mark = style_id(session, doc, "Score Mark", "character")
    assert ("b", mark, True, False) in runs(session, doc, target)


def test_tracked_emphasis_is_a_formatting_revision_and_stays_valid(box, opened):
    session, doc = opened
    assert box.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Writer"}).ok
    target = body(session, doc)[1]
    ok(box, session, doc, target=target, text="The mean was **4.0/5** over the *year*.")
    assert element(session, doc, target).find(f".//{W}rPrChange") is not None
    assert word_docs.validation(box, session, doc)["new"] == []


def test_styled_text_lays_out_as_the_same_text_written_by_hand(box, made_once):
    """The layout check: set_text's result renders as a paragraph written with the same
    styles by hand (the text set plain, the character styles applied to ranges)."""
    plain = "Mean 2.0/5, below 3.0/5 in May."

    def drawn(session, doc):
        return [image.data for image in word_docs.call(box, session, "render", doc=doc, pages=[1], width=600).images]

    session = box.session()
    doc = session.open(made_once, "a.docx")
    target = body(session, doc)[1]
    ok(box, session, doc, target=target, paragraph_style="Report Body",
       text="Mean **2.0/5**, below *3.0/5* in [May]{Score Mark}.")
    written = drawn(session, doc)
    hand = box.session()
    other = hand.open(made_once, "b.docx")
    ok(box, hand, other, target=target, text="x")
    ok(box, hand, other, target=target, text=plain, paragraph_style="Report Body")
    assert [s for _, s, _, _ in runs(hand, other, target) if s] == []
    for span, style in (("5:10", "Strong"), ("18:23", "Emphasis"), ("27:30", "Score Mark")):
        word_docs.call(box, hand, "word_format", doc=other, targets=[f"{target}@{span}"], character_style=style)
    assert all_pixels(written) == all_pixels(drawn(hand, other))


def test_a_value_written_by_reference_inside_markup_is_kept_on_its_paragraph():
    with Toolbox([*TOOLS, ds_put], formats=[FORMAT, GRID], groups=GROUPS, workers=0) as toolbox:
        session = _session(toolbox)
        assert dispatch(toolbox, session, "ds_put", id="scores").ok
        target = word_docs.paragraphs(session, "d1")[0][0]
        for written in ("Gamma rated **{{d:scores/Gamma/rating}}** of [5]{Strong}.",
                        "[Gamma {{d:scores/Gamma/rating}}]{Strong} rated."):
            result = dispatch(toolbox, session, "word_set_text", doc="d1", target=target, text=written)
            assert result.ok and result.data["facts"][0]["text"] == "1.25" and result.data["styled"]
            kept = text_facts(session, "d1")[target]
            assert [(f["text"], text(session, "d1", target)[f["at"]:f["at"] + 4]) for f in kept] == \
                [("1.25", "1.25")]
