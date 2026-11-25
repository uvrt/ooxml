"""What a document says, read the same way for the tools' output and for Word's: the facts a
re-save must keep, the Markdown of its final view, where its paragraphs are laid out, and its
field results.  Word's answers are documents (or a PDF); each is read here through the tools,
so both sides of a comparison pass through one reader.

Must not: import pytest or docx-agent's library (only its tool host, ``docx_agent.tools``), or
drive Word (``word_side.py`` does).
"""

from __future__ import annotations

import io
import re
import zipfile
from typing import Any

from compare import KEYWORD_ONLY_FIELDS, PAGE_NUMBER_FIELDS

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
W14 = "http://schemas.microsoft.com/office/word/2010/wordml"
_W = "{%s}" % W

#: The elements that record a tracked change (each carries its author and date).
REVISION_MARKS = ("ins", "del", "moveFrom", "moveTo", "rPrChange", "pPrChange", "tblPrChange", "trPrChange",
                  "tcPrChange", "sectPrChange", "numberingChange", "cellIns", "cellDel", "cellMerge")
#: The parts whose paragraphs, revisions and comments a re-save must keep.
_STORIES = re.compile(r"^word/(document|header\d*|footer\d*|footnotes|endnotes|comments)\.xml$")
#: Fields whose result is the clock's: Word's update gives today's, the tools' the session's.
VOLATILE_FIELDS = frozenset({"DATE", "TIME", "PRINTDATE", "SAVEDATE", "EDITTIME"})
#: A block's id comment in ``word_read``'s Markdown.
_ID_COMMENT = re.compile(r"[ \t]*<!-- (?:p|t)(?::[0-9A-Fa-f]+|@[^\s>]+)(?: [^>]*?)? -->[ \t]*\n?")
#: A table cell's or table's id in ``word_read``'s HTML form of a table.
_DATA_ID = re.compile(r' data-id="[^"]*"')


def _parts(data: bytes) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return {name: archive.read(name) for name in archive.namelist() if _STORIES.match(name)}


def _margin_comments(parts: dict[str, bytes]) -> set[str]:
    """The ``w:id`` of every comment marked in a header or footer part."""
    out: set[str] = set()
    for name, body in parts.items():
        if re.match(r"word/(header|footer)\d*\.xml$", name):
            found = re.findall(rb'<w:comment(?:RangeStart|RangeEnd|Reference) [^>]*?w:id="(\d+)"', body)
            out |= {k.decode() for k in found}
    return out


def _text(element: Any, tags: tuple[str, ...]) -> str:
    return "".join(node.text or "" for node in element.iter(*(_W + t for t in tags)))


def _field_results(root: Any, keywords: frozenset[str]) -> dict[Any, tuple[int, str]]:
    """Each ``w:t`` in the result of a field with one of these keywords, in a story: the field's
    occurrence (counted in document order) and keyword.  Simple fields and complex ones (which
    may span paragraphs and nest) alike; the innermost field a text is the result of counts."""
    out: dict[Any, tuple[int, str]] = {}
    stack: list[list[Any]] = []          # [occurrence, instruction, in its result]
    count = 0
    for node in root.iter(_W + "fldSimple", _W + "fldChar", _W + "instrText", _W + "t"):
        tag = node.tag
        if tag == _W + "fldSimple":
            count += 1
            keyword = (node.get(_W + "instr") or "").split(None, 1)[0:1]
            if keyword and keyword[0].upper() in keywords:
                for text in node.iter(_W + "t"):
                    out.setdefault(text, (count, keyword[0].upper()))
        elif tag == _W + "fldChar":
            kind = node.get(_W + "fldCharType")
            if kind == "begin":
                count += 1
                stack.append([count, "", False])
            elif kind == "separate" and stack:
                stack[-1][2] = True
            elif kind == "end" and stack:
                stack.pop()
        elif tag == _W + "instrText":
            if stack and not stack[-1][2]:
                stack[-1][1] += node.text or ""
        elif stack and stack[-1][2] and node not in out:
            keyword = stack[-1][1].split(None, 1)[0:1]
            if keyword and keyword[0].upper() in keywords:
                out[node] = (stack[-1][0], keyword[0].upper())
    return out


def _paragraph_fields(paragraph: Any, results: dict[Any, tuple[int, str]]) -> list[list[str]]:
    """The ``[keyword, result]`` of each field (of :func:`_field_results`) with text in this
    paragraph, the result as the paragraph holds it."""
    found: dict[int, list[str]] = {}
    for text in paragraph.iter(_W + "t"):
        if text in results:
            occurrence, keyword = results[text]
            found.setdefault(occurrence, [keyword, ""])[1] += text.text or ""
    return [found[k] for k in sorted(found)]


