"""Charts (ROADMAP.md, "Charts and SmartArt"): data, titles and legend, the embedded workbook
kept in step, wherever Word holds a chart -- the body, a header, a footnote, a text box, a
group, floating -- through ``edit_chart``.

Every edit runs on every chart of the Word-saved fixtures (``crates/ooxml-oracle/fixtures/
generated/charts``, ``tools/make_chart_fixtures.py``) and goes through every gate:

* **round trip** -- edit, save, reopen, read back;
* **the workbook agrees** -- the saved document's embedded workbook opened independently
  (``tests/xlsx.py``, not ooxml-edit's reader): every formula's cells hold what its cache
  holds, tables cover the data and are named after their headers, shared-string counts are
  right;
* **validity** -- the check's validation, schema order in the chart part among them, adds
  nothing;
* **undo and redo** -- back to the original parts, workbook included, and on to the edited
  ones, part for part;
* **render** -- docx2svg draws the edited chart (its new label among the page's text) and
  warns of nothing new.
"""

from __future__ import annotations

import io
import posixpath
import re
import zipfile
from pathlib import Path

import pytest
from lxml import etree
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs
from xlsx import check_chart_against_workbook
from conftest import FIXTURE_DIR  # noqa: E402

CHARTS = FIXTURE_DIR / "generated" / "charts"
C = "{http://schemas.openxmlformats.org/drawingml/2006/chart}"
A = "{http://schemas.openxmlformats.org/drawingml/2006/main}"
WP = "{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}"
WPG = "{http://schemas.microsoft.com/office/word/2010/wordprocessingGroup}"
R_ID = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id"


def chart_fixtures() -> list[Path]:
    return sorted(CHARTS.glob("*.docx"))


def entries(data: bytes) -> dict[str, bytes]:
    """A package's parts, uncompressed, by name."""
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        return {name: package.read(name) for name in package.namelist()}


def _box() -> Toolbox:
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0)


def charts_of(box, session, doc: str = "d1") -> list[str]:
    """The charts ``describe`` lists, in its order."""
    return [d["id"] for d in word_docs.call(box, session, "describe", doc=doc).data.get("drawings", [])
            if d["kind"] == "chart"]


def _cases() -> list[tuple[str, str]]:
    out = []
    with _box() as box:
        for path in chart_fixtures():
            session = box.session()
            session.open(path.read_bytes(), path.name)
            out += [(path.name, chart) for chart in charts_of(box, session)]
    return out


CASES = _cases()


@pytest.fixture(scope="module")
def box():
    with _box() as toolbox:
        yield toolbox


def opened(box, data: bytes, name: str = "charts.docx"):
    session = box.session()
    session.open(data, name)
    return session


def chart(box, session, **arguments):
    return box.dispatch(session, "edit_chart", {"doc": "d1", **arguments})


def read(box, session, target: str) -> dict:
    result = chart(box, session, target=target, action="read")
    assert result.ok, result.to_json()
    return result.data


def chart_part(data: bytes, drawing: str) -> str:
    """The chart part of the drawing ``d:<id>`` (``d:<group>/<member>``), whichever story
    holds it."""
    number = drawing.partition(":")[2].rpartition("/")[2]
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        stories = [n for n in package.namelist() if re.fullmatch(r"word/[^/]+\.xml", n)]
    for story in stories:
        root = word_docs.tree(data, story)
        for named in root.iter(WP + "docPr", WPG + "cNvPr"):
            if named.get("id") != number:
                continue
            frame = named.getparent()
            found = next(frame.iter(C + "chart"), None)
            if found is None:
                continue
            folder, name = posixpath.split(story)
            rels = word_docs.tree(data, f"{folder}/_rels/{name}.rels")
            target = next(r.get("Target") for r in rels if r.get("Id") == found.get(R_ID))
            return posixpath.normpath(posixpath.join(folder, target))
    raise AssertionError(f"no chart {drawing}")


