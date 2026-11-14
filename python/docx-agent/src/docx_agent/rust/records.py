"""What a check's facts read of each of the body's blocks besides its layout, made per block
(lane F): a block's record is made when the block is made or changed, with docx-agent's own
rules of :mod:`docx_agent.layout_facts` and the modules they call, and handed to the Rust
owner with the block's measuring input; the facts of a range are put together there from the
records of the range's blocks (and, for the few facts that count over the document, every
block's), never by reading the document again.

A record depends on the block and on what every block shares -- the styles, the numbering,
the settings, the theme: a change to one of those parts makes every record again (the
mirror's whole export, :meth:`~docx_agent.rust.mirror.Mirror.everything`).

Paragraph (a top-level ``w:p``)::

    {"st": style name, "kwn", "kt", "pbb": its keeps and page break before (effective),
     "bs", "be": a page break run at its start / end, "content": a break, tab, field...,
     "state": empty_paragraphs' state, "sb", "sa": space before / after (pt),
     "txt", "cap", "colon": table_parted's kinds, "df": direct_formatting's entry,
     "pu": punctuation_spacing's entries, "lj": text_language's judgement,
     "num": numbering_skips_level's membership, "fld": its fields, "bm": its bookmarks,
     "draw": what it draws besides its text (since_last_check)}

Table (a top-level ``w:tbl``)::

    {"rows": [[lead id, keeps, direct]], "kt": kept together, "wn": last row keeps with
     the next, "hr": header rows, "hl": header_look, "cols": grid columns, "rkn": each row
     keeps with the next (docx2svg's rule), "gm", "rg": table_grid_mismatch's entries
     (off the grid, ragged), "al": accept_loses_text's, "er": empty_paragraphs' (its cells),
     "paras": its paragraphs' records, "kmid", "klast": keep_together's reading,
     "fld", "bm", "draw"}
"""
from __future__ import annotations

import hashlib
from typing import Any

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
_P, _TBL = _W + "p", _W + "tbl"


class Context:
    """What the records of one reading share: the document's style names, default styles,
    the language cascade and the numbering read once."""

    def __init__(self, document: Any) -> None:
        from ..edit import typography
        from ..layout_facts import _Facts

        self.document = document
        facts = _Facts.__new__(_Facts)
        facts.document = document
        facts.style_names = {s.id: s.name or s.id for s in document.styles}
        default = document.styles.default("paragraph")
        facts.default_paragraph = default.id if default is not None else None
        facts.kept_leads = set()
        self.facts = facts
        self.default_name = default.name if default is not None else None
        self.cascade = typography.Cascade(document)
        #: Per paragraph style id: whether it is the Caption style (``minimums.is_caption``).
        self.captions: dict[str, bool] = {}
        #: ``layout_facts._first_label`` by ``(numId, label)``.
        self.first_labels: dict = {}
        self._numbering = None
        self.guess: list[str] = []
        self.own: list = []
        self.revisions: list = []
        self.main = document.package.document_part()

    @property
    def numbering(self) -> Any:
        if self._numbering is None:
            from ..edit.numbering import Numbering

            self._numbering = Numbering(self.document)
        return self._numbering

    def is_caption(self, element: Any) -> bool:
        """``minimums.is_caption``, by the paragraph's style id once per reading."""
        from ..edit.minimums import is_caption

        properties = element.find(_W + "pPr")
        node = properties.find(_W + "pStyle") if properties is not None else None
        if node is None:
            return False
        style_id = node.get(_W + "val") or ""
        if style_id not in self.captions:
            self.captions[style_id] = is_caption(self.document, element)
        return self.captions[style_id]

    @property
    def index(self) -> Any:
        """The body's index (built when a table's record first asks: a paragraph's needs none)."""
        return self.document._index(self.main)


def _hash(value: Any) -> str:
    return hashlib.blake2b(repr(value).encode(), digest_size=8).hexdigest()


