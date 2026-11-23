"""Word itself on chart edits made through ``edit_chart``, opt-in (``pytest -m oracle``;
ROADMAP.md, "Charts and SmartArt"; SmartArt's ``edit_smartart`` is out of the Word tools
until its port).

* Every chart fixture with the edit set applied -- every chart's first value far above its
  axis, a category and a series renamed, a category and a series added, a title --
  **exports from Word unprompted**, its PDF showing the new labels and series names, and
  **each value axis reaching the new value**.
* **Word's chart text is docx2svg's**, by docx2svg's F.20 method: every span of text Word
  draws inside a chart -- string, face, weight, size on the device grid, the anchor of its
  advance box and its baseline within 0.5 pt -- against the text docx2svg's chart layout
  draws there, and whatever docx2svg draws that Word does not.
* **A Word re-save keeps** every chart's caches, formulas and embedded workbook (byte for
  byte), the independent workbook check holds on what Word wrote, and what Word does change
  is what was measured (``c:lang`` in its interface's language, a ``c16:uniqueId`` given to
  each series docx-agent added).
* Tracked, Word's review counts **no revision** for a chart edit, which its PDF shows.
"""

from __future__ import annotations

import ctypes
import re
import sys
from pathlib import Path

import pytest
from ooxml_edit.tools import Toolbox

import oracle
from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs
from test_charts import CHARTS, chart_part, charts_of, check_document_charts, entries, read

pytestmark = pytest.mark.oracle

ROOT = Path(__file__).resolve().parents[1]
DOCX2SVG_TOOLS = ROOT.parent / "docx2svg" / "tools"


@pytest.fixture(scope="module")
def word():
    try:
        with oracle.session() as session:
            yield session
    except oracle.WordBusy as busy:
        pytest.skip(str(busy))


def _text(pdf: Path) -> str:
    return "\n".join(oracle.pdf_pages(pdf))


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


# -- the edit set -------------------------------------------------------------------------------


def _call(box, session, target: str, **arguments):
    result = box.dispatch(session, "edit_chart", {"doc": "d1", "target": target, **arguments})
    assert result.ok, result.to_json()
    return result


def chart_edit_set(box, session) -> dict:
    """Every chart's edits, made with ``edit_chart``, and what each should show:
    ``{id: {labels, names, value}}``."""
    expected = {}
    for identifier in charts_of(box, session):
        model = read(box, session, identifier)
        tag = identifier.replace("d:", "").replace("/", "m")
        numeric = all(not isinstance(c, str) for c in model["categories"])
        largest = max(v for s in model["series"] for v in s["values"] if v is not None)
        value = round(largest * 10 + 7)
        _call(box, session, identifier, action="set_value", series="0", category="0", value=value)
        labels, names = [], []
        if not numeric:
            _call(box, session, identifier, action="rename_category", category="1", text=f"Kat{tag}")
            _call(box, session, identifier, action="add_category", text=f"Neu{tag}",
                  values=[3] * len(model["series"]))
            labels += [f"Kat{tag}", f"Neu{tag}"]
        _call(box, session, identifier, action="rename_series", series="0", text=f"Serie{tag}")
        points = len(read(box, session, identifier)["categories"])
        _call(box, session, identifier, action="add_series", text=f"Extra{tag}", values=[2] * points)
        pie = model["types"][0] in ("pie", "doughnut")
        if not pie:  # a pie's legend names its categories, not its series
            names += [f"Serie{tag}", f"Extra{tag}"]
        _call(box, session, identifier, action="set_title", text=f"Titel {tag}")
        expected[identifier] = {"labels": labels, "names": names, "title": f"Titel {tag}",
                                "value": None if pie else value}
    return expected


def edited(box, name: str) -> tuple[bytes, dict]:
    """The fixture with the edit set applied: its bytes and what it should show."""
    session = box.session()
    session.open((CHARTS / name).read_bytes(), name)
    expected = chart_edit_set(box, session)
    return word_docs.saved(session, "d1"), expected


def _numbers(text: str) -> list[float]:
    """Every number Word drew on a line of its own (axis labels), in its own notation
    (``1.200`` and ``1,5`` in a Dutch interface)."""
    out = []
    for line in text.splitlines():
        for token in line.split():
            token = token.replace(".", "").replace(",", ".") if re.fullmatch(r"-?\d{1,3}(\.\d{3})+", token) \
                else token.replace(",", ".")
            if re.fullmatch(r"-?\d+(\.\d+)?", token):
                out.append(float(token))
    return out


CHART_FIXTURES = ["charts.docx", "chart-places.docx"]


