"""The structured, read-only JSON state of a document (ROADMAP.md, "A full-state SVG? No.
A structured state, yes").

``Document.state(range=None, *, view="final", layout=False, xml=False)`` returns plain
JSON-ready data: what an agent inspects when Markdown is not enough ("why is this
paragraph 11 pt?").  It reads exactly what ``to_markdown`` reads (the same reader, view and
range), and like it changes nothing and stamps nothing.

**Schema** ``docx-agent/state``, **version 1**.  Keys follow pptx-agent's JSON conventions:
``"t"`` for text, ``"b"``/``"i"`` for bold/italic, and *absent means inherited* (for a
run's effective formatting: the same as its paragraph's baseline).  Lengths are points.

``{"schema": "docx-agent/state", "version": 1, "view", "range", "document", "styles",
"blocks", "notes", "comments", "revisions", "layout"?}``

* ``document``: ``compatibility_mode``, ``kind`` (``docx``...), ``stories`` (``name``,
  ``part``, ``kind``), ``sections`` (``id``, ``type``, ``page`` [w, h], ``margins``
  {top, right, bottom, left}, ``columns``, ``headers``/``footers`` {type: story}).
* ``styles``: every style the blocks use, by name: ``id``, ``kind``, ``based_on`` (a name).
* ``blocks``, in order, each one of:

  - **paragraph** -- ``id``; ``volatile`` (true when the id is positional or a repeat: it
    will change when the paragraph is first edited); ``joins`` (ids of paragraphs the view
    joins into this one, in order: a paragraph whose mark is deleted joins the next, and the
    joined paragraph is named by the last one, whose properties it has -- as Word keeps the
    last paragraph on accepting the deletion, measured); ``story``; ``style`` (name);
    ``markdown`` (the construct: ``paragraph``, ``heading``, ``bullet``, ``ordered``,
    ``quote``, ``code``, ``rule``) and ``level``; ``list`` {``num_id``, ``level``,
    ``format``, ``label``, ``number``, ``from_style``}; ``t`` (the text in the view: ``\\t``
    tab, ``\\v`` line break); ``effective`` (the paragraph's formatting and its runs'
    baseline: ``font``, ``sz``, ``color``, ``b``, ``i``, ``alignment``, ``indent`` {left,
    right, first_line, hanging}, ``spacing`` {before, after, line, rule},
    ``keep_with_next``, ``keep_together``, ``page_break_before``, ``outline_level``);
    ``runs`` (``t``, ``style``, ``direct`` -- the properties declared on the run itself --
    and the effective values that differ from the baseline: ``b``, ``i``, ``u``, ``strike``,
    ``dstrike``, ``caps``, ``small_caps``, ``sz``, ``font``, ``color``, ``highlight``,
    ``valign``; ``rev`` when the run is inside a revision); ``new_text`` (round 58: what new
    text written over the paragraph's gets -- its first run of text's formatting, else its
    mark's -- where that differs from ``effective``: ``font``, ``sz``, ``color``, ``b``,
    ``i``; absent where it is the baseline); ``links`` (``id``, ``href``,
    ``t``); ``bookmarks``; ``notes``; ``comments``;
    ``revisions``; ``pictures`` (``id``, ``name``, ``alt``, ``floating``, ``size``);
    ``drawings`` (``id``, ``kind``, ``name``, ``alt``, ``floating``; a chart's ``chart``
    and a SmartArt diagram's ``diagram``, ooxml-edit's JSON of either, which
    ``Chart.apply`` and ``Diagram.apply`` take back; a group's ``members``, each ``id``,
    ``kind``, ``name`` and its ``chart`` or ``diagram``); ``controls``
    (inline content controls: ``id``, ``kind``); ``fields`` (``instruction``, ``result``
    as cached); ``text_boxes`` (``id``, ``blocks``); ``section`` (``id``, ``type``) when the
    paragraph ends one.
  - **table** -- ``id``, ``volatile``, ``story``, ``style``, ``columns``,
    ``header_rows``, ``rows``: lists of cells (``id``, ``row``, ``column``, ``row_span``,
    ``column_span``, ``fill``, ``text_color``, ``blocks``); a vertically merged cell appears
    once, in its first row.  Round 59: ``fill`` is the cell's fill as drawn (``RRGGBB``, or
    ``None``: the table style's regions under the table's look, direct shading, theme colours
    resolved) and ``text_color`` the colour its text is drawn in (automatic text white on a
    dark fill, as Word draws it); runs of several colours: ``"mixed"`` and ``text_colors``.
  - **content_control** (block level) -- ``id``, ``kind``, ``tag``, ``alias``, ``blocks``.

  With ``layout=True`` each paragraph and table also has ``placements`` (``page``,
  ``top``, ``bottom``, ``lines``, ``story``, ``column``) or ``unknown`` {``reason``,
  ``after_page``}, from docx2svg's layout; with ``xml=True``, ``xml``: the base64 of each
  ``w:p`` (a list, for a joined paragraph) or of the ``w:tbl``.
* ``notes``: the notes the blocks reference (``id``, ``kind``, ``blocks``).
* ``comments``: the comments the blocks reference (``id``, ``author``, ``initials``,
  ``date``, ``parent``, ``done``, ``t``: what the comment says, ``paragraphs``: the
  paragraphs that reference it, ``anchor``: the text it is attached to, ``{"id", "t"}``
  -- a range id and its text in the state's view -- or ``None``).
* ``revisions``: every revision in the blocks (``id``, ``kind``, ``author``, ``date``,
  ``paragraph`` -- or ``table`` for a row's or table's own -- and ``t``).
* ``layout`` (with ``layout=True``): ``page_count``, ``pages_known``, ``stopped``.
"""