def paragraph_record(ctx: Context, element: Any, identifier: str, *, top: bool) -> dict[str, Any]:
    """A paragraph's record (``top``: a top-level one, with what only those need)."""
    from ..edit import effective as _effective
    from ..edit.text_language import _judge, primary, PROFILES
    from ..edit.typography import paragraph_problems
    from ..layout_facts import _CONTENT, _page_break_at, _state

    from ..edit import text as _text

    document = ctx.document
    # A check of numbers' text of it (textfacts.number_texts).
    text = _text.paragraph_text(element).replace(_text.OBJECT, " ")
    rec: dict[str, Any] = {"id": identifier, "nt": text, "nd": any(ch.isdigit() for ch in text)}
    found = ctx.facts._direct(element, identifier)
    if found is not None:
        rec["df"] = found
    problems = paragraph_problems(document, ctx.cascade, identifier, element, ctx.guess, ctx.own)
    if problems:
        rec["pu"] = problems
    judged, why = _judge(ctx.cascade, element, identifier)
    if judged is not None:
        rec["lj"] = {"tag": judged.tag, "source": judged.source, "lang": judged.found.language,
                     "conf": judged.found.confidence, "words": judged.found.words,
                     "prim": primary(judged.tag), "profiled": primary(judged.tag) in PROFILES}
    elif why is not None:
        rec["lw"] = why
    number = _numbering(ctx, element)
    if number is not None:
        rec["num"] = number
    if top:
        from ..edit.formatting import read_paragraph

        applied = _effective.paragraph(document, element)
        styled = _effective.paragraph(document, element, direct=False)
        properties = element.find(_W + "pPr")
        # What a keep's trial writes (Document._keep_values): the direct keeps and the styles'.
        rec.update({"ktd": bool(read_paragraph(properties, "keep_together")),
                    "kwnd": bool(read_paragraph(properties, "keep_with_next")),
                    "kts": bool(styled.keep_together), "kwns": bool(styled.keep_with_next)})
        text = "".join(node.text or "" for node in element.iter(_W + "t")).strip()
        rec.update({
            "st": ctx.facts.style_of(element),
            "kwn": bool(applied.keep_with_next), "kt": bool(applied.keep_together),
            "pbb": bool(applied.page_break_before),
            "bs": _page_break_at(element, start=True), "be": _page_break_at(element, start=False),
            "content": any(node.tag in _CONTENT for node in element.iter() if isinstance(node.tag, str)),
            "state": _state(document, element),
            "sb": applied.space_before, "sa": applied.space_after,
            "txt": bool(text), "colon": text.endswith(":"),
            "cap": bool(text) and ctx.is_caption(element),
        })
    return rec


def _numbering(ctx: Context, element: Any) -> dict[str, Any] | None:
    """``numbering_skips_level``'s reading of a paragraph (:func:`docx_agent.layout_facts._skips_level`)."""
    from ..edit.numbering import definition_of, membership, paragraph_style_id, style_list
    from ..layout_facts import _LEVEL_REF, _first_label

    document = ctx.document
    found = membership(document, element)
    if found is None:
        return None
    style_id = paragraph_style_id(element)
    if found.from_style and style_list(document, style_id) is None:
        return None
    definition = definition_of(document, found.num_id)
    if definition is None:
        return None
    name = ctx.facts.style_names.get(style_id, style_id) if style_id else ctx.default_name
    shown = sorted({int(n) - 1 for n in _LEVEL_REF.findall(found.text or "")})
    key = (found.num_id, found.text)
    if key not in ctx.first_labels:
        ctx.first_labels[key] = _first_label(ctx.numbering, found.num_id, found.text, shown)
    return {"def": definition.get(_W + "abstractNumId") or _hash(id(definition)), "lvl": found.level,
            "style": name, "list": found.num_id, "label": found.text, "shown": shown,
            "first": ctx.first_labels[key]}


def table_record(ctx: Context, element: Any, identifier: str, model_table: Any, model: Any) -> dict[str, Any]:
    """A top-level table's record: ``model_table`` its docx2svg model, ``model`` the
    document's (the mirror's)."""
    from docx2svg.table import row_keeps_with_next

    from ..edit.table_format import kept_together, row_keeps
    from ..layout_facts import (_columns, _header_rows, header_look, ragged_rows, table_accept_loses,
                                table_empty_runs, table_grid_mismatch)

    document = ctx.document
    index = ctx.index
    rows = row_keeps(document, element)
    kept = kept_together(rows)
    entry = index.entry_for(element)

    def id_of(paragraph: Any) -> str | None:
        found = index.entry_for(paragraph) if paragraph is not None else None
        return found.id if found is not None else None

    rec: dict[str, Any] = {
        "id": identifier,
        "rows": [[id_of(lead), bool(keeps), bool(declared is not None and declared != styled), bool(declared),
                  bool(styled)] for lead, keeps, styled, declared in rows],
        "kt": bool(kept), "wn": bool(rows and rows[-1][1]),
        "hr": _header_rows(element), "cols": _columns(element),
        "hl": header_look(document, model_table, element),
        "rkn": [bool(row_keeps_with_next(model, row)) for row in model_table.rows] if model_table is not None
        else None,
        "kmid": all(keeps for lead, keeps, _, _ in rows if lead is not None),
        "klast": all(keeps for lead, keeps, _, _ in rows[:-1] if lead is not None),
    }
    tables = [entry] if entry is not None else []
    ids = {t.id for t in tables}
    mismatched = [e for e in table_grid_mismatch(document, keep=ids.__contains__) if e.get("drawn") != "ragged"]
    ragged = [r for r in (ragged_rows(document, t) for t in tables) if r is not None]
    if mismatched:
        rec["gm"] = mismatched
    off_grid = {e["table"] for e in mismatched}
    ragged = [r for r in ragged if r["after_accept"] or r["table"] not in off_grid]
    if ragged:
        rec["rg"] = ragged
    lost = []
    for t in tables:
        lost += table_accept_loses(document, t, ctx.revisions)
    if lost:
        rec["al"] = lost
    runs = []
    for t in tables:
        runs += table_empty_runs(document, index, t)
    if runs:
        rec["er"] = runs
    # Its rows' deciding paragraphs: their keeps are the table's (direct_formatting).
    ctx.facts.kept_leads = {id(row[0]) for row in rows} if kept else set()
    try:
        rec["paras"] = [paragraph_record(ctx, p, id_of(p) or "", top=False) for p in element.iter(_P)
                        if id_of(p) is not None]
    finally:
        ctx.facts.kept_leads = set()
    from ..tools.textfacts import _cell_id

    _TC = _W + "tc"
    for found, p in zip(rec["paras"], [p for p in element.iter(_P) if id_of(p) is not None]):
        cell = next(p.iterancestors(_TC), None)
        found["cell"] = _cell_id(index, cell) if cell is not None else None
    return rec


