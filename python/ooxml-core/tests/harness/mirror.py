"""The Python shim's side of a document laid out by the Rust owner.

A :class:`Mirror` follows one docx-agent document (its live lxml tree): docx2svg's model of
the body, one unit per top-level body element (its model blocks, the flow items they make),
and the docx-agent id of every paragraph and table.  After an edit, :meth:`Mirror.changes`
takes the change set from what the edit reported (the ids changed, created, removed) --
no whole-document digest -- finds the top-level elements they are in, parses those again
with docx2svg's parser, resolves them into the Rust kernel's measuring input (docx2svg's
resolve, as lane S's exporter does) and returns one commit: splices of the item list.

What is not modelled is refused (``Unsupported``), never approximated: notes, drawings,
frames, drop caps, columns, hyphenation, merged or nested cells, vertical alignment in
cells, floating tables, page-number formats other than decimal.
"""
from __future__ import annotations

import copy
import dataclasses
import json
import math
import os
import sys
import time
from fractions import Fraction

import docx2svg
from docx2svg import linebreak, vertical
from docx2svg import table as table_model
from docx2svg.linebreak import ListCounters, is_multi_column
from docx2svg.model import Paragraph, Table
from docx2svg.parse import document as pdoc
from docx2svg.resolve import autospaced, resolve_paragraph
from docx2svg.resolve import character_format, resolve_run
from docx2svg.vertical import autospace_kept, paragraph_borders_px

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import export as lane_s  # noqa: E402  (lane S's exporter: measuring inputs)

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
_P, _TBL = _W + "p", _W + "tbl"
#: Content a paragraph of the flow may not hold here (not modelled by the prototype).
_REFUSED = tuple(_W + t for t in ("drawing", "pict", "object", "footnoteReference", "endnoteReference",
                                  "framePr", "txbxContent"))
PAGE_FIELDS = frozenset({"PAGE", "NUMPAGES", "PAGEREF"})


class Unsupported(Exception):
    pass


class Px(lane_s.Px):
    pass


class Wd(lane_s.Wd):
    pass


class Exporter(lane_s.Exporter):
    """Lane S's exporter, with each computed field's run and the paragraph's bookmarks."""

    def measure_input(self, paragraph, section, label, pp=None):
        try:
            out = super().measure_input(paragraph, section, label, pp)
        except SystemExit as why:
            raise Unsupported(str(why)) from None
        offset = 0
        for run, entry in zip(paragraph.runs, out["runs"]):
            if run.field is not None and run.field.keyword in ("SEQ", "SECTIONPAGES", "STYLEREF"):
                raise Unsupported(f"a {run.field.keyword} field")
            if run.field is not None and run.field.keyword in PAGE_FIELDS:
                from docx2svg.fields import _argument, switches

                for kind, argument in switches(run.field.instruction):
                    if not (kind == "*" and argument.lower() in ("mergeformat", "charformat")):
                        raise Unsupported(f"field switch \\{kind} {argument}")
                entry["field"] = [run.field.keyword,
                                  _argument(run.field.instruction) if run.field.keyword == "PAGEREF" else None]
            if not entry["vanish"]:
                resolved = resolve_run(self.document, paragraph, run)
                if resolved.get("vertAlign") in ("superscript", "subscript"):
                    entry["va"] = True
            offset += len(run.text)
        return out


_PARA_ID = "{http://schemas.microsoft.com/office/word/2010/wordml}paraId"


def key_of(element) -> str | None:
    """docx-agent's id of a body paragraph or table, from its paraId (``p:``, ``t:`` + the
    first row's) -- the index's rule for a unique, valid paraId (checked against the index
    in :meth:`Mirror.verify`)."""
    if element.tag == _P:
        raw = element.get(_PARA_ID)
        return f"p:{raw.upper()}" if raw else None
    row = element.find(_W + "tr")
    raw = row.get(_PARA_ID) if row is not None else None
    return f"t:{raw.upper()}" if raw else None


