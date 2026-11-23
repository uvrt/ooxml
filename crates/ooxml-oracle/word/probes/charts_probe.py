#!/usr/bin/env python3
"""Measure what Word does with charts and SmartArt the tools wrote or edited.

Probe documents are made here through the tools (``probe_tools``: ``new_document``,
``edit_chart``), some then given part forms no tool writes by rewriting their chart or
diagram parts, or edited from the committed fixtures (``crates/ooxml-oracle/fixtures/generated/charts``),
and Word -- through ``python/docx-agent/tests/oracle.py``'s machine-wide lock, no clipboard -- exports
each to PDF and saves it again.  What it drew is read off the PDF (text, and the face,
size, weight and colour of a title's characters), and what it wrote off the saved file:

* ``cache`` -- a chart whose caches disagree with its embedded workbook (the cache says
  ``Q1``... ``Cached``, 10 to 40; the workbook ``W1``... ``Booked``, 900s), with no
  ``c:autoUpdate``, with ``0`` and with ``1``: which Word draws, and whether a save
  refreshes the cache from the workbook;
* ``extensions`` -- ``charts.docx`` with every chart edited through ``edit_chart`` (a value, a
  category and a series added, a category and a series renamed, a title): the ``c14``,
  ``c16r2``, ``c16r3`` and ``c16:uniqueId`` extensions after the tools' edit and after Word's
  save, and the workbooks;
* ``titles`` -- one chart per title form: a paragraph with no ``a:defRPr``, an empty one, the
  tools' ``set_title`` form, a title with no text (``c:title`` alone), and an axis title and
  a title in the tools' form: how Word draws each, and what it writes back;
* ``tracking`` -- ``charts.docx`` with tracking on and the chart edits made tracked: the
  revisions Word's own review counts, and what its PDF shows;
* ``places`` -- ``chart-places.docx`` with the chart in the header, the footnote, the
  text box, the group and the floating one each edited: Word's PDF, and the ids Word keeps
  (``docPr``, the group member's ``wpg:cNvPr``);
* ``smartart`` -- variants of ``smartart.docx``: the data model's text changed with the drawing
  stale, and the drawing's text alone changed: what Word's PDF shows, and whether a save
  rewrites the drawing;
* ``dictionary`` -- Word's AppleScript dictionary (``sdef``) searched for chart data,
  Edit Data or SmartArt objects (no Word needed).

What it found is written to ``crates/ooxml-oracle/word/observations/charts-word.json``: facts read off Word's
PDFs and files, never the files themselves.

    python crates/ooxml-oracle/word/probes/charts_probe.py              # every probe, the observations written
    python crates/ooxml-oracle/word/probes/charts_probe.py titles       # one probe, printed

Changed since docx-agent 0.2.0 (no Python document API): every edit is the tools'; a chart's
``data`` is what ``edit_chart`` ``read`` gives; ``docx_agent_warns`` holds the tools' warnings.
Dropped, as they measured forms only docx-agent's Python wrote: the ``titles`` probe's
ooxml-edit axis-title template (``Plain ...``), the ``tracking`` probe's SmartArt edits and the
``smartart`` variants made through the diagram API (``drawing-missing``, ``added-*``,
``removed-*``, ``text-exact``, ``top-added-dropped``): no tool edits SmartArt.  The recorded
observation keeps them until a re-run.

Nothing Word writes is committed: only these facts.
"""

from __future__ import annotations

import io
import json
import re
import subprocess
import sys
import zipfile
from pathlib import Path

from lxml import etree

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402  (the driver, the fixture makers and the cases on the path)
from probe_paths import ROOT  # noqa: E402

import oracle  # noqa: E402
import probe_tools  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "charts-word.json"
FIXTURES = probe_paths.FIXTURES / "generated" / "charts"
C = "{http://schemas.openxmlformats.org/drawingml/2006/chart}"
A = "{http://schemas.openxmlformats.org/drawingml/2006/main}"
DGM = "{http://schemas.openxmlformats.org/drawingml/2006/diagram}"
DSP = "{http://schemas.microsoft.com/office/drawing/2008/diagram}"
WPG = "{http://schemas.microsoft.com/office/word/2010/wordprocessingGroup}"
WP = "{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}"