from __future__ import annotations

import base64
import copy
from typing import TYPE_CHECKING, Any

from lxml import etree

from .edit import annotations as _annotations
from .edit import effective as _effective
from .markdown import select
from .markdown.read import Marker, ParagraphRecord, Reader, TableRecord, referenced_notes, walk_records

if TYPE_CHECKING:  # pragma: no cover
    from .edit.document import Document

SCHEMA = "docx-agent/state"
VERSION = 1

#: A block's ``placements`` before :meth:`_Writer.place` sets them (or ``unknown``).
_PENDING = object()

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
_R = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}"

#: Effective run fields, the JSON key each is written under.
_RUN_FIELDS = (("bold", "b"), ("italic", "i"), ("underline", "u"), ("strike", "strike"),
               ("double_strike", "dstrike"), ("caps", "caps"), ("small_caps", "small_caps"), ("size", "sz"),
               ("font", "font"), ("color", "color"), ("highlight", "highlight"), ("vertical_align", "valign"))


def state(document: "Document", range: str | None = None, *, view: str = "final", layout: bool = False,
          xml: bool = False, style_map=None, placed=None) -> dict:
    """The document's state as JSON-ready data (schema ``docx-agent/state`` version 1).

    ``placed`` (round 59, R134), with ``layout``: who places the blocks --
    ``placed(asked)`` gives ``(placements, info)``: per entry of ``asked`` (an id, then the
    ids it joins) :func:`~docx_agent.layout.placements_of`'s answer, and the layout's
    ``page_count``, ``pages_known`` and ``stopped`` -- or ``None``; a tool answers it from
    the worker holding the layout.  Default (or ``None`` from it): the document's own
    :meth:`~docx_agent.Document.layout`, read here."""
    reader = Reader(document, view, style_map)
    writer = _Writer(document, reader, layout=layout, xml=xml)
    blocks: list = []
    records_all: list = []
    cell = None
    for part, records, kind in select(reader, range):
        if kind == "notes":
            writer.note_labels += [identifier for where, identifier, _ in document._notes() if where == part]
            continue
        if kind == "cell":
            cell = _cell(document, range)
        blocks += writer.blocks(records)
        records_all += records
    writer.note_labels += [n for r in walk_records(records_all) for n in referenced_notes(r)]
    out = {
        "schema": SCHEMA,
        "version": VERSION,
        "view": reader.view_name,
        "range": range,
        "document": writer.document_info(),
        "styles": {},
        "blocks": blocks,
        "notes": writer.notes(),
        "comments": writer.comments(),
        "revisions": writer.revisions(),
    }
    if cell is not None:
        out["cell"] = cell
    out["styles"] = writer.styles()
    if layout:
        out["layout"] = writer.place(placed)
    return out


