#!/usr/bin/env python3
"""The text engine's differential: what docx2svg's line breaker was asked and answered.

Lays every document given out with docx2svg (``convert_docx_to_layout``) and records,
for every paragraph the layout breaks into lines (each call of
:func:`docx2svg.linebreak.break_paragraph`, at the width the layout breaks it at), the
breaker's input -- the paragraph's runs with each character's resolved format, its
marks, its list label, its geometry and its hyphenation -- and its answer: every piece
with its width and the kern pair it is charged with, and every line.  Nothing of
docx2svg changes: the call is wrapped for the run of this tool only.

The Rust text engine (``crates/ooxml-text``, ``crates/ooxml-fonts``) reads the same
records -- ``cargo run -p ooxml-oracle --bin text-diff -- <records>`` -- measures and
breaks each paragraph itself, from the fonts it finds in the same folders, and reports
every difference.

Usage::

    python tools/text_differential.py OUT.jsonl.gz DOCX_OR_FOLDER...

Each line of the output is a JSON object: ``{"document": ...}`` once per document (its
path, the font folders and Word's settings the fonts were looked up with, its embedded
faces and its font table's classes), then ``{"paragraph": ...}`` per distinct paragraph
input, and ``{"report": ...}`` at the document's end (the substitutions docx2svg made).
Identical paragraph inputs within a document are written once.

**Replay**: run as a pytest plugin over docx2svg's own suite (every fixture and probe
document the tests lay out, with whatever faces each test gives)::

    pytest -p text_differential --text-differential=OUT_DIR tests

every paragraph record then carries ``"glyphs"`` -- every advance, kern pair and script
metric the breaker was answered -- and the Rust engine is answered the same: the
engine is compared alone, whatever fonts the test used.  One file per test process.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import re
import sys
import time
import zipfile
from fractions import Fraction
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "src"))

from docx2svg import convert_docx_to_layout, linebreak  # noqa: E402
from docx2svg.fonts import WORD_FONTS, font_table_classes  # noqa: E402
from docx2svg.resolve import character_format, resolve_mark, resolve_paragraph, resolve_run  # noqa: E402
from docx2svg.resolve.cascade import Resolved  # noqa: E402

KINDS = {linebreak.GLYPH: "g", linebreak.SPACE: "s", linebreak.TAB: "t", linebreak.BREAK: "b",
         linebreak.SOFT_HYPHEN: "h", linebreak.OBJECT: "o"}


def number(value) -> str:
    """An exact length as ``"n"`` or ``"n/d"``."""
    value = Fraction(value)
    return str(value.numerator) if value.denominator == 1 else f"{value.numerator}/{value.denominator}"


class Formats:
    """The document's distinct character formats, numbered in order of first use."""

    def __init__(self) -> None:
        self.ids: dict = {}
        self.new: list = []

    def of(self, fmt) -> int:
        key = (fmt.face, bool(fmt.bold), bool(fmt.italic), fmt.half_points, fmt.vertical_align)
        found = self.ids.get(key)
        if found is None:
            found = self.ids[key] = len(self.ids)
            self.new.append([found, *key])
        return found


def run_input(document, paragraph, run, formats: Formats) -> dict:
    resolved = resolve_run(document, paragraph, run)
    out: dict = {"text": run.text, "marks": [[p, k] for p, k in run.breaks]}
    if resolved.get("vanish"):
        out["vanish"] = True
        return out
    ids = [formats.of(character_format(resolved, char, document)) for char in run.text]
    out["formats"] = ids[:1] if len(set(ids)) == 1 else ids
    out["hyphen_format"] = formats.of(character_format(resolved, "-", document))
    out["tracking"] = resolved.get("spacing") or 0
    out["scale"] = resolved.get("w") or 100
    out["kern"] = resolved.get("kern") or 0
    out["caps"] = bool(resolved.get("caps"))
    out["note_number"] = bool(run.note_number)
    out["lang"] = resolved.get("lang.val")
    out["no_proof"] = bool(resolved.get("noProof"))
    return out


def label_input(document, paragraph, text: str, pp, formats: Formats) -> dict | None:
    """What :func:`linebreak.label_pieces` reads, or ``None`` where it makes no pieces."""
    num_id, ilvl = pp.get("numPr.numId"), pp.get("numPr.ilvl", 0) or 0
    level = document.numbering.get(num_id, {}).get(ilvl) if num_id else None
    if level is None:
        return None
    mark = resolve_mark(document, paragraph)
    values = dict(mark.values)
    values.update(level.run)
    resolved = Resolved(values, dict(mark.origins))
    out: dict = {"alignment": getattr(level, "alignment", None), "suffix": level.suffix or "tab",
                 "caps": bool(resolved.get("caps"))}
    space = character_format(resolved, " ", document)
    out["space_format"] = formats.of(space)
    if level.picture_bullet is not None:
        pixels = linebreak.picture_bullet_pixels(document, level)
        out["picture"] = None if pixels is None else [space.half_points, pixels[1]]
        text = ""
    out["chars"] = [[char, formats.of(character_format(resolved, char, document))] for char in text]
    return out


