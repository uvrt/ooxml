#!/usr/bin/env python3
"""The DrawingML painter's differential harness (test support; lane P3c).

Draws each chart of the given files as ooxml-common's golden harness draws one
(``tests/goldens.py``: ``chart_element``, then ``svg_document``) and as the Rust
``drawing-svg --chart`` binary draws it (``ooxml-chart`` laid out, ``ooxml-render`` painted),
and compares the two documents byte for byte. Each chart is drawn in each of the frames given
under PowerPoint's rules and Word's; a golden fixture also in the frame its manifest states.
Every difference is printed with where the documents first part; the exit status is 1 when
there is any.

    python crates/ooxml-oracle/python/drawing_diff.py --rust target/debug/drawing-svg --tree python
    python crates/ooxml-oracle/python/drawing_diff.py --rust ... --frame 480x288 --frame 90x60 a.pptx
    python crates/ooxml-oracle/python/drawing_diff.py --rust ... --slides deck.pptx ...

``--slides`` instead resolves each deck's slides with pptx2svg (``convert_pptx_to_model``,
its own fonts, no embedded faces), freezes each slide's elements as the golden harness
freezes a scene (``goldens.to_data``), and compares ooxml-common's ``svg_document`` of them
with ``drawing-svg --scene``'s, under both rules.

The goldens themselves are gated by ``cargo test``'s ``tests/integration/chart_goldens.rs`` and
``tests/integration/scene_goldens.rs``; this reaches every chart part of the repository's packages, at
several sizes. Python's behaviour is not changed: this only calls ooxml-common's public
functions and the golden harness.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET

sys.path.insert(0, str(Path(__file__).resolve().parent))
import chart_diff  # noqa: E402  (puts ooxml-common on the path)

ROOT = Path(__file__).resolve().parents[3]


def python_document(case: dict) -> str:
    sys.path.insert(0, str(ROOT / "python" / "ooxml-common" / "tests"))
    import goldens

    if case["part"] is None:
        root = ET.parse(case["file"]).getroot()
    else:
        with zipfile.ZipFile(case["file"]) as package:
            root = ET.fromstring(package.read(case["part"]))
    element, notes = goldens.chart_element(root, case["width_pt"], case["height_pt"], case["rules"], case["locale"])
    if element is None:
        return "".join(f"<!-- {note} -->\n" for note in notes)
    return goldens.svg_document([element], element.transform.extent_width, element.transform.extent_height,
                                case["rules"], notes)


def rust_documents(binary: str, cases: list[dict]) -> list[str]:
    out = []
    for case in cases:
        result = subprocess.run([binary, "--chart"], input=json.dumps(case) + "\n", capture_output=True, text=True,
                                check=False)
        out.append(result.stdout if result.returncode == 0 else f"error: {result.stderr.strip()}")
    return out


def slide_cases(decks: list[Path]) -> list[tuple[str, dict]]:
    sys.path.insert(0, str(ROOT / "python" / "ooxml-common" / "tests"))
    sys.path.insert(0, str(ROOT / "python" / "pptx2svg" / "src"))
    from datetime import datetime

    import goldens
    from pptx2svg import ConvertOptions, convert_pptx_to_model

    out = []
    for deck in decks:
        resolved = convert_pptx_to_model(str(deck), ConvertOptions(use_embedded_fonts=False, now=datetime(2026, 1, 1)))
        for number, slide in enumerate(resolved.slides, 1):
            try:
                elements = goldens.to_data(list(slide.elements))
            except TypeError as error:
                out.append((f"{deck.name} slide {number}", {"error": str(error)}))
                continue
            out.append((f"{deck.name} slide {number}", {
                "width_emu": resolved.slide_size.width, "height_emu": resolved.slide_size.height,
                "elements": elements, "_python": slide.elements}))
    return out


def compare_slides(binary: str, decks: list[Path]) -> int:
    sys.path.insert(0, str(ROOT / "python" / "ooxml-common" / "tests"))
    import goldens

    failures = total = 0
    for name, scene in slide_cases(decks):
        if "error" in scene:
            print(f"{name}: not frozen ({scene['error']})")
            failures += 1
            continue
        elements = scene.pop("_python")
        for rules in ("powerpoint", "word"):
            total += 1
            expected = goldens.svg_document(elements, scene["width_emu"], scene["height_emu"], rules)
            result = subprocess.run([binary, "--scene", rules], input=json.dumps(scene), capture_output=True,
                                    text=True, check=False)
            produced = result.stdout if result.returncode == 0 else f"error: {result.stderr.strip()}"
            if produced == expected:
                continue
            failures += 1
            at = next((i for i, (a, b) in enumerate(zip(produced, expected)) if a != b),
                      min(len(produced), len(expected)))
            print(f"{name} {rules} at {at}:")
            print(f"  rust:   {produced[max(0, at - 80):at + 120]!r}")
            print(f"  python: {expected[max(0, at - 80):at + 120]!r}")
    print(f"{total - failures} of {total} slide documents drawn equal")
    return 1 if failures else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--rust", required=True, help="the drawing-svg binary")
    parser.add_argument("--tree", type=Path, help="every package and chart XML below this folder")
    parser.add_argument("--frame", action="append", default=[], help="a frame WxH in points (repeatable)")
    parser.add_argument("--slides", action="store_true", help="the files are decks: compare their slides")
    parser.add_argument("files", nargs="*", type=Path)
    args = parser.parse_args()
    if args.slides:
        return compare_slides(args.rust, list(args.files))
    files = list(args.files) + (chart_diff.chart_files(args.tree) if args.tree else [])
    frames = [tuple(float(v) for v in frame.split("x")) for frame in args.frame]
    cases = chart_diff.layout_cases(files)
    if frames:
        cases += chart_diff.layout_cases(files, frames)
    rust = rust_documents(args.rust, cases)
    failures = 0
    for case, produced in zip(cases, rust):
        expected = python_document(case)
        if produced == expected:
            continue
        failures += 1
        at = next((i for i, (a, b) in enumerate(zip(produced, expected)) if a != b), min(len(produced), len(expected)))
        print(f"{case['file']} {case['part']} {case['rules']} {case['width_pt']}x{case['height_pt']} at {at}:")
        print(f"  rust:   {produced[max(0, at - 80):at + 120]!r}")
        print(f"  python: {expected[max(0, at - 80):at + 120]!r}")
    print(f"{len(cases) - failures} of {len(cases)} charts drawn equal")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
