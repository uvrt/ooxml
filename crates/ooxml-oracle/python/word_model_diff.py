#!/usr/bin/env python3
"""The Word model's differential harness (test support; phase PL, lane L2).

Writes docx2svg's layout input for each document -- its parsed model (``parse_package``),
every body paragraph's and table's resolved formats, each run's format per character slot,
and the body's list labels as the layout counts them -- in the JSON form the Rust model's
``word-model`` binary writes (``crates/ooxml-oracle/src/export.rs``), runs that binary on the
same documents, and compares the two field by field.  Every difference is printed with its
JSON path; the exit status is 1 when there is any.

    python crates/ooxml-oracle/python/word_model_diff.py --rust target/release/word-model a.docx b.docx
    python crates/ooxml-oracle/python/word_model_diff.py --rust ... --tree DIR      # every .docx below DIR

What docx2svg keeps as DrawingML (a shape's fill, outline, effects, geometry; a picture's
crop; ``w14:textFill``) is the DrawingML model's (phase P3) and compared by presence only;
a VML text path's painting settings likewise.  Python's behaviour is not changed: this only
reads docx2svg's public parse and resolve functions.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
for package in ("docx2svg", "ooxml-common"):
    source = ROOT / "python" / package / "src"
    if str(source) not in sys.path:
        sys.path.insert(0, str(source))

from docx2svg.linebreak import ListCounters, text_box_labels  # noqa: E402
from docx2svg.model import Graphic, Paragraph, Table, TabStop  # noqa: E402
from docx2svg.parse.document import _section, parse_package  # noqa: E402
from docx2svg.parse.properties import FontRef  # noqa: E402
from docx2svg.resolve import resolve_mark, resolve_paragraph, resolve_run  # noqa: E402
from docx2svg.resolve.fonts import _character_format  # noqa: E402
from docx2svg.table import SIDES, resolve_table  # noqa: E402

DRAWINGML_KEYS = frozenset({"textFill", "textOutline"})


def value(v, key: str = ""):
    """A declared or resolved property value as the Rust export writes it."""
    if key in DRAWINGML_KEYS:
        return {"drawingml": key}
    if v is None or isinstance(v, (bool, int, str)):
        return v
    if isinstance(v, FontRef):
        return {"face": v.face, "theme": v.theme}
    if isinstance(v, TabStop):
        return [v.position_twips, v.alignment, v.leader]
    if isinstance(v, (tuple, list)):
        return [value(item) for item in v]
    if isinstance(v, dict):
        return {str(k): value(item) for k, item in v.items()}
    return {"unexported": type(v).__name__}


def props(declared: dict) -> dict:
    return {str(k): value(v, str(k)) for k, v in declared.items()}


def run_props(properties):
    if properties is None:
        return None
    return {"style_id": properties.style_id, "declared": props(properties.declared)}


def position(p):
    return {"relative": p.relative, "offset": p.offset, "align": p.align}


def graphic(g: Graphic):
    out = {
        "kind": g.kind, "offset": list(g.offset), "extent": list(g.extent), "rotation": g.rotation,
        "flip_h": g.flip_h, "flip_v": g.flip_v, "relationship": g.relationship, "text": blocks(g.text),
        "children": [graphic(c) for c in g.children], "child_offset": list(g.child_offset),
        "child_extent": list(g.child_extent), "unsupported": list(g.unsupported), "uri": g.uri, "path": g.path,
    }
    if g.kind != "textpath":
        out["body"] = {str(k): v for k, v in g.body.items()}
    return out


def anchor(a):
    return {
        "extent": list(a.extent), "effect": list(a.effect), "h": position(a.h), "v": position(a.v),
        "simple": list(a.simple) if a.simple is not None else None, "wrap": a.wrap, "wrap_text": a.wrap_text,
        "distance": list(a.distance), "behind": a.behind, "relative_height": a.relative_height,
        "allow_overlap": a.allow_overlap, "layout_in_cell": a.layout_in_cell, "locked": a.locked,
        "graphic": graphic(a.graphic) if a.graphic is not None else None, "identifier": a.identifier,
        "polygon": [list(p) for p in a.polygon] if a.polygon is not None else None, "vml": a.vml,
    }


def drawing(d):
    if d is None:
        return None
    if isinstance(d, str):
        return {"picture": str(d)}
    return graphic(d)


def run(r):
    return {
        "text": r.text, "properties": run_props(r.properties), "breaks": [[i, m] for i, m in r.breaks],
        "path": r.path, "drawings": [drawing(d) for d in r.drawings], "anchors": [anchor(a) for a in r.anchors],
        "field": None if r.field is None else {
            "instruction": r.field.instruction, "keyword": r.field.keyword,
            "cached": [[t, run_props(p)] for t, p in r.field.cached],
        },
        "toc_link": r.toc_link,
    }


def section_props(element) -> dict:
    return section_props_of(_section(element, 0, 0))


def paragraph(p: Paragraph):
    pp = p.properties
    return {
        "type": "p", "path": p.path, "para_id": p.para_id, "runs": [run(r) for r in p.runs],
        "properties": None if pp is None else {
            "style_id": pp.style_id, "declared": props(pp.declared), "mark": run_props(pp.run_properties),
        },
        "table_style_id": p.table_style_id, "table_conditions": list(p.table_conditions), "joins": list(p.joins),
        "mark_deleted": p.mark_deleted, "section": section_props(p.section) if p.section is not None else None,
        "bookmarks": [list(b) for b in p.bookmarks],
    }


def table(t: Table):
    return {
        "type": "tbl", "style_id": t.style_id, "path": t.path, "grid": list(t.grid), "properties": props(t.properties),
        "row_properties": [props(r) for r in t.row_properties],
        "cell_properties": [[props(c) for c in r] for r in t.cell_properties],
        "look": dict(t.look), "conditions": [[list(c) for c in r] for r in t.conditions],
        "cell_paths": [list(r) for r in t.cell_paths], "joined": t.joined,
        "rows": [[blocks(cell) for cell in row] for row in t.rows],
    }


def blocks(items) -> list:
    return [paragraph(b) if isinstance(b, Paragraph) else table(b) for b in items]


def border(b):
    return value(b) if b is not None else None


def resolved_table(document, t: Table) -> dict:
    r = resolve_table(document, t)
    return {
        "path": t.path, "properties": props(r.properties), "indent_stated": r.indent_stated, "grid": list(r.grid),
        "mismatched": [list(m) for m in r.mismatched],
        "rows": [{
            "index": row.index, "properties": props(row.properties), "grid_before": row.grid_before,
            "grid_after": row.grid_after,
            "cells": [{
                "row": c.row, "index": c.index, "column": c.column, "span": c.span, "properties": props(c.properties),
                "merge": c.merge, "path": c.path, "margins": [c.margins[s] for s in SIDES],
                "borders": [border(c.borders.get(s)) for s in SIDES], "own": [c.own[s] for s in SIDES],
                "left_edge": [border(c.left_edge[0]), c.left_edge[1]],
                "right_edge": [border(c.right_edge[0]), c.right_edge[1]],
            } for c in row.cells],
        } for row in r.rows],
    }


def formats(document, resolved) -> dict:
    complex_script = bool(resolved.get("cs") or resolved.get("rtl"))
    out = {}
    for slot in (("cs",) if complex_script else ("ascii", "hAnsi", "eastAsia")):
        f = _character_format(resolved, slot, complex_script, document)
        out[slot] = {"face": f.face, "half_points": f.half_points, "bold": f.bold, "italic": f.italic,
                     "vertical_align": f.vertical_align, "position": f.position,
                     "border": list(f.border) if f.border is not None else None}
    return out


def resolved(document) -> tuple[list, list, list]:
    paragraphs, tables, labels = [], [], []
    counters = ListCounters()

    def walk(block) -> None:
        if isinstance(block, Table):
            tables.append(resolved_table(document, block))
            for row in block.rows:
                for cell in row:
                    for inner in cell:
                        walk(inner)
            return
        pp = resolve_paragraph(document, block)
        mark = resolve_mark(document, block)
        runs = [resolve_run(document, block, r) for r in block.runs]
        paragraphs.append({
            "path": block.path, "paragraph": props(pp.values), "mark": props(mark.values),
            "runs": [props(r.values) for r in runs], "formats": [formats(document, r) for r in runs],
            "mark_formats": formats(document, mark),
        })
        labels.append(counters.label(document, block, pp) if pp.get("numPr.numId") else None)

    for block in document.body:
        walk(block)
    return paragraphs, tables, labels


def story_labels(document, items) -> list:
    """A story's labels, one counter for the story, its paragraphs in document order."""
    counters, out = ListCounters(), []

    def walk(block) -> None:
        if isinstance(block, Table):
            for row in block.rows:
                for cell in row:
                    for inner in cell:
                        walk(inner)
            return
        pp = resolve_paragraph(document, block)
        out.append(counters.label(document, block, pp) if pp.get("numPr.numId") else None)

    for block in items:
        walk(block)
    return out


