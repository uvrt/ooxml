"""Effective formatting: what Word applies, read through docx2svg's resolver.

docx2svg's :mod:`docx2svg.resolve` has the style cascade measured against Word --
``w:docDefaults``, table style and its conditional formats, numbering, paragraph style,
character style, direct formatting; toggle properties as Word combines them, not as
ECMA-376 says; theme fonts -- and says which level every value came from.  This module
asks it, over the document's current bytes (parsed once per document state), rather than
keep a second cascade that would drift from the measured one.

``paragraph.effective`` and ``run.effective`` return :class:`EffectiveParagraph` and
:class:`EffectiveRun`: plain values (points, names), and ``explain()``, docx2svg's account
of where each came from.
"""

from __future__ import annotations

import dataclasses
import re
import threading
from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from docx2svg import model as _model
from docx2svg.parse.document import parse_package
from docx2svg.parse.properties import read_paragraph_properties, read_run_properties
from docx2svg.paths import path_part, resolve_path, row_level
from docx2svg.resolve import character_format, resolve_paragraph, resolve_run
from docx2svg.table import lead_paragraph

from ..oxml.xml import Element

if TYPE_CHECKING:  # pragma: no cover
    from .document import Document

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

_ALIGNMENTS = {"left": "left", "start": "left", "center": "center", "right": "right", "end": "right",
               "both": "justify", "distribute": "distribute"}


@dataclass(frozen=True)
class EffectiveRun:
    """A run's formatting as Word applies it."""

    bold: bool
    italic: bool
    underline: str | None
    strike: bool
    double_strike: bool
    caps: bool
    small_caps: bool
    #: Points.
    size: float
    #: The face the run's first character is drawn in (its script's slot; theme fonts
    #: resolved), or ``None`` where Word falls back to its own.
    font: str | None
    #: ``RRGGBB`` or ``auto``.
    color: str
    highlight: str | None
    vertical_align: str | None
    hidden: bool
    #: The character style applied, as declared (``None`` for the default).
    style: str | None
    _resolved: object = field(default=None, repr=False, compare=False)

    def explain(self) -> str:
        """Where every value came from, one line per property (docx2svg's account)."""
        return self._resolved.explain()


@dataclass(frozen=True)
class EffectiveParagraph:
    """A paragraph's formatting as Word applies it.  Lengths in points."""

    style: str | None
    alignment: str
    indent_left: float
    indent_right: float
    first_line: float
    hanging: float
    space_before: float
    space_after: float
    #: A multiple (``line_rule`` ``auto``) or points (``exact``, ``atLeast``).
    line_spacing: float | None
    line_rule: str
    keep_with_next: bool
    keep_together: bool
    page_break_before: bool
    widow_control: bool
    outline_level: int | None
    #: The list the paragraph is in: ``(numId, level)``, or ``None``.
    numbering: tuple[int, int] | None
    _resolved: object = field(default=None, repr=False, compare=False)

    def explain(self) -> str:
        return self._resolved.explain()


