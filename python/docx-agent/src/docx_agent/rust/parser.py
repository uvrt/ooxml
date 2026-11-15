"""The Rust owner's first input made by the Rust parser (lane I1).

``ooxml_core.parse_export`` reads the body of ``word/document.xml`` in Rust -- paragraphs,
tables, runs, complex fields threaded through the body, bookmarks, ``REF`` fields replaced
by their bookmarks' text, list labels counted -- and makes the export the mirror makes
(:meth:`docx_agent.rust.mirror.Mirror.export_with`), byte for byte.  What a block's formatting
resolves to it asks of :class:`Resolution` once per distinct input: the style cascade, a
run's character formats, a paragraph's geometry and borders, a table's grid and cells
(docx2svg's own functions, on a model of the package's definitions alone -- styles,
numbering, settings, theme, headers and footers -- never of the body), and each block's
record (:mod:`.records`).  Anything either side does not model refuses the document whole:
the mirror exports it as before (:data:`STATS` counts why).

``DOCX_AGENT_RUST_PARSER=0`` turns the Rust parser off (the mirror's export, as before).
"""
from __future__ import annotations

import copy
import dataclasses
import io
import json
import os
import time
import zipfile
from collections import Counter
from fractions import Fraction
from typing import Any

from .exporter import RULES, Unsupported

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
_WNS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_P, _TBL = _W + "p", _W + "tbl"

#: The environment variable that turns the Rust parser off (``0``).
ENV = "DOCX_AGENT_RUST_PARSER"

#: What the Rust parser did (tests and measurements read it; never in results).
STATS: Counter = Counter()


def enabled() -> bool:
    return os.environ.get(ENV, "1").strip() != "0"


def _q(value: Any) -> list[int]:
    f = Fraction(value)
    return [f.numerator, f.denominator]


def _stub(document: Any) -> bytes:
    """The package with an empty body (its section properties kept): what the definitions'
    model is parsed from -- styles, numbering, settings, theme, headers and footers."""
    from lxml import etree

    package = document.package
    main = package.document_part()
    root = package.tree(main)
    body = root.find(_W + "body")
    last = body[len(body) - 1] if len(body) else None
    section = etree.tostring(last) if last is not None and last.tag == _W + "sectPr" else b""
    declarations = "".join(f' xmlns:{k}="{v}"' if k else f' xmlns="{v}"' for k, v in (root.nsmap or {}).items())
    qname = root.tag.split("}")[1]
    prefix = root.prefix + ":" if root.prefix else ""
    stub = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><{prefix}{qname}{declarations}>'
            f'<{"w:" if "w" in (root.nsmap or {}) else ""}body>').encode() + section + \
        f'</{"w:" if "w" in (root.nsmap or {}) else ""}body></{prefix}{qname}>'.encode()
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_STORED) as z:
        for part in package.part_names:
            data = stub if part == main else package.read(part)
            if data is not None:
                z.writestr(part.lstrip("/"), data)
    return out.getvalue()


class _Ex:
    """What ``exporter.fonts_out`` reads of an exporter."""

    def __init__(self, advances, metrics, fonts, chars, kern):
        self.advances, self.metrics = advances, metrics
        self.font_list = [tuple(f) for f in fonts]
        self.chars = set(chars)
        self.kern_fonts = set(kern)