def placement_keys(where: Any) -> dict:
    """What a block's dict says of where it is placed (:func:`~docx_agent.layout.
    placements_of`'s answer for it): ``placements``, each page and column it is drawn in,
    or ``unknown``, why and the page it comes after."""
    if where:
        return {"placements": [{"page": p.page, "top": round(p.top, 2), "bottom": round(p.bottom, 2),
                                "lines": p.lines, "story": p.story, "column": p.column} for p in where]}
    return {"unknown": {"reason": getattr(where, "reason", "not-drawn"),
                        "after_page": getattr(where, "after_page", None)}}


def _cell(document: "Document", address: str) -> dict:
    """What a state of one cell says of the cell itself: its id, table, place and spans, and
    the properties it states (:meth:`~docx_agent.Document.cell_properties`)."""
    found = document.get(address)
    table = found._table.id
    properties = document.cell_properties(table, found.row, found.column)
    return {"id": found.id, "table": table, "row": found.row, "column": found.column,
            "properties": properties}


class _Writer:
    def __init__(self, document: "Document", reader: Reader, *, layout: bool, xml: bool) -> None:
        self.document = document
        self.reader = reader
        self.with_layout = layout
        self.with_xml = xml
        #: ``(block's dict, [id, ids it joins])`` for each block to place (:meth:`place`).
        self._to_place: list[tuple[dict, list[str]]] = []
        self.note_labels: list[str] = []
        self.paragraph_ids: list[str] = []
        self.comment_places: dict[str, list[str]] = {}
        self.used_styles: set[str] = set()
        #: id(w:tbl) -> (table id, element) for every table written; the element is held.
        self.tables: dict[int, tuple] = {}
        #: id(w:tbl) -> (element, {cell path: docx2svg's resolved cell}) (:meth:`_resolved_cell`).
        self._cells: dict[int, tuple] = {}

    def place(self, placed=None) -> dict:
        """Each block's ``placements`` (or ``unknown``), in the place :meth:`_place` kept for
        them, all asked at once (``placed``, else the document's layout here), and the
        layout's ``page_count``, ``pages_known`` and ``stopped``."""
        from .layout import placements_of

        asked = [identifiers for _, identifiers in self._to_place]
        found = placed(asked) if placed is not None else None
        if found is None:
            lay = self.document.layout()
            answers = placements_of(lay, asked)
            info = {"page_count": lay.page_count, "pages_known": lay.pages_known,
                    "stopped": None if lay.stopped is None else {
                        "page": lay.stopped.page, "reason": lay.stopped.reason, "at": lay.stopped.at}}
        else:
            answers, info = found
        for (out, _), where in zip(self._to_place, answers):
            value = placement_keys(where)
            # In the block's dict where _place kept the key, the keys after it after it.
            items = list(out.items())
            out.clear()
            for key, item in items:
                if item is _PENDING:
                    out.update(value)
                else:
                    out[key] = item
        return info

    # -- blocks --------------------------------------------------------------------------------

    def blocks(self, records: list) -> list:
        out: list = []
        stack: list[list] = [out]
        for record in records:
            if isinstance(record, Marker):
                if record.begin:
                    control = {"type": "content_control", "id": record.id, "kind": record.kind,
                               "tag": record.tag, "alias": record.alias, "blocks": []}
                    stack[-1].append(control)
                    stack.append(control["blocks"])
                elif len(stack) > 1:
                    stack.pop()
            elif isinstance(record, ParagraphRecord):
                stack[-1].append(self.paragraph(record))
            elif isinstance(record, TableRecord):
                stack[-1].append(self.table(record))
        return out

    def paragraph(self, record: ParagraphRecord) -> dict:
        self.paragraph_ids += [record.id] + record.joins
        default = self.reader._default_style
        style_id = record.style_id or (default.id if default else None)
        if style_id:
            self.used_styles.add(style_id)
        out: dict = {"type": "paragraph", "id": record.id}
        if record.volatile:
            out["volatile"] = True
        if record.joins:
            out["joins"] = list(record.joins)
        out["story"] = record.story
        out["style"] = record.style_name
        out["markdown"] = record.construct
        if record.construct in ("heading", "bullet", "ordered"):
            out["level"] = record.level if record.construct == "heading" else record.level + 1
        if record.membership is not None:
            member = record.membership
            out["list"] = {"num_id": member.num_id, "level": member.level, "format": member.format,
                           "label": record.label, "number": record.number, "from_style": member.from_style}
        out["t"] = record.text
        element = record.elements[-1]
        base = _effective.run(self.document, element, None, "a")
        para = _effective.paragraph(self.document, element)
        out["effective"] = {
            "font": base.font, "sz": base.size, "color": base.color, "b": base.bold, "i": base.italic,
            "alignment": para.alignment,
            "indent": {"left": para.indent_left, "right": para.indent_right, "first_line": para.first_line,
                       "hanging": para.hanging},
            "spacing": {"before": para.space_before, "after": para.space_after, "line": para.line_spacing,
                        "rule": para.line_rule},
            "keep_with_next": para.keep_with_next, "keep_together": para.keep_together,
            "page_break_before": para.page_break_before, "outline_level": para.outline_level,
        }
        out["runs"] = [self.run(run, base) for run in record.runs]
        written = new_text(self.document, element, base)
        if written:
            out["new_text"] = written
        for key, values in (("links", record.links), ("bookmarks", record.bookmarks), ("notes", record.notes),
                            ("comments", record.comments), ("revisions", [r for r in record.revisions if r]),
                            ("pictures", record.pictures), ("drawings", record.drawings),
                            ("controls", record.controls), ("fields", record.fields)):
            if values:
                if key == "links":
                    values = [{"id": v["id"], "href": v["href"], "t": v["text"]} for v in values]
                if key == "pictures":
                    values = [{**v, "size": list(v["size"]) if v["size"] else None} for v in values]
                out[key] = list(values)
        for comment in record.comments:
            self.comment_places.setdefault(comment, []).append(record.id)
        if record.text_boxes:
            out["text_boxes"] = [{"id": identifier, "blocks": self.blocks(self.reader.container(box, record.part))}
                                 for identifier, box in record.text_boxes]
        if record.section is not None:
            out["section"] = {"id": record.section[0], "type": record.section[1]}
        if self.with_layout:
            self._place(out, record.id, record.joins)
        if self.with_xml:
            out["xml"] = [base64.b64encode(etree.tostring(e)).decode("ascii") for e in record.elements]
        return out

    def run(self, run, base) -> dict:
        out: dict = {"t": run.text}
        if run.style:
            out["style"] = run.style
            self.used_styles.update(s.id for s in self.reader._styles.values() if s.name == run.style)
        if run.direct:
            out["direct"] = list(run.direct)
        for name, key in _RUN_FIELDS:
            value = getattr(run.effective, name)
            if value != getattr(base, name):
                out[key] = value
        if run.revision:
            out["rev"] = run.revision
        return out

    def table(self, record: TableRecord) -> dict:
        self.tables[id(record.element)] = (record.id, record.element)
        style = record.element.find(f"{_W}tblPr/{_W}tblStyle")
        if style is not None:
            self.used_styles.add(style.get(_W + "val"))
        out: dict = {"type": "table", "id": record.id}
        if record.volatile:
            out["volatile"] = True
        out.update({"story": record.story, "style": record.style_name, "columns": record.columns,
                    "header_rows": record.header_rows})
        out["rows"] = [[{"id": cell.id, "row": cell.row, "column": cell.column, "row_span": cell.row_span,
                         "column_span": cell.column_span, **self.cell_colours(cell),
                         "blocks": self.blocks(cell.records)} for cell in line]
                       for line in record.rows]
        if self.with_layout:
            self._place(out, record.id)
        if self.with_xml:
            out["xml"] = base64.b64encode(etree.tostring(record.element)).decode("ascii")
        return out

    def cell_colours(self, cell) -> dict:
        """Round 59: ``fill`` -- the cell's fill as docx2svg draws it, ``RRGGBB`` or ``None``:
        its table style's regions under the table's look, the style's own shading, the
        cell's direct shading, theme colours with their tint or shade (docx2svg's
        :func:`~docx2svg.table.resolve_table`, measured against Word in round 58) -- and
        ``text_color``, the colour its text is drawn in: each run's colour, an automatic one
        drawn white on a fill darker than Word's measured threshold (Rec. 601 luma below 60,
        :func:`docx2svg.layout.automatic_text_on_cell`) and black elsewhere.  Runs of
        different colours: ``"mixed"``, and ``text_colors`` the distinct ones in order.  A cell
        with no text: the colour new text there would get (its first paragraph's).  Nothing
        for a cell docx2svg does not draw (in a text box)."""
        from docx2svg.layout import HIGHLIGHT, _color, _shading, automatic_text_on_cell

        resolved = self._resolved_cell(cell.element) if cell.element is not None else None
        if resolved is None:
            return {}
        shd = resolved.properties.get("shd")
        fill = _shading(shd)
        automatic = automatic_text_on_cell(shd)

        def drawn(run_resolved, shaded: bool) -> str:
            colour = _color(run_resolved.get("color"))
            if colour is not None:
                return colour.upper()
            if (automatic is not None and not shaded and not HIGHLIGHT.get(run_resolved.get("highlight") or "")
                    and _shading(run_resolved.get("shd")) is None):
                return automatic
            return "000000"

        colours: list[str] = []
        first = None
        for record in cell.records:
            if not isinstance(record, ParagraphRecord):
                continue
            element = record.elements[-1]
            first = first if first is not None else element
            shaded = _shading(_effective.paragraph(self.document, element)._resolved.get("shd")) is not None
            for run in record.runs:
                if run.text and not run.effective.hidden:
                    value = drawn(run.effective._resolved, shaded)
                    if value not in colours:
                        colours.append(value)
        if not colours and first is not None:
            shaded = _shading(_effective.paragraph(self.document, first)._resolved.get("shd")) is not None
            colours.append(drawn(_effective.run(self.document, first, None, "a")._resolved, shaded))
        out: dict = {"fill": fill.upper() if fill else None}
        if len(colours) == 1:
            out["text_color"] = colours[0]
        elif colours:
            out["text_color"] = "mixed"
            out["text_colors"] = colours
        return out

    def _resolved_cell(self, tc):
        """docx2svg's resolved cell (:class:`docx2svg.table.Cell`) of our ``w:tc``, or ``None``;
        each table resolved once per state."""
        from docx2svg.table import resolve_table

        table = next(tc.iterancestors(_W + "tbl"), None)
        if table is None:
            return None
        kept = self._cells.get(id(table))
        if kept is None:
            cells: dict = {}
            model = _effective._model_for(self.document, table)
            found = model.table(table)
            if found is not None:
                resolved = resolve_table(model.model, found)
                cells = {cell.path: cell for row in resolved.rows for cell in row.cells}
            kept = self._cells[id(table)] = (table, cells)
        path = _effective._path_of(self.document, tc)
        return kept[1].get(path[1]) if path is not None else None

    def _place(self, out: dict, identifier: str, joined: "list[str] | tuple" = ()) -> None:
        # Placed once every block is written (:meth:`place`): asked of the layout at once.
        # docx2svg draws a joined paragraph's lines under the first paragraph's id.
        out["placements"] = _PENDING
        self._to_place.append((out, [identifier, *joined]))

    # -- the rest ------------------------------------------------------------------------------

    def notes(self) -> list:
        out = []
        seen: set[str] = set()
        queue = list(self.note_labels)
        notes = {identifier: (part, element) for part, identifier, element in self.document._notes()}
        while queue:
            label = queue.pop(0)
            if label in seen or label not in notes:
                continue
            seen.add(label)
            part, element = notes[label]
            records = self.reader.container(element, part)
            out.append({"id": label, "kind": "footnote" if label.startswith("fn:") else "endnote",
                        "blocks": self.blocks(records)})
            queue += [n for r in walk_records(records) for n in referenced_notes(r)]
        return out

    def comments(self) -> list:
        out = []
        for record in self.document._comment_records():
            if record["id"] not in self.comment_places:
                continue
            comment = self.document.comment(record["id"])
            # Where it is attached: the range its markers hold, in this state's view.
            anchor = self.document._comment_anchor(record["id"], self.reader.view)
            out.append({"id": record["id"], "author": record["author"], "initials": record["initials"],
                        "date": record["date"], "parent": record["parent"], "done": record["done"],
                        "t": comment.text, "paragraphs": self.comment_places[record["id"]],
                        "anchor": None if anchor is None else {"id": anchor.id, "t": anchor.text}})
        return out

    def revisions(self) -> list:
        wanted = set(self.paragraph_ids)
        out = []
        for part, identifier, element in self.document._revision_elements():
            paragraph = _annotations._paragraph_id(self.document, part, element)
            entry = {"id": identifier, "kind": _annotations.revision_kind(element),
                     "author": element.get(_W + "author"), "date": element.get(_W + "date")}
            text = _annotations.revision_text(element)
            if paragraph is not None and paragraph in wanted:
                out.append({**entry, "paragraph": paragraph, "t": text})
            elif paragraph is None:
                # A row's or a table's own revision: reported with the table written.
                node = element.getparent()
                while node is not None and id(node) not in self.tables:
                    node = node.getparent()
                if node is not None:
                    out.append({**entry, "table": self.tables[id(node)][0], "t": text})
        return out

    def styles(self) -> dict:
        out = {}
        styles = self.reader._styles
        for style_id in sorted(self.used_styles, key=lambda s: (styles[s].name or s) if s in styles else s):
            style = styles.get(style_id)
            if style is None:
                continue
            based = styles.get(style.based_on) if style.based_on else None
            out[style.name or style.id] = {"id": style.id, "kind": style.kind,
                                           "based_on": (based.name or based.id) if based else style.based_on}
        return out

    def document_info(self) -> dict:
        package = self.document.package
        stories = []
        for story in self.document.stories:
            root = package.tree(story.part)
            stories.append({"name": story.name, "part": story.part,
                            "kind": root.tag.rpartition("}")[2] if root is not None else None})
        sections = []
        for section in self.document.sections():
            node = section._sectPr
            kind = node.find(_W + "type")
            size = node.find(_W + "pgSz")
            margins = node.find(_W + "pgMar")
            columns = node.find(_W + "cols")
            entry: dict = {"id": section.id, "type": kind.get(_W + "val") if kind is not None else "nextPage"}
            if size is not None:
                entry["page"] = [_points(size.get(_W + "w")), _points(size.get(_W + "h"))]
                if size.get(_W + "orient"):
                    entry["orientation"] = size.get(_W + "orient")
            if margins is not None:
                entry["margins"] = {side: _points(margins.get(_W + side)) for side in ("top", "right", "bottom", "left")}
            entry["columns"] = int(columns.get(_W + "num") or 1) if columns is not None else 1
            for tag in ("headerReference", "footerReference"):
                refs = {}
                for ref in node.findall(_W + tag):
                    target = package.related_part(package.document_part(), ref.get(_R + "id"))
                    if target is not None:
                        refs[ref.get(_W + "type") or "default"] = self.document._story_of(target)
                if refs:
                    entry["headers" if tag == "headerReference" else "footers"] = refs
            sections.append(entry)
        return {"compatibility_mode": self.document.compatibility_mode, "kind": package.kind,
                "stories": stories, "sections": sections}


