#!/usr/bin/env python3
"""The chart differential harness (test support; phase P3, lane P3b2).

Reads every chart part of each file as ooxml-common reads it -- a package's parts whose root
is ``c:chartSpace``, or a bare chart XML file such as ooxml-common's golden fixtures -- and
writes the model (``chart.read.parse_chart_space``) in the JSON form the Rust ``chart-model``
binary writes (``crates/ooxml-oracle/src/chart/``), runs the binary on the same files, and
compares the two field by field. Every difference is printed with its JSON path; the exit
status is 1 when there is any.

    python crates/ooxml-oracle/python/chart_diff.py --rust target/release/chart-model a.pptx b.xml
    python crates/ooxml-oracle/python/chart_diff.py --rust ... --tree DIR   # every package and chart XML below DIR
    python crates/ooxml-oracle/python/chart_diff.py --rust ... --layout --tree DIR   # each chart laid out
    python crates/ooxml-oracle/python/chart_diff.py --freeze crates/ooxml-oracle/tests/data/chart-layout.json.gz

``--freeze`` writes ooxml-common's layout of the repository's charts (the golden fixtures
under their rules, the committed packages' chart parts under both) with the cases, for
``cargo test``'s ``tests/chart_layout.rs`` to compare the Rust layout with where Python is
not installed. Regenerate it whenever ooxml-common's chart layout changes on purpose.

With ``--layout`` each chart is laid out as ooxml-common's golden harness lays it out
(``tests/goldens.py``'s ``chart_element``: its consumer, its style), under PowerPoint's rules
and Word's: a golden fixture in the frame and with the locale its manifest states, a
package's chart part in a 480 x 288 pt frame. The elements, the chart data and the notes are
compared field by field.

Python's behaviour is not changed: this only reads ooxml-common's public functions.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import sys
import zipfile
from types import SimpleNamespace
from pathlib import Path
from xml.etree import ElementTree as ET

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import drawingml_diff as dd  # noqa: E402  (puts ooxml-common on the path)

from ooxml_common.chart import read as chart_read  # noqa: E402
from ooxml_common.xmlutil import local_name  # noqa: E402

PACKAGES = (".pptx", ".docx", ".xlsx", ".pptm", ".docm", ".xlsm")


def dump(value):
    """``drawingml_diff.dump``, with a set as a sorted list."""
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        out = {"@": type(value).__name__}
        for item in dataclasses.fields(value):
            out[item.name] = dump(getattr(value, item.name))
        return out
    if isinstance(value, (set, frozenset)):
        return sorted(dump(v) for v in value)
    if isinstance(value, dict):
        return {"{}": [[dump(k), dump(v)] for k, v in value.items()]}
    if isinstance(value, (list, tuple)):
        return [dump(v) for v in value]
    return dd.dump(value)


def part(root) -> dict:
    chart = chart_read.parse_chart_space(root)
    return {"model": None if chart is None else dump(chart)}


def python_record(path: Path) -> dict:
    if path.suffix == ".xml":
        return {"file": str(path), "parts": {str(path): part(ET.parse(path).getroot())}}
    parts = {}
    with zipfile.ZipFile(path) as package:
        for name in sorted(n for n in package.namelist() if n.endswith(".xml")):
            try:
                root = ET.fromstring(package.read(name))
            except ET.ParseError:
                continue
            if local_name(root.tag) == "chartSpace":
                parts[name] = part(root)
    return {"file": str(path), "parts": parts}


def chart_files(folder: Path) -> list[Path]:
    """Every package below ``folder``, and every XML file whose root is ``c:chartSpace``."""
    found = []
    for path in sorted(folder.rglob("*")):
        if path.suffix in PACKAGES:
            found.append(path)
        elif path.suffix == ".xml":
            try:
                if local_name(ET.parse(path).getroot().tag) == "chartSpace":
                    found.append(path)
            except ET.ParseError:
                continue
    return found


LAYOUT_FRAME = (480.0, 288.0)


def layout_cases(files: list[Path], frames: list[tuple[float, float]] | None = None) -> list[dict]:
    """Each chart of ``files`` with its frame, rules and locale; with ``frames``, each chart
    in each of those frames under both rules instead."""
    if frames:
        cases = []
        for case in layout_cases(files):
            if case["rules"] != "powerpoint" and any(c["file"] == case["file"] and c["part"] == case["part"]
                                                     for c in cases):
                continue
            for width, height in frames:
                for rules in ("powerpoint", "word"):
                    cases.append(dict(case, width_pt=width, height_pt=height, rules=rules))
        return cases
    manifest = {}
    for path in files:
        listing = path.parent.parent / "charts.json"
        if path.suffix == ".xml" and listing.exists() and not manifest.get(str(listing)):
            manifest[str(listing)] = json.loads(listing.read_text(encoding="utf-8"))
    cases = []
    for path in files:
        if path.suffix == ".xml":
            entry = manifest.get(str(path.parent.parent / "charts.json"), {}).get(path.stem)
            width, height = (entry["width_pt"], entry["height_pt"]) if entry else LAYOUT_FRAME
            for rules in (entry["rules"] if entry else ("powerpoint", "word")):
                cases.append({"file": str(path), "part": None, "width_pt": width, "height_pt": height,
                              "rules": rules, "locale": entry.get("locale") if entry else None})
            continue
        for name in python_record(path)["parts"]:
            for rules in ("powerpoint", "word"):
                cases.append({"file": str(path), "part": name, "width_pt": LAYOUT_FRAME[0],
                              "height_pt": LAYOUT_FRAME[1], "rules": rules, "locale": None})
    return cases


def python_layout(case: dict) -> dict:
    """A case laid out by ooxml-common, as the golden harness lays it out."""
    sys.path.insert(0, str(dd.ROOT / "python" / "ooxml-common" / "tests"))
    import goldens

    if case["part"] is None:
        root = ET.parse(case["file"]).getroot()
    else:
        with zipfile.ZipFile(case["file"]) as package:
            root = ET.fromstring(package.read(case["part"]))
    variant = case.get("variant")
    if variant:
        element, notes = variant_element(goldens, root, case)
    else:
        element, notes = goldens.chart_element(root, case["width_pt"], case["height_pt"], case["rules"],
                                               case["locale"])
    if element is None:
        return {"notes": notes}
    return {"notes": notes, "elements": [dump(child) for child in element.children], "data": dump(element.chart)}


#: The style variants ``--variants`` lays each chart out in as well: what the renderers set
#: that the golden harness does not (an East Asian face, a 2007 deck, Word's line colour and
#: its own "Chart Title", text at another size).
VARIANTS = {
    "east-asian": {"font_family_ea": "MS Gothic"},
    "office-2007": {"office_2007": True},
    "line-colour": {"line_color": "#898989", "default_title": "Chart Title"},
    "small-text": {"font_family": "Arial", "font_size": 7.0},
}


def variant_element(goldens, root, case):
    """``goldens.chart_element`` with the case's style variant applied."""
    import dataclasses

    from ooxml_common.chart import layout as chart_layout
    from ooxml_common.chart import read as chart_read

    settings = VARIANTS[case["variant"]]
    rules = case["rules"]
    source = chart_read.parse_chart_space(root)
    if source is None:
        return None, ["no c:chart"]
    plots = chart_layout.drawable_plots(source)
    if not plots:
        return None, ["not drawn: " + ",".join(sorted(plot.kind for plot in source.plots))]
    style = goldens.chart_style(source, rules)
    style = dataclasses.replace(style, **{k: v for k, v in settings.items() if k != "default_title"})
    builder = chart_layout.ChartBuilder(
        source, plots[0], width_pt=case["width_pt"], height_pt=case["height_pt"], style=style,
        resolve_fill=lambda paint: goldens.fill(paint, rules), resolve_outline=lambda line: goldens.outline(line, rules),
        resolve_text=lambda rich, text, size, align: goldens.title_text(rich, text, size, align, rules),
        resolve_typeface=lambda face: "Aptos" if face and face.startswith("+") else face,
        plots=plots, rules=goldens.RULES[rules], locale=goldens.locale(case["locale"]))
    builder.default_title = settings.get("default_title")
    children, data = builder.build()
    notes = []
    if builder.flattened_three_d:
        notes.append("flattened: " + ",".join(builder.flattened_three_d))
    if builder.default_title_wanted:
        notes.append("default title wanted")
    notes.extend(f"axis title not drawn: {reason}" for reason in dict.fromkeys(builder.axis_titles_not_drawn))
    return SimpleNamespace(children=children, chart=data), notes