def _extent(data: bytes, identifier: str) -> tuple[float, float]:
    """A chart's frame in pt: its drawing's ``wp:extent``, or its group member's ``a:ext``."""
    WP = "{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}"
    WPG = "{http://schemas.microsoft.com/office/word/2010/wordprocessingGroup}"
    A = "{http://schemas.openxmlformats.org/drawingml/2006/main}"
    number = identifier.partition(":")[2].rpartition("/")[2]
    import io
    import zipfile

    with zipfile.ZipFile(io.BytesIO(data)) as package:
        stories = [n for n in package.namelist() if re.fullmatch(r"word/[^/]+\.xml", n)]
    for story in stories:
        for named in word_docs.tree(data, story).iter(WP + "docPr", WPG + "cNvPr"):
            if named.get("id") == number:
                frame = named.getparent()
                extent = frame.find(WP + "extent")
                if extent is None:
                    extent = next(frame.iter(A + "ext"))
                return int(extent.get("cx")) / 12700, int(extent.get("cy")) / 12700
    raise AssertionError(f"no drawing {identifier}")


def _in_chart(pages: list[list[tuple]], title: str, size: tuple[float, float]) -> list[tuple]:
    """Word's spans inside a chart, found by its (unique) title: centred at the top of a
    frame of the chart's size."""
    for spans in pages:
        found = next((s for s in spans if s[0] == title), None)
        if found is not None:
            middle, top = (found[3] + found[4]) / 2, found[5] - 30
            return [s for s in spans if middle - size[0] / 2 - 1 <= s[3] and s[4] <= middle + size[0] / 2 + 1
                    and top <= s[5] <= top + size[1] + 1]
    return []


@pytest.mark.parametrize("name", CHART_FIXTURES)
def test_word_exports_every_chart_edit_and_shows_it(word, box, name):
    """Every chart shows its new labels, series names and title, and its value axis reaches
    its new value -- read inside each chart, wherever Word put it."""
    data, expected = edited(box, name)
    outcome = word.export_pdf(data, name=f"charts-oracle-{Path(name).stem}")
    assert outcome, f"Word did not export {name}: {outcome.outcome} {outcome.detail}"
    pages = _word_spans(outcome.path)
    for identifier, shows in expected.items():
        inside = _in_chart(pages, shows["title"], _extent(data, identifier))
        texts = [s[0] for s in inside]
        for label in shows["labels"] + shows["names"]:
            assert label in texts, (identifier, label, texts)
        if shows["value"] is not None:
            numbers = _numbers("\n".join(texts))
            assert numbers and max(numbers) >= shows["value"], (identifier, shows["value"], numbers)


# -- Word's chart text against docx2svg's (F.20) --------------------------------------------------


def _word_spans(pdf: Path) -> list[list[tuple]]:
    """Per page, every run of characters Word drew on one baseline in one face and size, split
    where a gap opens: ``(text, font, size, left, right, baseline)`` in pt from the top left."""
    import pypdfium2
    import pypdfium2.raw as raw

    pages = []
    document = pypdfium2.PdfDocument(str(pdf))
    try:
        for page in document:
            height = page.get_height()
            textpage = page.get_textpage()
            chars = []
            for index in range(textpage.count_chars()):
                code = raw.FPDFText_GetUnicode(textpage.raw, index)
                if not code or chr(code).isspace():
                    chars.append(None)
                    continue
                box = raw.FS_RECTF()
                raw.FPDFText_GetLooseCharBox(textpage.raw, index, box)
                x, y = ctypes.c_double(), ctypes.c_double()
                raw.FPDFText_GetCharOrigin(textpage.raw, index, x, y)
                buffer = ctypes.create_string_buffer(256)
                flags = ctypes.c_int()
                raw.FPDFText_GetFontInfo(textpage.raw, index, buffer, 256, flags)
                matrix = raw.FS_MATRIX()
                raw.FPDFText_GetMatrix(textpage.raw, index, matrix)
                scale = (matrix.a ** 2 + matrix.b ** 2) ** 0.5 or 1.0
                size = round(raw.FPDFText_GetFontSize(textpage.raw, index) * scale, 2)
                chars.append((chr(code), buffer.value.decode("latin-1"), size, box.left, box.right,
                              round(height - y.value, 2)))
            spans: list[tuple] = []
            current, spaced = None, False
            for char in chars + [None, None]:
                if char is None:  # a space, or the page's end: a gap that may still join
                    if spaced and current is not None:
                        spans.append(current)
                        current = None
                    spaced = True
                    continue
                same = (current is not None and char[1] == current[1] and char[2] == current[2]
                        and abs(char[5] - current[5]) < 0.5)
                if same and char[3] - current[4] < (0.6 if spaced else 0.25) * char[2]:
                    current = (current[0] + (" " if spaced else "") + char[0], current[1], current[2], current[3],
                               char[4], current[5])
                else:
                    if current is not None:
                        spans.append(current)
                    current = char
                spaced = False
            pages.append(spans)
    finally:
        document.close()
    return pages