def _page_numbers_masked(paragraph: Any, results: dict[Any, tuple[int, str]]) -> str | None:
    """The paragraph's text with the result of each page-number field (of :func:`_field_results`)
    written as ``{KEYWORD}``, once per field; ``None`` where it holds none."""
    pieces: list[str] = []
    last, masked = None, False
    for text in paragraph.iter(_W + "t"):
        hit = results.get(text)
        if hit is None:
            pieces.append(text.text or "")
            last = None
        elif hit[0] != last:
            pieces.append("{%s}" % hit[1])
            last, masked = hit[0], True
    return "".join(pieces) if masked else None


def has_markup(data: bytes) -> bool:
    """Whether the document has tracked changes or comments (Word exports its final view)."""
    for body in _parts(data).values():
        if any(f"<w:{mark} ".encode() in body for mark in REVISION_MARKS) or b"<w:commentReference" in body:
            return True
    return False


def kept_facts(data: bytes) -> dict[str, Any]:
    """What a re-save must keep, story by story: every paragraph's paraId and text (deleted text
    apart; the results of the fields ``compare`` takes by keyword only listed with it, as
    ``fields``, for ``compare.masked_text``; where it holds a page number, its text with each
    page number's result written ``{KEYWORD}`` as ``page_numbers``, for ``compare.kept`` where the
    faces are not installed), the kind, author and date of the revisions, every
    comment's author, initials, date and text.  Paragraphs without a paraId are listed by text alone (Word gives them one).

    Three allowances for how Word saves, each measured by the suite (Word 16.106 for Mac; the
    evidence is Word's re-save in ``observations/suite-word.json``):

    * Word joins neighbouring runs that carry the same change (``e1/format-tracked``: 11
      ``rPrChange`` records written, 9 after Word's save, every author and date kept), so the
      revisions are a set of (kind, author, date), not a count.
    * Word writes ``comments.xml`` in the order of the comments' anchors (``e3/comments``: the
      added comments come first after its save), so that story's paragraphs and comments are
      compared in their own order.
    * Word renews the paraIds of the notes' separator and continuation separator on every save
      (``e4/report``, ``e4/notes-tracked``), which are no story's content and which no tool
      addresses, so they are left out; and Word writes notes parts holding nothing but those to a
      document that had none (``trial/w1-template-report``), so a story with no paragraph left is
      no story.

    And one rule where the tools part from Word on purpose: a comment marked in a header or
    footer is left out, on both sides.  Word drops such a comment on its own save
    (``trial/w2-review-comments``: the input's comment in its header gone after Word's re-save,
    every other comment kept); the tools never write one (``word_comments`` refuses the item,
    with the body as its way out) and never delete what the file already holds."""
    from lxml import etree

    parts = _parts(data)
    dropped = _margin_comments(parts)
    out: dict[str, Any] = {}
    for name, body in sorted(parts.items()):
        root = etree.fromstring(body)
        if name == "word/comments.xml":
            for comment in [c for c in root.iter(_W + "comment") if c.get(_W + "id") in dropped]:
                comment.getparent().remove(comment)
        story = name.removeprefix("word/").removesuffix(".xml")
        results = _field_results(root, KEYWORD_ONLY_FIELDS)
        page_numbers = _field_results(root, PAGE_NUMBER_FIELDS)
        paragraphs = []
        for paragraph in root.iter(_W + "p"):
            note = next((a for a in paragraph.iterancestors(_W + "footnote", _W + "endnote")), None)
            if note is not None and note.get(_W + "type") in ("separator", "continuationSeparator",
                                                              "continuationNotice"):
                continue
            entry = {"id": paragraph.get("{%s}paraId" % W14), "text": _text(paragraph, ("t",))}
            fields = _paragraph_fields(paragraph, results)
            if fields:
                entry["fields"] = fields
            masked = _page_numbers_masked(paragraph, page_numbers)
            if masked is not None:
                entry["page_numbers"] = masked
            deleted = _text(paragraph, ("delText",))
            if deleted:
                entry["deleted"] = deleted
            paragraphs.append(entry)
        revisions = sorted({(etree.QName(e).localname, e.get(_W + "author"), e.get(_W + "date"))
                            for e in root.iter(*(_W + m for m in REVISION_MARKS))})
        facts: dict[str, Any] = {"paragraphs": paragraphs}
        if revisions:
            facts["revisions"] = [list(r) for r in revisions]
        if story == "comments":
            facts["paragraphs"] = sorted(paragraphs, key=lambda e: (e["id"] or "", e["text"]))
            facts["comments"] = sorted([c.get(_W + "author"), c.get(_W + "initials"), c.get(_W + "date"),
                                        _text(c, ("t",))] for c in root.iter(_W + "comment"))
        if paragraphs or revisions:
            out[story] = facts
    return out