class _Model:
    """docx2svg's parse of one document state, with its paragraphs by our elements.

    The paragraphs are found as they are asked for (round 28, W8): one asked is looked up
    by the path docx2svg writes for it (:func:`_path_of`), and taken only where docx2svg
    resolves that path to it (:func:`~docx2svg.paths.resolve_path`); a paragraph that
    cannot be found so maps every paragraph, as each was before.  Mapping them all resolved
    every path from the part's root, each step listing its siblings: work that grew with
    the square of the document, done again after every edit."""

    def __init__(self, document: "Document", parsed: "tuple[bytes, _model.Document] | None" = None) -> None:
        self.document = document
        data = document.to_bytes()
        # Round 41: docx2svg's parse of these very bytes, where the caller has it (a worker's
        # check: the one its layout was made from, :func:`docx2svg.parsed_document`).
        if parsed is not None and parsed[0] == data:
            self.model = parsed[1]
        else:
            # Lane G: the blocks of the document's last parse kept (docx2svg's KeptBlocks, the
            # same model either way): a parse after an edit builds the blocks it changed.
            kept = _kept_blocks(document)
            if kept is None:
                self.model = parse_package(data)
            else:
                with kept[1]:
                    self.model = parse_package(data, kept=kept[0], segments=_segments(document, kept))
        self._paragraphs: dict[int, _model.Paragraph] = {}
        self._elements: dict[int, Element] = {}
        self._tables: dict[int, _model.Table] | None = None
        self._held: list[Element] = []
        self._mapped = False
        self._by_path: dict[tuple[str, str], _model.Paragraph] | None = None
        #: What looking paths up lists once per state (round 33): each parent's children by
        #: name, and each path prefix's element (:class:`PathFinder`).
        self._paths = PathFinder()

    @property
    def paragraphs(self) -> dict[int, _model.Paragraph]:
        """docx2svg's paragraph of each of our ``w:p`` it draws, by the element's ``id()``."""
        self._map_all()
        return self._paragraphs

    @property
    def elements(self) -> dict[int, Element]:
        """Our element of each of docx2svg's paragraphs, by its ``id()``."""
        self._map_all()
        return self._elements

    def _each(self):
        """``(paragraph, part)`` for every paragraph docx2svg read, in the order mapped."""
        package = self.document.package
        for block in _paragraphs(self.model.body):
            yield block, package.document_part()
        for relationship, blocks in self.model.stories.items():
            part = self.model.story_parts.get(relationship)
            for block in _paragraphs(blocks):
                yield block, part
        parts = _Parts(self.document)
        for notes in (self.model.footnotes, self.model.endnotes):
            for blocks in notes.values():
                for block in _paragraphs(blocks):
                    yield block, path_part(parts, block.path)

    def _map_all(self) -> None:
        if self._mapped:
            return
        self._mapped = True
        for block, part in self._each():
            self._map(block, part)

    def _map(self, paragraph: _model.Paragraph, part: str | None) -> None:
        if part is None or not paragraph.path:
            return
        element = self._paths.resolve(self.document.package.tree(part), paragraph.path)
        if element is not None and element.tag == _W + "p":
            self._held.append(element)  # one proxy per element while the map lives: stable id()s
            self._paragraphs.setdefault(id(element), paragraph)
            self._elements.setdefault(id(paragraph), element)

    def _found(self, element: Element) -> _model.Paragraph | None:
        """docx2svg's paragraph of ``element``, as :attr:`paragraphs` maps it."""
        if self._mapped:
            return self._paragraphs.get(id(element))
        if id(element) in self._paragraphs:
            return self._paragraphs[id(element)]
        where = _path_of(self.document, element, self._paths)
        if where is not None:
            if self._by_path is None:
                self._by_path = {}
                for block, part in self._each():
                    if part is not None and block.path:
                        self._by_path.setdefault((part, block.path), block)
            found = self._by_path.get(where)
            if found is not None and self._paths.resolve(self.document.package.tree(where[0]), found.path) is element:
                self._held.append(element)
                self._paragraphs[id(element)] = found
                self._elements.setdefault(id(found), element)
                return found
        self._paragraphs.clear()
        self._elements.clear()
        self._map_all()
        return self._paragraphs.get(id(element))

    def table(self, element: Element) -> _model.Table | None:
        """docx2svg's table of our ``w:tbl`` (nested ones too), or ``None``."""
        if self._tables is None:
            self._tables = {}
            parts = _Parts(self.document)
            places = [(self.model.body, self.document.package.document_part())]
            places += [(blocks, self.model.story_parts.get(relationship))
                       for relationship, blocks in self.model.stories.items()]
            for notes in (self.model.footnotes, self.model.endnotes):
                places += [(blocks, None) for blocks in notes.values()]
            for blocks, part in places:
                for table in _all_tables(blocks):
                    owner = part if part is not None else path_part(parts, table.path)
                    if owner is None or not table.path:
                        continue
                    found = resolve_path(self.document.package.tree(owner), table.path)
                    if found is not None and found.tag == _W + "tbl":
                        self._held.append(found)
                        self._tables.setdefault(id(found), table)
        return self._tables.get(id(element))

    def paragraph(self, element: Element) -> _model.Paragraph:
        found = self._found(element)
        if found is not None and not found.joins:
            return found
        # Not drawn on its own (joined across a deleted mark, in a text box): read it here.
        properties = element.find(_W + "pPr")
        style = properties.find(_W + "pStyle") if properties is not None else None
        mark = properties.find(_W + "rPr") if properties is not None else None
        declared = read_paragraph_properties(properties)
        return _model.Paragraph(
            properties=_model.ParagraphProperties(
                style_id=style.get(_W + "val") if style is not None else None, declared=declared,
                run_properties=_model.RunProperties(declared=read_run_properties(mark))),
            table_style_id=_table_style(element),
            table_conditions=found.table_conditions if found is not None else ())