def _model(data: bytes):
    """docx2svg's charts, page by page: each chart's texts and frame (the chart space's fill),
    in pt -- docx2svg's ``tools/read_chart_probe.py``."""
    sys.path.insert(0, str(DOCX2SVG_TOOLS))
    import docx2svg
    import read_chart_probe as probe

    layout = docx2svg.convert_docx(data, docx2svg.ConvertOptions()).layout
    charts = []
    for page in layout.pages:
        for placed in page.floats:
            for primitive in placed.primitives:
                if primitive.kind == "markup" and primitive.what == "chart":
                    found = probe.model_chart(primitive.markup, primitive.transform)
                    boxes = [f[1:] for f in found["fills"]]
                    frame = max(boxes, key=lambda b: (b[2] - b[0]) * (b[3] - b[1]))
                    charts.append((found["texts"], frame))
    return charts, probe


def compare_chart_text(pdf: Path, data: bytes, titles: dict[str, str]) -> tuple[list[int], list, list[str]]:
    """``[agree, compared]`` over the text of every chart docx2svg draws, what disagreed, and
    the charts (by id) Word draws and docx2svg does not -- F.20's method, each chart's text
    taken relative to its title (``titles``: id -> its unique title), so that where the page
    puts a chart, which is the page layout's business and not the chart's, does not count."""
    model, probe = _model(data)
    pages = _word_spans(pdf)
    agree = compared = 0
    problems = []
    undrawn = []
    for identifier, title in titles.items():
        spans = next((page for page in pages if any(s[0] == title for s in page)), [])
        word_title = next((s for s in spans if s[0] == title), None)
        drawn = next(((texts, frame) for texts, frame in model if any(t[0] == title for t in texts)), None)
        if word_title is None:
            problems.append((identifier, "not drawn by Word"))
            continue
        if drawn is None:
            undrawn.append(identifier)
            continue
        texts, frame = drawn
        mine = next(t for t in texts if t[0] == title)
        dx0 = (word_title[3] + word_title[4]) / 2 - mine[4]
        dy0 = word_title[5] - mine[5]
        inside = [s for s in spans if frame[0] - 1 + dx0 <= s[3] and s[4] <= frame[2] + 1 + dx0
                  and frame[1] - 1 + dy0 <= s[5] <= frame[3] + 1 + dy0]
        used = set()
        for text, font, size, left, right, baseline in inside:
            # Word writes a fraction with its host's decimal separator (``0,5`` here); docx2svg
            # does not model the host's locale (its F.20, "Not measured").
            text = re.sub(r"(?<=\d),(?=\d)", ".", text)
            family, bold = probe.family_of(font)
            candidates = [(k, t) for k, t in enumerate(texts) if k not in used and t[0] == text]
            compared += 1
            if not candidates:
                problems.append((identifier, text, "not drawn by docx2svg"))
                continue

            def anchor(t):
                return (left if t[3] == "start" else right if t[3] == "end" else (left + right) / 2) - dx0

            k, best = min(candidates, key=lambda c: abs(anchor(c[1]) - c[1][4]) + abs(baseline - dy0 - c[1][5]))
            used.add(k)
            face = best[1].replace(" ", "").lower() == family.replace(" ", "").lower() and best[7] == bold
            dx, dy = best[4] - anchor(best), best[5] - (baseline - dy0)
            if face and abs(probe.device(best[2]) - size) < 0.06 and abs(dx) <= 0.5 and abs(dy) <= 0.5:
                agree += 1
            else:
                problems.append((identifier, text, f"{family}{' bold' if bold else ''} {size} vs {best[1]}"
                                 f"{' bold' if best[7] else ''} {round(probe.device(best[2]), 2)}, "
                                 f"dx {dx:+.2f} dy {dy:+.2f}"))
        extra = [t for k, t in enumerate(texts) if k not in used]
        compared += len(extra)
        problems += [(identifier, t[0], "drawn by docx2svg, not by Word") for t in extra]
    return [agree, compared], problems, undrawn


#: Charts docx2svg does not draw (test_charts.NOT_DRAWN; proposed to it): the footnote's and
#: the text box's.
UNDRAWN = {"charts.docx": [], "chart-places.docx": ["d:4", "d:2"]}
#: What the comparison misses, by chart (proposed to docx2svg): the header's chart, a short
#: one (126 pt by 63 pt), where docx2svg sets the category labels 1 pt lower against the
#: title than Word and draws a value axis label, ``0``, Word leaves out -- five texts.
KNOWN = {"charts.docx": {}, "chart-places.docx": {"d:1": 5}}


