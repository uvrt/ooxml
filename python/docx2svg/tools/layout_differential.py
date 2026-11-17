#!/usr/bin/env python3
"""The Word layout's differential: what docx2svg's layout made of each document, box by box.

Lays every document given out with docx2svg (``convert_docx_to_layout``) and records, for
the body's last layout pass, its flow -- every top-level item, a paragraph with its lines
(the line breaker's pieces from ``start`` to ``end``), each line's height, its spacing,
borders, keeps and section facts; a table, a stand-in box or an obstacle by its kind --
its pagination (every page's start and :class:`docx2svg.paginate.PageInfo`), and every
body line the layout drew (its paragraph's path, its number, its top, pitch and
baseline, its column).  Nothing of docx2svg changes: ``paginate.flow``,
``paginate.paginate`` and ``linebreak.break_paragraph`` are wrapped for the run of this
tool only.

The Rust layout (``crates/ooxml-word-layout``) lays the same documents out over the same
fonts -- ``cargo run -p ooxml-oracle --bin layout-diff -- <records>`` -- and reports every
difference.

Usage::

    python tools/layout_differential.py OUT.jsonl.gz DOCX_OR_FOLDER...

Each line of the output is a JSON object: ``{"document": ...}``, then ``{"fonts": ...}``
(as ``text_differential.py`` writes it) and ``{"layout": ...}``, or ``{"refused": ...}``.
Lengths are exact, ``"n"`` or ``"n/d"`` px.
"""

from __future__ import annotations

import argparse
import gzip
import json
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "src"))
sys.path.insert(0, str(HERE))

from docx2svg import convert_docx_to_layout, linebreak, paginate  # noqa: E402
from docx2svg import fields as fields_module  # noqa: E402
from docx2svg.fonts import font_table_classes  # noqa: E402
from text_differential import documents, embedded_fonts, location, number  # noqa: E402


def height(h) -> list:
    return [number(h.pitch), number(h.text_above), number(h.text_below), number(h.natural), h.rule, h.line]


def position(p) -> list:
    return [p.item, p.line, list(p.part)]


def cell_entry(entry) -> list:
    """One entry of a cell: ``[block, kind, lines, pitches, more]`` -- a paragraph's lines
    (``[start, end, forced]``), pitches and spacing; a nested table's pitch and record; a
    stand-in box's pitch."""
    pitches = [number(h.pitch) for h in entry.heights]
    if entry.table is not None:
        return [entry.block, "table", None, pitches, table_record(entry.table)]
    if entry.paragraph is None:  # neither a paragraph nor a table: a stand-in box
        return [entry.block, "box", None, pitches, None]
    return [entry.block, "para", [[line.start, line.end, line.forced] for line in entry.lines], pitches,
            {"hidden": entry.hidden, "anchors": len(entry.anchors), "before": entry.before, "after": entry.after,
             "top_border": number(entry.top_border), "border": number(entry.border),
             "notes": any(kind.startswith(("footnoteReference:", "endnoteReference:"))
                          for run in entry.paragraph.runs for _, kind in run.breaks)}]


def table_record(flow) -> dict:
    """A table's flow (:class:`docx2svg.table.TableFlow`): its grid, each row's top border,
    height, keep and cells, its bottom border and header rows, and what of it was laid out
    by approximate rules."""
    from docx2svg import table as table_model

    return {
        "approximation": flow.approximation,
        # What approximate rule laid out, other than a table's widths or a frame in a cell
        # (which the Rust layout lays out too): a stand-in box, an unmodelled drawing.
        "unmodelled": sorted({reason for reason, _message, _path in flow.approximated} - {"table", "frame"}),
        "floating": flow.floating is not None or flow.nested_floating is not None,
        "grid": [number(x) for x in flow.grid_twips],
        "rows": [{"top_border": number(row.top_border), "height": number(row.height),
                  "keep": table_model.keeps_with_next(row),
                  "cells": [[cell_entry(entry) for entry in cell.paragraphs] for cell in row.cells]}
                 for row in flow.rows],
        "bottom_border": number(flow.bottom_border),
        "header_rows": table_model.header_rows(flow),
    }


def item_record(item, paragraphs: dict, breaks: dict) -> dict:
    if isinstance(item, paginate.Obstacle):
        return {"kind": "obstacle", "block": item.block, "section": item.section}
    if isinstance(item, paginate.TableItem):
        return {"kind": "table", "block": item.block, "section": item.section, "rows": item.count,
                "section_start": item.section_start, "table": table_record(item.flow)}
    if item.placeholder is not None:  # docx2svg's own name for its stand-in box
        return {"kind": "box", "block": item.block, "section": item.section}
    paragraph = paragraphs.get(item.block)
    lines = breaks.get(id(paragraph)) if paragraph is not None else None
    return {
        "kind": "para", "block": item.block, "section": item.section,
        "heights": [height(h) for h in item.heights],
        "lines": lines,
        "page_breaks": sorted(item.page_breaks), "column_breaks": sorted(item.column_breaks),
        "undrawn": sorted(item.undrawn),
        "before": item.before, "after": item.after, "contextual": item.contextual, "style": item.style,
        "border": number(item.border), "top_border": number(item.top_border),
        "keep_next": item.keep_next, "keep_lines": item.keep_lines, "page_break_before": item.page_break_before,
        "widow": item.widow_control, "section_start": item.section_start, "section_mark": item.section_mark,
        "approximations": [list(a) for a in item.approximations],
        "notes": bool(item.notes) or bool(getattr(paragraph, "note", None)) or any(
            kind.startswith(("footnoteReference:", "endnoteReference:"))
            for run in (paragraph.runs if paragraph is not None else ()) for _, kind in run.breaks),
        "anchors": len(item.anchors), "drop_cap": item.drop_cap is not None,
    }