#: The steps :func:`_path_of` writes: an element named so, in a parent named so.
_STEPS = {"p": ("body", "tc", "hdr", "ftr"), "tbl": ("body", "tc"), "tr": ("tbl",), "tc": ("tr",)}


def _path_of(document: "Document", element: Element, paths: "PathFinder | None" = None) -> tuple[str, str] | None:
    """``(part, path)``: the path docx2svg writes for a paragraph in a body, a header or a
    footer, in tables there too (``w:body/w:tbl[2]/w:tr[1]/w:tc[3]/w:p[1]``: each step the
    k-th child of its local name, a table's rows and a row's cells counted as docx2svg
    counts them, :func:`~docx2svg.paths.row_level`); ``None`` for one anywhere else (a
    content control, a text box, a note), whose paragraph is found by mapping them all.
    ``paths`` lists each parent's children once (a long table's rows: once, not once per
    paragraph in it)."""
    paths = paths if paths is not None else PathFinder()
    steps: list[str] = []
    node = element
    while True:
        parent = node.getparent()
        name = _local(node.tag)
        if parent is None:
            if name in ("hdr", "ftr"):
                steps.append(f"w:{name}")
                break
            return None
        outer = _local(parent.tag)
        if name == "body" and outer == "document" and parent.getparent() is None:
            steps.append("w:body")
            node = parent
            break
        if outer not in _STEPS.get(name, ()) or not node.tag.startswith(_W) or not parent.tag.startswith(_W):
            return None
        place = paths.place(parent, node, name)
        if place is None:
            return None
        steps.append(f"w:{name}[{place + 1}]")
        node = parent
    root = node if node.getparent() is None else node.getparent()
    part = next((p for p in document._parts() if document.package.tree(p) is root), None)
    if part is None:
        return None
    return part, "/".join(reversed(steps))