def parts(data: bytes) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return {name: archive.read(name) for name in archive.namelist()}


def replaced(data: bytes, edits: dict[str, object]) -> bytes:
    """``data`` with each named part's text changed by its function."""
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            content = source.read(info.filename)
            if info.filename in edits:
                content = edits[info.filename](content.decode("utf-8")).encode("utf-8")
            target.writestr(info, content)
    return out.getvalue()


def chart_ids(session: probe_tools.Session) -> list[str]:
    """The document's charts' ids, as ``describe`` lists them."""
    return [d["id"] for d in session.call("describe").data["drawings"] if d.get("kind") == "chart"]


def chart_parts(data: bytes) -> dict[str, str]:
    """Each chart's part by its id (through its workbook's relationship)."""
    session = probe_tools.Session(data)
    files = parts(data)
    out = {}
    for identifier in chart_ids(session):
        workbook = (session.call("edit_chart", target=identifier, action="read").data.get("workbook") or {})
        name = (workbook.get("workbook") or "").rsplit("/", 1)[-1]
        out[identifier] = next((rels.replace("/_rels/", "/").removesuffix(".rels") for rels, content in files.items()
                                if rels.startswith("word/charts/_rels/") and name and name.encode() in content), None)
    return out


def charts_of(data: bytes) -> list[dict]:
    """Every chart in a saved document, in ``describe``'s order: its id, data, caches and extensions."""
    import hashlib

    session = probe_tools.Session(data)
    files = parts(data)
    out = []
    for identifier, part in chart_parts(data).items():
        read = session.call("edit_chart", target=identifier, action="read").data
        root = etree.fromstring(files[part]) if part else etree.Element(C + "chartSpace")
        series = []
        for entry, ser in zip(read["series"], root.iter(C + "ser")):
            series.append({
                "name": entry["name"],
                "unique_id": next((n.get("val") for n in ser.iter() if isinstance(n.tag, str)
                                   and n.tag.endswith("}uniqueId")), None),
                "ext_uris": [e.get("uri") for e in ser.findall(f"{C}extLst/{C}ext")],
            })
        workbook = (read.get("workbook") or {}).get("workbook")
        out.append({
            "id": identifier, "part": part,
            "data": {k: read.get(k) for k in ("type", "title", "categories", "series")}, "series": series,
            "c14_style": root.find(".//{http://schemas.microsoft.com/office/drawing/2007/8/2/chart}style") is not None,
            "c16r2_declared": "http://schemas.microsoft.com/office/drawing/2015/06/chart" in root.nsmap.values(),
            "c16r3": root.find(".//{http://schemas.microsoft.com/office/drawing/2017/03/chart}dataDisplayOptions16")
            is not None,
            "lang": (root.find(C + "lang").get("val") if root.find(C + "lang") is not None else None),
            "auto_update": (root.find(f"{C}externalData/{C}autoUpdate").get("val")
                            if root.find(f"{C}externalData/{C}autoUpdate") is not None else None),
            "workbook": workbook,
            "workbook_sha": hashlib.sha256(files[workbook]).hexdigest()[:16] if workbook in files else None,
            "formulas": [f.text for f in root.iter(C + "f")],
        })
    return out


def added_charts(title: str, charts: list[tuple[str, list[str], dict[str, list]]]) -> probe_tools.Session:
    """A new document of a paragraph ``title`` and, per chart, a paragraph naming it and the
    chart (a column chart, no legend) the tools add after it."""
    session = probe_tools.Session(probe_tools.new())
    last = session.paragraph_ids()[0]
    session.call("word_set_text", target=last, text=title)
    names = [label for label, _, _ in charts]
    made = session.call("word_insert_markdown", at=f"after:{last}",
                        markdown="\n\n".join(names) + "\n").data["blocks"]
    for block, (_, categories, data) in zip(made, charts):
        session.call("edit_chart", target=block, action="add", chart_type="column", categories=categories,
                     data=[{"name": name, "values": values} for name, values in data.items()], width=288,
                     position="none")
    return session


