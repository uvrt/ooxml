"""docx2svg's layout of one document as the Rust kernels' input and reference (moved in from
the performance lanes' prototype).

    python export.py in.docx out.json [--edit KIND@POS]

Lays the document out with docx2svg (as docx-agent does: its layout options), and writes:

* ``fonts`` -- per face the layout measured with (docx2svg's own ``InstalledFonts``): its
  vertical metrics, the advance of every character the document uses, and every non-zero
  legacy kern pair between those characters.  Nothing comes from anywhere but docx2svg.
* ``formats`` -- the character formats (face, sizes, position, border) the text uses.
* ``items`` -- the final pass's flow: each top-level paragraph with what measuring it needs
  (its runs as text and formats, its label, its geometry, its line rule) and docx2svg's
  results (pieces' widths, lines, line heights), and its pagination properties; each table
  with its rows, cells and cell paragraphs the same way.
* ``pages`` -- docx2svg's pagination: each page's start position and page info.

Every length is an integer: px over ``px_den``, layout-unit widths over ``w_den`` (the least
common multiple of every denominator met, checked exact).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import sys
import time
from fractions import Fraction

import docx2svg
from docx2svg import linebreak, paginate, vertical
from docx2svg import lines as line_model
from docx2svg import table as table_model
from docx2svg.resolve import character_format, resolve_mark, resolve_paragraph, resolve_run, script_half_points

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))


class Px:
    __slots__ = ("v",)

    def __init__(self, v):
        self.v = Fraction(v)


class Wd:
    __slots__ = ("v",)

    def __init__(self, v):
        self.v = Fraction(v)


RULES = {"auto": 0, "exact": 1, "atLeast": 2}
KINDS = {linebreak.GLYPH: 0, linebreak.SPACE: 1, linebreak.TAB: 2, linebreak.BREAK: 3, linebreak.SOFT_HYPHEN: 4,
         linebreak.OBJECT: 5}


class Exporter:
    def __init__(self, document, advances, metrics):
        self.document = document
        self.advances = advances
        self.metrics = metrics
        self.fonts: dict = {}  # (face, bold, italic) -> index
        self.font_list: list = []
        self.formats: dict = {}
        self.format_list: list = []
        self.styles: dict = {None: -1}
        self.chars: set = set()
        self.kern_fonts: set = set()

    def style(self, name):
        if name not in self.styles:
            self.styles[name] = len(self.styles) - 1
        return self.styles[name]

    def font(self, face, bold, italic):
        key = (face, bool(bold), bool(italic))
        if key not in self.fonts:
            self.fonts[key] = len(self.font_list)
            self.font_list.append(key)
        return self.fonts[key]

    def fmt(self, f, width_hp=None):
        """A character format's index: font, size, width size (a script's), position, border."""
        font = self.font(f.face, f.bold, f.italic) if f.face else -1
        key = (font, f.half_points, width_hp if width_hp is not None else f.half_points, f.position or 0,
               tuple(f.border) if f.border else None)
        if key not in self.formats:
            self.formats[key] = len(self.format_list)
            self.format_list.append(key)
        return self.formats[key]

    def width_hp(self, f):
        if not f.vertical_align:
            return f.half_points
        m = self.metrics(f.face, f.bold, f.italic)
        return script_half_points(f.half_points, f.vertical_align, m)

    # -- a paragraph's measuring input -------------------------------------------------

    def measure_input(self, paragraph, section, label, pp=None):
        document = self.document
        pp = pp or resolve_paragraph(document, paragraph)
        runs = []
        for run in paragraph.runs:
            resolved = resolve_run(document, paragraph, run)
            if run.note_number:
                raise SystemExit("a note number: not exported")
            entry = {"text": run.text, "vanish": bool(resolved.get("vanish"))}
            if not entry["vanish"]:
                fmts = []
                for char in run.text:
                    f = character_format(resolved, char, document)
                    fmts.append(self.fmt(f, self.width_hp(f)))
                    drawn = char.upper() if resolved.get("caps") else char
                    self.chars.add(drawn)
                    self.chars.add(char)
                entry["fmts"] = fmts if len(set(fmts)) > 1 else (fmts[:1] or [self.fmt(character_format(resolved, " ", document))])
                entry["tracking"] = linebreak.twips_to_units(resolved.get("spacing") or 0) if resolved.get("spacing") else 0
                entry["scale"] = resolved.get("w") or 100
                entry["kern"] = resolved.get("kern") or 0
                entry["caps"] = bool(resolved.get("caps"))
                # a run's hyphen ("-") and space, for marks
                hy = character_format(resolved, "-", document)
                entry["hyphen_fmt"] = self.fmt(hy, self.width_hp(hy))
                self.chars.add("-")
                if entry["kern"]:
                    for k in set(entry["fmts"]):
                        self.kern_fonts.add(self.format_list[k][0])
            entry["marks"] = [[pos, kind] for pos, kind in run.breaks]
            for _, kind in run.breaks:
                if kind == "noBreakHyphen":
                    self.chars.add("\u2011")
                if kind not in ("tab", "br", "textWrapping", "page", "column", "cr", "noBreakHyphen", "softHyphen"):
                    raise SystemExit(f"mark {kind!r}: not exported")
            runs.append(entry)
        out = {"runs": runs}
        mark = resolve_mark(document, paragraph)
        out["mark_fmt"] = self.fmt(character_format(mark, " ", document))
        if label is not None:
            out["label"] = self.label_input(paragraph, label, pp, mark)
        # height items of the label (lines.label_format: the level's text with "1"s)
        lf = line_model.label_format(document, paragraph, pp, mark)
        if any(isinstance(x, line_model.PictureLabel) for x in lf):
            raise SystemExit("picture bullet: not exported")
        out["label_height_fmts"] = [self.fmt(x) for x in lf]
        g = linebreak.geometry(document, paragraph, section, pp)
        if g.hyphen_zone is not None or g.hyphen_gap is not None or g.hyphen_limit:
            raise SystemExit("hyphenation: not exported")
        out["geometry"] = {"first_start": Wd(g.first_start), "start": Wd(g.start), "right": Wd(g.right),
                           "tabs": [[Wd(t.position), t.alignment] for t in g.tabs],
                           "default_tab": Wd(g.default_tab),
                           "hanging_stop": Wd(g.hanging_stop) if g.hanging_stop is not None else None,
                           "squeeze": g.squeeze, "tab_origin": Wd(g.tab_origin)}
        out["rule"] = RULES[pp.get("spacing.lineRule", "auto")]
        out["line"] = pp.get("spacing.line", 240)
        out["columns"] = linebreak.is_multi_column(section)
        return out

    def label_input(self, paragraph, text, pp, mark):
        from docx2svg.resolve.cascade import Resolved

        document = self.document
        num_id, ilvl = pp.get("numPr.numId"), pp.get("numPr.ilvl", 0) or 0
        level = document.numbering.get(num_id, {}).get(ilvl) if num_id else None
        if level is None:
            return None
        if level.picture_bullet is not None:
            raise SystemExit("picture bullet: not exported")
        values = dict(mark.values)
        values.update(level.run)
        resolved = Resolved(values, dict(mark.origins))
        caps = bool(resolved.get("caps"))
        chars = []
        for char in text:
            f = character_format(resolved, char, document)
            drawn = char.upper() if caps else char
            self.chars.add(drawn)
            chars.append([drawn, self.fmt(f)])
        sp = character_format(resolved, " ", document)
        self.chars.add(" ")
        alignment = getattr(level, "alignment", None)
        return {"chars": chars, "align": "center" if alignment == "center" else (
            "right" if alignment in ("right", "end") else "left"),
                "suffix": level.suffix or "tab", "space_fmt": self.fmt(sp)}

    # -- docx2svg's own results ----------------------------------------------------------

    @staticmethod
    def height(h):
        return [Px(h.pitch), Px(h.text_above), Px(h.text_below), RULES[h.rule], h.line, Px(h.natural)]

    @staticmethod
    def ref(pieces, broken):
        return {"widths": [Wd(p.width) for p in pieces], "kinds": [KINDS[p.kind] for p in pieces],
                "lines": [[ln.start, ln.end, int(ln.forced) | 2 * int(ln.emergency) | 4 * int(ln.hyphenated)]
                          for ln in broken]}


def digest(item) -> str:
    """A top-level block's content, as the change set names it (not where it stands)."""
    h = hashlib.blake2b(digest_size=12)

    def para(p):
        h.update(repr(p.properties).encode())
        for run in p.runs:
            h.update(repr((run.text, run.properties, run.breaks)).encode())

    if isinstance(item, docx2svg.model.Table):
        h.update(repr(item.properties).encode())
        for row in item.rows:
            h.update(b"row")
            for cell in row:
                h.update(b"cell")
                for inner in cell:
                    if isinstance(inner, docx2svg.model.Table):
                        raise SystemExit("nested table: not exported")
                    para(inner)
    else:
        para(item)
    return h.hexdigest()