class PathFinder:
    """Looking up docx2svg's paths of one document state's elements without listing a
    parent's children once per element under it (round 33: every cell paragraph of a
    432-row table listed the table's rows twice -- once to write its path, once to resolve
    it -- which made reading the table grow with the square of its rows).

    :meth:`place` is an element's 1-based place among its parent's children of its name,
    as :func:`_path_of` counts them; :meth:`resolve` is
    :func:`~docx2svg.paths.resolve_path`, resolving each path prefix once.  Both keep
    what they listed by the parent element (held, so ids stay its own) and its number of
    children: a parent changed since is listed again.  Kept on the per-state
    :class:`_Model`, dropped with it after any change."""

    def __init__(self) -> None:
        self._places: dict[tuple[int, str], tuple] = {}
        self._prefixes: dict[tuple[int, str], tuple] = {}

    def place(self, parent: Element, node: Element, name: str) -> int | None:
        key = (id(parent), name)
        kept = self._places.get(key)
        if kept is None or kept[0] is not parent or kept[1] != len(parent):
            if name in ("tr", "tc"):
                siblings = self.rows(parent, name)
            else:
                # (0.1.118: the children of that tag found by lxml, not each child looked at)
                siblings = list(parent.iterchildren(node.tag))
            # The same local name in another namespace: docx2svg counts it.
            clash = any(child.tag != node.tag for child in parent.iterchildren("{*}" + name))
            kept = (parent, len(parent), None if clash else {id(child): k for k, child in enumerate(siblings)},
                    siblings)
            self._places[key] = kept
        if kept[2] is None:
            return None
        return kept[2].get(id(node))

    def rows(self, parent: Element, name: str) -> list:
        """:func:`~docx2svg.paths.row_level` of ``parent``, listed once."""
        key = (id(parent), "rows:" + name)
        kept = self._places.get(key)
        if kept is None or kept[0] is not parent or kept[1] != len(parent):
            kept = (parent, len(parent), list(row_level(parent, name)))
            self._places[key] = kept
        return kept[2]

    def named(self, parent: Element, name: str) -> list:
        """``parent``'s element children of local name ``name`` (docx2svg's
        ``element_children``), listed once."""
        key = (id(parent), "named:" + name)
        kept = self._places.get(key)
        if kept is None or kept[0] is not parent or kept[1] != len(parent):
            # (0.1.118: lxml's own walk of the children of that local name, any namespace:
            # element_children's list, not each child looked at)
            kept = (parent, len(parent), list(parent.iterchildren("{*}" + name)))
            self._places[key] = kept
        return kept[2]

    def resolve(self, root: Element | None, path: str) -> Element | None:
        """:func:`~docx2svg.paths.resolve_path` of ``path`` in ``root``: a path of indexed
        ``w:`` steps (``w:body/w:tbl[2]/w:tr[5]/w:tc[1]/w:p[1]``) step by step, each prefix
        resolved once (docx2svg's own resolver for each step, from the element the prefix
        before it names); any other path whole."""
        if root is None or not path:
            return resolve_path(root, path)
        steps = path.split("/")
        if len(steps) < 2 or not all(step.startswith("w:") and step.endswith("]") and "@" not in step
                                     for step in steps[1:]) or steps[0] != "w:body":
            return resolve_path(root, path)
        node = resolve_path(root, steps[0])
        prefix = steps[0]
        for step in steps[1:]:
            if node is None:
                return None
            prefix = f"{prefix}/{step}"
            key = (id(root), prefix)
            kept = self._prefixes.get(key)
            if kept is not None and kept[0] is root and kept[1] is node and kept[2] == len(node):
                node = kept[3]
                continue
            name, _, index = step[2:-1].partition("[")
            if name in ("tr", "tc") and index.isdigit():
                # resolve_path's own rule for a row or cell step, its rows listed once.
                rows = self.rows(node, name)
                found = rows[int(index) - 1] if 0 < int(index) <= len(rows) else None
            elif _INDEX.fullmatch(index) and _NAME.fullmatch(name) and _local(node.tag) not in _DRAWINGS \
                    and not (name in _DRAWINGS and _local(node.tag) == "r"):
                # Round 45: resolve_path's rule for any other indexed step -- the children of
                # that name, in order -- its children listed once (a body of a thousand
                # blocks was listed again for every paragraph of a layout's lines).
                named = self.named(node, name)
                found = named[int(index) - 1] if 0 < int(index) <= len(named) else None
            else:
                found = resolve_path(node, step)
            self._prefixes[key] = (root, node, len(node), found)
            node = found
        return node


#: :meth:`PathFinder.resolve`'s own steps: a name and a 1-based index as docx2svg's paths
#: write them; and the drawings whose steps count otherwise (a group's members, a run's
#: anchors and inlines), left to docx2svg's resolver.
_NAME = re.compile(r"\w+")
_INDEX = re.compile(r"\d+")
_DRAWINGS = frozenset({"anchor", "inline"})


def _local(tag) -> str:
    return tag.rpartition("}")[2] if isinstance(tag, str) else ""