def pdf_text(path: Path) -> str:
    return "\n".join(oracle.pdf_pages(path))


def chars_of(path: Path, text: str) -> dict | None:
    """How Word drew ``text`` (its first occurrence): face, size, weight, colour."""
    found = oracle.find_in_pdf(oracle.pdf_chars(path), text)
    if not found:
        return None
    first = found[0]
    return {"font": first.font.split("+", 1)[-1], "size": first.size, "weight": first.weight,
            "color": "%02X%02X%02X" % first.color, "stroked": first.stroked}


# -- cache against workbook ------------------------------------------------------------------------


def cache_document() -> bytes:
    """Three charts whose caches (``Q1``... ``Cached<k>``, 10 to 40) disagree with their
    workbooks (``W1``... ``Booked<k>``, 900s): the tools add each from the workbook's data, then
    the caches are rewritten here, and ``c:autoUpdate`` left out, ``0`` and ``1``."""
    updates = (None, "0", "1")
    session = added_charts("Cache against workbook.", [
        (f"autoUpdate {update}.", ["W1", "W2", "W3", "W4"], {f"Booked{k}": [900, 910, 920, 930]})
        for k, update in enumerate(updates, start=1)])
    data = session.data()
    edits = {}
    for k, (identifier, part) in enumerate(chart_parts(data).items(), start=1):
        update = updates[k - 1]

        def cached(xml: str, k: int = k, update: str | None = update) -> str:
            for w, q in zip(("W1", "W2", "W3", "W4"), ("Q1", "Q2", "Q3", "Q4")):
                xml = xml.replace(f"<c:v>{w}</c:v>", f"<c:v>{q}</c:v>")
            for booked, cache in zip(("900", "910", "920", "930"), ("10", "20", "30", "40")):
                xml = xml.replace(f"<c:v>{booked}</c:v>", f"<c:v>{cache}</c:v>")
            xml = xml.replace(f"<c:v>Booked{k}</c:v>", f"<c:v>Cached{k}</c:v>")
            auto = "" if update is None else f'<c:autoUpdate val="{update}"/>'
            return re.sub(r"(<c:externalData [^>]*>)(?:<c:autoUpdate [^>]*/>)?", rf"\g<1>{auto}", xml)

        edits[part] = cached
    return replaced(data, edits)


def probe_cache(word: oracle.Session) -> dict:
    data = cache_document()
    pdf = word.export_pdf(data, name="charts-cache")
    saved = word.resave(data, name="charts-cache")
    text = pdf_text(pdf.path) if pdf else ""
    out = {"exported": bool(pdf), "saved": bool(saved)}
    for k in (1, 2, 3):
        out[f"chart{k}"] = {
            "pdf_shows_cache_name": f"Cached{k}" in text, "pdf_shows_workbook_name": f"Booked{k}" in text,
        }
    out["pdf_shows_cache_categories"] = all(q in text for q in ("Q1", "Q4"))
    out["pdf_shows_workbook_categories"] = any(w in text for w in ("W1", "W4"))
    out["pdf_value_labels"] = sorted(set(re.findall(r"\b\d{2,4}\b", text)) - {"0"})
    if saved:
        out["saved_charts"] = [{"data": c["data"], "auto_update": c["auto_update"], "workbook_sha": c["workbook_sha"]}
                               for c in charts_of(saved.path.read_bytes())]
        out["input_workbooks"] = [c["workbook_sha"] for c in charts_of(data)]
    return out


# -- extensions ------------------------------------------------------------------------------------