def bookmarks_of(paragraph) -> list:
    offsets, total = [], 0
    for run in paragraph.runs:
        offsets.append(total)
        total += len(run.text)
    offsets.append(total)
    return [[name, offsets[min(run, len(paragraph.runs))]] for kind, _, name, run in paragraph.bookmarks
            if kind == "start" and name]


@dataclasses.dataclass
class Unit:
    """One top-level body element and what it makes."""
    element: object
    key: str | None
    blocks: list            # docx2svg model blocks (one here: no content controls)
    #: The complex fields open before it (docx2svg's ``_Fields.stack``), copied; ``None``: none.
    stack: list | None = None
    #: Its paragraphs (cells' too) with their resolved properties, and the numbered ones.
    paras: list | None = None
    numbered: list | None = None


class Mirror:
    def __init__(self, document, timing: dict | None = None):
        t0 = time.perf_counter()
        self.document = document
        self.main = document.package.document_part()
        self.body = document.package.tree(self.main).find(_W + "body")
        data = document.to_bytes()
        from docx_agent.layout import layout_options

        options = docx2svg.ConvertOptions(**layout_options(document, {}))
        self.fonts = docx2svg._fonts(data, options)
        self.advances, self.metrics = self.fonts, self.fonts.metrics
        d2 = docx2svg.parse.parse_package(data)
        if options.locale is not None:
            d2.locale = options.locale
        from docx2svg.layout import _Placer

        placer = _Placer(d2, data, self.advances, self.metrics, self.fonts.decorations)
        _source, original, features, _ = placer._prepare()
        if any(f not in (None, "field") for f in features):
            raise Unsupported(f"body features {sorted({f for f in features if f})}")
        if original.footnotes or original.endnotes:
            pass  # notes referenced from the body are refused per paragraph
        self.d2 = original
        self.ex = Exporter(self.d2, self.advances, self.metrics)
        for c in "0123456789":
            self.ex.chars.add(c)
        self.heights_cache: dict = {}
        # units: one per top-level p/tbl, each with the complex fields open before it
        elements = [c for c in self.body if c.tag in (_P, _TBL)]
        if len(elements) != len(self.d2.body):
            raise Unsupported("the body's elements and docx2svg's blocks differ (content controls, joins)")
        self.units: list[Unit] = []
        fields = pdoc._Fields([], cross_references=True, styleref=True)
        for element, block in zip(elements, self.d2.body):
            stack = copy.deepcopy(fields.stack) if fields.stack else None
            # thread the open fields through the body as the parser does
            name = element.tag.split("}")[1]
            pdoc._block_of(element, name, "w:body/w:" + name + "[0]", None, (), fields, [])
            unit = Unit(element, key_of(element), [block], stack)
            self._index_unit(unit)
            self.units.append(unit)
        if fields.stack:
            raise Unsupported("a complex field open at the body's end")
        self._pos = None
        self._keys = None
        self.sections_n = len(self.d2.sections)
        if self.sections_n != 1:
            raise Unsupported("more than one section")
        self.labels: dict[int, str | None] = {}
        self.numbered: dict[int, bool] = {}
        self._label_pass()
        self.parts_seen = {p: self._part_bytes(p) for p in self._watched_parts()}
        self.timing = {"open_prepare": time.perf_counter() - t0}

    # -- the model --------------------------------------------------------------------

    def _index_unit(self, unit: Unit) -> None:
        raws = getattr(self, "by_raw", None)
        if raws is None:
            raws = self.by_raw = {}
        for node in unit.element.iter(_P, _W + "tr"):
            raw = node.get(_PARA_ID)
            if raw:
                raws[raw.upper()] = unit
        paras, numbered = [], []
        for block in unit.blocks:
            for paragraph in self._paragraphs_of(block):
                pp = resolve_paragraph(self.d2, paragraph)
                paras.append((paragraph, pp))
                if pp.get("numPr.numId"):
                    numbered.append((paragraph, pp))
        unit.paras, unit.numbered = paras, numbered

    def _touch(self):
        """The unit list changed: positions and indexes are made again when asked."""
        self._pos = None
        self._keys = None

    def positions(self) -> dict:
        if self._pos is None:
            self._pos = {id(u.element): k for k, u in enumerate(self.units)}
        return self._pos

    def keys(self) -> dict:
        found = getattr(self, "_keys", None)
        if found is None:
            found = self._keys = {u.key: k for k, u in enumerate(self.units)}
        return found

    def _paragraphs_of(self, block):
        if isinstance(block, Table):
            for row in block.rows:
                for cell in row:
                    for inner in cell:
                        if isinstance(inner, Table):
                            raise Unsupported("a nested table")
                        yield inner
        else:
            yield block

    def _label_pass(self):
        """Every list label of the body, counted in document order (``paginate.flow``):
        ``{id(paragraph): label}`` and whether each numbered paragraph is; returns the
        paragraphs whose label changed."""
        counters = ListCounters()
        labels, numbered = {}, {}
        for unit in self.units:
            for paragraph, pp in unit.numbered:
                numbered[id(paragraph)] = True
                labels[id(paragraph)] = counters.label(self.d2, paragraph, pp)
        changed = {k for k, v in labels.items() if self.labels.get(k, "\0") != v}
        self.labels, self.numbered = labels, numbered
        return changed

    def _watched_parts(self):
        package = self.document.package
        rels = [p for p in ("word/styles.xml", "word/numbering.xml", "word/settings.xml") if package.has_part(p)]
        return rels

    def _part_bytes(self, part):
        package = self.document.package
        return package.read(part)

    # -- items ------------------------------------------------------------------------

    def unit_items(self, k: int) -> list:
        """The flow items (export dicts, Px/Wd unconverted) of unit ``k``."""
        unit = self.units[k]
        out = []
        for block in unit.blocks:
            if isinstance(block, Table):
                out.append(self.table_item(k, unit, block))
            else:
                out.append(self.para_item(k, unit, block))
        return out

    def _neighbour_paragraph(self, k: int, side: int):
        """The paragraph before (side -1) or after (+1) unit ``k``'s, in document order, cells
        included (``numbered`` of the block before or after)."""
        j = k + side
        if not 0 <= j < len(self.units):
            return None
        block = self.units[j].blocks[0 if side > 0 else -1]
        paragraphs = list(self._paragraphs_of(block))
        if not paragraphs:
            return self._neighbour_paragraph(j, side)
        return paragraphs[0 if side > 0 else -1]

    def para_item(self, k: int, unit: Unit, item: Paragraph) -> dict:
        document = self.d2
        element = unit.element
        if any(True for _ in element.iter(*_REFUSED)):
            raise Unsupported(f"{unit.key}: a drawing, note, frame or text box")
        pp = unit.paras[0][1]
        if pp.get("framePr") or any(run.anchors for run in item.runs):
            raise Unsupported(f"{unit.key}: frame or anchor")
        if any(kind == "drawing" for run in item.runs for _, kind in run.breaks):
            raise Unsupported(f"{unit.key}: drawing")
        if item.note:
            raise Unsupported("a note paragraph")
        section_number = 0
        section = document.sections[section_number]
        if is_multi_column(section):
            raise Unsupported("columns")
        label = self.labels.get(id(item))
        numbered = self.numbered.get(id(item), False)
        before = pp.get("spacing.before", 0) or 0
        after = pp.get("spacing.after", 0) or 0
        prev_p, next_p = self._neighbour_paragraph(k, -1), self._neighbour_paragraph(k, +1)
        if autospaced(document, pp, "before") and not autospace_kept(
                numbered=numbered, neighbour_numbered=self.numbered.get(id(prev_p), False) if prev_p is not None else None):
            before = 0
        if autospaced(document, pp, "after") and not autospace_kept(
                numbered=numbered, neighbour_numbered=self.numbered.get(id(next_p), False) if next_p is not None
                else False):
            after = 0
        neighbours = []
        for j in (k - 1, k + 1):
            if 0 <= j < len(self.units) and not isinstance(self.units[j].blocks[0], Table):
                neighbours.append(self.units[j].paras[0][1])
            else:
                neighbours.append(None)
        top_border, border_height = paragraph_borders_px(pp, *neighbours)
        widow = pp.get("widowControl")
        measure = self.ex.measure_input(item, section, label, pp)
        return {"t": "p", "block": k, "key": unit.key, "bookmarks": bookmarks_of(item),
                "section": section_number, "before": before, "after": after,
                "contextual": bool(pp.get("contextualSpacing")),
                "style": self.ex.style(item.properties.style_id if item.properties else None),
                "border": Px(border_height), "top_border": Px(top_border),
                "keep_next": bool(pp.get("keepNext")), "keep_lines": bool(pp.get("keepLines")),
                "pbb": bool(pp.get("pageBreakBefore")), "widow": None if widow is None else bool(widow),
                "section_start": k == 0, "section_mark": False, "measure": measure}

    def table_item(self, k: int, unit: Unit, table: Table) -> dict:
        document, ex = self.d2, self.ex
        if any(True for _ in unit.element.iter(*_REFUSED)):
            raise Unsupported(f"{unit.key}: a drawing or note in a table")
        section = document.sections[0]
        cell_paragraphs = list(self._paragraphs_of(table))
        labels = {n: self.labels.get(id(p)) for n, p in enumerate(cell_paragraphs) if id(p) in self.labels}
        preceding = self.units[k - 1].blocks[-1] if k > 0 else None
        try:
            f = table_model.flow_table(document, table, section, self.advances, self.metrics, block=0, labels=labels,
                                       features=None, cache=self.heights_cache,
                                       previous=preceding if isinstance(preceding, Paragraph) else None)
        except table_model.Unsupported as why:
            raise Unsupported(f"table: {why.args[0]}") from None
        if f.floating is not None or table_model._merges(f) or f.approximation is not None:
            raise Unsupported("floating or merged table")
        # docx-agent's ids of the cell paragraphs, in the same order
        rows_el = unit.element.findall(_W + "tr")
        rows = []
        for r, row in enumerate(f.rows):
            tr = row.row.properties.get("trHeight")
            cells_el = rows_el[r].findall(_W + "tc") if r < len(rows_el) else []
            cells = []
            for c, cell in enumerate(row.cells):
                if (cell.cell.properties.get("vAlign") or "top") not in ("top",):
                    raise Unsupported("a cell's vertical alignment")
                ps_el = cells_el[c].findall(_P) if c < len(cells_el) else []
                paras = []
                for j, cp in enumerate(cell.paragraphs):
                    if cp.paragraph is None or cp.table is not None or cp.drops or cp.positioned:
                        raise Unsupported("cell content")
                    m = ex.measure_input(cp.paragraph, section, cp.label, cp.resolved)
                    g = cp.geometry
                    m["geometry"] = {"first_start": Wd(g.first_start), "start": Wd(g.start), "right": Wd(g.right),
                                     "tabs": [[Wd(t.position), t.alignment] for t in g.tabs],
                                     "default_tab": Wd(g.default_tab),
                                     "hanging_stop": Wd(g.hanging_stop) if g.hanging_stop is not None else None,
                                     "squeeze": g.squeeze, "tab_origin": Wd(g.tab_origin)}
                    widow = cp.resolved.get("widowControl")
                    raw = ps_el[j].get(_PARA_ID) if j < len(ps_el) else None
                    paras.append({"block": 0, "key": f"p:{raw.upper()}" if raw else None,
                                  "bookmarks": bookmarks_of(cp.paragraph),
                                  "before": cp.before, "after": cp.after, "contextual": cp.contextual,
                                  "style": ex.style(cp.style), "effective_style": ex.style(cp.effective_style),
                                  "top_border": Px(cp.top_border), "border": Px(cp.border), "hidden": cp.hidden,
                                  "widow": None if widow is None else bool(widow),
                                  "keep_next": bool(cp.resolved.get("keepNext")), "measure": m})
                cells.append({"mt": Px(vertical.twips_to_px(cell.cell.margins["top"])),
                              "mb": Px(vertical.twips_to_px(cell.cell.margins["bottom"])),
                              "merge": cell.cell.merge, "paragraphs": paras})
            rows.append({"top_border": Px(row.top_border), "header": bool(row.row.properties.get("tblHeader")),
                         "cant_split": bool(row.row.properties.get("cantSplit")),
                         "keeps_next": table_model.keeps_with_next(row),
                         "tr_height": [Px(vertical.twips_to_px(tr[0])) if tr[0] else Px(0), tr[1]] if tr else None,
                         "cells": cells})
        first = f.rows[0].cells[0].paragraphs[0] if f.rows and f.rows[0].cells and f.rows[0].cells[0].paragraphs \
            else None
        return {"t": "t", "block": k, "key": unit.key, "section": 0, "section_start": k == 0,
                "bottom_border": Px(f.bottom_border), "previous_style": ex.style(f.previous_style),
                "default_style": ex.style(f.default_style),
                "first_style": ex.style(first.style) if first is not None else -2, "rows": rows}

    # -- encoding ---------------------------------------------------------------------

    def _dens(self, obj):
        dens = {"px": vertical.LAYOUT_UNIT_PX.denominator, "w": 1}

        def walk(o):
            if isinstance(o, lane_s.Px):
                dens["px"] = math.lcm(dens["px"], o.v.denominator)
            elif isinstance(o, lane_s.Wd):
                dens["w"] = math.lcm(dens["w"], o.v.denominator)
            elif isinstance(o, dict):
                for v in o.values():
                    walk(v)
            elif isinstance(o, list):
                for v in o:
                    walk(v)
        walk(obj)
        return dens

    def encode(self, obj) -> str:
        px_den, w_den = self.px_den, self.w_den

        def default(o):
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

    def fonts_json(self):
        return lane_s.fonts_out(self.ex)

    def export(self) -> tuple[str, dict]:
        """The whole document as the owner's first input (JSON), and timings."""
        t0 = time.perf_counter()
        items = []
        for k in range(len(self.units)):
            items.extend(self.unit_items(k))
        t1 = time.perf_counter()
        document = self.d2
        sections = []
        for number, s in enumerate(document.sections):
            sections.append(self._section_json(number, s))
        obj = {"compat": document.compatibility_mode, "even_odd": bool(document.even_and_odd_headers),
               "dpi": vertical.DEVICE_DPI, "styles": len(self.ex.styles), "fonts": None,
               "formats": [list(f) for f in self.ex.format_list], "sections": sections, "items": items}
        obj["fonts"] = self.fonts_json()
        dens = self._dens(obj)
        # room for what later edits bring: twips (1/20 pt) and eighths of a point in px
        self.px_den = math.lcm(dens["px"], 96 * 20)
        self.w_den = math.lcm(dens["w"], 20)
        obj["px_den"], obj["w_den"] = self.px_den, self.w_den
        lu = vertical.LAYOUT_UNIT_PX * self.px_den
        assert lu.denominator == 1
        obj["lu"] = int(lu)
        self.chars_sent = set(self.ex.chars)
        self.fonts_sent = len(self.ex.font_list)
        self.formats_sent = len(self.ex.format_list)
        self.kern_sent = set(self.ex.kern_fonts)
        text = self.encode(obj)
        t2 = time.perf_counter()
        return text, {"resolve_all_s": t1 - t0, "encode_s": t2 - t1, "items": len(items), "bytes": len(text)}

    def _section_json(self, number, s):
        from docx2svg import paginate

        # The stories' heights as docx2svg's flow computes them (headers and footers).
        stories = paginate._stories(self.d2, self.advances, self.metrics, self.heights_cache).get(number)
        return {"top": Px(vertical.twips_to_px(abs(s.margins.top))), "top_neg": s.margins.top < 0,
                "header": Px(vertical.twips_to_px(s.margins.header)),
                "bottom": Px(vertical.twips_to_px(s.page_size.height_twips - abs(s.margins.bottom))),
                "bottom_neg": s.margins.bottom < 0,
                "footer": Px(vertical.twips_to_px(s.page_size.height_twips - s.margins.footer)),
                "title_page": bool(s.title_page), "start_type": s.start_type,
                "new_page": True, "page_number_start": s.page_number_start,
                "stories": {k: [Px(a), Px(b)] for k, (a, b) in stories.items()} if stories else None}

    # -- change sets --------------------------------------------------------------------

    def top_of(self, element):
        """The top-level body element an element is in (``None``: not the body's)."""
        node = element
        while node is not None:
            parent = node.getparent()
            if parent is self.body:
                return node
            node = parent
        return None

    def changes(self, results: list[dict], version: int, verify: bool = False) -> tuple[str, dict]:
        """One commit (JSON) for the edits ``results`` reported (their changed, created and
        removed ids), against the tree as it is now (under the document's lock)."""
        t0 = time.perf_counter()
        stats: dict = {}
        document = self.document
        ids, removed = set(), set()
        for r in results:
            ids.update(r.get("changed") or [])
            ids.update(r.get("created") or [])
            removed.update(r.get("removed") or [])
            for old, new in (r.get("renamed") or {}).items():
                ids.add(new)
                removed.add(old)
        # parts other than the body: styles, numbering, settings -- everything again
        dirty = document.package.dirty_parts
        parts_changed = []
        for part in self.parts_seen:
            if part in dirty:
                now = self._part_bytes(part)
                if now != self.parts_seen[part]:
                    parts_changed.append(part)
                    self.parts_seen[part] = now
        if parts_changed:
            return self._everything(version, parts_changed, t0)
        # the units the ids are in (paraIds: no index of docx-agent's, which an edit drops)
        aliases = document._aliases
        touched_units = {}
        for identifier in ids:
            while identifier in aliases:
                identifier = aliases[identifier]
            raw = identifier.partition(":")[2].partition("/")[0].partition("#")[0].upper()
            unit = self.by_raw.get(raw)
            if unit is not None:
                touched_units[id(unit)] = unit
        pos = self.positions()
        touched = {pos[id(u.element)] for u in touched_units.values()
                   if id(u.element) in pos and self.units[pos[id(u.element)]] is u}
        gone, anchors = set(), set()
        if any(r.get("created") or r.get("removed") or r.get("renamed") for r in results) or \
                any(u.element.getparent() is not self.body for u in touched_units.values()):
            # the body's elements against the units: inserted ones after their last known
            # neighbour, units whose element left
            seen = set()
            last = -1
            for child in self.body:
                if child.tag not in (_P, _TBL):
                    continue
                k = pos.get(id(child))
                if k is None:
                    anchors.add(last + 1)
                else:
                    seen.add(k)
                    last = k
            if len(seen) != len(self.units):
                gone = set(range(len(self.units))) - seen
        tops = {}
        t1 = time.perf_counter()
        stats["change_set_s"] = t1 - t0
        stats["ids"] = len(ids) + len(removed)
        if not anchors and not gone:
            if not touched:
                span = None
                region_units = []
            else:
                lo, hi = min(touched), max(touched) + 1
                span = (lo, hi, hi)
                region_units = [self.units[k] for k in range(lo, hi)]
                reparse = {id(self.units[k].element) for k in touched}
        else:
            ks = touched | gone | anchors
            lo, hi = min(ks), min(max(ks) + 1, len(self.units))
            while lo > 0 and self.units[lo - 1].element.getparent() is not self.body:
                lo -= 1
            while hi < len(self.units) and self.units[hi].element.getparent() is not self.body:
                hi += 1
            node = self.units[lo - 1].element.getnext() if lo > 0 else self.body[0]
            stop = self.units[hi].element if hi < len(self.units) else None
            old = {id(u.element): u for u in self.units[lo:hi]}
            region_units = []
            reparse = {id(self.units[k].element) for k in touched}
            while node is not None and node is not stop:
                if node.tag in (_P, _TBL):
                    unit = old.get(id(node))
                    if unit is None:
                        unit = Unit(node, key_of(node), [])
                        reparse.add(id(node))
                    region_units.append(unit)
                elif node.tag != _W + "sectPr":
                    raise Unsupported(f"body element {node.tag}")
                node = node.getnext()
            span = (lo, hi, lo + len(region_units))
        runs = []
        if span is not None:
            lo, hi_old, _ = span
            old_units = self.units[lo:hi_old]
            region_units = self._reparse(lo, hi_old, region_units, reparse)
            self.units[lo:hi_old] = region_units
            self._touch()
            # the runs of units that differ, in the new list's places
            new_ids, old_ids = {id(u) for u in region_units}, {id(u) for u in old_units}
            i = j = 0
            while i < len(old_units) or j < len(region_units):
                if i < len(old_units) and j < len(region_units) and old_units[i] is region_units[j]:
                    i += 1
                    j += 1
                    continue
                a_i, a_j = i, j
                while i < len(old_units) and id(old_units[i]) not in new_ids:
                    i += 1
                while j < len(region_units) and id(region_units[j]) not in old_ids:
                    j += 1
                if (i, j) == (a_i, a_j):          # a unit moved: not made by these edits
                    raise Unsupported("units out of order")
                runs.append((lo + a_j, lo + j, i - a_i))
        t2 = time.perf_counter()
        stats["parse_s"] = t2 - t1
        relabelled = self._label_pass() if runs else set()
        t3 = time.perf_counter()
        stats["labels_s"] = t3 - t2
        # made again besides the runs: a neighbour on each side, and the units relabelled
        covered = set()
        for start, end, _ in runs:
            covered.update(range(start, end))
        extra = set()
        for start, end, _ in runs:
            extra.update(j for j in (start - 1, end) if 0 <= j < len(self.units))
        if relabelled:
            for k, u in enumerate(self.units):
                if any(id(p) in relabelled for p, _ in u.numbered):
                    extra.add(k)
        splices = sorted(runs + [(k, k + 1, 1) for k in extra - covered])
        redo = covered | extra
        out = []
        n_items = 0
        for at, end, remove in splices:
            items = []
            for k in range(at, end):
                items.extend(self.unit_items(k))
            n_items += len(items)
            out.append({"at": at, "remove": remove, "items": items})
        t4 = time.perf_counter()
        stats["resolve_s"] = t4 - t3
        commit = {"version": version, "splices": out}
        if len(self.ex.font_list) != self.fonts_sent or self.ex.chars != self.chars_sent or \
                self.ex.kern_fonts != self.kern_sent:
            commit["fonts"] = self.fonts_json()
            self.chars_sent, self.fonts_sent, self.kern_sent = set(self.ex.chars), len(self.ex.font_list), \
                set(self.ex.kern_fonts)
        if len(self.ex.format_list) != self.formats_sent:
            commit["formats"] = [list(f) for f in self.ex.format_list]
            self.formats_sent = len(self.ex.format_list)
        text = self.encode(commit)
        t5 = time.perf_counter()
        stats.update({"encode_s": t5 - t4, "items": n_items, "bytes": len(text), "units": len(redo),
                      "total_s": t5 - t0})
        if verify:
            stats["verify"] = self.verify()
        return text, stats

    def _everything(self, version: int, parts: list, t0: float) -> tuple[str, dict]:
        """Styles, numbering or settings changed: the whole body resolved again (as
        docx2svg lays the document out whole then, ``relayout.FALLBACKS``), one splice of
        every item, the measuring tables sent whole."""
        old_units = len(self.units)
        dens = (self.px_den, self.w_den)
        fresh = Mirror(self.document)
        self.__dict__.update({k: v for k, v in fresh.__dict__.items()})
        self.px_den, self.w_den = dens
        self._touch()
        t1 = time.perf_counter()
        items = []
        for k in range(len(self.units)):
            items.extend(self.unit_items(k))
        t2 = time.perf_counter()
        commit = {"version": version, "splices": [{"at": 0, "remove": old_units, "items": items}],
                  "fonts": self.fonts_json(), "formats": [list(f) for f in self.ex.format_list]}
        self.chars_sent, self.fonts_sent, self.kern_sent = set(self.ex.chars), len(self.ex.font_list), \
            set(self.ex.kern_fonts)
        self.formats_sent = len(self.ex.format_list)
        text = self.encode(commit)
        t3 = time.perf_counter()
        return text, {"everything": parts, "change_set_s": 0.0, "parse_s": t1 - t0, "labels_s": 0.0,
                      "resolve_s": t2 - t1, "encode_s": t3 - t2, "items": len(items), "bytes": len(text),
                      "units": len(self.units), "total_s": t3 - t0}

    def _reparse(self, lo: int, hi: int, region: list, reparse: set) -> list:
        """The region's units (replacing units ``[lo, hi)``), those in ``reparse`` parsed
        again, the complex fields open threaded through as the parser threads them -- and on
        past the region while a unit's open fields are not what they were."""
        before = self.units[lo].stack if lo < len(self.units) else None
        fields = pdoc._Fields([], cross_references=True, styleref=True)
        fields.stack = copy.deepcopy(before) if before else []
        out = []
        for n, unit in enumerate(region):
            stack = copy.deepcopy(fields.stack) if fields.stack else None
            if id(unit.element) in reparse or pdoc._open_state(unit.stack or []) != pdoc._open_state(stack or []):
                blocks = self._parse_one(unit.element, fields)
                unit = Unit(unit.element, unit.key, blocks, stack)
                self._index_unit(unit)
            elif stack is None and unit.stack is None and (
                    (region[n + 1].stack if n + 1 < len(region) else
                     self.units[hi].stack if hi < len(self.units) else None) is None):
                pass                      # no field open before or after it: nothing to thread
            else:
                name = unit.element.tag.split("}")[1]
                pdoc._block_of(unit.element, name, "w:body/w:" + name + "[0]", None, (), fields, [])
                unit.stack = stack
            out.append(unit)
        after = self.units[hi].stack if hi < len(self.units) else None
        if pdoc._open_state(fields.stack) != pdoc._open_state(after or []):
            raise Unsupported("an edit changed the complex fields open after it")
        return out

    def _parse_one(self, element, fields) -> list:
        out: list = []
        name = element.tag.split("}")[1]
        pdoc._block_of(element, name, "w:body/w:" + name + "[0]", None, (), fields, out)
        blocks = pdoc._following_properties(pdoc._join(out))[0]
        if len(blocks) != 1:
            raise Unsupported("an element of several blocks")
        if any(r.field is not None and r.field.keyword in ("REF", "SEQ") for b in blocks
               for p in self._paragraphs_of(b) for r in p.runs):
            # REF and SEQ: computed from the whole body (docx2svg's _prepare)
            from docx2svg.fields import cross_references

            here = [i for i, u in enumerate(self.units) if u.element is element]
            if not here:
                raise Unsupported("a REF or SEQ field in a new block")
            body = [u.blocks[0] for u in self.units]
            body[here[0]] = blocks[0]
            done, problems = cross_references(body)
            if problems:
                raise Unsupported(f"cross reference problems {problems}")
            blocks = [done[here[0]]]
        return blocks

    def verify(self) -> dict:
        """The units against the tree (tests only, a whole-document check): the units'
        elements are the body's, in order, and each unit's model is what a parse of the
        body gives now (paths aside)."""
        import re

        elements = [c for c in self.body if c.tag in (_P, _TBL)]
        same_order = len(elements) == len(self.units) and all(a is b.element for a, b in zip(elements, self.units))
        data = self.document.to_bytes()
        fresh = docx2svg.parse.parse_package(data)
        from docx2svg.fields import cross_references

        body, _ = cross_references(fresh.body)
        norm = lambda b: re.sub(r"'w:body[^']*'", "", re.sub(r"path='[^']*'", "", repr(b)))  # noqa: E731
        stale = [k for k, (u, b) in enumerate(zip(self.units, body)) if norm(u.blocks[0]) != norm(b)]
        index = self.document._index(self.main)
        keys_ok = all((index.entry_for(u.element).id if index.entry_for(u.element) else None) == u.key
                      for u in self.units)
        return {"order_ok": same_order, "count_ok": len(body) == len(self.units), "stale_units": stale[:10],
                "stale": len(stale), "keys_ok": keys_ok}