class Resolution:
    """docx2svg's resolution of the document's definitions, asked by the Rust parser."""

    def __init__(self, document: Any, records: bool = True) -> None:
        import docx2svg
        from docx2svg.layout import _Placer
        from docx_agent.layout import layout_options

        self.document = document
        data = _stub(document)
        options = docx2svg.ConvertOptions(**layout_options(document, {}))
        self.fonts = docx2svg._fonts(data, options)
        self.advances, self.metrics = self.fonts, self.fonts.metrics
        d2 = docx2svg.parse.parse_package(data)
        if options.locale is not None:
            d2.locale = options.locale
        placer = _Placer(d2, data, self.advances, self.metrics, self.fonts.decorations)
        _source, original, features, _ = placer._prepare()
        self.d2 = original
        if len(self.d2.sections) != 1:
            raise Unsupported("more than one section")
        self.section = self.d2.sections[0]
        from docx2svg.linebreak import is_multi_column

        if is_multi_column(self.section):
            raise Unsupported("columns")
        self.paragraphs: dict = {}
        self.tables: dict = {}
        self.heights_cache: dict = {}
        self.records = records
        main = document.package.document_part()
        self.body = document.package.tree(main).find(_W + "body")
        self.elements = [c for c in self.body if c.tag in (_P, _TBL)]
        self.result: dict = {}

    # -- the questions ------------------------------------------------------------------

    def __call__(self, kind: str, arg: str) -> str:
        found = getattr(self, "_" + kind)(json.loads(arg))
        return found if isinstance(found, str) else json.dumps(found, separators=(",", ":"))

    def paragraph(self, key: dict) -> Any:
        k = (key["ppr"], key["tbl"], tuple(key["cond"]))
        found = self.paragraphs.get(k)
        if found is None:
            from docx2svg.parse import document as pdoc
            from docx2svg.resolve import resolve_paragraph
            from ooxml_common.xmlutil import parse_xml

            element = parse_xml(f'<w:p xmlns:w="{_WNS}">{key["ppr"] or ""}</w:p>')
            paragraph = pdoc._paragraph(element, key["tbl"], tuple(key["cond"]))
            found = self.paragraphs[k] = (paragraph, resolve_paragraph(self.d2, paragraph))
        return found

    @staticmethod
    def _fmt(f, width) -> list:
        return [f.face, bool(f.bold), bool(f.italic), f.half_points, width if width is not None else f.half_points,
                f.position or 0, list(f.border) if f.border else None]

    def _width_hp(self, f):
        from docx2svg.resolve import script_half_points

        if not f.vertical_align:
            return f.half_points
        return script_half_points(f.half_points, f.vertical_align, self.metrics(f.face, f.bold, f.italic))

    def _slots(self, resolved, width: bool) -> list:
        from docx2svg.resolve.fonts import _character_format

        complex_script = bool(resolved.get("cs") or resolved.get("rtl"))
        out = []
        for slot in ("ascii", "hAnsi", "eastAsia", "cs"):
            f = _character_format(resolved, "cs" if complex_script else slot, complex_script, self.d2)
            out.append([self._fmt(f, self._width_hp(f)), self._fmt(f, None)] if width else self._fmt(f, None))
        return out

    def _para(self, key: dict) -> dict:
        from docx2svg import linebreak
        from docx2svg.resolve import autospaced
        from docx2svg.resolve.cascade import _paragraph_style_id

        from .mirror import draw_key

        paragraph, pp = self.paragraph(key)
        out: dict = {}
        if pp.get("framePr"):
            return {"refused": True, "why": "frame or anchor"}
        g = linebreak.geometry(self.d2, paragraph, self.section, pp)
        if g.hyphen_zone is not None or g.hyphen_gap is not None or g.hyphen_limit:
            return {"refused": True, "why": "hyphenation: not exported"}
        widow = pp.get("widowControl")
        num_id = pp.get("numPr.numId")
        out.update({
            "before": pp.get("spacing.before", 0) or 0, "after": pp.get("spacing.after", 0) or 0,
            "auto_before": autospaced(self.d2, pp, "before"), "auto_after": autospaced(self.d2, pp, "after"),
            "contextual": bool(pp.get("contextualSpacing")), "keep_next": bool(pp.get("keepNext")),
            "keep_lines": bool(pp.get("keepLines")), "pbb": bool(pp.get("pageBreakBefore")),
            "widow": None if widow is None else bool(widow),
            "num": [num_id if num_id else 0, pp.get("numPr.ilvl", 0) or 0],
            "rule": RULES[pp.get("spacing.lineRule", "auto")], "line": pp.get("spacing.line", 240),
            "draw": draw_key(pp.values), "eff_style": _paragraph_style_id(self.d2.styles, paragraph)})
        if not isinstance(out["num"][0], int):
            raise Unsupported("a list id that is not a number")
        return out

    def _run(self, arg: dict) -> dict:
        from docx2svg.linebreak import twips_to_units
        from docx2svg.model import Run
        from docx2svg.parse import document as pdoc
        from docx2svg.resolve import resolve_run
        from ooxml_common.xmlutil import parse_xml

        from .mirror import draw_key

        paragraph, _ = self.paragraph(arg["p"])
        rpr = arg["rpr"]
        properties = pdoc._run_properties(parse_xml(rpr)) if rpr else None
        run = Run(text="", properties=properties, toc_link=bool(arg["toc"]))
        resolved = resolve_run(self.d2, paragraph, run)
        spacing = resolved.get("spacing")
        return {"vanish": bool(resolved.get("vanish")), "caps": bool(resolved.get("caps")),
                "tracking": twips_to_units(spacing) if spacing else 0, "scale": resolved.get("w") or 100,
                "kern": resolved.get("kern") or 0,
                "va": resolved.get("vertAlign") in ("superscript", "subscript"), "draw": draw_key(resolved.values),
                "cs": bool(resolved.get("cs") or resolved.get("rtl")), "hint": resolved.get("rFonts.hint"),
                "ea_lang": resolved.get("lang.eastAsia"), "slots": self._slots(resolved, True)}

    def _mark(self, arg: dict) -> dict:
        from docx2svg import lines as line_model
        from docx2svg.resolve import character_format, resolve_mark
        from docx2svg.resolve.cascade import Resolved

        paragraph, pp = self.paragraph(arg["p"])
        mark = resolve_mark(self.d2, paragraph)
        out: dict = {"space": self._fmt(character_format(mark, " ", self.d2), None)}
        num_id, ilvl = pp.get("numPr.numId"), pp.get("numPr.ilvl", 0) or 0
        level = self.d2.numbering.get(num_id, {}).get(ilvl) if num_id else None
        if level is None:
            out["label"] = None
        else:
            if level.picture_bullet is not None:
                raise Unsupported("picture bullet: not exported")
            values = dict(mark.values)
            values.update(level.run)
            resolved = Resolved(values, dict(mark.origins))
            alignment = getattr(level, "alignment", None)
            out["label"] = {"caps": bool(resolved.get("caps")), "cs": bool(resolved.get("cs") or resolved.get("rtl")),
                            "hint": resolved.get("rFonts.hint"), "ea_lang": resolved.get("lang.eastAsia"),
                            "slots": self._slots(resolved, False),
                            "align": "center" if alignment == "center" else (
                                "right" if alignment in ("right", "end") else "left"),
                            "suffix": level.suffix or "tab"}
        lf = line_model.label_format(self.d2, paragraph, pp, mark)
        if any(isinstance(x, line_model.PictureLabel) for x in lf):
            raise Unsupported("picture bullet: not exported")
        out["label_height"] = [self._fmt(x, None) for x in lf]
        return out

    def _cell_section(self, budget: int):
        from docx2svg.model import PageMargins, PageSize

        section = self.section
        return dataclasses.replace(section, page_size=PageSize(max(budget, 1), section.page_size.height_twips),
                                   margins=PageMargins(section.margins.top, 0, section.margins.bottom, 0,
                                                       section.margins.header, section.margins.footer, 0),
                                   column_count=1, columns=())

    def _geom(self, arg: dict) -> dict:
        from docx2svg import linebreak

        paragraph, pp = self.paragraph(arg["p"])
        section = self.section if arg["width"] is None else self._cell_section(arg["width"])
        g = linebreak.geometry(self.d2, paragraph, section, pp)
        return {"first_start": _q(g.first_start), "start": _q(g.start), "right": _q(g.right),
                "tabs": [[_q(t.position), t.alignment] for t in g.tabs], "default_tab": _q(g.default_tab),
                "hanging_stop": _q(g.hanging_stop) if g.hanging_stop is not None else None,
                "squeeze": g.squeeze, "tab_origin": _q(g.tab_origin)}

    def _borders(self, arg: dict) -> list:
        from docx2svg.vertical import paragraph_borders_px

        pp = self.paragraph(arg["p"])[1]
        prev = self.paragraph(arg["prev"])[1] if arg["prev"] is not None else None
        nxt = self.paragraph(arg["next"])[1] if arg["next"] is not None else None
        top, bottom = paragraph_borders_px(pp, prev, nxt)
        return [_q(top), _q(bottom)]

    def _levels(self, num_id: int) -> Any:
        levels = self.d2.numbering.get(num_id)
        if not levels:
            return None
        key = self.d2.numbering_lists.get(num_id, ("num", num_id))
        out = []
        for ilvl, level in levels.items():
            if not isinstance(ilvl, int):
                raise Unsupported("a list level that is not a number")
            out.append({"ilvl": ilvl, "text": level.text, "start": level.start, "format": level.format,
                        "legal": bool(level.legal), "restart": getattr(level, "restart", None)})
        return {"key": repr(key), "restarts": [ilvl for n, ilvl in self.d2.numbering_restarts if n == num_id],
                "levels": out}

    # -- tables -------------------------------------------------------------------------

    def _skeleton(self, xml: str) -> Any:
        found = self.tables.get(xml)
        if found is None:
            from docx2svg.parse import document as pdoc
            from lxml.etree import fromstring as parse_xml

            element = parse_xml(xml.encode())
            table, moved, _ = pdoc._table(element, "", pdoc._Fields([], cross_references=True, styleref=True))
            if moved:
                raise Unsupported("a table the final view splits")
            found = self.tables[xml] = (table, element)
        return found

    def _real_paragraph(self, words: dict) -> Any:
        from docx2svg.model import Field, Run
        from docx2svg.parse import document as pdoc
        from ooxml_common.xmlutil import parse_xml

        paragraph, _ = self.paragraph(words["p"])
        runs = []
        for r in words["runs"]:
            properties = pdoc._run_properties(parse_xml(r["rpr"])) if r["rpr"] else None
            field = Field(r["field"][1], r["field"][0]) if r["field"] else None
            runs.append(Run(text=r["text"], properties=properties, breaks=tuple((a, b) for a, b in r["marks"]),
                            field=field, toc_link=bool(r["toc"])))
        return dataclasses.replace(paragraph, runs=tuple(runs))

    def _table(self, arg: dict) -> dict:
        """``flow_table``'s resolution and sizing of a table (the mirror's ``table_item``
        reads no more of it): its cells' content measured for autofit, its grid, each cell's
        width, rows' borders and properties."""
        from docx2svg import linebreak, vertical
        from docx2svg import table as tm
        from docx2svg.vertical import twips_to_px

        from .mirror import _table_draw

        skeleton, element = self._skeleton(arg["xml"])
        cells = iter(arg["cells"])
        rows = tuple(tuple(tuple(self._real_paragraph(w) for w in next(cells)) for _ in row) for row in skeleton.rows)
        table = dataclasses.replace(skeleton, rows=rows)
        document, section = self.d2, self.section
        try:
            resolved = tm.resolve_table(document, table)
            unmodelled = tm._unmodelled(resolved)
            if unmodelled:
                raise Unsupported(unmodelled)
            if tm.floating_of(document, table, resolved.properties) is not None:
                raise Unsupported("floating or merged table")
            page_left = section.margins.left + section.margins.gutter
            column_width = linebreak.column_width_twips(section)
            if tm.in_percent(resolved) and resolved.rows and resolved.rows[0].cells and all(
                    (cell.properties.get("tcW") or (0, None))[1] == "dxa" for cell in resolved.rows[0].cells):
                raise Unsupported("a table in percent over cells in dxa (not read here)")
            autofit = (resolved.properties.get("tblLayout") or "autofit") != "fixed"
            kinds = {(cell.properties.get("tcW") or (0, None))[1] for cell in resolved.rows[0].cells} if (
                resolved.rows and resolved.rows[0].cells) else set()
            if autofit and not (tm.in_percent(resolved) and kinds in ({"pct"}, {"dxa"})):
                try:
                    resolved.extents = {(row.index, cell.index): tm.cell_extent(document, cell, section, self.advances,
                                                                                 self.metrics)
                                        for row in resolved.rows for cell in row.cells if cell.merge != "continue"}
                except tm.Unsupported:
                    if tm.in_percent(resolved) or tm.autofit_unmodelled(resolved):
                        raise
                    resolved.extents = None
            lines = tm.grid_lines(document, resolved, column_width)
            if autofit and resolved.extents is None:
                raise Unsupported("an autofit table not sized from its content (not read here)")
        except tm.Unsupported as why:
            raise Unsupported(f"table: {why.args[0]}") from None
        if len(resolved.grid) != len(table.grid) and autofit:
            raise Unsupported("an autofit table off its grid (not read here)")
        out_rows = []
        columns = len(lines) - 1
        previous_cells: list = []
        for row in resolved.rows:
            out_cells = []
            for cell in row.cells:
                if (cell.properties.get("vAlign") or "top") not in ("top",):
                    raise Unsupported("a cell's vertical alignment")
                if cell.properties.get("hideMark"):
                    raise Unsupported("a cell hiding its mark (not read here)")
                if cell.merge is not None:
                    raise Unsupported("floating or merged table")
                box = tm.cell_box(document, resolved, page_left, lines, cell)
                out_cells.append({"budget": box.budget, "mt": _q(twips_to_px(cell.margins["top"])),
                                  "mb": _q(twips_to_px(cell.margins["bottom"]))})
            tr = row.properties.get("trHeight")
            out_rows.append({
                "top_border": _q(tm._edge_width(previous_cells, list(row.cells), columns)),
                "header": bool(row.properties.get("tblHeader")), "cant_split": bool(row.properties.get("cantSplit")),
                "tr_height": [_q(vertical.twips_to_px(tr[0])) if tr[0] else [0, 1], tr[1]] if tr else None,
                "cells": out_cells})
            previous_cells = list(row.cells)
        default = document.styles.default("paragraph") if document.styles is not None else None
        bottom = tm._edge_width(list(resolved.rows[-1].cells), [], columns) if resolved.rows else Fraction(0)
        self.result[arg["xml"]] = (table, resolved)
        return {"rows": out_rows, "bottom_border": _q(bottom), "draw": _table_draw(element),
                "default_style": default.style_id if default else None}

    # -- records and the document's own --------------------------------------------------

    def _record(self, arg: dict) -> str:
        if not self.records:
            return "null"
        from . import records

        k = arg["k"]
        element = self.elements[k]
        document = self.document
        key = _key_of(element)
        if element.tag == _P:
            rec = records.paragraph_record(self.ctx, element, key, top=True)
        else:
            skeleton, _ = self._skeleton(arg["xml"])
            rec = records.table_record(self.ctx, element, key, skeleton, self.d2)
            rec.update(_table_geometry(self, element, skeleton))
        return json.dumps(rec, separators=(",", ":"))

    def _fonts(self, arg: dict) -> str:
        """``Mirror.fonts_json`` for a commit that measures new faces or characters."""
        from . import exporter as lane_s

        ex = _Ex(self.advances, self.metrics, arg["fonts"], arg["chars"], arg["kern"])
        return json.dumps(lane_s.fonts_out(ex), separators=(",", ":"))

    def _document(self, arg: dict) -> dict:
        from docx2svg import vertical

        from . import exporter as lane_s
        from .check import constants

        ex = _Ex(self.advances, self.metrics, arg["fonts"], arg["chars"], arg["kern"])
        fonts = lane_s.fonts_out(ex)
        sections = []
        from docx2svg import paginate

        for number, s in enumerate(self.d2.sections):
            stories = paginate._stories(self.d2, self.advances, self.metrics, self.heights_cache).get(number)
            sections.append({"top": _q(vertical.twips_to_px(abs(s.margins.top))), "top_neg": s.margins.top < 0,
                             "header": _q(vertical.twips_to_px(s.margins.header)),
                             "bottom": _q(vertical.twips_to_px(s.page_size.height_twips - abs(s.margins.bottom))),
                             "bottom_neg": s.margins.bottom < 0,
                             "footer": _q(vertical.twips_to_px(s.page_size.height_twips - s.margins.footer)),
                             "title_page": bool(s.title_page), "start_type": s.start_type,
                             "new_page": True, "page_number_start": s.page_number_start,
                             "stories": [[k, _q(a), _q(b)] for k, (a, b) in stories.items()] if stories else None})
        self.result["chars"] = set(arg["chars"])
        self.result["drawn"] = {(f, c) for f, c in arg["drawn"]}
        self.result["faces"] = arg["faces"]
        self.result["fonts"] = (list(map(tuple, arg["fonts"])), set(arg["kern"]))
        constants_value = constants(self.document, self) if self.records else None
        lu = vertical.LAYOUT_UNIT_PX
        return {"compat": self.d2.compatibility_mode, "even_odd": bool(self.d2.even_and_odd_headers),
                "dpi": vertical.DEVICE_DPI, "fonts": json.dumps(fonts, separators=(",", ":")),
                "constants": json.dumps(constants_value, separators=(",", ":")), "sections": sections,
                "lu": _q(lu), "lu_den": lu.denominator}