def level(lv) -> dict:
    return {
        "level": lv.level, "format": lv.format, "text": lv.text, "paragraph": props(lv.paragraph), "run": props(lv.run),
        "suffix": lv.suffix, "start": lv.start, "restart": lv.restart, "legal": lv.legal, "alignment": lv.alignment,
        "picture_bullet": lv.picture_bullet, "style": lv.style, "single": lv.single,
    }


def style(s) -> dict:
    return {
        "style_id": s.style_id, "kind": s.kind, "name": s.name, "based_on": s.based_on, "next_style": s.next_style,
        "link": s.link, "default": s.default, "paragraph": props(s.paragraph), "run": props(s.run),
        "conditional": {k: [props(part) for part in parts] for k, parts in s.conditional.items()},
        "table": props(s.table), "table_row": props(s.table_row), "table_cell": props(s.table_cell),
    }


def fonts(f) -> dict:
    return {"latin": f.latin, "east_asian": f.east_asian, "complex_script": f.complex_script,
            "scripts": dict(f.scripts)}


def python_model(data: bytes) -> dict:
    d = parse_package(data)
    paragraphs, tables, labels = resolved(d)
    h = d.hyphenation
    return {
        "body": blocks(d.body),
        "sections": [{**section_props_of(s), "first_paragraph": s.first_paragraph, "last_paragraph": s.last_paragraph}
                     for s in d.sections],
        "styles": None if d.styles is None else {
            "run_defaults": props(d.styles.run_defaults), "paragraph_defaults": props(d.styles.paragraph_defaults),
            "styles": {k: style(v) for k, v in d.styles.styles.items()},
        },
        "font_scheme": None if d.font_scheme is None else {"major": fonts(d.font_scheme.major),
                                                            "minor": fonts(d.font_scheme.minor)},
        "theme_colors": dict(d.theme_colors), "color_map": dict(d.color_map),
        "numbering": {str(n): {str(k): level(lv) for k, lv in levels.items()} for n, levels in d.numbering.items()},
        "numbering_lists": {str(k): v for k, v in d.numbering_lists.items()},
        "numbering_restarts": sorted([list(r) for r in d.numbering_restarts]),
        "picture_bullets": {str(k): {"relationship": b.relationship, "part": b.part,
                                     "pixels": list(b.pixels) if b.pixels else None}
                            for k, b in d.picture_bullets.items()},
        "theme_font_lang": dict(d.theme_font_lang), "compatibility_mode": d.compatibility_mode,
        "default_tab_stop": d.default_tab_stop, "compat_options": sorted(d.compat_options),
        "compat_settings": dict(d.compat_settings), "even_and_odd_headers": d.even_and_odd_headers,
        "hyphenation": None if h is None else {"zone": h.zone, "limit": h.limit, "no_caps": h.no_caps},
        "endnote_position": d.endnote_position, "endnote_named": sorted(d.endnote_named),
        "footnote_named": sorted(d.footnote_named),
        "footnotes": {str(k): blocks(v) for k, v in d.footnotes.items()},
        "footnote_separators": {k: blocks(v) for k, v in d.footnote_separators.items()},
        "footnote_separator_ids": dict(d.footnote_separator_ids),
        "endnotes": {str(k): blocks(v) for k, v in d.endnotes.items()},
        "endnote_separators": {k: blocks(v) for k, v in d.endnote_separators.items()},
        "endnote_separator_ids": dict(d.endnote_separator_ids),
        "stories": {k: blocks(v) for k, v in d.stories.items()}, "story_parts": dict(d.story_parts),
        "cached_fields": list(d.cached_fields), "warnings": list(d.warnings),
        "labels": labels, "resolved_paragraphs": paragraphs, "resolved_tables": tables,
        "story_labels": {k: story_labels(d, v) for k, v in d.stories.items()},
        "text_box_labels": dict(text_box_labels(d)),
    }