def edited_charts(tracking: bool = False) -> tuple[bytes, list[str]]:
    """``charts.docx`` with every chart edited through ``edit_chart`` (tracked: by ``Probe``,
    tracking on), and the tools' warnings."""
    session = probe_tools.Session((FIXTURES / "charts.docx").read_bytes())
    if tracking:
        session.call("word_set_tracking", on=True, author="Probe")
    warned: list[str] = []
    for identifier in chart_ids(session):
        read = session.call("edit_chart", target=identifier, action="read").data
        numeric = read.get("types") == ["scatter"]
        first = read["series"][0]["name"]
        for arguments in ({"action": "set_value", "series": "0", "category": "0", "value": 777},
                          {"action": "add_category", "text": "9.5" if numeric else "Added",
                           "values": [None] * len(read["series"])},
                          {"action": "add_series", "text": "New series", "values": [5] * (len(read["categories"]) + 1)},
                          {"action": "rename_category", "category": "1", "text": "2.2" if numeric else "Renamed"},
                          {"action": "rename_series", "series": "0", "text": f"{first} edited"},
                          {"action": "set_title", "text": f"Edited {identifier}"}):
            warned += session.call("edit_chart", target=identifier, **arguments).to_json().get("warnings", [])
    return session.data(), [str(w) for w in warned]


def probe_extensions(word: oracle.Session) -> dict:
    data, _ = edited_charts()
    pdf = word.export_pdf(data, name="charts-extensions")
    saved = word.resave(data, name="charts-extensions")
    out = {"exported": bool(pdf), "ours": charts_of(data)}
    if saved:
        out["saved"] = charts_of(saved.path.read_bytes())
    if pdf:
        text = pdf_text(pdf.path)
        out["pdf_has"] = {label: label in text for label in ("Added", "Renamed", "New series", "777", "Edited d:1")}
    return out


# -- titles ----------------------------------------------------------------------------------------


def _rich(text: str, ppr: str, *, vertical: bool = False) -> str:
    rot = ' rot="-5400000" vert="horz"' if vertical else ""
    return (f"<c:tx><c:rich><a:bodyPr{rot}/><a:lstStyle/><a:p>{ppr}<a:r><a:rPr lang=\"en-US\"/><a:t>{text}</a:t>"
            "</a:r></a:p></c:rich></c:tx>")


#: The title forms, each chart's series named ``Series <form>``.
TITLE_FORMS = ("none", "empty", "word", "auto")


def titles_document() -> bytes:
    """A chart per title form: the tools add each and set its title (``set_title``: the
    ``word`` form), then the title's text is rewritten here: a paragraph with no ``a:defRPr``
    (``none``), an empty one (``empty``), and none at all (``auto``, ``c:title`` alone)."""
    session = added_charts("Titles.", [(f"Title {key}.", ["A", "B"], {f"Series {key}": [1, 2]})
                                       for key in TITLE_FORMS])
    for identifier in chart_ids(session):
        session.call("edit_chart", target=identifier, action="set_title", text="Word style")
    data = session.data()
    forms = {"none": _rich("No defaults", ""), "empty": _rich("Empty defaults", "<a:pPr><a:defRPr/></a:pPr>"),
             "auto": ""}
    edits = {}
    for key, part in zip(TITLE_FORMS, chart_parts(data).values()):
        if key in forms:
            edits[part] = lambda xml, tx=forms[key]: re.sub(r"(<c:title>)<c:tx>.*?</c:tx>", rf"\g<1>{tx}", xml,
                                                             count=1, flags=re.S)
    return replaced(data, edits)


def axis_titles_document() -> bytes:
    """A chart whose value axis and chart get a title through the tools."""
    session = added_charts("Axis titles.", [("Chart 2.", ["A", "B"], {"S2": [1, 2]})])
    identifier = chart_ids(session)[0]
    session.call("edit_chart", target=identifier, action="set_axis_title", axis="value", text="Styled axis")
    session.call("edit_chart", target=identifier, action="set_title", text="Styled title")
    return session.data()