class _Parts:
    def __init__(self, document: "Document") -> None:
        self._package = document.package
        self.main_document_part = document.package.document_part()

    def related(self, part: str, relationship_type: str) -> list[str]:
        return self._package.related_parts_of_type(part, relationship_type)


def _paragraphs(blocks):
    for block in blocks or ():
        if isinstance(block, _model.Paragraph):
            yield block
        elif isinstance(block, _model.Table):
            for row in block.rows:
                for cell in row:
                    yield from _paragraphs(cell)


def _all_tables(blocks):
    for block in blocks or ():
        if isinstance(block, _model.Table):
            yield block
            for row in block.rows:
                for cell in row:
                    yield from _all_tables(cell)


def row_leads(document: "Document", table: Element) -> list[Element | None]:
    """Per row of ``table`` (a ``w:tbl``), the paragraph whose keep-with-next keeps the row
    with the next -- the first of its first cell, a nested table's first there -- as
    docx2svg lays rows out by (:func:`docx2svg.table.lead_paragraph`); ``None`` for a row
    with none.  Empty for a table docx2svg does not read."""
    state = model(document)
    found = state.table(table)
    if found is None:
        return []
    leads = (lead_paragraph(row) for row in found.rows)
    root = table.getroottree().getroot()
    return [_drawn(root, lead, state._paths) for lead in leads]


def _drawn(root: Element, paragraph: _model.Paragraph | None, paths: PathFinder | None = None) -> Element | None:
    """Our ``w:p`` of docx2svg's ``paragraph`` in the part ``root`` is the root of."""
    resolve = paths.resolve if paths is not None else resolve_path
    found = resolve(root, paragraph.path) if paragraph is not None and paragraph.path else None
    return found if found is not None and found.tag == _W + "p" else None


def _table_style(element: Element) -> str | None:
    node = element.getparent()
    while node is not None and node.tag != _W + "tbl":
        node = node.getparent()
    if node is None:
        return None
    style = node.find(f"{_W}tblPr/{_W}tblStyle")
    return style.get(_W + "val") if style is not None else ""


def _kept_blocks(document: "Document") -> "list | None":
    """The document's own :class:`docx2svg.KeptBlocks`, the lock its parses take (one at a
    time) and its last segments (:func:`_segments`); ``None`` for a read copy (of one version) and with the edit path per change
    off (``DOCX_AGENT_INCREMENTAL=0``)."""
    if document.__dict__.get("_live") is not None:
        return None
    from ..revalidate import enabled

    if not enabled():
        return None
    found = document.__dict__.get("_effective_kept")
    if found is None:
        from docx2svg.parse.document import KeptBlocks

        found = document.__dict__.setdefault("_effective_kept", [KeptBlocks(), threading.Lock(), {}])
    return found


def _segments(document: "Document", kept: list) -> list | None:
    """docx2svg's segments of the main part as it is now (:func:`docx2svg.relayout.segments`),
    read from the last ones (its ``previous`` scan, its ``known`` segments): only the stretch
    an edit changed is read again.  ``None`` (the parse reads them) for a main part named
    otherwise."""
    from docx2svg.relayout import segments

    package = document.package
    main = package.document_part()
    if main != "word/document.xml":
        return None
    xml = package.read(main) or b""
    scanned: list = []
    keep: dict = {}
    found = segments(xml, kept[2].get("known"), keep, previous=kept[2].get("scan"), scanned=scanned)
    kept[2]["known"] = keep
    kept[2]["scan"] = scanned[0] if scanned else None
    return found


def seed_model(document: "Document", data: bytes, parsed: "_model.Document") -> None:
    """Take ``parsed``, docx2svg's parse of ``data``, as the document's (:func:`model`)
    where ``data`` is its bytes now (round 41: a worker's check reads the parse its layout
    was made from rather than parse the same bytes again); else nothing changes."""
    document._effective = (document._state, _Model(document, (data, parsed)))