def check_document_charts(box, data: bytes, original: bytes | None = None) -> int:
    """Every chart in a saved document agrees with its embedded workbook; returns the
    formulas checked."""
    session = opened(box, data)
    parts = entries(data)
    before = entries(original) if original is not None else {}
    checked = 0
    for drawing in charts_of(box, session):
        workbook = (read(box, session, drawing).get("workbook") or {}).get("workbook")
        content = None if workbook is None else parts[workbook]
        checked += check_chart_against_workbook(parts[chart_part(data, drawing)], content,
                                                edited=workbook is None or before.get(workbook) != content)
    return checked


def test_the_fixtures_have_every_kind_and_place(box):
    kinds = {}
    for path in chart_fixtures():
        session = opened(box, path.read_bytes(), path.name)
        for drawing in charts_of(box, session):
            kinds.setdefault("+".join(read(box, session, drawing)["types"]), []).append(drawing)
    assert {"bar", "line", "pie", "doughnut", "scatter", "bar+line"} <= set(kinds)
    data = (CHARTS / "chart-places.docx").read_bytes()
    session = opened(box, data, "chart-places.docx")
    where = {drawing: _story_of(data, chart_part(data, drawing), drawing) for drawing in charts_of(box, session)}
    assert where == {"d:4": "word/document.xml", "d:5/7": "word/document.xml", "d:8": "word/document.xml",
                     "d:1": "word/header1.xml", "d:2": "word/footnotes.xml"}


def _story_of(data: bytes, part: str, drawing: str) -> str:
    """The story whose relationships name the chart part."""
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        for rels in package.namelist():
            if not rels.startswith("word/_rels/"):
                continue
            story = "word/" + posixpath.basename(rels)[:-len(".rels")]
            for rel in word_docs.tree(data, rels):
                if posixpath.normpath(posixpath.join("word", rel.get("Target", ""))) == part:
                    return story
    raise AssertionError(f"no story names {part} ({drawing})")


def test_every_original_chart_agrees_with_its_workbook(box):
    """The gate's own baseline: Word's fixtures are consistent before any edit."""
    for path in chart_fixtures():
        data = path.read_bytes()
        assert check_document_charts(box, data, data) >= 0


def test_reading(box):
    session = opened(box, (CHARTS / "charts.docx").read_bytes())
    first = read(box, session, "d:1")
    assert first["types"] == ["bar"] and first["title"] == "Sales by region"
    assert first["categories"] == ["Q1", "Q2", "Q3", "Q4"]
    assert [s["name"] for s in first["series"]] == ["North", "South", "West"]
    assert first["series"][0]["values"] == [4.3, 2.5, 3.5, 4.5]
    assert set(first["axes"]) == {"category", "value"} and first["legend"] is True
    workbook = first["workbook"]["workbook"]
    assert workbook.startswith("word/embeddings/") and workbook.endswith(".xlsx")
    assert read(box, session, "d:5")["categories"] == [0.7, 1.8, 2.6, 3.1, 4.4]
    assert read(box, session, "d:6")["types"] == ["bar", "line"]


def test_a_group_member_is_addressed_by_its_group_and_its_own_id(box):
    session = opened(box, (CHARTS / "chart-places.docx").read_bytes())
    assert read(box, session, "d:5/7")["series"][1]["name"] == "Paired"
    for target in ("d:5/99", "d:5", "d:5/6"):           # none, the group, a shape: not a chart
        result = chart(box, session, target=target, action="read")
        assert not result.ok, (target, result.to_json())


def test_describe_lists_a_chart_in_a_group_with_its_alt_text(box):
    """Round 59: a group member answers ``alt_text`` as a drawing does (its ``cNvPr/@descr``),
    so ``describe`` lists a chart held in a group."""
    data = (CHARTS / "chart-places.docx").read_bytes()
    root = word_docs.tree(data)
    member = next(e for e in root.iter(WPG + "cNvPr") if e.get("id") == "7")
    member.set("descr", "Members by year, paired")
    session = opened(box, word_docs.replaced(data, {"word/document.xml": etree.tostring(
        root, xml_declaration=True, encoding="UTF-8", standalone=True)}), "places.docx")
    result = box.dispatch(session, "describe", {"doc": "d1"})
    assert result.ok, result.to_json()
    drawings = {d["id"]: d for d in result.data["drawings"]}
    assert drawings["d:5/7"] == {"id": "d:5/7", "kind": "chart", "alt_text": "Members by year, paired"}