def export(data: bytes, options, lines_only: bool = False):
    captured = []
    original = paginate.paginate

    def capture(document, items, *a, **k):
        result = original(document, items, *a, **k)
        captured.append((document, items, result))
        return result

    advances_box = {}
    orig_broken = line_model.broken

    def broken(document, paragraph, section, advances, metrics=None, label=None):
        advances_box["a"] = (advances, metrics)
        return orig_broken(document, paragraph, section, advances, metrics, label)

    paginate.paginate = capture
    line_model.broken = broken
    try:
        t, w = time.process_time(), time.perf_counter()
        lay, _, fonts = docx2svg.lay_out_docx(data, options, keep=False)
        timing = {"layout_cpu": round(time.process_time() - t, 3), "layout_wall": round(time.perf_counter() - w, 3)}
    finally:
        paginate.paginate = original
        line_model.broken = orig_broken
    first_doc = captured[0][0]
    document, items, result = captured[-1]
    advances, metrics = advances_box["a"]
    ex = Exporter(document, advances, metrics)
    skipped: dict = {}
    if lines_only:
        return export_lines(ex, document, items, timing)
    if items.wraps or items.columns or items.separators or getattr(items, "page_stories", None):
        raise SystemExit("wraps/columns/notes/page stories: not exported")
    mode = document.compatibility_mode
    body_blocks = paginate._blocks_of_body(document)
    first_blocks = paginate._blocks_of_body(first_doc)
    para_of = {block: item for item, block in body_blocks}
    digests = {block: digest(item) for item, block in first_blocks}
    out_items = []
    for item in items:
        if isinstance(item, paginate.Para):
            if item.notes or item.source is not None or item.placeholder is not None or item.drop_cap is not None \
                    or item.continuation is not None or item.anchors or item.auto_hyphen_ends:
                raise SystemExit(f"paragraph {item.block}: not exported")
            p = para_of[item.block]
            section = document.sections[item.section]
            label = items.labels.get(item.block)
            pp = resolve_paragraph(document, p)
            pieces, br = line_model.broken(document, p, section, advances, metrics, label)
            assert len(br) == item.count, (item.block, len(br), item.count)
            out_items.append({
                "t": "p", "block": item.block, "digest": digests.get(item.block), "section": item.section,
                "before": item.before, "after": item.after, "contextual": item.contextual,
                "style": ex.style(item.style), "border": Px(item.border), "top_border": Px(item.top_border),
                "keep_next": item.keep_next, "keep_lines": item.keep_lines, "pbb": item.page_break_before,
                "widow": item.widow_control, "section_start": item.section_start,
                "section_mark": item.section_mark, "page_breaks": sorted(item.page_breaks),
                "heights": [ex.height(h) for h in item.heights],
                "measure": ex.measure_input(p, section, label, pp),
                "ref": ex.ref(pieces, br),
            })
        elif isinstance(item, paginate.TableItem):
            f = item.flow
            if f.floating is not None or table_model._merges(f) or f.approximation is not None:
                raise SystemExit(f"table {item.block}: not exported")
            rows = []
            table = f.table
            for row in f.rows:
                tr = row.row.properties.get("trHeight")
                cells = []
                for cell in row.cells:
                    paras = []
                    for cp in cell.paragraphs:
                        if cp.paragraph is None or cp.table is not None or cp.drops or cp.positioned:
                            raise SystemExit("cell content: not exported")
                        csec = _cell_section(document, item, f, cell)
                        m = ex.measure_input(cp.paragraph, csec, cp.label, cp.resolved)
                        # the cell's geometry as docx2svg computed it (the grid is resolution)
                        g = cp.geometry
                        m["geometry"] = {"first_start": Wd(g.first_start), "start": Wd(g.start), "right": Wd(g.right),
                                         "tabs": [[Wd(t.position), t.alignment] for t in g.tabs],
                                         "default_tab": Wd(g.default_tab),
                                         "hanging_stop": Wd(g.hanging_stop) if g.hanging_stop is not None else None,
                                         "squeeze": g.squeeze, "tab_origin": Wd(g.tab_origin)}
                        widow = cp.resolved.get("widowControl")
                        paras.append({
                            "block": cp.block, "before": cp.before, "after": cp.after, "contextual": cp.contextual,
                            "style": ex.style(cp.style), "effective_style": ex.style(cp.effective_style),
                            "top_border": Px(cp.top_border), "border": Px(cp.border), "hidden": cp.hidden,
                            "widow": None if widow is None else bool(widow), "keep_next": bool(cp.resolved.get("keepNext")),
                            "heights": [ex.height(h) for h in cp.heights], "measure": m,
                            "ref": ex.ref(cp.pieces, cp.lines)})
                    index = {id(cp): k for k, cp in enumerate(cell.paragraphs)}
                    cells.append({
                        "mt": Px(vertical.twips_to_px(cell.cell.margins["top"])),
                        "mb": Px(vertical.twips_to_px(cell.cell.margins["bottom"])),
                        "merge": cell.cell.merge, "paragraphs": paras, "after": Px(cell.after),
                        "content": Px(cell.content),
                        "stack": [[Px(s.above), Px(s.pitch), Px(s.below), s.number, index[id(s.paragraph)]]
                                  for s in cell.stack]})
                rows.append({"top_border": Px(row.top_border), "height": Px(row.height),
                             "header": bool(row.row.properties.get("tblHeader")),
                             "cant_split": bool(row.row.properties.get("cantSplit")),
                             "keeps_next": table_model.keeps_with_next(row),
                             "tr_height": [Px(vertical.twips_to_px(tr[0])) if tr[0] else Px(0), tr[1]] if tr else None,
                             "cells": cells})
            first = f.rows[0].cells[0].paragraphs[0] if f.rows and f.rows[0].cells and \
                f.rows[0].cells[0].paragraphs else None
            out_items.append({
                "t": "t", "block": item.block, "digest": digests.get(item.block), "section": item.section,
                "section_start": item.section_start, "bottom_border": Px(f.bottom_border),
                "previous_style": ex.style(f.previous_style), "default_style": ex.style(f.default_style),
                "first_style": ex.style(first.style) if first is not None else -2,
                "rows": rows})
        else:
            raise SystemExit(f"flow item {type(item).__name__}: not exported")
    # fonts: metrics, advances of every character met, kern pairs among them
    chars = sorted(ex.chars | {" ", "-"})
    font_out = []
    for k, (face, bold, italic) in enumerate(ex.font_list):
        m = metrics(face, bold, italic)
        adv = {}
        upm = None
        for c in chars:
            a = advances.advance(face, bold, italic, c)
            if a is not None:
                adv[c] = a[0]
                if upm is None:
                    upm = a[1]
                elif upm != a[1]:
                    raise SystemExit("a glyph fallback of another unitsPerEm: not exported")
        kern = []
        if k in ex.kern_fonts:
            for left in chars:
                for right in chars:
                    v = advances.kern(face, bold, italic, left, right)
                    if v is None:
                        raise SystemExit(f"no kerning for {face}")
                    if v:
                        kern.append([left, right, v])
        font_out.append({"face": face, "bold": bold, "italic": italic, "upm": upm or (m.units_per_em if m else 0),
                         "metrics": [m.units_per_em, m.ascent, m.descent, m.line_gap, m.superscript_size, m.subscript_size,
                                     m.superscript_offset, m.subscript_offset] if m else None,
                         "adv": adv, "kern": kern, "kern_known": k in ex.kern_fonts})
    sections = []
    for number, s in enumerate(document.sections):
        stories = items.stories.get(number)
        sections.append({
            "top": Px(vertical.twips_to_px(abs(s.margins.top))), "top_neg": s.margins.top < 0,
            "header": Px(vertical.twips_to_px(s.margins.header)),
            "bottom": Px(vertical.twips_to_px(s.page_size.height_twips - abs(s.margins.bottom))),
            "bottom_neg": s.margins.bottom < 0,
            "footer": Px(vertical.twips_to_px(s.page_size.height_twips - s.margins.footer)),
            "title_page": bool(s.title_page), "start_type": s.start_type,
            "new_page": paginate._new_page_section(document, number) if number else True,
            "page_number_start": s.page_number_start,
            "stories": {k: [Px(a), Px(b)] for k, (a, b) in stories.items()} if stories else None})
    pages = [{"start": [s.item, s.line, list(s.part)]} for s in result.starts]
    info = [[i.number, i.section, i.section_first, i.story, i.blank] for _, i in sorted(result.info.items())]
    return {
        "compat": mode, "even_odd": bool(document.even_and_odd_headers), "dpi": vertical.DEVICE_DPI,
        "styles": len(ex.styles), "fonts": font_out,
        "formats": [list(f) for f in ex.format_list], "sections": sections, "items": out_items,
        "pages": pages, "info": info, "stopped": result.stopped, "timing": timing,
    }