def model(document: "Document") -> _Model:
    """docx2svg's parse of the document as it is now, cached until the next change."""
    cached = document._effective
    if cached is None or cached[0] != document._state:
        cached = (document._state, _Model(document))
        document._effective = cached
    return cached[1]


def run(document: "Document", paragraph: Element, run_element: Element | None,
        first: str | None = None, *, direct: bool = True) -> EffectiveRun:
    """``run_element``'s formatting in ``paragraph``; ``first`` is its first character (read
    from the paragraph when not given), which decides the script and so the face.  With
    ``run_element`` ``None``: what a run with no properties of its own would get there --
    the paragraph's baseline, which the Markdown reader measures emphasis against.  With
    ``direct`` false: what the run would get with its character style but none of its
    direct formatting -- what its styles give it."""
    state = _model_for(document, paragraph)
    owner = state.paragraph(paragraph)
    properties = run_element.find(_W + "rPr") if run_element is not None else None
    style = properties.find(_W + "rStyle") if properties is not None else None
    style_id = style.get(_W + "val") if style is not None else None
    if first is None:
        from .text import run_text

        first = (run_text(paragraph, run_element) if run_element is not None else "") or "a"
    return _run(state, owner, style_id, read_run_properties(properties) if direct else {}, first)


def _run(state: _Model, owner: _model.Paragraph, style_id: str | None, declared: dict,
         first: str) -> EffectiveRun:
    model_run = _model.Run("", _model.RunProperties(style_id=style_id, declared=declared))
    resolved = resolve_run(state.model, owner, model_run)
    character = character_format(resolved, first[0], state.model)
    underline = resolved.get("u")
    return EffectiveRun(
        bold=character.bold, italic=character.italic,
        underline=None if underline in (None, "none") else underline,
        strike=bool(resolved.get("strike")), double_strike=bool(resolved.get("dstrike")),
        caps=bool(resolved.get("caps")), small_caps=bool(resolved.get("smallCaps")),
        size=character.half_points / 2, font=character.face,
        color=resolved.get("color") or "auto", highlight=resolved.get("highlight"),
        vertical_align=resolved.get("vertAlign"), hidden=bool(resolved.get("vanish")),
        style=style_id, _resolved=resolved)


def paragraph(document: "Document", element: Element, *, direct: bool = True) -> EffectiveParagraph:
    """``element``'s paragraph formatting as Word applies it; with ``direct`` false, what
    its styles (and its list, which clearing direct formatting keeps) give it."""
    state = _model_for(document, element)
    owner = state.paragraph(element)
    if not direct and owner.properties is not None:
        kept = {k: v for k, v in owner.properties.declared.items() if k.startswith("numPr.")}
        owner = dataclasses.replace(owner, properties=dataclasses.replace(owner.properties, declared=kept))
    return _paragraph(state, owner)


def style_look(document: "Document", style_id: str | None, *, table_style: str | None = None,
               conditions: tuple[str, ...] = (), character_style: str | None = None,
               ) -> tuple[EffectiveParagraph, EffectiveRun]:
    """What a paragraph of ``style_id`` (``None``: the default paragraph style) with no
    direct formatting, and a run in it with none, get: the style's look after inheritance.
    With ``table_style`` (``""``: the default table style) the paragraph is in a cell of a
    table of that style, under the cell's conditional formats ``conditions``
    (``("wholeTable",)`` for a cell no row or column format reaches).  With
    ``character_style`` the run is in that character style."""
    state = model(document)
    owner = _model.Paragraph(
        properties=_model.ParagraphProperties(style_id=style_id, declared={},
                                              run_properties=_model.RunProperties(declared={})),
        table_style_id=table_style, table_conditions=tuple(conditions))
    return _paragraph(state, owner), _run(state, owner, character_style, {}, "a")