def section_props_of(s) -> dict:
    m = s.margins
    return {
        "page_size": [s.page_size.width_twips, s.page_size.height_twips, s.page_size.orientation],
        "margins": [m.top, m.right, m.bottom, m.left, m.header, m.footer, m.gutter],
        "start_type": s.start_type, "column_count": s.column_count, "column_space": s.column_space,
        "columns": [list(c) for c in s.columns], "column_separator": s.column_separator,
        "headers": [list(h) for h in s.headers], "footers": [list(f) for f in s.footers],
        "title_page": s.title_page, "page_number_start": s.page_number_start,
        "page_number_format": s.page_number_format, "vertical_alignment": s.vertical_alignment,
        "endnote_format": s.endnote_format, "endnote_start": s.endnote_start, "endnote_restart": s.endnote_restart,
        "footnote_format": s.footnote_format, "footnote_start": s.footnote_start,
        "footnote_restart": s.footnote_restart, "footnote_position": s.footnote_position,
        "line_numbering": list(s.line_numbering) if s.line_numbering is not None else None,
    }


def differences(a, b, path: str = "", out: list | None = None, limit: int = 200) -> list:
    """Every place ``a`` (Python's) and ``b`` (Rust's) differ, as ``(path, a, b)``."""
    out = [] if out is None else out
    if len(out) >= limit:
        return out
    if isinstance(a, dict) and isinstance(b, dict):
        for key in sorted(set(a) | set(b)):
            if key not in a or key not in b:
                out.append((f"{path}/{key}", a.get(key, "<absent>"), b.get(key, "<absent>")))
            else:
                differences(a[key], b[key], f"{path}/{key}", out, limit)
        return out
    if isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            out.append((f"{path} (length)", len(a), len(b)))
        for k, (x, y) in enumerate(zip(a, b)):
            differences(x, y, f"{path}/{k}", out, limit)
        return out
    if a != b or type(a) is not type(b):
        out.append((path, a, b))
    return out