def _key_of(element: Any) -> str | None:
    from .mirror import key_of

    return key_of(element)


def _table_geometry(resolution: Resolution, element: Any, table: Any) -> dict:
    from .mirror import _table_geometry as geometry

    class _Self:
        d2 = resolution.d2

    return geometry(_Self(), element, table)


def export(document: Any, records: bool = True) -> tuple[str | None, dict, Resolution | None]:
    """The owner's first input made by the Rust parser (``None`` where it refuses the
    document: ``stats["refused"]`` says why), its statistics and the resolution it asked
    (its ``parser``: the Rust parser following the document)."""
    import ooxml_core

    t0 = time.perf_counter()
    try:
        resolution = Resolution(document, records=records)
    except Unsupported as why:
        STATS[f"refused: {str(why)[:60]}"] += 1
        return None, {"refused": str(why)}, None
    t1 = time.perf_counter()
    data = document.package.read(document.package.document_part())
    ctx = _RecordsContext(document, resolution) if records else None
    try:
        if ctx is not None:
            ctx.__enter__()
        parser, text, stats = ooxml_core.Parser.open(data, resolution)
        resolution.parser = parser
    finally:
        if ctx is not None:
            ctx.__exit__(None, None, None)
    stats = json.loads(stats)
    stats["resolution_s"] = t1 - t0
    stats["total_s"] = time.perf_counter() - t0
    if text is None:
        STATS[f"refused: {stats['refused'][:60]}"] += 1
    else:
        STATS["exported"] += 1
    return text, stats, resolution