def _paragraph(state: _Model, owner: _model.Paragraph) -> EffectiveParagraph:
    resolved = resolve_paragraph(state.model, owner)

    def points(key: str) -> float:
        value = resolved.get(key)
        return (value or 0) / 20

    rule = resolved.get("spacing.lineRule") or "auto"
    line = resolved.get("spacing.line")
    line_spacing = None if line is None else (line / 240 if rule == "auto" else line / 20)
    num_id = resolved.get("numPr.numId")
    numbering = (num_id, resolved.get("numPr.ilvl") or 0) if num_id else None
    style = owner.properties.style_id if owner.properties else None
    return EffectiveParagraph(
        style=style, alignment=_ALIGNMENTS.get(resolved.get("jc") or "left", resolved.get("jc")),
        indent_left=points("ind.left"), indent_right=points("ind.right"),
        first_line=points("ind.firstLine"), hanging=points("ind.hanging"),
        space_before=points("spacing.before"), space_after=points("spacing.after"),
        line_spacing=line_spacing, line_rule=rule,
        keep_with_next=bool(resolved.get("keepNext")), keep_together=bool(resolved.get("keepLines")),
        page_break_before=bool(resolved.get("pageBreakBefore")),
        widow_control=bool(resolved.get("widowControl", True)),
        outline_level=resolved.get("outlineLvl"), numbering=numbering, _resolved=resolved)


# -- one top-level table's model (lane H1) ---------------------------------------------------------

def _model_for(document: "Document", element: Element) -> _Model:
    """:func:`model`, or -- for an element of a table :func:`table_model` made a model of,
    in the state it was made for -- that table's own model."""
    kept = document.__dict__.get("_effective_tables")
    if kept is not None and kept[0] == document._state and kept[1]:
        node = element
        while node is not None:
            found = kept[1].get(id(node))
            if found is not None and found[0] is node:
                return found[1]
            node = node.getparent()
    return model(document)


def table_model(document: "Document", table: Element) -> bool:
    """Make a top-level table of the body read its paragraphs' effective formatting from
    docx2svg's parse of that table alone and of the parts every block's formatting reads
    (styles, numbering, theme, settings), rather than of the whole document, until the
    document changes -- the same models, at a table's cost (lane H1: ``word_edit_table``'s
    Markdown of its table).  Whether it does.

    The same because nothing else of the body reaches a table's paragraphs' properties,
    their table style and conditional formats, or their places, but a paragraph mark
    deleted in it or just before it (docx2svg's final view joins those across blocks,
    ``_join`` and ``_following_properties``).  Not for a table with one, one inside a
    content control or anything but the body, a document with picture bullets or a part
    missing, nor with ``DOCX_AGENT_INCREMENTAL=0``: the whole document's model, as before.
    With ``DOCX_AGENT_FULL_VALIDATE`` the Markdown read is compared with the whole
    document's (``docx_agent.markdown.read``)."""
    cached = document._effective
    if cached is not None and cached[0] == document._state:
        return False                        # the whole model is made already
    kept = document.__dict__.get("_effective_tables")
    if kept is None or kept[0] != document._state:
        kept = (document._state, {})
        document.__dict__["_effective_tables"] = kept
    found = kept[1].get(id(table))
    if found is not None and found[0] is table:
        return True
    made = _table_model(document, table)
    if made is None:
        return False
    kept[1][id(table)] = (table, made)
    return True


_BLOCK_NAMES = frozenset({"p", "tbl", "sdt", "customXml"})


