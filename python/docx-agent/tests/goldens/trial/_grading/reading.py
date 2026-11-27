"""The trial checks' reading of a Word document: the package read with ``zipfile`` and
``xml.etree``, and -- for what only the tools can tell (validation, Accept All and Reject All,
a chart's data, a block's Markdown, effective formatting, where a block is laid out) -- the
document opened in a session of docx-agent's tools.

:class:`Document` offers the few readings the checks make, by the names they used before
docx-agent's document API was retired (P4e): ``paragraphs``, ``tables``, ``revisions``,
``comments``, ``fields``, ``charts`` and the rest.  Grader-side only: nothing here writes the
document a task made.

Must not: import docx-agent's document API (there is none) or lxml.
"""
from __future__ import annotations

import datetime as dt
import io
import json
import posixpath
import re
import zipfile
import xml.etree.ElementTree as ET
from functools import cached_property
from pathlib import Path
from typing import Any

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
W14 = "{http://schemas.microsoft.com/office/word/2010/wordml}"
W15 = "{http://schemas.microsoft.com/office/word/2012/wordml}"
R = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}"
WP = "{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}"
A = "{http://schemas.openxmlformats.org/drawingml/2006/main}"
PIC = "{http://schemas.openxmlformats.org/drawingml/2006/picture}"
C = "{http://schemas.openxmlformats.org/drawingml/2006/chart}"
PR = "{http://schemas.openxmlformats.org/package/2006/relationships}"
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"
DC = "{http://purl.org/dc/elements/1.1/}"
MAIN = "word/document.xml"

# -- the session -----------------------------------------------------------------------------

_BOX: Any = None


def _toolbox() -> Any:
    """One toolbox for the process (opened on first use, closed with the process)."""
    global _BOX
    if _BOX is None:
        from ooxml_edit.tools import Toolbox

        from docx_agent.tools import FORMAT, GROUPS, TOOLS

        _BOX = Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0)
        _BOX.__enter__()
    return _BOX


class Opened:
    """A document's bytes open in a session of its own."""

    def __init__(self, data: bytes, name: str = "graded.docx") -> None:
        self.box = _toolbox()
        self.session = self.box.session()
        self.doc = self.session.open(data, name)

    def call(self, tool: str, **arguments: Any) -> Any:
        result = self.box.dispatch(self.session, tool, {"doc": self.doc, **arguments})
        if not result.ok:
            raise RuntimeError(f"{tool} {arguments}: {json.dumps(result.to_json(), default=str)[:400]}")
        return result

    def data(self) -> bytes:
        return self.session.save(self.doc).data


# -- small readers ---------------------------------------------------------------------------


def _parents(root: ET.Element) -> dict[ET.Element, ET.Element]:
    return {child: parent for parent in root.iter() for child in parent}


def _ancestors(element: ET.Element, parents: dict) -> list[ET.Element]:
    out = []
    while element in parents:
        element = parents[element]
        out.append(element)
    return out


def _text(element: ET.Element, parents: dict, *, deleted: bool = False) -> str:
    """Text of ``element`` in the final view (``deleted``: the deleted text alone): a tab
    ``\\t``, a break ``\\v``."""
    out = []
    for node in element.iter():
        tag = node.tag
        if tag not in (_W + "t", _W + "delText", _W + "tab", _W + "br", _W + "cr", _W + "noBreakHyphen"):
            continue
        gone = any(a.tag in (_W + "del", _W + "moveFrom") for a in _ancestors(node, parents))
        if deleted:
            if tag == _W + "delText" or (gone and tag == _W + "t"):
                out.append(node.text or "")
            continue
        if gone or tag == _W + "delText":
            continue
        if tag == _W + "t":
            out.append(node.text or "")
        elif tag == _W + "tab":
            if parents.get(node) is not None and parents[node].tag == _W + "r":
                out.append("\t")
        elif tag == _W + "noBreakHyphen":
            out.append("‑")
        else:
            out.append("\v")
    return "".join(out)


class CallableList(list):
    """A list a check may also call (``doc.stories`` and ``doc.stories()`` alike)."""

    def __call__(self) -> "CallableList":
        return self