class _ModelParagraphs:
    """docx-agent's ``effective._Model`` for the records, answered from each paragraph's own
    properties (and its table's style and its cell's conditional formats) on the
    definitions' model: what ``effective.paragraph`` reads of a paragraph -- not a parse of
    the document."""

    def __new__(cls, document: Any, d2: Any) -> Any:
        from ..edit import effective

        class _Model(effective._Model):
            def __init__(self, document: Any, d2: Any) -> None:
                self.document = document
                self.model = d2
                self.fallback = None
                self._paragraphs, self._elements, self._tables, self._held = {}, {}, None, []
                self._mapped, self._by_path = False, None
                self._paths = effective.PathFinder()
                self._found_cache: dict = {}

            @property
            def paragraphs(self):
                return self._whole().paragraphs

            @property
            def elements(self):
                return self._whole().elements

            def _whole(self):
                if self.fallback is None:
                    self.fallback = effective._Model(self.document)
                    STATS["model_whole"] += 1
                return self.fallback

            def _found(self, element):
                found = self._found_cache.get(id(element))
                if found is not None and found[0] is element:
                    return found[1]
                from docx2svg.parse import document as pdoc

                style, conditions = _table_context(element)
                paragraph = pdoc._paragraph(_properties_only(element), style, conditions)
                self._found_cache[id(element)] = (element, paragraph)
                return paragraph

            def paragraph(self, element):
                return self._found(element)

            def table(self, element):
                return self._whole().table(element)

        return _Model(document, d2)


