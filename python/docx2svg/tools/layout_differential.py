#!/usr/bin/env python3
"""The Word layout's differential: what docx2svg's layout made of each document, box by box.

Lays every document given out with docx2svg (``convert_docx_to_layout``) and records, for
the body's last layout pass, its flow -- every top-level item, a paragraph with its lines
(the line breaker's pieces from ``start`` to ``end``), each line's height, its spacing,
borders, keeps and section facts; a table, a stand-in box or an obstacle by its kind --
its pagination (every page's start and :class:`docx2svg.paginate.PageInfo`, and the
footnotes at its foot: how many lines of a note the page before could not hold, and of
each note how many of its lines are on the page), every body line the layout drew (its
paragraph's path, its number, its top, pitch and baseline, its column), and apart from
them the footnotes' lines drawn at its foot; and of each page its frame (its size in px,
its text column's edges and its text area's top and bottom) and the body blocks it draws
something of (``Page.blocks``).  Nothing of docx2svg changes:
``paginate.flow``, ``paginate.paginate`` and ``linebreak.break_paragraph`` are wrapped for
the run of this tool only.

The Rust layout (``crates/ooxml-word-layout``) lays the same documents out over the same
fonts -- ``cargo run -p ooxml-oracle --bin layout-diff -- <records>`` -- and reports every
difference.

Usage::

    python tools/layout_differential.py OUT.jsonl.gz DOCX_OR_FOLDER...

Each line of the output is a JSON object: ``{"document": ...}``, then ``{"fonts": ...}``
(as ``text_differential.py`` writes it) and ``{"layout": ...}``, or ``{"refused": ...}``;
with ``--svg`` also ``{"svg": [...]}``, every page as ``convert_docx_to_svg`` writes it
(``cargo run -p ooxml-oracle --bin svg-diff -- <records>``).
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

from docx2svg import convert_docx_to_layout, convert_docx_to_svg, linebreak, paginate  # noqa: E402
from docx2svg import fields as fields_module  # noqa: E402
from docx2svg.fonts import font_table_classes  # noqa: E402
from text_differential import documents, embedded_fonts, location, number  # noqa: E402


#: Where a footnote's (or its separator's) line comes from: drawn at the page's foot, not
#: a line of the body's flow (endnotes are).
FOOTNOTE_PATHS = ("w:footnote",)


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


def update_records(document, layout) -> tuple[list, list]:
    """What docx-agent's ``word_fields`` update writes, read from this layout as it reads
    it (``edit.fields._page_field``, ``_page_text``): each body page-number field's result
    on the page its paragraph is first drawn on -- ``[path, run, keyword, text or None]``,
    a ``PAGEREF`` through its bookmark's paragraph -- and every body paragraph's
    ``PAGEREF x \\h`` (a table of contents entry's page), ``[path, text]``."""
    first: dict[str, object] = {}
    for page in layout.pages:
        for line in page.lines:
            if line.story is None:
                first.setdefault(line.path, page)
    stopped = any(page.stop is not None for page in layout.pages)

    def fmt(info):
        value = document.sections[info.section].page_number_format if info is not None else None
        return None if value == "decimal" else value

    def page_text(instruction, path):
        page = first.get(path)
        if page is None:
            return None
        info = page.info
        number_ = info.number if info is not None else page.number + 1
        try:
            return fields_module.field_text(instruction.replace("PAGEREF", "PAGE", 1), "PAGE", page=number_,
                                            pages=None, section_pages=None, section_format=fmt(info))
        except fields_module.Uncomputable:
            return None

    paragraphs = fields_module.body_paragraphs(document.body)
    marks = fields_module.bookmarks(paragraphs)
    updates = []
    for paragraph in paragraphs:
        for k, run in enumerate(paragraph.runs):
            if run.field is None or run.field.keyword not in ("PAGE", "NUMPAGES", "SECTIONPAGES", "PAGEREF"):
                continue
            keyword, instruction = run.field.keyword, run.field.instruction
            text = None
            if keyword == "PAGEREF":
                name = fields_module._argument(instruction)
                target = marks.get(name) if name else None
                if target is not None:
                    text = page_text(instruction, paragraphs[target[0][0]].path)
            elif paragraph.path in first:
                page = first[paragraph.path]
                info = page.info
                section_pages = None
                if info is not None and not stopped:
                    section_pages = sum(1 for p in layout.pages if p.info is not None
                                        and p.info.section == info.section and not p.info.blank)
                try:
                    text = fields_module.field_text(instruction, keyword,
                                                    page=info.number if info else page.number + 1,
                                                    pages=None if stopped else len(layout.pages),
                                                    section_pages=section_pages, section_format=fmt(info))
                except fields_module.Uncomputable:
                    text = None
            updates.append([paragraph.path, k, keyword, text])
    references = [[paragraph.path, page_text(" PAGEREF x \\h ", paragraph.path)] for paragraph in paragraphs]
    return updates, references


def layout_record(capture: Capture, layout) -> dict:
    document, items, result = capture.last
    paragraphs = {block: item for item, block in paginate._blocks_of_body(document)
                  if not isinstance(item, paginate.Table)}
    breaks = getattr(items, "layout_breaks", {})
    pages = []
    for page in layout.pages:
        info = page.info
        body = [line for line in page.lines if line.story is None]
        pages.append({
            "info": None if info is None else [info.number, info.section, info.section_first, info.story, info.blank],
            "lines": [[line.path, line.number, number(line.top), number(line.pitch), line.baseline, line.column]
                      for line in body if not line.path.startswith(FOOTNOTE_PATHS)],
            "note_lines": [[line.path, line.number, number(line.top), number(line.pitch), line.baseline, line.column]
                           for line in body if line.path.startswith(FOOTNOTE_PATHS)],
            "stop": None if page.stop is None else page.stop.reason,
            "frame": [page.width_px, page.height_px] + [number(v) for v in page.column],
            "blocks": page.blocks,
        })
    fields = [[block, k, run.text]
              for block, paragraph in enumerate(fields_module.body_paragraphs(document.body))
              for k, run in enumerate(paragraph.runs)
              if run.field is not None and run.field.keyword in ("PAGE", "NUMPAGES", "SECTIONPAGES", "PAGEREF")]
    updates, references = update_records(document, layout)
    notes = [[page, len(carried), carried[0].note if carried else None,
               [[lines[0].note if lines else None, taken, len(lines)] for lines, taken in placed]]
             for page, (carried, placed) in sorted(result.notes.items())]
    return {
        "fields": fields,
        "updates": updates,
        "references": references,
        "notes": notes,
        "flow": [item_record(item, paragraphs, breaks) for item in items],
        "starts": [position(p) for p in result.starts],
        "info": [[k, i.number, i.section, i.section_first, i.story, i.blank] for k, i in sorted(result.info.items())],
        "stopped": result.stopped,
        "pages": pages,
        "warnings": sorted({code for code, *_ in getattr(layout, "warnings", [])}) if hasattr(layout, "warnings")
        else [],
    }


def record_document(path: Path, write, timings: list, svg: bool = False) -> None:
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
        if svg:
            # Every page as convert_docx_to_svg writes it (laid out again, deterministic).
            write({"svg": convert_docx_to_svg(package)})
    timings.append([str(path), round(time.perf_counter() - started, 3)])


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("out")
    parser.add_argument("paths", nargs="+")
    parser.add_argument("--svg", action="store_true", help="also record every page's SVG (for svg-diff)")
    args = parser.parse_args(argv)
    timings: list = []
    opener = gzip.open if args.out.endswith(".gz") else open
    with opener(args.out, "wt", encoding="utf-8") as out:
        def write(record: dict) -> None:
            out.write(json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n")

        for path in documents(args.paths):
            record_document(path, write, timings, svg=args.svg)
            print(f"{path}", file=sys.stderr)
    print(json.dumps(timings), file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