class Reader:
    """A tool host's reads of a document, as the comparisons need them."""

    def __init__(self, box: Any) -> None:
        self.box = box

    def _open(self, data: bytes) -> Any:
        session = self.box.session()
        session.open(data, "answer.docx")
        return session

    def _call(self, session: Any, tool: str, **arguments: Any) -> Any:
        result = self.box.dispatch(session, tool, {"doc": "d1", **arguments})
        if not result.ok:
            raise ToolsRefused(f"{tool} {arguments}: {result.to_json().get('error')}")
        return result

    def _markdown(self, session: Any) -> str:
        out, cursor = [], None
        while True:
            result = self._call(session, "word_read", view="final", stories="all",
                                **({"cursor": cursor} if cursor else {}))
            out.append(result.data)
            cursor = result.next_cursor
            if not cursor:
                return _DATA_ID.sub("", _ID_COMMENT.sub("", "".join(out))).strip()

    def reviewed(self, data: bytes, action: str) -> bytes:
        """The tools' Accept All (``action`` "accept") or Reject All ("reject") of ``data``."""
        session = self._open(data)
        if has_markup(data):
            self._call(session, "word_changes", action=action, all=True)
        return session.entry("d1").snapshot()

    def markdown(self, data: bytes) -> str:
        """The final view's Markdown, every story, block ids left out: a re-save's ids are
        compared by :func:`kept_facts`, and Word's own Accept All gives a merged cell's first
        paragraph a new paraId (``e3/accepted-tables``), which is Word's review, not the tools'."""
        return self._markdown(self._open(data))

    def placements(self, data: bytes) -> dict[str, Any]:
        """The page count and each body paragraph's first page (1-based) with its text, as the
        tools lay the document out."""
        session = self._open(data)
        pages = self._call(session, "describe").data.get("pages")
        found = self._call(session, "word_inspect", target="body", layout=True).data
        paragraphs = []
        for block in found.get("blocks", []):
            places = block.get("placements") or []
            text = block.get("t") or ""
            if block.get("type") == "paragraph" and places and text.strip():
                paragraphs.append([places[0]["page"], text])
        return {"pages": pages, "paragraphs": paragraphs}

    def fields(self, data: bytes, *, update: bool) -> list[list[str]]:
        """Every field's keyword and result (whitespace collapsed), after the tools' update of
        all of them when ``update``; the clock's fields left out."""
        session = self._open(data)
        if update:
            self._call(session, "word_fields", action="update")
        out = self._call(session, "word_fields", action="list").data
        return [[f["keyword"], " ".join((f.get("result") or "").split())] for f in out
                if f["keyword"] not in VOLATILE_FIELDS]


class ToolsRefused(RuntimeError):
    """The tools refused a call a check needs to read their output (``word_inspect``'s layout of
    a text box it cannot lay out yet, say): their answer is not there to compare."""


def answer(reader: Reader, data: bytes, check: str) -> Any:
    """:func:`ours`, or the :class:`ToolsRefused` that stood in its way (a difference from
    Word's answer, ``compare.judge``)."""
    try:
        return ours(reader, data, check)
    except ToolsRefused as refused:
        return refused


#: The checks, cheapest first, each the tools' answer and Word's compared (``compare.judge``).
CHECKS = ("kept", "accept", "reject", "pages", "fields")


def ours(reader: Reader, data: bytes, check: str) -> Any:
    """The tools' answer to ``check`` about their output ``data``: the facts it keeps, its
    final view after the tools' Accept All or Reject All, where the tools lay out its accepted
    form, its field results after the tools' update."""
    if check == "kept":
        return kept_facts(data)
    if check in ("accept", "reject"):
        return reader.markdown(reader.reviewed(data, check))
    if check == "pages":
        return reader.placements(reader.reviewed(data, "accept"))
    if check == "fields":
        return reader.fields(data, update=True)
    raise ValueError(check)