def _saved_titles(data: bytes) -> list[dict]:
    session = probe_tools.Session(data)
    files = parts(data)
    out = []
    for identifier, part in chart_parts(data).items():
        root = etree.fromstring(files[part])
        title = root.find(f"{C}chart/{C}title")
        axis = root.find(f".//{C}valAx/{C}title")
        out.append({"id": identifier,
                    "title": session.call("edit_chart", target=identifier, action="read").data.get("title"),
                    "title_xml": None if title is None else re.sub(r' xmlns:\w+="[^"]+"', "",
                                                                    etree.tostring(title).decode()),
                    "axis_xml": None if axis is None else re.sub(r' xmlns:\w+="[^"]+"', "",
                                                                  etree.tostring(axis).decode())})
    return out


def docx2svg_title(data: bytes, text: str) -> dict | None:
    """How docx2svg draws ``text``: the ``<text>`` element's face, size and weight."""
    import docx2svg

    for svg in docx2svg.convert_docx_to_svg(data, docx2svg.ConvertOptions()):
        root = etree.fromstring(svg.encode())
        for node in list(root.iter("{http://www.w3.org/2000/svg}tspan")) + list(root.iter("{http://www.w3.org/2000/svg}text")):
            if " ".join("".join(node.itertext()).split()) == text:
                found = {}
                current = node
                while current is not None:
                    for key in ("font-family", "font-size", "font-weight", "fill"):
                        if key not in found and current.get(key) is not None:
                            found[key] = current.get(key)
                    current = current.getparent()
                return found
    return None


def probe_titles(word: oracle.Session) -> dict:
    out = {}
    for name, data, texts in (("titles", titles_document(), ["No defaults", "Empty defaults", "Word style",
                                                                "Series auto", "Chart Title", "Grafiektitel"]),
                              ("axis", axis_titles_document(), ["Styled axis", "Styled title"])):
        pdf = word.export_pdf(data, name=f"charts-{name}")
        saved = word.resave(data, name=f"charts-{name}")
        entry = {"exported": bool(pdf), "ours": _saved_titles(data)}
        if pdf:
            entry["word_draws"] = {t: chars_of(pdf.path, t) for t in texts}
            entry["pdf_text"] = [line for line in pdf_text(pdf.path).splitlines() if line.strip()][:40]
        entry["docx2svg_draws"] = {t: docx2svg_title(data, t) for t in texts}
        if saved:
            entry["saved"] = _saved_titles(saved.path.read_bytes())
        out[name] = entry
    return out


# -- tracking --------------------------------------------------------------------------------------

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


def probe_tracking(word: oracle.Session) -> dict:
    out = {}
    for name, data in (("charts", edited_charts(tracking=True)[0]),):
        # The count is the script's answer, which the cache does not keep: always ask Word.
        for stale in oracle.ORACLE_DIR.glob(f"charts-tracking-{name}-*-counted.docx"):
            stale.unlink()
        counted = word.run_script(_COUNT_SCRIPT, data, name=f"charts-tracking-{name}", tag="counted")
        pdf = word.export_pdf(data, name=f"charts-tracking-{name}")
        entry = {"word_says": counted.detail if counted else f"failed: {counted.detail}",
                 "ours_has_markup": oracle._zip_has_markup(data)}
        if pdf:
            text = pdf_text(pdf.path)
            entry["pdf_has"] = {label: label in text for label in ("Added", "Renamed", "New series", "777", "Drafted",
                                                                   "Much faster edits", "Smaller files")}
        if counted:
            entry["saved_has_markup"] = oracle._zip_has_markup(counted.path.read_bytes())
        out[name] = entry
    # The tools' warnings on a tracked chart edit (once docx-agent's Python warnings).
    session = probe_tools.Session((FIXTURES / "charts.docx").read_bytes())
    session.call("word_set_tracking", on=True, author="Probe")
    result = session.call("edit_chart", target=chart_ids(session)[0], action="set_value", series="0", category="0",
                          value=1)
    out["docx_agent_warns"] = [str(w) for w in result.to_json().get("warnings", [])]
    return out