def test_a_chart_in_a_nested_group_is_addressed_down_the_groups(box):
    data = (CHARTS / "chart-places.docx").read_bytes()
    root = word_docs.tree(data)
    frame = next(root.iter(WPG + "graphicFrame"))
    nested = etree.SubElement(frame.getparent(), WPG + "grpSp")
    etree.SubElement(nested, WPG + "cNvPr", id="9", name="Group 9")
    nested.append(frame)
    session = opened(box, word_docs.replaced(data, {"word/document.xml": etree.tostring(
        root, xml_declaration=True, encoding="UTF-8", standalone=True)}), "nested.docx")
    assert charts_of(box, session) == ["d:4", "d:5/9/7", "d:8", "d:1", "d:2"]
    assert not chart(box, session, target="d:5/9", action="read").ok        # a group
    done = chart(box, session, target="d:5/9/7", action="set_value", series="0", category="0", value=99)
    assert done.ok, done.to_json()
    again = opened(box, word_docs.saved(session, "d1"))
    assert read(box, again, "d:5/9/7")["series"][0]["values"][0] == 99


def test_a_drawing_that_is_not_a_chart_is_refused(box):
    session = opened(box, (CHARTS / "smartart.docx").read_bytes())
    assert not chart(box, session, target="d:1", action="read").ok


# ------------------------------------------------------------------------------------------
# Each edit, through every gate
# ------------------------------------------------------------------------------------------


def _numeric(model) -> bool:
    return bool(model["categories"]) and all(not isinstance(c, str) for c in model["categories"])


def _label(value) -> str:
    return value if isinstance(value, str) else f"{value:g}"


def _edit_value(model):
    index = len(model["categories"]) - 1
    calls = [{"action": "set_value", "series": str(len(model["series"]) - 1), "category": str(index),
              "value": 1234.5}]
    return calls, lambda m: m["series"][-1]["values"][index] == 1234.5, "1234.5"


def _edit_blank(model):
    values = list(model["series"][0]["values"])
    values[1] = None
    return ([{"action": "set_values", "series": "0", "values": values}],
            lambda m: m["series"][0]["values"][1] is None, None)


def _edit_values(model):
    wanted = [float(k) + 0.5 for k in range(len(model["categories"]))]
    return ([{"action": "set_values", "series": "0", "values": wanted}],
            lambda m: m["series"][0]["values"] == wanted, None)


def _edit_category(model):
    label = 0.25 if _numeric(model) else "Relabelled"
    return ([{"action": "rename_category", "category": "0", "text": _label(label)}],
            lambda m: m["categories"][0] == label, None if _numeric(model) else "Relabelled")


def _edit_series_name(model):
    last = str(len(model["series"]) - 1)
    return ([{"action": "rename_series", "series": last, "text": "Renamed series"}],
            lambda m: m["series"][-1]["name"] == "Renamed series", "Renamed series")


def _edit_add_category(model):
    label = 9.75 if _numeric(model) else "Added"
    count = len(model["categories"])
    calls = [{"action": "add_category", "text": _label(label), "values": [7] * len(model["series"])}]
    return (calls, lambda m: len(m["categories"]) == count + 1 and m["categories"][-1] == label
            and all(s["values"][-1] == 7 for s in m["series"]), None if _numeric(model) else "Added")


def _edit_remove_category(model):
    second = model["categories"][1]
    return ([{"action": "remove_category", "category": "0"}], lambda m: m["categories"][0] == second, None)


def _edit_add_series(model):
    count = len(model["series"])
    calls = [{"action": "add_series", "text": "Added series", "values": [2] * len(model["categories"])}]
    # A pie's or doughnut's legend names its categories, not its series.
    shown = None if model["types"][0] in ("pie", "doughnut") else "Added series"
    return calls, lambda m: len(m["series"]) == count + 1 and m["series"][-1]["name"] == "Added series", shown


def _edit_remove_series(model):
    calls = []
    names = [s["name"] for s in model["series"]]
    if len(names) < 2:
        calls.append({"action": "add_series", "text": "Doomed", "values": [1] * len(model["categories"])})
        names.append("Doomed")
    calls.append({"action": "remove_series", "series": "0"})
    return calls, lambda m: [s["name"] for s in m["series"]] == names[1:], None