class Capture:
    """The body's last flow and pagination, and the line breaker's lines per paragraph."""

    def __init__(self) -> None:
        self.flows: list = []
        self.last = None
        self.advances = None

    def flow(self, original, document, *args, **kwargs):
        self.flows.append({})
        try:
            items = original(document, *args, **kwargs)
        finally:
            breaks = self.flows.pop()
        items.layout_breaks = breaks
        return items

    def break_paragraph(self, original, document, paragraph, section, advances, metrics=None, label=None):
        self.advances = advances
        items, lines = original(document, paragraph, section, advances, metrics, label)
        if self.flows:
            self.flows[-1][id(paragraph)] = [[line.start, line.end, line.forced] for line in lines]
        return items, lines

    def paginate(self, original, document, items, *args, **kwargs):
        result = original(document, items, *args, **kwargs)
        self.last = (document, items, result)
        return result


def layout_record(capture: Capture, layout) -> dict:
    document, items, result = capture.last
    paragraphs = {block: item for item, block in paginate._blocks_of_body(document)
                  if not isinstance(item, paginate.Table)}
    breaks = getattr(items, "layout_breaks", {})
    pages = []
    for page in layout.pages:
        info = page.info
        pages.append({
            "info": None if info is None else [info.number, info.section, info.section_first, info.story, info.blank],
            "lines": [[line.path, line.number, number(line.top), number(line.pitch), line.baseline, line.column]
                      for line in page.lines if line.story is None],
            "stop": None if page.stop is None else page.stop.reason,
        })
    fields = [[block, k, run.text]
              for block, paragraph in enumerate(fields_module.body_paragraphs(document.body))
              for k, run in enumerate(paragraph.runs)
              if run.field is not None and run.field.keyword in ("PAGE", "NUMPAGES", "SECTIONPAGES", "PAGEREF")]
    return {
        "fields": fields,
        "flow": [item_record(item, paragraphs, breaks) for item in items],
        "starts": [position(p) for p in result.starts],
        "info": [[k, i.number, i.section, i.section_first, i.story, i.blank] for k, i in sorted(result.info.items())],
        "stopped": result.stopped,
        "pages": pages,
        "warnings": sorted({code for code, *_ in getattr(layout, "warnings", [])}) if hasattr(layout, "warnings")
        else [],
    }


def record_document(path: Path, write, timings: list) -> None:
    package = path.read_bytes()
    capture = Capture()
    originals = (paginate.flow, paginate.paginate, linebreak.break_paragraph)
    paginate.flow = lambda *a, **k: capture.flow(originals[0], *a, **k)
    paginate.paginate = lambda *a, **k: capture.paginate(originals[1], *a, **k)
    linebreak.break_paragraph = lambda *a, **k: capture.break_paragraph(originals[2], *a, **k)
    started = time.perf_counter()
    write({"document": {"path": str(path.resolve())}})
    try:
        layout = convert_docx_to_layout(package)
    except Exception as error:  # noqa: BLE001 -- a document docx2svg refuses is reported, not fatal
        write({"refused": f"{type(error).__name__}: {error}"})
        layout = None
    finally:
        paginate.flow, paginate.paginate, linebreak.break_paragraph = originals
    fonts = capture.advances
    if fonts is not None:
        write({"fonts": {
            "dirs": [[location(str(d)), str(d)] for d in fonts.dirs],
            "substitutes": fonts.substitutes, "recorded_faces": fonts.recorded_faces,
            "fallbacks": fonts.fallbacks, "embedded": embedded_fonts(package),
            "font_table": font_table_classes(package),
        }})
    if layout is not None and capture.last is not None:
        write({"layout": layout_record(capture, layout)})
    timings.append([str(path), round(time.perf_counter() - started, 3)])


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("out")
    parser.add_argument("paths", nargs="+")
    args = parser.parse_args(argv)
    timings: list = []
    opener = gzip.open if args.out.endswith(".gz") else open
    with opener(args.out, "wt", encoding="utf-8") as out:
        def write(record: dict) -> None:
            out.write(json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n")

        for path in documents(args.paths):
            record_document(path, write, timings)
            print(f"{path}", file=sys.stderr)
    print(json.dumps(timings), file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
