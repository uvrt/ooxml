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


def has_markup(data: bytes) -> bool:
    """Whether the document has tracked changes or comments (Word exports its final view)."""
    for body in _parts(data).values():
        if any(f"<w:{mark} ".encode() in body for mark in REVISION_MARKS) or b"<w:commentReference" in body:
            return True
    return False


def kept_facts(data: bytes) -> dict[str, Any]:
    """What a re-save must keep, story by story: every paragraph's paraId and text (deleted text
    apart), the kind, author and date of the revisions, every comment's author, initials, date and
    text.  Paragraphs without a paraId are listed by text alone (Word gives them one).

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
        paragraphs = []
        for paragraph in root.iter(_W + "p"):
            note = next((a for a in paragraph.iterancestors(_W + "footnote", _W + "endnote")), None)
            if note is not None and note.get(_W + "type") in ("separator", "continuationSeparator",
                                                              "continuationNotice"):
                continue
            entry = {"id": paragraph.get("{%s}paraId" % W14), "text": _text(paragraph, ("t",))}
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
            raise RuntimeError(f"{tool} {arguments}: {result.to_json().get('error')}")
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