def geometry_input(geometry: linebreak.Geometry) -> dict:
    return {
        "first_start": number(geometry.first_start), "start": number(geometry.start),
        "right": number(geometry.right),
        "tabs": [[number(stop.position), stop.alignment] for stop in geometry.tabs],
        "default_tab": number(geometry.default_tab),
        "hanging_stop": None if geometry.hanging_stop is None else number(geometry.hanging_stop),
        "squeeze": geometry.squeeze, "tab_origin": number(geometry.tab_origin),
        "hyphen_zone": None if geometry.hyphen_zone is None else number(geometry.hyphen_zone),
        "hyphen_next": geometry.hyphen_next,
        "hyphen_gap": None if geometry.hyphen_gap is None else number(geometry.hyphen_gap),
        "hyphen_limit": geometry.hyphen_limit,
    }


def answer(items, lines, advances) -> dict:
    pieces = []
    for j, piece in enumerate(items):
        try:
            kern = linebreak._kern(piece, items[j + 1], advances) if j + 1 < len(items) else 0
        except linebreak.Unmeasurable:
            kern = None
        pieces.append([KINDS[piece.kind], piece.char, number(piece.width), piece.source, number(piece.hyphen),
                       piece.run, piece.auto, piece.proof_end, piece.height,
                       None if kern is None else number(kern)])
    return {"pieces": pieces,
            "lines": [[line.start, line.end, line.forced, line.emergency, line.hyphenated] for line in lines]}


def embedded_fonts(package: bytes) -> list:
    """``[family, style, part, key]`` of every embedded face, as docx2svg's
    :func:`docx2svg.fonts.embedded_faces` finds them (the part not yet read)."""
    try:
        archive = zipfile.ZipFile(io.BytesIO(package))
        table = archive.read("word/fontTable.xml").decode("utf-8")
        rels_xml = archive.read("word/_rels/fontTable.xml.rels").decode("utf-8")
    except (KeyError, zipfile.BadZipFile):
        return []
    names = set(archive.namelist())
    rels = dict(re.findall(r'Id="([^"]+)"[^>]*Target="([^"]+)"', rels_xml))
    rels.update({k: v for v, k in re.findall(r'Target="([^"]+)"[^>]*Id="([^"]+)"', rels_xml)})
    out = []
    for name, body in re.findall(r'<w:font w:name="([^"]+)"(.*?)</w:font>', table, re.S):
        for style, rid, key in re.findall(r'<w:embed(\w+) r:id="([^"]+)" w:fontKey="\{([^}]+)\}"', body):
            target = rels.get(rid)
            if target is not None and f"word/{target}" in names:
                out.append([name, style, f"word/{target}", key])
    return out


class Answers:
    """An ``Advances`` (and its ``metrics``) remembering every answer it gives."""

    def __init__(self, inner, metrics) -> None:
        self.inner = inner
        self.metrics_of = metrics
        self.advances: dict = {}
        self.kerns: dict = {}
        self.script: dict = {}

    def advance(self, face, bold, italic, char):
        found = self.inner.advance(face, bold, italic, char)
        self.advances[(face, bool(bold), bool(italic), char)] = found
        return found

    def kern(self, face, bold, italic, left, right):
        found = self.inner.kern(face, bold, italic, left, right)
        self.kerns[(face, bool(bold), bool(italic), left, right)] = found
        return found

    def metrics(self, face, bold=False, italic=False):
        found = self.metrics_of(face, bold, italic) if self.metrics_of is not None else None
        self.script[(face, bool(bold), bool(italic))] = None if found is None else [
            found.units_per_em, found.superscript_size, found.subscript_size,
            getattr(found, "superscript_offset", None), getattr(found, "subscript_offset", None)]
        return found

    def record(self) -> dict:
        return {
            "advance": [[*key, *(value if value is not None else (None, None))] for key, value in self.advances.items()],
            "kern": [[*key, value] for key, value in self.kerns.items()],
            "metrics": [[*key, value] for key, value in self.script.items()],
        }