# -- places ----------------------------------------------------------------------------------------


def edited_places() -> bytes:
    """``chart-places.docx`` with each chart's first series' second value, first category
    and first series' name changed, the names tagged with the chart's id."""
    session = probe_tools.Session((FIXTURES / "chart-places.docx").read_bytes())
    for identifier in chart_ids(session):
        tag = identifier.replace("d:", "").replace("/", "m")
        session.call("edit_chart", target=identifier, action="set_value", series="0", category="1", value=4321)
        session.call("edit_chart", target=identifier, action="rename_category", category="0", text=f"Cat{tag}")
        session.call("edit_chart", target=identifier, action="rename_series", series="0", text=f"Name{tag}")
    return session.data()


def _ids(data: bytes) -> dict:
    root = etree.fromstring(parts(data)["word/document.xml"])
    return {"docPr": sorted(int(n.get("id")) for n in root.iter(WP + "docPr")),
            "group_members": [(n.get("id"), n.get("name")) for n in root.iter(WPG + "cNvPr")],
            "charts": chart_ids(probe_tools.Session(data))}


def probe_places(word: oracle.Session) -> dict:
    data = edited_places()
    pdf = word.export_pdf(data, name="charts-places")
    saved = word.resave(data, name="charts-places")
    out = {"exported": bool(pdf), "ours": _ids(data)}
    if pdf:
        text = pdf_text(pdf.path)
        out["pdf_has"] = {f"{kind}{tag}": f"{kind}{tag}" in text for tag in ("1", "2", "4", "5m7", "8")
                          for kind in ("Cat", "Name")}
        out["pdf_has_4321"] = "4321" in text or "4.321" in text or "4 321" in text or "4500" in text
        out["pdf_numbers"] = sorted(set(re.findall(r"\b\d[\d.]{2,5}\b", text)))
    if saved:
        saved_data = saved.path.read_bytes()
        out["saved"] = _ids(saved_data)
        out["saved_charts"] = [{"id": c["id"], "data": c["data"]} for c in charts_of(saved_data)]
        names = parts(saved_data)
        out["saved_media"] = sorted(n for n in names if n.startswith("word/media/"))
        out["input_media"] = sorted(n for n in parts(data) if n.startswith("word/media/"))
        out["fallback_image_changed"] = {n: parts(data).get(n) != names.get(n) for n in out["saved_media"]}
    return out


# -- SmartArt --------------------------------------------------------------------------------------


def _data_parts(files: dict[str, bytes]) -> list[str]:
    """The diagrams' data model parts, in the order the document names them."""
    rels = files.get("word/_rels/document.xml.rels", b"").decode()
    return ["word/" + t for t in re.findall(r'Target="(diagrams/data\d+\.xml)"', rels)]


def _drawing_part(files: dict[str, bytes], data_part: str) -> str | None:
    """A data model's cached drawing (``dsp:dataModelExt``'s relationship), if any."""
    found = re.search(rb'dataModelExt [^>]*relId="([^"]+)"', files[data_part])
    rels = files.get("word/_rels/document.xml.rels", b"").decode()
    if not found:
        return None
    target = re.search(rf'Id="{found.group(1).decode()}"[^>]*Target="([^"]+)"', rels) or re.search(
        rf'Target="([^"]+)"[^>]*Id="{found.group(1).decode()}"', rels)
    name = "word/" + target.group(1) if target else None
    return name if name in files else None


def _smartart_variant(name: str) -> bytes:
    """``smartart.docx`` with its second diagram's ``Goals`` changed in the data model alone
    (``model-stale``) or in the drawing alone (``drawing-only``)."""
    data = (FIXTURES / "smartart.docx").read_bytes()
    files = parts(data)
    second = _data_parts(files)[1]
    if name == "model-stale":
        return replaced(data, {second: lambda xml: xml.replace("<a:t>Goals</a:t>", "<a:t>Objectives</a:t>", 1)})
    if name == "drawing-only":
        return replaced(data, {_drawing_part(files, second): lambda xml: xml.replace(
            "<a:t>Goals</a:t>", "<a:t>Drawn only</a:t>", 1)})
    raise KeyError(name)


