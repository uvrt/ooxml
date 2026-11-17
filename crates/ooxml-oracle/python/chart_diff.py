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

Python's behaviour is not changed: this only reads ooxml-common's public functions.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import sys
import zipfile
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--rust", required=True, help="the chart-model binary")
    parser.add_argument("--tree", action="append", default=[], help="every package and chart XML below this folder")
    parser.add_argument("--json", help="write the per-file counts here")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args()
    files = [Path(f) for f in args.files]
    for folder in args.tree:
        files += chart_files(Path(folder))
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