def _edit_title(model):
    return ([{"action": "set_title", "text": "A new title"}], lambda m: m["title"] == "A new title",
            "A new title")


def _edit_axis_title(model):
    if "value" not in model["axes"]:
        return ([{"action": "set_title", "text": "No axes"}], lambda m: m["title"] == "No axes", "No axes")
    return ([{"action": "set_axis_title", "axis": "value", "text": "Units"}],
            lambda m: m["axes"]["value"] == "Units", "Units")


def _edit_legend(model):
    shown = model["legend"]
    return ([{"action": "set_legend", "position": "none" if shown else "right"}],
            lambda m: m["legend"] is not shown, None)


EDITS = [_edit_value, _edit_blank, _edit_values, _edit_category, _edit_series_name, _edit_add_category,
         _edit_remove_category, _edit_add_series, _edit_remove_series, _edit_title, _edit_axis_title,
         _edit_legend]


def _applied(box, session, chart_id: str, edit):
    """The edit's calls made, each ok; what to read back, and the text it shows."""
    calls, read_back, shown = edit(read(box, session, chart_id))
    for arguments in calls:
        result = chart(box, session, target=chart_id, **arguments)
        assert result.ok, (arguments, result.to_json())
        assert not result.warnings, result.warnings      # every fixture chart has its workbook
    return len(calls), read_back, shown


def _svg_text(svgs: list[str]) -> str:
    out = []
    for svg in svgs:
        root = etree.fromstring(svg.encode())
        out += [" ".join("".join(n.itertext()).split()) for n in root.iter("{http://www.w3.org/2000/svg}text")]
    return "\n".join(out)


def test_a_value_axis_without_one_is_refused(box):
    """A chart with no value axis (a pie) refuses an axis title."""
    session = opened(box, (CHARTS / "charts.docx").read_bytes())
    pie = next(c for c in charts_of(box, session) if "value" not in read(box, session, c)["axes"])
    assert not chart(box, session, target=pie, action="set_axis_title", axis="value", text="Units").ok


@pytest.mark.parametrize("edit", EDITS, ids=lambda f: f.__name__[6:])
@pytest.mark.parametrize("fixture,chart_id", CASES, ids=[f"{f[:-5]}-{c}" for f, c in CASES])
def test_chart_edit(box, fixture, chart_id, edit):
    session = opened(box, (CHARTS / fixture).read_bytes(), fixture)
    data = word_docs.saved(session, "d1")
    before = word_docs.validation(box, session, "d1")
    steps, read_back, shown = _applied(box, session, chart_id, edit)
    edited = word_docs.saved(session, "d1")
    assert read_back(read(box, opened(box, edited), chart_id)), f"{edit.__name__} did not read back"
    assert check_document_charts(box, edited, data) > 0
    assert word_docs.validation(box, session, "d1")["new"] == before["new"]
    changed = {name for name, content in entries(edited).items() if entries(data).get(name) != content}
    assert changed and all(n.startswith(("word/charts/", "word/embeddings/")) for n in changed), changed
    # One undo step per call.
    undone = box.dispatch(session, "undo", {"doc": "d1", "steps": 50})
    assert undone.ok and undone.data["steps"] == steps, undone.to_json()
    assert entries(word_docs.saved(session, "d1")) == entries(data)
    assert box.dispatch(session, "undo", {"doc": "d1", "steps": 50, "redo": True}).ok
    assert entries(word_docs.saved(session, "d1")) == entries(edited)


#: Charts docx2svg does not draw. None since docx2svg draws a chart in a footnote (related
#: from the notes part) and one inline in a text box.
NOT_DRAWN: set = set()


def _numbers(text: str) -> list[float]:
    return [float(t) for t in re.findall(r"^-?\d+(?:\.\d+)?$", text, re.M)]


def _drawn(data: bytes) -> tuple[set, str]:
    """docx2svg's warnings and the text it draws, of a saved document."""
    import docx2svg

    layout = docx2svg.lay_out_docx(data)[0]
    warned = {getattr(w, "code", w) for w in layout.warnings}
    return warned, _svg_text(docx2svg.convert_docx_to_svg(data))