class Recorder:
    def __init__(self, out, replay: bool = False) -> None:
        self.out = out
        self.replay = replay
        self.formats = Formats()
        self.seen: set = set()
        self.advances = None
        self.counts = {"calls": 0, "written": 0, "unmeasurable": 0}

    def write(self, record: dict) -> None:
        self.out.write(json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n")

    def call(self, original, document, paragraph, section, advances, metrics=None, label=None):
        self.advances = advances
        self.counts["calls"] += 1
        given = advances
        if self.replay:
            advances = Answers(advances, metrics)
            metrics = advances.metrics if metrics is not None else None
        error = None
        try:
            items, lines = original(document, paragraph, section, advances, metrics, label)
        except linebreak.Unmeasurable as why:
            error, items, lines = str(why), None, None
        pp = resolve_paragraph(document, paragraph)
        hyphenation = document.hyphenation
        record = {
            "runs": [run_input(document, paragraph, run, self.formats) for run in paragraph.runs],
            "label": label_input(document, paragraph, label, pp, self.formats) if label is not None else None,
            "geometry": geometry_input(linebreak.geometry(document, paragraph, section, pp)),
            "columns": linebreak.is_multi_column(section),
            "hyphenation": None if hyphenation is None or pp.get("suppressAutoHyphens") else {
                "mode15": (document.compatibility_mode or 0) >= 15, "no_caps": bool(hyphenation.no_caps)},
        }
        expected = {"error": error} if error is not None else answer(items, lines, advances)
        if self.replay:
            record["glyphs"] = advances.record()
        key = hashlib.sha1(json.dumps(record, sort_keys=True).encode()).hexdigest()
        if key not in self.seen:
            self.seen.add(key)
            if self.formats.new:
                self.write({"formats": self.formats.new})
                self.formats.new = []
            record["expected"] = expected
            self.counts["written"] += 1
            self.counts["unmeasurable"] += error is not None
            self.write({"paragraph": record})
        self.advances = given
        if error is not None:
            raise linebreak.Unmeasurable(error)
        return items, lines


def location(path: str) -> str:
    if Path(path) == WORD_FONTS:
        return "bundle"
    return "folder"


def record_document(path: Path, out, timings: list) -> dict:
    package = path.read_bytes()
    recorder = Recorder(out)
    original = linebreak.break_paragraph

    def wrapped(document, paragraph, section, advances, metrics=None, label=None):
        return recorder.call(original, document, paragraph, section, advances, metrics, label)

    linebreak.break_paragraph = wrapped
    started = time.perf_counter()
    try:
        recorder.write({"document": {"path": str(path.resolve())}})
        try:
            convert_docx_to_layout(package)
        except Exception as error:  # noqa: BLE001 -- a document docx2svg refuses is reported, not fatal
            recorder.write({"refused": f"{type(error).__name__}: {error}"})
    finally:
        linebreak.break_paragraph = original
    fonts = recorder.advances
    if fonts is not None:
        recorder.write({"fonts": {
            "dirs": [[location(str(d)), str(d)] for d in fonts.dirs],
            "substitutes": fonts.substitutes, "recorded_faces": fonts.recorded_faces,
            "fallbacks": fonts.fallbacks, "embedded": embedded_fonts(package),
            "font_table": font_table_classes(package),
        }})
        recorder.write({"report": {
            "substitutions": {k: v.as_dict() for k, v in fonts.substitutions.items()},
            "missing": sorted(fonts.missing),
            "glyph_fallbacks": [[f, c, n] for (f, c), n in sorted(fonts.glyph_fallbacks.items())],
        }})
    timings.append([str(path), round(time.perf_counter() - started, 3), recorder.counts])
    return recorder.counts


def documents(paths) -> list[Path]:
    out = []
    for given in paths:
        given = Path(given)
        out.extend(sorted(given.rglob("*.docx")) if given.is_dir() else [given])
    return [p for p in out if not p.name.startswith("~$")]


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("out")
    parser.add_argument("paths", nargs="+")
    args = parser.parse_args(argv)
    timings: list = []
    opener = gzip.open if args.out.endswith(".gz") else open
    with opener(args.out, "wt", encoding="utf-8") as out:
        for path in documents(args.paths):
            counts = record_document(path, out, timings)
            print(f"{path}: {counts}", file=sys.stderr)
    print(json.dumps(timings), file=sys.stderr)
    return 0


# -- pytest plugin (replay over docx2svg's own suite) -----------------------------------


def pytest_addoption(parser) -> None:
    parser.addoption("--text-differential", default=None,
                     help="write the text engine's differential records (replay) into this folder")


def pytest_configure(config) -> None:
    folder = config.getoption("--text-differential")
    if not folder:
        return
    Path(folder).mkdir(parents=True, exist_ok=True)
    worker = getattr(config, "workerinput", {}).get("workerid", "main")
    out = gzip.open(Path(folder) / f"suite-{worker}.jsonl.gz", "wt", encoding="utf-8")
    state = {"recorder": None, "test": None}
    original = linebreak.break_paragraph
    from docx2svg import hyphenate

    watched = (linebreak, hyphenate)
    rules = [{name: value for name, value in vars(module).items()
              if not name.startswith("__") and name != "break_paragraph"} for module in watched]

    def wrapped(document, paragraph, section, advances, metrics=None, label=None):
        if any(vars(module).get(name) is not value
               for module, kept in zip(watched, rules) for name, value in kept.items()):
            # A test that changes the breaker's own rules (to refute a hypothesis): its
            # lines are not docx2svg's, and are not compared.
            return original(document, paragraph, section, advances, metrics, label)
        if state["recorder"] is None:
            state["recorder"] = Recorder(out, replay=True)
            state["recorder"].write({"document": {"path": state["test"] or "(collection)"}})
        return state["recorder"].call(original, document, paragraph, section, advances, metrics, label)

    linebreak.break_paragraph = wrapped

    class Plugin:
        def pytest_runtest_setup(self, item) -> None:
            state["recorder"], state["test"] = None, item.nodeid

        def pytest_unconfigure(self, config) -> None:
            linebreak.break_paragraph = original
            out.close()

    config.pluginmanager.register(Plugin(), "text-differential-state")


if __name__ == "__main__":
    raise SystemExit(main())