def compare_layouts(binary: str, files: list[Path], out_json: str | None,
                    frames: list[tuple[float, float]] | None = None, variants: bool = False) -> int:
    cases = layout_cases(files, frames)
    if variants:
        cases = [dict(case, variant=name) for case in cases for name in VARIANTS]
    rust = dd.run_rust(binary, ["--layout"], "\n".join(json.dumps(c) for c in cases))
    total, summary = 0, {}
    for case, answer in zip(cases, rust):
        out: list[str] = []
        dd.diff(python_layout(case), answer, "", out)
        label = (f"{case['file']}{'' if case['part'] is None else ':' + case['part']} "
                 f"[{case['rules']} {case['width_pt']:g}x{case['height_pt']:g} {case.get('variant') or ''}]")
        summary[label] = len(out)
        total += len(out)
        for line in out[:12]:
            print(f"  {line}")
        if out:
            print(f"{label}: {len(out)} difference(s)")
    print(f"{len(cases)} layout case(s), {sum(1 for v in summary.values() if v)} with differences, {total} difference(s)")
    if out_json:
        Path(out_json).write_text(json.dumps(summary, indent=1) + "\n", encoding="utf-8")
    return 1 if total else 0


def freeze(out: Path) -> int:
    """The repository's layout cases and ooxml-common's answers, gzipped JSON."""
    import gzip

    root = dd.ROOT
    files = chart_files(root / "python" / "ooxml-common" / "tests" / "fixtures")
    files += [p for p in chart_files(root / "python") if p.suffix != ".xml"]
    records = []
    for case in layout_cases(files):
        answer = python_layout(case)
        relative = dict(case, file=str(Path(case["file"]).resolve().relative_to(root)))
        records.append({"case": relative, "python": answer})
    data = json.dumps(records, separators=(",", ":"), sort_keys=True).encode()
    out.write_bytes(gzip.compress(data, 9, mtime=0))
    print(f"{len(records)} case(s) frozen in {out}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--rust", help="the chart-model binary")
    parser.add_argument("--freeze", help="write the repository's layout cases and Python's answers here")
    parser.add_argument("--tree", action="append", default=[], help="every package and chart XML below this folder")
    parser.add_argument("--json", help="write the per-file counts here")
    parser.add_argument("--layout", action="store_true", help="lay each chart out and compare the drawing")
    parser.add_argument("--frames", help="with --layout: frames WxH,WxH... (points) each chart is laid out in")
    parser.add_argument("--variants", action="store_true", help="with --layout: each style variant as well")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args()
    if args.freeze:
        return freeze(Path(args.freeze))
    if not args.rust:
        parser.error("--rust is required")
    files = [Path(f) for f in args.files]
    for folder in args.tree:
        files += chart_files(Path(folder))
    if args.layout:
        frames = [tuple(float(v) for v in f.split("x")) for f in args.frames.split(",")] if args.frames else None
        return compare_layouts(args.rust, files, args.json, frames, args.variants)
    total, charts, summary = 0, 0, {}
    for path in files:
        out: list[str] = []
        python = python_record(path)
        if not python["parts"]:
            continue
        charts += len(python["parts"])
        rust = dd.run_rust(args.rust, [str(path)])[0]
        dd.diff(python, rust, "", out)
        summary[str(path)] = {"charts": len(python["parts"]), "differences": len(out)}
        total += len(out)
        for line in out[:40]:
            print(f"  {line}")
        print(f"{path}: {len(python['parts'])} chart(s), {len(out)} difference(s)")
    print(f"{charts} chart(s) in {len(summary)} file(s), {total} difference(s)")
    if args.json:
        Path(args.json).write_text(json.dumps(summary, indent=1) + "\n", encoding="utf-8")
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main())