def fields_of(element: Any, stack: list) -> tuple[list[list[Any]], list[str]]:
    """The fields begun in a block, ``[[keyword, bookmark named or None, contents]...]`` in
    order, as :func:`docx_agent.edit.fields.scan` finds them -- ``stack``, the fields open
    before it (``[[instruction, separated]...]``), carried on (changed in place) -- and the
    names of the bookmarks it starts.  ``contents`` is a table of contents' result as far as
    the block tells it: ``"empty"`` (none, or Word's "no entries"), ``"some"``, or ``None``
    (its result goes on past the block, nothing in it yet).  Raises ``Unsupported`` for a
    field whose instruction may go on past the block."""
    from ..edit.fields import EMPTY_TOC, _deleted
    from .exporter import Unsupported

    begun: list[list[Any]] = []
    tocs: list[list[Any]] = []          # [field, text so far, ended]
    for node in element.iter():
        tag = node.tag
        if tag == _W + "t" and tocs:
            if not _deleted(node):
                for toc in tocs:
                    if toc[3] and not toc[2]:
                        toc[1].append(node.text or "")
            continue
        if tag == _W + "tab" and tocs and node.getparent().tag == _W + "r":
            for toc in tocs:
                if toc[3] and not toc[2]:
                    toc[1].append("\t")
            continue
        if tag == _P and tocs:
            for toc in tocs:
                if toc[3] and not toc[2]:
                    toc[1].append("\n")
            continue
        if tag not in (_W + "fldChar", _W + "instrText", _W + "fldSimple") or _deleted(node):
            continue
        if tag == _W + "fldSimple":
            instruction = node.get(_W + "instr") or ""
            found = [_keyword(instruction), _target(instruction), None]
            if found[0] == "TOC":
                text = "".join(t.text or "" for t in node.iter(_W + "t"))
                found[2] = "empty" if text.strip() in ("", EMPTY_TOC) else "some"
            begun.append(found)
            continue
        if tag == _W + "instrText":
            if stack and stack[-1][1] is None:
                stack[-1][0] += node.text or ""
            continue
        kind = node.get(_W + "fldCharType")
        if kind == "begin":
            field = [None, None, None]
            stack.append(["", None, field])
            begun.append(field)
        elif kind == "separate" and stack:
            top = stack[-1]
            top[1] = True
            field = top[2]
            if field is not None:
                field[0], field[1] = _keyword(top[0]), _target(top[0])
                if field[0] == "TOC":
                    tocs.append([field, [], False, True])
        elif kind == "end" and stack:
            top = stack.pop()
            field = top[2]
            if field is not None:
                if field[0] is None:
                    field[0], field[1] = _keyword(top[0]), _target(top[0])
                for toc in tocs:
                    if toc[0] is field:
                        toc[2] = True
                        toc[0][2] = "empty" if "".join(toc[1]).strip("\n").strip() in ("", EMPTY_TOC) \
                            else "some"
    for toc in tocs:
        if not toc[2]:
            text = "".join(toc[1]).strip("\n").strip()
            toc[0][2] = "some" if text and not EMPTY_TOC.startswith(text) else None
    if any(entry[1] is None for entry in stack):
        raise Unsupported("a field's instruction going on past its block")
    for entry in stack:
        entry[2] = None                 # (the block's own: the next block's are not its)
    out = []
    for field in begun:
        if field[0] is None:
            raise Unsupported("a field begun with no instruction read")
        out.append(list(field))
    names = [node.get(_W + "name") or "" for node in element.iter(_W + "bookmarkStart")]
    return out, names


def _keyword(instruction: str) -> str:
    words = instruction.split()
    return words[0].upper() if words else ""


def _target(instruction: str) -> str | None:
    words = instruction.split()
    if not words or words[0].upper() not in ("REF", "PAGEREF", "NOTEREF"):
        return None
    return words[1] if len(words) > 1 else ""