@pytest.mark.parametrize("name", CHART_FIXTURES)
def test_words_chart_text_is_docx2svgs(word, box, name):
    """Every chart docx2svg draws, its every text as Word draws it: 85 of 85 in the body's six
    charts, all of the group's and the floating one's; the header's, but for :data:`KNOWN`."""
    if not (DOCX2SVG_TOOLS / "read_chart_probe.py").exists():
        pytest.skip("docx2svg's checkout is not beside this one")
    data, expected = edited(box, name)
    outcome = word.export_pdf(data, name=f"charts-oracle-{Path(name).stem}")
    assert outcome
    titles = {identifier: shows["title"] for identifier, shows in expected.items()}
    (agree, compared), problems, undrawn = compare_chart_text(outcome.path, data, titles)
    assert undrawn == UNDRAWN[name]
    missed: dict[str, int] = {}
    for problem in problems:
        missed[problem[0]] = missed.get(problem[0], 0) + 1
    assert compared > 20 and missed == KNOWN[name], problems
    assert compared - agree == sum(KNOWN[name].values())


# -- re-save -----------------------------------------------------------------------------------


@pytest.mark.parametrize("name", CHART_FIXTURES)
def test_a_word_resave_keeps_caches_formulas_and_workbooks(word, box, name):
    data, _ = edited(box, name)
    outcome = word.resave(data, name=f"charts-oracle-{Path(name).stem}")
    assert outcome, f"Word did not save {name}: {outcome.outcome} {outcome.detail}"
    saved_data = outcome.path.read_bytes()
    ours, saved = box.session(), box.session()
    ours.open(data, name)
    saved.open(saved_data, name)
    assert charts_of(box, saved) == charts_of(box, ours)
    C = "{http://schemas.openxmlformats.org/drawingml/2006/chart}"
    ours_parts, saved_parts = entries(data), entries(saved_data)
    for identifier in charts_of(box, ours):
        mine, theirs = read(box, ours, identifier), read(box, saved, identifier)
        assert {k: theirs[k] for k in ("categories", "series", "title")} == \
            {k: mine[k] for k in ("categories", "series", "title")}, identifier
        root_ours = word_docs.tree(data, chart_part(data, identifier))
        root_theirs = word_docs.tree(saved_data, chart_part(saved_data, identifier))
        assert [f.text for f in root_theirs.iter(C + "f")] == [f.text for f in root_ours.iter(C + "f")]
        book = mine["workbook"]["workbook"]
        assert saved_parts[theirs["workbook"]["workbook"]] == ours_parts[book], identifier
        # What Word changes, as measured: its interface's language, and an id for each series
        # added (none was written).
        ids = [n.get("val") for n in root_theirs.iter() if isinstance(n.tag, str) and n.tag.endswith("}uniqueId")]
        assert len(ids) == len(theirs["series"]) and len(set(ids)) == len(ids)
        assert root_theirs.find(C + "lang").get("val") == "nl-NL"  # this Word's interface
    assert check_document_charts(box, saved_data, saved_data) > 0
    assert set(saved_parts) >= {n for n in ours_parts if n.startswith(("word/charts/chart",))}


# -- tracking ----------------------------------------------------------------------------------


#: Word counts the revisions and says whether it tracks (once ``crates/ooxml-oracle/word/probes/charts_probe.py``).
_COUNT_SCRIPT = '''
on run argv
  set inputPath to item 1 of argv
  set outputPath to item 2 of argv
  with timeout of 170 seconds
    tell application "Microsoft Word"
      activate
      open (POSIX file inputPath)
      set d to active document
      set n to count of revisions of d
      set t to track revisions of d
      save as d file name outputPath file format format document
      close d saving no
      quit saving no
    end tell
  end timeout
  return "revisions=" & n & " tracking=" & t
end run
'''


def test_word_counts_no_revision_for_tracked_chart_edits(word, box):
    session = box.session()
    session.open((CHARTS / "charts.docx").read_bytes(), "charts.docx")
    assert box.dispatch(session, "word_set_tracking", {"doc": "d1", "on": True, "author": "Oracle"}).ok
    _call(box, session, "d:1", action="set_value", series="0", category="0", value=66)
    _call(box, session, "d:1", action="add_category", text="Tracked", values=[1, 2, 3])
    data = word_docs.saved(session, "d1")
    for stale in oracle.ORACLE_DIR.glob("charts-oracle-tracked-*-counted.docx"):
        stale.unlink()
    counted = word.run_script(_COUNT_SCRIPT, data, name="charts-oracle-tracked", tag="counted")
    assert counted, counted.detail
    assert "revisions=0" in counted.detail and "tracking=true" in counted.detail
    pdf = word.export_pdf(data, name="charts-oracle-tracked")
    assert pdf and "Tracked" in _text(pdf.path)