def para_entry(ex, block, section_number, label, p, pp, pieces, br, heights, measure):
    return {"t": "p", "block": block, "digest": None, "section": section_number, "before": 0, "after": 0,
            "contextual": False, "style": -1, "border": Px(0), "top_border": Px(0), "keep_next": False,
            "keep_lines": False, "pbb": False, "widow": None, "section_start": False, "section_mark": False,
            "page_breaks": [], "heights": [ex.height(h) for h in heights], "measure": measure,
            "ref": ex.ref(pieces, br)}


def export_lines(ex, document, items, timing):
    """Line breaking only: every paragraph of the flow (body and cells) the measure kernel
    can take, as paragraphs; what it cannot, counted by reason."""
    advances, metrics = ex.advances, ex.metrics
    para_of = {block: item for item, block in paginate._blocks_of_body(document)}
    out_items, skipped = [], {}

    def skip(why):
        why = str(why)[:60]
        skipped[why] = skipped.get(why, 0) + 1

    for item in items:
        try:
            if isinstance(item, paginate.Para):
                if item.placeholder is not None:
                    skip("stand-in box")
                    continue
                p = para_of[item.block]
                section = document.sections[item.section]
                label = items.labels.get(item.block)
                pp = resolve_paragraph(document, p)
                pieces, br = line_model.broken(document, p, section, advances, metrics, label)
                if pieces is None:
                    skip("unmeasurable")
                    continue
                if item.source is not None or len(br) != item.count:
                    skip("broken again beside a drawing")
                    continue
                out_items.append(para_entry(ex, item.block, item.section, label, p, pp, pieces, br, item.heights,
                                            ex.measure_input(p, section, label, pp)))
            elif isinstance(item, paginate.TableItem):
                for row in item.flow.rows:
                    for cell in row.cells:
                        for cp in cell.paragraphs:
                            try:
                                if cp.paragraph is None or cp.drops or cp.positioned:
                                    skip("cell content")
                                    continue
                                m = ex.measure_input(cp.paragraph, document.sections[item.section], cp.label,
                                                     cp.resolved)
                                g = cp.geometry
                                m["geometry"] = {"first_start": Wd(g.first_start), "start": Wd(g.start),
                                                 "right": Wd(g.right),
                                                 "tabs": [[Wd(t.position), t.alignment] for t in g.tabs],
                                                 "default_tab": Wd(g.default_tab),
                                                 "hanging_stop": Wd(g.hanging_stop) if g.hanging_stop is not None
                                                 else None, "squeeze": g.squeeze, "tab_origin": Wd(g.tab_origin)}
                                out_items.append(para_entry(ex, cp.block, item.section, cp.label, cp.paragraph,
                                                            cp.resolved, cp.pieces, cp.lines, cp.heights, m))
                            except SystemExit as why:
                                skip(why)
            else:
                skip(type(item).__name__)
        except SystemExit as why:
            skip(why)
    obj = {"compat": document.compatibility_mode, "even_odd": False, "dpi": vertical.DEVICE_DPI,
           "styles": len(ex.styles), "formats": [list(f) for f in ex.format_list], "sections": [],
           "items": out_items, "pages": [], "info": [], "stopped": None, "timing": timing, "skipped": skipped}
    obj["fonts"] = fonts_out(ex)
    return obj