@pytest.mark.parametrize("edit", [_edit_value, _edit_category, _edit_add_series, _edit_title],
                         ids=lambda f: f.__name__[6:])
@pytest.mark.parametrize("fixture,chart_id", CASES, ids=[f"{f[:-5]}-{c}" for f, c in CASES])
def test_chart_edit_renders(box, fixture, chart_id, edit):
    """docx2svg draws the edited chart from its caches: a new label is on the page, a value
    axis reaches a new value, and nothing is warned that the unedited document was not."""
    session = opened(box, (CHARTS / fixture).read_bytes(), fixture)
    warned, _ = _drawn(word_docs.saved(session, "d1"))
    axes = "value" in read(box, session, chart_id)["axes"]
    _, _, shown = _applied(box, session, chart_id, edit)
    now, text = _drawn(word_docs.saved(session, "d1"))
    assert now <= warned
    if (fixture, chart_id) in NOT_DRAWN:
        assert shown is None or shown not in text  # pinned: the day docx2svg draws it, say so
    elif edit is _edit_value:
        assert not axes or max(_numbers(text)) >= 1234.5, (chart_id, sorted(_numbers(text))[-3:])
    elif shown is not None:
        assert shown in text, (shown, chart_id)


# ------------------------------------------------------------------------------------------
# Refusals, cache-only charts, tracking
# ------------------------------------------------------------------------------------------


def test_a_refused_edit_changes_nothing(box):
    session = opened(box, (CHARTS / "charts.docx").read_bytes())
    data = word_docs.saved(session, "d1")
    for arguments in ({"action": "set_value", "series": "0", "category": "0", "value": "not a number"},
                      {"action": "set_value", "series": "0", "category": "99", "value": 1}):
        assert not chart(box, session, target="d:1", **arguments).ok, arguments
    assert word_docs.saved(session, "d1") == data
    assert not box.dispatch(session, "undo", {"doc": "d1"}).ok          # nothing to undo


def _linked(data: bytes, chart_part_: str) -> bytes:
    """``data`` with the chart's workbook linked from outside the document."""
    rels = chart_part_.replace("charts/", "charts/_rels/") + ".rels"
    buffer = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as out:
        for info in source.infolist():
            content = source.read(info)
            if info.filename == rels:
                content = re.sub(rb'Target="[^"]+\.xlsx"', b'Target="https://example.org/book.xlsx" '
                                 b'TargetMode="External"', content)
            out.writestr(info, content)
    return buffer.getvalue()


def test_a_chart_whose_workbook_is_linked_is_edited_in_its_cache_with_a_warning(box):
    data = (CHARTS / "charts.docx").read_bytes()
    session = opened(box, _linked(data, chart_part(data, "d:2")))
    assert (read(box, session, "d:2").get("workbook") or {}).get("workbook") is None
    done = chart(box, session, target="d:2", action="set_value", series="0", category="0", value=5)
    assert done.ok, done.to_json()
    assert any("Word's Edit Data will show the old values" in w for w in done.warnings), done.warnings
    assert read(box, opened(box, word_docs.saved(session, "d1")), "d:2")["series"][0]["values"][0] == 5


def test_tracked_chart_edits_are_applied_untracked_with_a_warning(box):
    """Word has no revision for a chart's data (measured: its review counts none): tracked,
    the edit is applied, untracked, and a warning says so."""
    session = opened(box, (CHARTS / "charts.docx").read_bytes())
    assert box.dispatch(session, "word_set_tracking", {"doc": "d1", "on": True, "author": "Agent"}).ok
    done = chart(box, session, target="d:1", action="set_value", series="0", category="0", value=9)
    assert done.ok and done.warnings, done.to_json()
    edited = word_docs.saved(session, "d1")
    assert read(box, opened(box, edited), "d:1")["series"][0]["values"][0] == 9
    assert word_docs.revisions(session, "d1") == []
    assert box.dispatch(session, "word_set_tracking", {"doc": "d1", "on": False}).ok
    quiet = chart(box, session, target="d:1", action="set_value", series="0", category="0", value=10)
    assert quiet.ok and not quiet.warnings, quiet.to_json()   # untracked: no warning