def _properties_only(element: Any) -> Any:
    from lxml import etree

    out = etree.Element(_P)
    properties = element.find(_W + "pPr")
    if properties is not None:
        out.append(copy.deepcopy(properties))
    return out


def _table_context(element: Any) -> tuple:
    """A paragraph's table style (``""``: its table names none) and its cell's conditional
    formats, as docx2svg's parser gives them; ``(None, ())`` outside a table."""
    from docx2svg.parse.document import _table_look, cell_conditions
    from ooxml_common.xmlutil import element_child as child

    cell = element.getparent()
    if cell is None or cell.tag != _W + "tc":
        return None, ()
    row = cell.getparent()
    table = row.getparent()
    properties = table.find(_W + "tblPr")
    style_node = properties.find(_W + "tblStyle") if properties is not None else None
    style = (style_node.get(_W + "val") if style_node is not None else None) or ""
    look = _table_look(child(properties, "tblLook") if properties is not None else None)
    rows = table.findall(_W + "tr")
    cells = row.findall(_W + "tc")
    return style, cell_conditions(look, rows.index(row), cells.index(cell), len(rows), len(cells))


class _RecordsContext:
    """The records made with a model of each paragraph's own properties (docx-agent's
    ``effective.model``), put back after (as :class:`docx_agent.rust.check._records`)."""

    def __init__(self, document: Any, resolution: Resolution) -> None:
        self.document, self.resolution = document, resolution

    def __enter__(self) -> None:
        from ..edit.numbering import reading
        from .records import Context

        self.saved = self.document.__dict__.get("_effective")
        self.document._effective = (self.document._state, _ModelParagraphs(self.document, self.resolution.d2))
        self.resolution.ctx = Context(self.document)
        self.reading = reading(self.document)
        self.reading.__enter__()

    def __exit__(self, *exc: Any) -> None:
        self.reading.__exit__(*exc)
        self.resolution.ctx = None
        self.document._effective = self.saved