def fonts_out(ex):
    advances, metrics = ex.advances, ex.metrics
    chars = sorted(ex.chars | {" ", "-"})
    font_out = []
    for k, (face, bold, italic) in enumerate(ex.font_list):
        m = metrics(face, bold, italic) if face else None
        adv = {}
        upm = None
        for c in chars:
            a = advances.advance(face, bold, italic, c) if face else None
            if a is not None:
                if upm is not None and upm != a[1]:
                    continue  # a glyph fallback of another unitsPerEm: left out (the kernel stops at it)
                adv[c] = a[0]
                upm = a[1]
        kern = []
        if k in ex.kern_fonts:
            for left in chars:
                for right in chars:
                    v = advances.kern(face, bold, italic, left, right)
                    if v:
                        kern.append([left, right, v])
        font_out.append({"face": face, "bold": bold, "italic": italic, "upm": upm or (m.units_per_em if m else 0),
                         "metrics": [m.units_per_em, m.ascent, m.descent, m.line_gap, m.superscript_size,
                                     m.subscript_size, m.superscript_offset, m.subscript_offset] if m else None,
                         "adv": adv, "kern": kern, "kern_known": k in ex.kern_fonts})
    return font_out


def _cell_section(document, item, flow, cell):
    """The section a cell's paragraphs were broken in (only its multi-column flag is read)."""
    return document.sections[item.section]