# ------------------------------------------------------------------------------------------
# Reading: word_read
# ------------------------------------------------------------------------------------------


def test_word_read_summarises_each_chart_in_its_drawings_comment(box):
    session = opened(box, (CHARTS / "charts.docx").read_bytes())
    data = word_docs.saved(session, "d1")
    text = word_docs.read(box, session, "d1")
    assert ('d:1 chart "Chart 1" [chart: bar; title: Sales by region; series: North, South, West; '
            "categories: Q1, Q2, Q3, Q4]") in text
    assert "[chart: bar + line; series: Revenue, Cost, Margin; categories: Q1, Q2, Q3, Q4]" in text
    assert word_docs.saved(session, "d1") == data
    places = opened(box, (CHARTS / "chart-places.docx").read_bytes())
    text = word_docs.read(box, places, "d1", stories="all")
    assert '<!-- d:5 group "Group 5" [d:5/7 chart: bar; series: Grouped, Paired; categories: A, B, C] -->' in text
    assert "[chart: bar; series: Noted; categories: A, B, C]" in text  # in the footnote


def test_word_read_lists_a_diagrams_node_text(box):
    session = opened(box, (CHARTS / "smartart.docx").read_bytes())
    text = word_docs.read(box, session, "d1")
    assert "[smartart: Plan, Write, Measure, Review, Ship]" in text
    assert "[smartart: Goals [Faster edits, Fewer prompts], Risks [Stale caches, Lost formulas]]" in text


def test_a_title_in_a_comment_is_written_as_comment_text(box):
    """The summaries are inside comments: a title full of ``-->`` is written as comment text."""
    session = opened(box, (CHARTS / "charts.docx").read_bytes())
    assert chart(box, session, target="d:1", action="set_title", text="Sales --> up <!-- and -->").ok
    assert "Sales - -&gt; up" in word_docs.read(box, session, "d1")


# ------------------------------------------------------------------------------------------
# What a new title is written as, and in which language
# ------------------------------------------------------------------------------------------


def test_a_new_title_is_written_as_offices_chart_style_writes_one(box):
    """Word draws this form as its own titles look -- Aptos 14 pt, not bold, 595959 -- and
    saves it back unchanged; an empty ``a:defRPr`` Word draws bold at 18 pt
    (``crates/ooxml-oracle/word/probes/charts_probe.py``, ``titles``)."""
    session = opened(box, (CHARTS / "charts.docx").read_bytes())
    assert chart(box, session, target="d:2", action="set_title", text="Visitors and signups").ok
    assert chart(box, session, target="d:2", action="set_axis_title", axis="value", text="People").ok
    data = word_docs.saved(session, "d1")
    root = word_docs.tree(data, chart_part(data, "d:2"))
    title = root.find(f"{C}chart/{C}title")
    defaults = title.find(f".//{A}defRPr")
    assert (defaults.get("sz"), defaults.get("b")) == ("1400", "0")
    assert defaults.find(f"{A}latin").get("typeface") == "+mn-lt"
    assert [n.get("val") for n in defaults.iter(f"{A}lumMod", f"{A}lumOff")] == ["65000", "35000"]
    assert title.find(f".//{A}r/{A}rPr").get("lang") == "en-US"
    axis = root.find(f".//{C}valAx/{C}title")
    assert axis.find(f".//{A}bodyPr").get("rot") == "-5400000"
    assert axis.find(f".//{A}defRPr").get("sz") == "1000"


def test_new_chart_text_is_in_the_documents_default_language(box):
    W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

    def dutch(xml: str) -> str:
        root = etree.fromstring(xml.encode("utf-8"))
        root.find(f"{W}docDefaults/{W}rPrDefault/{W}rPr/{W}lang").set(f"{W}val", "nl-NL")
        return etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True).decode()

    session = opened(box, word_docs.replaced((CHARTS / "charts.docx").read_bytes(), {"word/styles.xml": dutch}))
    assert chart(box, session, target="d:4", action="set_title", text="Plan en werkelijk").ok
    data = word_docs.saved(session, "d1")
    title = word_docs.tree(data, chart_part(data, "d:4")).find(f"{C}chart/{C}title")
    assert title.find(f".//{A}r/{A}rPr").get("lang") == "nl-NL"