# -- the document's parts ----------------------------------------------------------------------


class Story:
    def __init__(self, name: str, part: str) -> None:
        self.name, self.part = name, part

    def __str__(self) -> str:
        return f"Story {self.name} {self.part}"


class ListInfo:
    def __init__(self, num_id: str, level: int, format: str | None) -> None:
        self.num_id, self.level, self.format = num_id, level, format

    def __repr__(self) -> str:
        return f"ListInfo({self.format}, level {self.level})"


class Paragraph:
    def __init__(self, document: "Document", element: ET.Element, story: str, index: int) -> None:
        self.document, self._element, self.story = document, element, story
        para = element.get(W14 + "paraId")
        own = f"p:{para}" if para else f"p@{story}/{index}"
        self.id = own if story == "body" else f"{story}/{own}"
        self.text = _text(element, document._parents(story))

    @property
    def style_id(self) -> str | None:
        style = self._element.find(f"{_W}pPr/{_W}pStyle")
        return style.get(_W + "val") if style is not None else None

    @property
    def style_name(self) -> str | None:
        return self.document._style_name(self.style_id, "paragraph")

    @property
    def list(self) -> ListInfo | None:
        return self.document._list_of(self._element, self.style_id)

    def __repr__(self) -> str:
        return f"Paragraph({self.id}, {self.text[:30]!r})"


class Cell:
    def __init__(self, document: "Document", element: ET.Element, story: str) -> None:
        self._element = element
        self.paragraphs = [Paragraph(document, p, story, -1) for p in element if p.tag == _W + "p"]
        self.text = "\n".join(p.text for p in self.paragraphs)


class Table:
    def __init__(self, document: "Document", element: ET.Element, story: str) -> None:
        self.document, self._element, self.story = document, element, story
        self.rows = [tr for tr in element if tr.tag == _W + "tr"]
        self.column_count = len(element.findall(f"{_W}tblGrid/{_W}gridCol"))

    def cell(self, row: int, column: int) -> Cell:
        """The cell over grid column ``column`` of row ``row`` (a cell spanning columns is
        each of them)."""
        at = 0
        tr = self.rows[row]
        before = tr.find(f"{_W}trPr/{_W}gridBefore")
        if before is not None:
            at = int(before.get(_W + "val", "0"))
        for tc in (child for child in tr if child.tag == _W + "tc"):
            span = tc.find(f"{_W}tcPr/{_W}gridSpan")
            width = int(span.get(_W + "val")) if span is not None else 1
            if at <= column < at + width:
                return Cell(self.document, tc, self.story)
            at += width
        raise IndexError(f"no cell ({row}, {column})")


class Revision:
    def __init__(self, kind: str, element: ET.Element, story: str, paragraph_id: str | None, text: str) -> None:
        self.kind, self.story, self.paragraph_id, self.text = kind, story, paragraph_id, text
        self.id = f"rev:{element.get(_W + 'id')}"
        self.author = element.get(_W + "author")
        self.date = element.get(_W + "date")

    def __repr__(self) -> str:
        return f"Revision({self.kind}, {self.author}, {self.text[:30]!r})"


class Comment:
    def __init__(self, element: ET.Element, parents: dict) -> None:
        self.id = element.get(_W + "id")
        self.author = element.get(_W + "author")
        paragraphs = [p for p in element if p.tag == _W + "p"]
        self.text = "\n".join(_text(p, parents) for p in paragraphs)
        self.para_id = paragraphs[-1].get(W14 + "paraId") if paragraphs else None
        self.done = False
        self.parent: str | None = None
        self.replies: list[Comment] = []


class Field:
    def __init__(self, instruction: str, result: str, paragraph_id: str | None) -> None:
        self.instruction = " ".join(instruction.split())
        self.keyword = self.instruction.split()[0].upper() if self.instruction else ""
        self.result, self.paragraph_id = result, paragraph_id

    def __repr__(self) -> str:
        return f"Field({self.instruction!r})"


class Hyperlink:
    def __init__(self, target: str | None, text: str) -> None:
        self.target, self.text = target, text

    def __str__(self) -> str:
        return str(self.target)