def _table_model(document: "Document", table: Element) -> "_Model | None":
    from docx2svg.parse import document as pdoc
    from lxml import etree

    from ..revalidate import enabled

    if not enabled():
        return None
    package = document.package
    main = package.document_part()
    body = table.getparent()
    root = package.tree(main) if main else None
    if root is None or body is None or body.tag != _W + "body" or body.getparent() is not root:
        return None
    # A paragraph mark deleted in the table or just before it: joined across blocks.
    if any(pdoc._mark_removed(p) for p in table.iter(_W + "p")):
        return None
    place = 1
    before = True
    for node in table.itersiblings(preceding=True):
        if not isinstance(node.tag, str):
            continue
        name = _local(node.tag)
        if name == "tbl":
            place += 1
        if before and name in _BLOCK_NAMES:
            before = False
            if name in ("sdt", "customXml") or (name == "p" and pdoc._mark_removed(node)):
                return None
    shared = _shared_model(document)
    if shared is None:
        return None
    globals_, colours = shared
    from docx2svg.parse.properties import theme_colours
    from ooxml_common.xmlutil import parse_xml

    element = parse_xml(etree.tostring(table))
    out: list = []
    with theme_colours(colours):
        pdoc._block_of(element, "tbl", f"w:body/w:tbl[{place}]", None, (),
                       pdoc._Fields([], cross_references=True, styleref=True), out)
        blocks = pdoc._following_properties(pdoc._join(out))[0]
    import copy

    parsed = copy.copy(globals_)
    parsed.body = blocks
    parsed.paragraphs = [b for b in blocks if isinstance(b, _model.Paragraph)]
    state = object.__new__(_Model)
    state.document = document
    state.model = parsed
    state._paragraphs, state._elements, state._tables, state._held = {}, {}, None, []
    state._mapped, state._by_path = False, None
    state._paths = PathFinder()
    return state


def _shared_model(document: "Document"):
    """docx2svg's reading of what every block's formatting reads -- the styles (with
    ``w:docDefaults``), the numbering, the theme, the settings -- as ``parse_package``
    reads them, under the theme's colours (``(model, colours)``), kept by those parts'
    bytes; ``None`` where a part is missing or the numbering has picture bullets."""
    from docx2svg.opc import REL_NUMBERING, REL_SETTINGS, REL_STYLES, REL_THEME
    from docx2svg.parse import document as pdoc
    from docx2svg.parse.properties import theme_colours

    package = document.package
    main = package.document_part()

    def parts(kind: str) -> list[bytes] | None:
        out = []
        for name in package.related_parts_of_type(main, kind):
            data = package.read(name)
            if data is None:
                return None
            out.append(data)
        return out

    found = {kind: parts(kind) for kind in (REL_STYLES, REL_THEME, REL_SETTINGS, REL_NUMBERING)}
    if any(v is None for v in found.values()):
        return None
    key = tuple((kind, tuple(found[kind][:1])) for kind in sorted(found))
    kept = document.__dict__.get("_effective_shared")
    if kept is not None and kept[0] == key:
        return kept[1]
    styles = found[REL_STYLES][0] if found[REL_STYLES] else None
    theme = found[REL_THEME][0] if found[REL_THEME] else None
    settings = found[REL_SETTINGS][0] if found[REL_SETTINGS] else None
    numbering_xml = found[REL_NUMBERING][0] if found[REL_NUMBERING] else None
    if numbering_xml is not None and pdoc.parse_picture_bullets(numbering_xml):
        return None
    colours = pdoc.word_theme_colour(pdoc.parse_theme_colors(theme) if theme is not None else {},
                                     pdoc.parse_color_map(settings) if settings is not None else {})
    with theme_colours(colours):
        made = _model.Document()
        if styles is not None:
            made.default_run_properties, made.default_paragraph_properties = pdoc._doc_defaults(styles)
            made.styles = pdoc.parse_styles(styles)
        if theme is not None:
            made.font_scheme = pdoc.parse_theme(theme)
            made.theme_colors = pdoc.parse_theme_colors(theme)
            made.theme_formats = pdoc.parse_theme_formats(theme)
        if numbering_xml is not None:
            numbering = pdoc.parse_numbering(numbering_xml, made.styles)
            made.numbering = numbering.levels
            made.numbering_lists = numbering.lists
            made.numbering_restarts = numbering.restarts
        if settings is not None:
            for name, value in pdoc.settings_read(settings).items():
                setattr(made, name, value)
    document.__dict__["_effective_shared"] = (key, (made, colours))
    return made, colours