SMARTART_VARIANTS = ("model-stale", "drawing-only")
SMARTART_WORDS = ("Goals", "Objectives", "Drawn only", "Added child", "Risks", "Stale caches", "Exactly edited",
                  "Faster edits", "Sixth")


def _drawing_texts(data: bytes) -> list:
    files = parts(data)
    out = []
    for k, data_part in enumerate(_data_parts(files), start=1):
        model = etree.fromstring(files[data_part])
        points = list(model.iter(DGM + "pt"))
        texts = ["".join(t.text or "" for t in p.iter(A + "t")) for p in points
                 if p.get("type") in (None, "node", "doc")]
        part = _drawing_part(files, data_part)
        drawing = None
        if part is not None:
            drawing = ["".join(t.text or "" for t in p.iter(A + "t")) for p in etree.fromstring(files[part]).iter(A + "p")]
            drawing = [t for t in drawing if t]
        out.append({"id": data_part, "texts": [t for t in texts if t], "drawing": part, "drawing_texts": drawing,
                    "presentation_points": sum(1 for p in points if p.get("type") == "pres")})
    return out


def probe_smartart(word: oracle.Session) -> dict:
    out = {}
    for variant in SMARTART_VARIANTS:
        data = _smartart_variant(variant)
        pdf = word.export_pdf(data, name=f"smartart-{variant}")
        saved = word.resave(data, name=f"smartart-{variant}")
        entry = {"exported": bool(pdf), "ours": _drawing_texts(data)}
        if pdf:
            text = pdf_text(pdf.path)
            entry["pdf_has"] = {w: w in text for w in SMARTART_WORDS}
        if saved:
            entry["saved"] = _drawing_texts(saved.path.read_bytes())
        out[variant] = entry
    return out


# -- the dictionary ----------------------------------------------------------------------------------


def probe_dictionary(_word=None) -> dict:
    sdef = subprocess.run(["sdef", str(oracle.WORD_APP)], capture_output=True, text=True).stdout
    classes = re.findall(r'<class name="([^"]+)"', sdef)
    commands = re.findall(r'<command name="([^"]+)"', sdef)
    words = re.compile(r"chart|smart ?art|workbook|excel|edit data", re.I)
    return {
        "classes": sorted(c for c in classes if words.search(c)),
        "commands": sorted(c for c in commands if words.search(c)),
        "properties": sorted(set(re.findall(r'<property name="([^"]*(?:chart|smart)[^"]*)"', sdef, re.I))),
        "enumerators": sorted(set(re.findall(r'<enumerator name="((?:inline shape|shape type|link type)[^"]*'
                                             r'(?:chart|smart)[^"]*)"', sdef, re.I))),
        "has_run_vb_macro": "run VB macro" in commands,
    }


PROBES = {"cache": probe_cache, "extensions": probe_extensions, "titles": probe_titles, "tracking": probe_tracking,
          "places": probe_places, "smartart": probe_smartart, "dictionary": probe_dictionary}


def main(argv: list[str]) -> int:
    chosen = argv or list(PROBES)
    results = {}
    if chosen == ["dictionary"]:
        print(json.dumps(probe_dictionary(), indent=1))
        return 0
    with oracle.session() as word:
        for name in chosen:
            results[name] = PROBES[name](word)
            print(f"{name}: done", file=sys.stderr)
    if argv:
        print(json.dumps(results, indent=1, ensure_ascii=False, default=str))
        return 0
    OBSERVATIONS.write_text(json.dumps({"word": "16.106 for Mac", **results}, indent=1, ensure_ascii=False,
                                       default=str) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