class Control:
    def __init__(self, element: ET.Element, parents: dict) -> None:
        props = element.find(_W + "sdtPr")
        alias = props.find(_W + "alias") if props is not None else None
        tag = props.find(_W + "tag") if props is not None else None
        self.alias = alias.get(_W + "val") if alias is not None else None
        self.tag = tag.get(_W + "val") if tag is not None else None
        content = element.find(_W + "sdtContent")
        text = _text(content, parents) if content is not None else ""
        box = props.find(W14 + "checkbox") if props is not None else None
        date = props.find(_W + "date") if props is not None else None
        if box is not None:
            checked = box.find(W14 + "checked")
            self.value: Any = checked is not None and checked.get(W14 + "val") in ("1", "true")
        elif date is not None and date.get(_W + "fullDate"):
            self.value = dt.date.fromisoformat(date.get(_W + "fullDate")[:10])
        else:
            self.value = text


class Picture:
    def __init__(self, drawing: ET.Element) -> None:
        doc_pr = next(drawing.iter(WP + "docPr"))
        self.id = f"d:{doc_pr.get('id')}"
        self.alt_text = doc_pr.get("descr")


class Note:
    def __init__(self, kind: str, element: ET.Element, parents: dict) -> None:
        self.id = f"{kind}:{element.get(_W + 'id')}"
        self.text = " ".join(_text(p, parents) for p in element if p.tag == _W + "p").strip()


class Section:
    def __init__(self, element: ET.Element) -> None:
        size, margins = element.find(_W + "pgSz"), element.find(_W + "pgMar")
        self.page_width = int(size.get(_W + "w")) if size is not None else 0
        left = int(margins.get(_W + "left", "0")) if margins is not None else 0
        right = int(margins.get(_W + "right", "0")) if margins is not None else 0
        #: Twips.
        self.text_width = self.page_width - left - right


class Drawing:
    def __init__(self, element: ET.Element) -> None:
        extent = element.find(f".//{WP}extent")
        #: Points.
        self.size = (int(extent.get("cx")) / 12700, int(extent.get("cy")) / 12700)


class Style:
    def __init__(self, element: ET.Element) -> None:
        name = element.find(_W + "name")
        self.id = element.get(_W + "styleId")
        self.type = element.get(_W + "type")
        self.name = name.get(_W + "val") if name is not None else self.id
        self._element = element


class Styles(list):
    def find(self, name: str) -> Style | None:
        return next((s for s in self if (s.name or "").lower() == name.lower()), None)


class Placement:
    def __init__(self, page: int) -> None:
        self.page = page


class Layout:
    def __init__(self, document: "Document") -> None:
        self.document = document

    def where(self, block: str) -> list[Placement]:
        """Where ``word_inspect`` places a block: each page it is on."""
        data = self.document._opened.call("word_inspect", target=block, layout=True).data
        return [Placement(int(found["page"])) for item in data.get("blocks", [])
                for found in item.get("placements") or []]


class Chart:
    """A chart as ``edit_chart`` ``read`` gives it, and where its workbook is."""

    def __init__(self, document: "Document", drawing: str, workbook_part: str | None) -> None:
        self.id, self.workbook_part = drawing, workbook_part
        self.data = document._opened.call("edit_chart", target=drawing, action="read").data
        self.categories = list(self.data.get("categories") or [])
        self.series = [_Series(s) for s in self.data.get("series") or []]
        self.title = self.data.get("title")
        self.chart_types = list(self.data.get("types") or [])
        self.has_legend = bool(self.data.get("legend"))

    def axis_title(self, which: str) -> str | None:
        """The ``category`` or ``value`` axis's title."""
        return (self.data.get("axes") or {}).get(which)

    def workbook_values(self) -> dict:
        """What Edit Data holds: the categories' and each series' cells and values."""
        return self.data.get("workbook") or {}


class _Series:
    def __init__(self, data: dict) -> None:
        self.name, self.values = data.get("name"), data.get("values") or []


# -- the document ------------------------------------------------------------------------------