# -- the mirror the Rust parser keeps (lane I1, part 2) --------------------------------------------


class Unit:
    """One top-level body element as :class:`RustMirror` lists it (the Rust parser holds
    what it makes)."""

    __slots__ = ("element", "key")

    def __init__(self, element: Any, key: str | None) -> None:
        self.element = element
        self.key = key


class _Tables:
    def __init__(self, found: dict) -> None:
        self.chars = set(found["chars"])
        self.drawn = {(f, c) for f, c in found["drawn"]}


class RustMirror:
    """docx-agent's mirror of a document (:class:`docx_agent.rust.mirror.Mirror`), its body
    followed by the Rust parser: the first input made in Rust, each change set made from
    the elements an edit changed (:meth:`splice`), no docx2svg model of the body in Python."""

    rust = True

    def __init__(self, document: Any, resolution: Resolution, text: str) -> None:
        self.document = document
        self.resolution = resolution
        self.parser = resolution.parser
        self.d2 = resolution.d2
        self.fonts = resolution.fonts
        self.advances, self.metrics = resolution.advances, resolution.metrics
        self.heights_cache = resolution.heights_cache
        self.units = [Unit(element, _key_of(element)) for element in resolution.elements]
        tail = text[text.rindex('"px_den":'):]
        found = json.loads("{" + tail)
        self.px_den, self.w_den = found["px_den"], found["w_den"]
        head = json.loads(text)
        self.sections_sent = head["sections"]
        self._tables: dict | None = None

    # what check.py reads of a mirror

    def tables(self) -> dict:
        if self._tables is None:
            self._tables = json.loads(self.parser.tables())
        return self._tables

    @property
    def ex(self) -> _Tables:
        return _Tables(self.tables())

    def faces_list(self) -> list:
        return self.tables()["faces"]

    def encode(self, obj: Any) -> str:
        px_den, w_den = self.px_den, self.w_den
        from . import exporter as lane_s

        def default(o: Any) -> int:
            if isinstance(o, lane_s.Px):
                v = o.v * px_den
            elif isinstance(o, lane_s.Wd):
                v = o.v * w_den
            else:
                raise TypeError(type(o))
            if v.denominator != 1:
                raise Unsupported(f"a length off the export's denominators: {o.v}")
            return int(v)
        return json.dumps(obj, default=default, separators=(",", ":"))

    def _section_json(self, number: int, s: Any) -> dict:
        from .mirror import Mirror

        return Mirror._section_json(self, number, s)

    def sections_json(self) -> list:
        from .mirror import _sections_json

        return _sections_json(self)

    def story_of_part(self, part: str) -> Any:
        from .mirror import _story_of_part

        return _story_of_part(self, part)

    def restory(self, part: str, data: bytes) -> None:
        from .mirror import _restory

        _restory(self, part, data)

    def splice(self, lo: int, hi: int, elements: list, version: int, more: dict | None = None,
               same: list | None = None) -> tuple[str, dict]:
        """One commit (JSON) replacing units ``[lo, hi)`` by ``elements`` (the body's blocks
        there now), as the mirror's ``splice`` makes it; raises ``Unsupported`` where the
        Rust parser does not follow the change (the owner is then made again)."""
        from lxml import etree

        for element in elements:
            if element.tag not in (_P, _TBL):
                raise Unsupported(f"body element {element.tag}")
        old = {id(u): lo + k for k, u in enumerate(self.units[lo:hi])}
        region = []
        arguments: list = []
        for j, element in enumerate(elements):
            kept = same[j] if same is not None else None
            if kept is not None and id(kept) in old:
                kept.element = element
                region.append(kept)
                arguments.append(old[id(kept)])
            else:
                region.append(Unit(element, _key_of(element)))
                arguments.append(etree.tostring(element, with_tail=False).decode())
        units = self.units[:lo] + region + self.units[hi:]
        resolution = self.resolution
        resolution.elements = [u.element for u in units]
        text, stats = self.parser.splice(lo, hi, arguments, version,
                                         json.dumps(more, separators=(",", ":")) if more else None, resolution)
        stats = json.loads(stats)
        if text is None:
            STATS[f"stale: {stats['refused'][:60]}"] += 1
            raise Unsupported(stats["refused"])
        self.units = units
        self._tables = None
        STATS["splices"] += 1
        return text, stats


def open_mirror(document: Any) -> tuple[RustMirror | None, str | None, dict]:
    """The document's first input made by the Rust parser and the mirror that follows it,
    or ``(None, None, stats)`` where it refuses the document (counted in :data:`STATS`)."""
    text, stats, resolution = export(document)
    if text is None or resolution is None:
        return None, None, stats
    return RustMirror(document, resolution, text), text, stats