def finish(obj):
    """Every Px and Wd as an integer over one denominator each, checked exact."""
    dens = {"px": vertical.LAYOUT_UNIT_PX.denominator, "w": 1}

    def walk(o):
        if isinstance(o, Px):
            dens["px"] = math.lcm(dens["px"], o.v.denominator)
        elif isinstance(o, Wd):
            dens["w"] = math.lcm(dens["w"], o.v.denominator)
        elif isinstance(o, dict):
            for v in o.values():
                walk(v)
        elif isinstance(o, list):
            for v in o:
                walk(v)

    walk(obj)
    obj["px_den"] = dens["px"]
    obj["w_den"] = dens["w"]
    lu = vertical.LAYOUT_UNIT_PX * dens["px"]
    assert lu.denominator == 1
    obj["lu"] = int(lu)

    def default(o):
        if isinstance(o, Px):
            v = o.v * dens["px"]
        elif isinstance(o, Wd):
            v = o.v * dens["w"]
        else:
            raise TypeError(type(o))
        assert v.denominator == 1
        return int(v)

    return json.dumps(obj, default=default, separators=(",", ":"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("doc")
    ap.add_argument("out")
    ap.add_argument("--lines-only", action="store_true", help="line breaking only, every paragraph it can take")
    ap.add_argument("--edit", help="KIND@POS: one of edits.py's edits, made first")
    a = ap.parse_args()
    from docx_agent import Document
    from docx_agent.layout import layout_options

    data = open(a.doc, "rb").read()
    if a.edit:
        import edits

        kind, pos = a.edit.split("@")
        d = Document.open(data)
        edits.edit(d, kind, pos)
        data = d.to_bytes()
    options = {} if a.lines_only else layout_options(Document.open(data), {})
    obj = export(data, docx2svg.ConvertOptions(**options), lines_only=a.lines_only)
    with open(a.out, "w") as fh:
        fh.write(finish(obj))
    print(a.out, len(obj["items"]), "items", len(obj["pages"]), "pages", obj["timing"], obj.get("skipped", ""),
          flush=True)


if __name__ == "__main__":
    main()