class Document:
    """A Word document's package, as the trial's checks read it."""

    def __init__(self, data: bytes) -> None:
        self.data = data
        self._zip = zipfile.ZipFile(io.BytesIO(data))
        self._trees: dict[str, ET.Element] = {}
        self._parent_maps: dict[str, dict] = {}
        self._session: Opened | None = None

    @classmethod
    def open(cls, source: Any) -> "Document":
        return cls(source if isinstance(source, bytes) else Path(source).read_bytes())

    # -- parts

    def _tree(self, part: str) -> ET.Element | None:
        if part not in self._trees:
            if part not in self._zip.namelist():
                return None
            self._trees[part] = ET.fromstring(self._zip.read(part))
        return self._trees[part]

    def _rels(self, part: str) -> dict[str, tuple[str, str, str | None]]:
        """``{id: (type, target part or URL, mode)}`` of a part's relationships."""
        folder, name = posixpath.split(part)
        tree = self._tree(posixpath.join(folder, "_rels", name + ".rels"))
        out = {}
        for rel in tree if tree is not None else []:
            target, mode = rel.get("Target"), rel.get("TargetMode")
            if mode != "External":
                target = posixpath.normpath(posixpath.join(folder, target)) if not target.startswith("/") else target[1:]
            out[rel.get("Id")] = (rel.get("Type"), target, mode)
        return out

    @cached_property
    def stories(self) -> CallableList:
        """The body, then headers, footers, footnotes, endnotes and comments."""
        out = [Story("body", MAIN)]
        rels = self._rels(MAIN)
        for kind in ("header", "footer", "footnotes", "endnotes", "comments"):
            for _, (rtype, target, _mode) in sorted(rels.items(), key=lambda kv: kv[1][1]):
                if rtype == REL + kind:
                    out.append(Story(posixpath.splitext(posixpath.basename(target))[0], target))
        return CallableList(out)

    def _part_of(self, story: str) -> str:
        for found in self.stories:
            if found.name == story:
                return found.part
        raise KeyError(f"no story {story!r}")

    def _parents(self, story: str) -> dict:
        if story not in self._parent_maps:
            self._parent_maps[story] = _parents(self._tree(self._part_of(story)))
        return self._parent_maps[story]

    def paragraphs(self, story: str = "body") -> list[Paragraph]:
        """A story's paragraphs in order, those in its tables and controls too (not a text
        box's)."""
        root = self._tree(self._part_of(story))
        parents = self._parents(story)
        out = []
        for element in root.iter(_W + "p"):
            if any(a.tag == _W + "txbxContent" for a in _ancestors(element, parents)):
                continue
            out.append(Paragraph(self, element, story, len(out)))
        return out

    def tables(self, story: str = "body") -> list[Table]:
        root = self._tree(self._part_of(story))
        parents = self._parents(story)
        return [Table(self, t, story) for t in root.iter(_W + "tbl")
                if not any(a.tag in (_W + "tbl", _W + "txbxContent") for a in _ancestors(t, parents))]

    # -- styles and lists

    @cached_property
    def styles(self) -> Styles:
        tree = self._tree("word/styles.xml")
        return Styles(Style(s) for s in (tree.iter(_W + "style") if tree is not None else []))

    def _style(self, style_id: str | None, kind: str) -> Style | None:
        if style_id is None:
            return next((s for s in self.styles if s.type == kind and s._element.get(_W + "default") in ("1", "true")),
                        None)
        return next((s for s in self.styles if s.id == style_id and s.type == kind), None)

    def _style_name(self, style_id: str | None, kind: str) -> str | None:
        style = self._style(style_id, kind)
        return style.name if style is not None else style_id

    def _list_of(self, element: ET.Element, style_id: str | None) -> ListInfo | None:
        num = element.find(f"{_W}pPr/{_W}numPr")
        level = 0
        num_id = None
        if num is not None and num.find(_W + "numId") is not None:
            num_id = num.find(_W + "numId").get(_W + "val")
            ilvl = num.find(_W + "ilvl")
            level = int(ilvl.get(_W + "val")) if ilvl is not None else 0
        seen = set()
        style = self._style(style_id, "paragraph")
        while num_id is None and style is not None and style.id not in seen:
            seen.add(style.id)
            found = style._element.find(f"{_W}pPr/{_W}numPr")
            if found is not None and found.find(_W + "numId") is not None:
                num_id = found.find(_W + "numId").get(_W + "val")
                ilvl = found.find(_W + "ilvl")
                level = int(ilvl.get(_W + "val")) if ilvl is not None else 0
            based = style._element.find(_W + "basedOn")
            style = self._style(based.get(_W + "val"), "paragraph") if based is not None else None
        if num_id in (None, "0"):
            return None
        return ListInfo(num_id, level, self._number_format(num_id, level))

    def _number_format(self, num_id: str, level: int) -> str | None:
        tree = self._tree("word/numbering.xml")
        if tree is None:
            return None
        num = next((n for n in tree.iter(_W + "num") if n.get(_W + "numId") == num_id), None)
        if num is None:
            return None
        for override in num.iter(_W + "lvlOverride"):
            lvl = override.find(_W + "lvl")
            if override.get(_W + "ilvl") == str(level) and lvl is not None and lvl.find(_W + "numFmt") is not None:
                return lvl.find(_W + "numFmt").get(_W + "val")
        abstract = num.find(_W + "abstractNumId").get(_W + "val")
        for definition in tree.iter(_W + "abstractNum"):
            if definition.get(_W + "abstractNumId") == abstract:
                for lvl in definition.iter(_W + "lvl"):
                    if lvl.get(_W + "ilvl") == str(level):
                        fmt = lvl.find(_W + "numFmt")
                        return fmt.get(_W + "val") if fmt is not None else None
        return None

    # -- tracked changes and comments

    def revisions(self, author: str | None = None) -> list[Revision]:
        """Every tracked-change record of every story but the comments, in order."""
        out = []
        for story in self.stories:
            if story.name.startswith("comments"):
                continue
            root, parents = self._tree(story.part), self._parents(story.name)
            index = {p: n for n, p in enumerate(root.iter(_W + "p"))}
            for element in root.iter():
                kind = self._kind(element, parents)
                if kind is None:
                    continue
                holder = element if element.tag == _W + "p" else next(
                    (a for a in _ancestors(element, parents) if a.tag == _W + "p"), None)
                if holder is None:
                    para = None
                else:
                    own = holder.get(W14 + "paraId")
                    para = f"p:{own}" if own else f"p@{story.name}/{index[holder]}"
                    para = para if story.name == "body" else f"{story.name}/{para}"
                deleted = element.tag in (_W + "del", _W + "moveFrom")
                text = _text(element, parents, deleted=deleted) if kind in (
                    "insertion", "deletion", "move-from", "move-to") else ""
                out.append(Revision(kind, element, story.name, para, text))
        return [r for r in out if author is None or r.author == author]

    @staticmethod
    def _kind(element: ET.Element, parents: dict) -> str | None:
        tag, parent = element.tag, parents.get(element)
        where = parent.tag if parent is not None else None
        if tag in (_W + "ins", _W + "del"):
            word = "insertion" if tag == _W + "ins" else "deletion"
            if where == _W + "rPr" and parents.get(parent) is not None and parents[parent].tag == _W + "pPr":
                return f"paragraph-{word}"
            if where == _W + "trPr":
                return f"row-{word}"
            if where == _W + "rPr":
                return f"run-mark-{word}"
            return word
        return {_W + "moveFrom": "move-from", _W + "moveTo": "move-to", _W + "rPrChange": "formatting",
                _W + "pPrChange": "paragraph-formatting", _W + "tblPrChange": "table-formatting",
                _W + "trPrChange": "row-formatting", _W + "tcPrChange": "cell-formatting",
                _W + "sectPrChange": "section-formatting", _W + "cellIns": "cell-insertion",
                _W + "cellDel": "cell-deletion", _W + "cellMerge": "cell-merge",
                _W + "numberingChange": "numbering"}.get(tag)

    def changes(self) -> list[Revision]:
        return self.revisions()

    def comments(self, replies: bool = True) -> list[Comment]:
        """The comments; ``replies=False``: the threads' first comments, each with its
        replies."""
        tree = self._tree("word/comments.xml")
        if tree is None:
            return []
        parents = _parents(tree)
        found = [Comment(c, parents) for c in tree.iter(_W + "comment")]
        extended = self._tree("word/commentsExtended.xml")
        by_para = {c.para_id: c for c in found if c.para_id}
        for entry in extended.iter(W15 + "commentEx") if extended is not None else []:
            comment = by_para.get(entry.get(W15 + "paraId"))
            if comment is None:
                continue
            comment.done = entry.get(W15 + "done") in ("1", "true")
            comment.parent = entry.get(W15 + "paraIdParent")
        for comment in found:
            if comment.parent in by_para:
                by_para[comment.parent].replies.append(comment)
        return found if replies else [c for c in found if c.parent not in by_para]

    # -- fields, links, controls, drawings

    def fields(self) -> list[Field]:
        """Every field of every story but the comments: complex fields (nested ones too) and
        simple ones, each with its instruction, its result's text and its paragraph."""
        out: list[Field] = []
        for story in self.stories:
            if story.name.startswith("comments"):
                continue
            out += self._fields_of(story)
        return out

    def _fields_of(self, story: Story) -> list[Field]:
        root, parents = self._tree(story.part), self._parents(story.name)
        paragraphs = {p: Paragraph(self, p, story.name, n) for n, p in enumerate(root.iter(_W + "p"))}
        out: list[Field] = []
        stack: list[dict] = []
        last_paragraph = None
        for node in root.iter():
            if node.tag == _W + "p":
                if stack and last_paragraph is not None:
                    for open_field in stack:
                        if open_field["state"] == "result":
                            open_field["result"].append("\n")
                last_paragraph = node
                continue
            if node.tag == _W + "fldSimple":
                holder = next((a for a in _ancestors(node, parents) if a.tag == _W + "p"), None)
                out.append(Field(node.get(_W + "instr") or "", _text(node, parents),
                                 paragraphs[holder].id if holder is not None else None))
                continue
            if node.tag == _W + "fldChar":
                kind = node.get(_W + "fldCharType")
                if kind == "begin":
                    holder = next((a for a in _ancestors(node, parents) if a.tag == _W + "p"), None)
                    stack.append({"instr": [], "result": [], "state": "instr",
                                  "para": paragraphs[holder].id if holder is not None else None})
                elif kind == "separate" and stack:
                    stack[-1]["state"] = "result"
                elif kind == "end" and stack:
                    done = stack.pop()
                    out.append(Field("".join(done["instr"]), "".join(done["result"]).strip("\n"), done["para"]))
                continue
            if not stack:
                continue
            gone = any(a.tag in (_W + "del", _W + "moveFrom") for a in _ancestors(node, parents))
            if node.tag == _W + "instrText" and stack[-1]["state"] == "instr":
                stack[-1]["instr"].append(node.text or "")
            elif node.tag in (_W + "t", _W + "tab") and not gone:
                piece = (node.text or "") if node.tag == _W + "t" else "\t"
                for open_field in stack:
                    if open_field["state"] == "result":
                        open_field["result"].append(piece)
        return out

    def hyperlinks(self) -> list[Hyperlink]:
        rels = self._rels(MAIN)
        parents = self._parents("body")
        out = []
        for link in self._tree(MAIN).iter(_W + "hyperlink"):
            rid = link.get(R + "id")
            target = rels[rid][1] if rid in rels else ("#" + link.get(_W + "anchor", ""))
            out.append(Hyperlink(target, _text(link, parents)))
        return out

    def content_controls(self) -> list[Control]:
        parents = self._parents("body")
        return [Control(sdt, parents) for sdt in self._tree(MAIN).iter(_W + "sdt")]

    def pictures(self) -> list[Picture]:
        return [Picture(d) for d in self._tree(MAIN).iter(_W + "drawing") if next(d.iter(PIC + "pic"), None) is not None]

    def drawing(self, drawing: str) -> Drawing | None:
        number = drawing.partition(":")[2]
        for element in self._tree(MAIN).iter(_W + "drawing"):
            doc_pr = next(element.iter(WP + "docPr"), None)
            if doc_pr is not None and doc_pr.get("id") == number:
                return Drawing(element)
        return None

    def notes(self) -> list[Note]:
        out = []
        for kind, part, tag in (("fn", "word/footnotes.xml", "footnote"), ("en", "word/endnotes.xml", "endnote")):
            tree = self._tree(part)
            if tree is None:
                continue
            parents = _parents(tree)
            out += [Note(kind, n, parents) for n in tree.iter(_W + tag)
                    if n.get(_W + "type") in (None, "normal")]
        return out

    def sections(self) -> list[Section]:
        return [Section(s) for s in self._tree(MAIN).iter(_W + "sectPr")]

    @cached_property
    def properties(self) -> dict[str, Any]:
        tree = self._tree("docProps/core.xml")
        if tree is None:
            return {}
        out = {}
        for name, tag in (("title", DC + "title"), ("author", DC + "creator"), ("subject", DC + "subject")):
            found = tree.find(tag)
            if found is not None:
                out[name] = found.text or ""
        return out

    @cached_property
    def compatibility_mode(self) -> int | None:
        tree = self._tree("word/settings.xml")
        for setting in tree.iter(_W + "compatSetting") if tree is not None else []:
            if setting.get(_W + "name") == "compatibilityMode":
                return int(setting.get(_W + "val"))
        return None

    # -- what the tools tell

    @property
    def _opened(self) -> Opened:
        if self._session is None:
            self._session = Opened(self.data)
        return self._session

    def validate(self) -> list[str]:
        """The document's validation problems, as ``check`` lists them."""
        return list(self._opened.call("check", include=["validate"]).data["validate"]["problems"])

    def charts(self) -> list[Chart]:
        """The body's charts, in order, as ``edit_chart`` reads them."""
        out = []
        rels = self._rels(MAIN)
        for drawing in self._tree(MAIN).iter(_W + "drawing"):
            chart = next(drawing.iter(C + "chart"), None)
            if chart is None:
                continue
            doc_pr = next(drawing.iter(WP + "docPr"))
            part = rels[chart.get(R + "id")][1]
            book = None
            tree = self._tree(part)
            external = tree.find(C + "externalData") if tree is not None else None
            if external is not None:
                book = self._rels(part).get(external.get(R + "id"), (None, None))[1]
            out.append(Chart(self, f"d:{doc_pr.get('id')}", book))
        return out

    def to_markdown(self, range: str | None = None, *, ids: bool = True, view: str | None = None) -> str:
        """``word_read``'s Markdown (of ``range``; ``ids=False``: without the id comments)."""
        arguments: dict[str, Any] = {}
        if range is not None:
            arguments["range"] = range
        if view is not None:
            arguments["view"] = view
        pages, cursor = [], None
        while True:
            result = self._opened.call("word_read", **arguments, **({"cursor": cursor} if cursor else {}))
            pages.append(result.data)
            cursor = result.next_cursor
            if not cursor:
                break
        text = "".join(pages)
        if not ids:
            text = re.sub(r"[ \t]*<!-- (?:p|t|cc|d)[:@][^>]*-->[ \t]*\n?", "", text)
        return text

    def layout(self) -> Layout:
        return Layout(self)

    def state(self, block: str) -> dict:
        """A block's formatting as ``word_inspect`` gives it: ``{"blocks": [...]}``."""
        data = self._opened.call("word_inspect", target=block).data
        return data if "blocks" in data else {"blocks": [data]}

    def _changed(self, action: str, **arguments: Any) -> None:
        self._opened.call("word_changes", action=action, **arguments)
        self.__init__(self._opened.data())

    def accept_all(self) -> None:
        self._changed("accept", all=True)

    def reject_all(self) -> None:
        self._changed("reject", all=True)

    def accept(self, *, author: str) -> None:
        self._changed("accept", author=author)

    def reject(self, *, author: str) -> None:
        self._changed("reject", author=author)