def rust_models(binary: str, files: list[Path]) -> dict[str, dict]:
    found = subprocess.run([binary, *map(str, files)], capture_output=True, text=True, check=True)
    out = {}
    for line in found.stdout.splitlines():
        record = json.loads(line)
        out[record["file"]] = record
    return out


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--rust", required=True, help="the word-model binary")
    parser.add_argument("--tree", action="append", default=[], help="compare every .docx below this directory")
    parser.add_argument("--json", help="write the per-document results here")
    parser.add_argument("--show", type=int, default=20, help="differences printed per document")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args(argv)
    files = [Path(f) for f in args.files]
    for tree in args.tree:
        files += sorted(Path(tree).rglob("*.docx"))
    files = [f for f in files if not f.name.startswith("~$")]
    rust = rust_models(args.rust, files)
    results, total = {}, 0
    for path in files:
        try:
            python = json.loads(json.dumps(python_model(path.read_bytes())))
        except Exception as error:  # noqa: BLE001 -- docx2svg refuses it: recorded, not compared
            results[str(path)] = {"python_error": repr(error)}
            print(f"{path}: docx2svg refuses it: {error!r}")
            continue
        record = rust.get(str(path), {})
        if "error" in record:
            results[str(path)] = {"rust_error": record["error"]}
            total += 1
            print(f"{path}: the Rust model refuses it: {record['error']}")
            continue
        found = differences(python, record.get("model"))
        results[str(path)] = {"differences": len(found), "first": [[p, repr(a)[:200], repr(b)[:200]] for p, a, b in found[:50]]}
        total += len(found)
        print(f"{path}: {len(found)} difference(s)")
        for p, a, b in found[: args.show]:
            print(f"    {p}: docx2svg {a!r:.160} | rust {b!r:.160}")
    if args.json:
        Path(args.json).write_text(json.dumps({"documents": len(files), "differences": total, "results": results},
                                              indent=1))
    print(f"{len(files)} document(s), {total} difference(s)")
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main())