#: The formatting ``new_text`` compares (round 58, R126): the JSON key of each.
_NEW_TEXT_FIELDS = (("font", "font"), ("size", "sz"), ("color", "color"), ("bold", "b"), ("italic", "i"))


def new_text(document: "Document", element, base) -> dict:
    """Round 58 (R126): the formatting new text written over ``element``'s (``set_text``,
    ``word_edit_table``'s ``set_cells``) gets where it differs from the paragraph's baseline
    ``base`` -- its first run of text's, else (an empty paragraph) its mark's, as Word gives
    text typed there (measured, ``tools/row_format_probe.py``) -- ``{}`` where it is the
    baseline's.  A paragraph whose runs carry their formatting with a bare mark (as many
    writers leave a table's cells) states ``effective`` as its style's; this says what text
    written into it looks like."""
    from .edit.text import first_text_run
    from .oxml.xml import REVISION_PROPERTY_TAGS, make

    run = first_text_run(element)
    if run is None:
        mark = element.find(f"{_W}pPr/{_W}rPr")
        if mark is None:
            return {}
        properties = make("w:rPr")
        for child in mark:
            if isinstance(child.tag, str) and child.tag not in REVISION_PROPERTY_TAGS:
                properties.append(copy.deepcopy(child))
        if not len(properties):
            return {}
        run = make("w:r")
        run.append(properties)
    got = _effective.run(document, element, run, "a")
    return {key: getattr(got, name) for name, key in _NEW_TEXT_FIELDS if getattr(got, name) != getattr(base, name)}


def _points(twips: str | None) -> float | None:
    try:
        return int(twips) / 20
    except (TypeError, ValueError):
        return None


__all__ = ["SCHEMA", "VERSION", "state"]
