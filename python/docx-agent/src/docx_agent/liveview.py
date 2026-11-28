"""An application's live view of a document the Rust host holds (round 35, R27; lane LV3).

A page that shows a document as it is written -- each page as SVG, redrawn when an edit
changes it -- asks the session for the pages and for the ones an edit changed::

    from docx_agent.liveview import page_svg, view

    shown = view(session, "d1")                          # first call: every page changed
    for number in shown.changed:
        drawn = page_svg(session, "d1", number, shown.token)
        put(number, drawn.svg, drawn.images)             # pictures by name: img/3fa1....png
    ...
    shown = view(session, "d1", since=shown.token)       # after edits: the pages they changed

The changes feed sends less (phase 3 step CF): only the groups that changed on each page --
a paragraph, a table row's rules, a drawing -- each in its own coordinates, an offset alone
for a group that only moved, pictures by content address, once.  A :class:`FeedClient` keeps
the latest drawing of every page and applies each answer::

    from docx_agent.liveview import FeedClient

    client = FeedClient()
    client.update(session, "d1")                         # first call: every page
    put_all(client.page_count, client.svg)               # client.svg(page): the page as SVG
    ...
    answer = client.update(session, "d1")                # after edits: what changed since
    for number in client.changed(answer):
        put(number, client.svg(number))

docs/liveview.md is the client protocol: how an answer's ``list``, ``groups``, ``moved``,
``removed``, ``defs`` and ``order`` apply, and what a stale ``since`` and a full answer are.

The work is the Rust host's (``HostedSession.live_view`` and ``page_svg``, ooxml-edit's): the version is
laid out by the document's page engine, the last versions views were given are kept within
the session's memory budget, and a token names one (``<session>-<doc>-<state number>``).
This module finds the document's session and engine, and gives pictures back by name.
A document outside a session has no live view: open it in one.
"""

from __future__ import annotations

import base64
import hashlib
import re
from dataclasses import dataclass, field
from typing import Any, Mapping, MutableMapping

from ooxml_edit.tools import hosted as _hosted

__all__ = ["FeedClient", "PageSvg", "UnknownToken", "View", "ViewPage", "changes", "embed_images",
           "embed_pictures", "externalise_images", "page_svg", "view"]

#: :func:`page_svg`'s ``images``.
IMAGE_MODES = ("external", "embedded")


@dataclass(frozen=True)
class ViewPage:
    """One page: its 1-based ``number``, the ids of the body's top-level blocks it draws
    something of, in document order, and its size in points."""

    number: int
    blocks: tuple[str, ...]
    width: float
    height: float


@dataclass(frozen=True)
class View:
    """:func:`view`'s answer: this version's ``token`` (``since`` for the next view, and
    :func:`page_svg`'s), every page, the 1-based pages ``changed`` since ``since`` (every page
    without one, or with a ``stale`` one: ``note`` says why), and the state number laid out."""

    token: str
    pages: list[ViewPage]
    changed: list[int]
    since: str | None = None
    stale: bool = False
    note: str | None = None
    version: int | None = None

    @property
    def page_count(self) -> int:
        return len(self.pages)


@dataclass(frozen=True)
class PageSvg:
    """:func:`page_svg`'s answer: the page's SVG, the pictures it references by name (with
    ``images="external"``), and what was not drawn as Word draws it."""

    page: int
    token: str
    svg: str
    images: dict[str, bytes] = field(default_factory=dict)
    warnings: list[str] = field(default_factory=list)


#: A token :func:`page_svg` was given that names no version kept for the document (a
#: ``LookupError``).
UnknownToken = _hosted.UnknownToken


def view(where: Any, *args: str, since: str | None = None) -> View:
    """``view(session, doc)``: the document's pages now, which changed since ``since``, and a
    new token.  Raises the session's refusal (a ``ToolError``) for a version it does not lay
    out yet, ``TimeoutError`` past the session's limit on laying out."""
    session, doc, _ = _target(where, args, "view", 0)
    got = session.live_view(doc, since)
    pages = [ViewPage(number, tuple(blocks), width, height) for number, blocks, width, height in got["pages"]]
    return View(got["token"], pages, list(got["changed"]), got["since"], got["stale"], got["note"],
                got["version"])


def page_svg(where: Any, *args: Any, text: str = "text", images: str = "external",
             glyphs: dict | None = None) -> PageSvg:
    """``page_svg(session, doc, page, token)``: one page of the version ``token`` names, as
    SVG, even after the document moved on; its paragraph groups carry ``data-docx-agent-id``
    (that version's ids).  ``text``: ``"text"`` (``"outlines"`` is refused, the way out
    ``"text"``); ``images``: ``"external"`` -- each picture referenced by name, its bytes in
    the answer's ``images`` (:func:`externalise_images`) -- or ``"embedded"``.  Raises
    :class:`UnknownToken` for a token no view of the document gave or kept, ``IndexError``
    for a page the version does not have, ``ValueError`` for a mode there is none of."""
    if images not in IMAGE_MODES:
        raise ValueError(f"images must be one of {', '.join(IMAGE_MODES)}, not {images!r}")
    if glyphs is not None and text != "outlines":
        raise ValueError("glyphs is the glyph set of outlined pages: give text=\"outlines\" with it")
    session, doc, (page, token) = _target(where, args, "page_svg", 2)
    svg, warnings = session.page_svg(doc, page, token, text=text)
    store: dict[str, bytes] = {}
    if images == "external":
        svg = externalise_images(svg, store)
    return PageSvg(page, token, svg, store, list(warnings))


def changes(where: Any, *args: str, since: str | None = None, pages: bool = True) -> dict[str, Any]:
    """``changes(session, doc)``: what changed in the document after ``since`` (a ``token`` an
    answer gave; ``None``: everything) -- the changes feed's answer as
    ``HostedSession.changes`` gives it (docs/liveview.md).  ``pages=False`` never lays out:
    the version, parts and blocks only.  Raises the session's refusal (a ``ToolError``) for a
    version it does not lay out yet, ``TimeoutError`` past the session's limit on laying out."""
    session, doc, _ = _target(where, args, "changes", 0)
    return session.changes(doc, since, pages=pages)


def _quote(text: str) -> str:
    """``text`` as an attribute value, quoted as the Rust writer quotes it (``exact::quote``)."""
    escaped = text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    escaped = escaped.replace("\n", "&#10;").replace("\r", "&#13;").replace("\t", "&#9;")
    if '"' in escaped:
        if "'" in escaped:
            return '"' + escaped.replace('"', "&quot;") + '"'
        return "'" + escaped + "'"
    return '"' + escaped + '"'


@dataclass
class _Group:
    dy: int
    svg: str
    defs: str = ""


@dataclass
class FeedClient:
    """The latest drawing of every page of one document, kept by applying the changes feed's
    answers in order (docs/liveview.md): ``token`` (the next ``since``), each page's
    ``frames`` (``width``, ``height`` in points, ``width_px``, ``height_px``), its groups'
    order and content, and the ``pictures`` sent, by content address (``(mime, bytes)``)."""

    token: str | None = None
    frames: list[dict[str, Any]] = field(default_factory=list)
    order: list[list[str]] = field(default_factory=list)
    groups: list[dict[str, _Group]] = field(default_factory=list)
    pictures: dict[str, tuple[str, bytes]] = field(default_factory=dict)

    @property
    def page_count(self) -> int:
        return len(self.frames)

    def update(self, where: Any, *args: str) -> dict[str, Any]:
        """``update(session, doc)``: the answer since this client's ``token``, applied."""
        answer = changes(where, *args, since=self.token)
        self.apply(answer)
        return answer

    def apply(self, answer: Mapping[str, Any]) -> None:
        """Apply one answer (with pages): a full one replaces every page; otherwise the page
        list where it is given, then each changed page's removals, groups, offsets and order."""
        pages = answer.get("pages")
        if pages is None:
            raise ValueError("this answer has no pages: ask the changes feed with pages=True")
        if answer.get("full"):
            self.order, self.groups = [], []
        listed = pages.get("list")
        if listed is not None:
            self.frames = [dict(frame) for frame in listed]
            del self.order[len(listed):], self.groups[len(listed):]
            while len(self.order) < len(listed):
                self.order.append([])
                self.groups.append({})
        for change in pages["changed"]:
            k = change["page"] - 1
            groups = self.groups[k]
            for gone in change["removed"]:
                groups.pop(gone, None)
            defs = change["defs"]
            for id_, group in change["groups"].items():
                groups[id_] = _Group(group["dy"], group["svg"], defs.get(id_, ""))
            for id_, dy in change["moved"].items():
                groups[id_].dy = dy
            if "order" in change:
                self.order[k] = list(change["order"])
        self.pictures.update(answer.get("pictures", {}))
        self.token = answer["token"]

    @staticmethod
    def changed(answer: Mapping[str, Any]) -> list[int]:
        """The 1-based pages an answer changed."""
        return [change["page"] for change in answer["pages"]["changed"]]

    def svg(self, page: int, *, embed: bool = False) -> str:
        """Page ``page`` (1-based) as one SVG document, byte for byte what the feed's writer
        writes for it (``feed_svg``); pictures referenced ``href="picture:<hash>"``, or
        embedded with ``embed`` (:func:`embed_pictures`)."""
        k = page - 1
        frame = self.frames[k]
        out = [f'<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="{frame["width"]}" '
               f'height="{frame["height"]}" viewBox="0 0 {frame["width_px"]} {frame["height_px"]}">\n'
               f'<rect width="{frame["width_px"]}" height="{frame["height_px"]}" fill="#FFFFFF"/>']
        for id_ in self.order[k]:
            group = self.groups[k][id_]
            out.append(f'\n<g data-feed-group={_quote(id_)} transform="translate(0,{group.dy})">')
            if group.defs:
                out.append(f"\n<defs>{group.defs}</defs>")
            out.append(group.svg)
            out.append("\n</g>")
        out.append("\n</svg>\n")
        svg = "".join(out)
        return embed_pictures(svg, self.pictures) if embed else svg


#: A picture the changes feed draws by its content address.
_PICTURE_HREF = re.compile(r'href="picture:([0-9a-f]{32})"')


def embed_pictures(svg: str, pictures: Mapping[str, tuple[str, bytes]]) -> str:
    """``svg`` (a :class:`FeedClient` page's) with every picture it references by content
    address embedded as a ``data:`` URI; an address ``pictures`` lacks raises ``KeyError``."""

    def replace(match: re.Match) -> str:
        mime, data = pictures[match.group(1)]
        return f'href="data:{mime};base64,{base64.b64encode(data).decode("ascii")}"'

    return _PICTURE_HREF.sub(replace, svg)


def _target(where: Any, args: tuple, name: str, rest: int) -> tuple[Any, str, tuple]:
    """The session on the Rust host holding the Word document ``(session, doc, ...)`` names,
    that document, and the ``rest`` arguments after ``doc``."""
    tail = ", page, token" if rest else ""
    if not callable(getattr(where, "entry", None)) or len(args) != rest + 1:
        raise TypeError(f"{name} takes (session, doc{tail}): a document's live view is its session's -- "
                        f"open it in one (doc = session.open(data, name)) and call {name}(session, doc{tail})")
    if getattr(where, "live_view", None) is None or where.kind_of(args[0]) != "docx":
        raise TypeError(f"{args[0]} is not a Word document the session's Rust host holds; {name} draws "
                        "the pages of one")
    return where, args[0], tuple(args[1:])


# -- pictures by name (the names and forms pptx2svg.liveview gives) -------------------------------

#: A picture as docx2svg embeds it: ``href`` or ``xlink:href``, a media type, base64.
_DATA_HREF = re.compile(r'((?:xlink:)?href)="data:([^";,]+);base64,([A-Za-z0-9+/=]*)"')
_SNIFF = ((b"\x89PNG\r\n\x1a\n", "png"), (b"\xff\xd8\xff", "jpg"), (b"GIF87a", "gif"), (b"GIF89a", "gif"),
          (b"BM", "bmp"), (b"II*\x00", "tif"), (b"MM\x00*", "tif"))
_MIME = {"png": "image/png", "jpg": "image/jpeg", "gif": "image/gif", "bmp": "image/bmp", "tif": "image/tiff",
         "webp": "image/webp", "svg": "image/svg+xml"}
_EXTENSION = {mime: extension for extension, mime in _MIME.items()}
_NAMED_HREF = re.compile(r'((?:xlink:)?href)="(?!data:|#)([^"]*)"')
_IMAGE_BY_NAME = re.compile(r'<image\b[^>]*?\bhref="(?!data:|#)([^"]+)"')


def _format(data: bytes, mime: str) -> str:
    for magic, extension in _SNIFF:
        if data.startswith(magic):
            return extension
    if data[:4] == b"RIFF" and data[8:12] == b"WEBP":
        return "webp"
    head = data[:512].lstrip()
    if head.startswith(b"<svg") or (head.startswith(b"<?xml") and b"<svg" in data[:4096]):
        return "svg"
    if mime in _EXTENSION:
        return _EXTENSION[mime]
    return re.sub(r"[^A-Za-z0-9.+-]", "_", mime.partition("/")[2] or "bin")


def externalise_images(svg: str, store: MutableMapping[str, bytes], prefix: str = "img/") -> str:
    """``svg`` with every embedded picture (``href="data:...;base64,..."``) referenced by
    name instead, its bytes put in ``store`` under that name: ``prefix``, the first 32 hex
    digits of the bytes' SHA-256, ``.`` and the format the bytes are (else the media type's
    subtype) -- the names pptx2svg's ``liveview.externalise_images`` gives, so one picture
    on several pages, or in a deck and a document, is one entry."""
    if "base64," not in svg:
        return svg

    def replace(match: re.Match) -> str:
        attribute, mime, encoded = match.groups()
        data = base64.b64decode(encoded)
        name = f"{prefix}{hashlib.sha256(data).hexdigest()[:32]}.{_format(data, mime)}"
        store.setdefault(name, data)
        return f'{attribute}="{name}"'

    return _DATA_HREF.sub(replace, svg)


def embed_images(svg: str, images: Mapping[str, bytes]) -> str:
    """``svg`` with every picture :func:`externalise_images` named embedded again; a name
    ``images`` lacks raises ``KeyError`` naming it."""

    def replace(match: re.Match) -> str:
        attribute, name = match.groups()
        data = images.get(name)
        if data is None:
            return match.group(0)
        extension = name.rpartition(".")[2]
        return f'{attribute}="data:{_MIME.get(extension, "image/" + extension)};base64,' \
               f'{base64.b64encode(data).decode("ascii")}"'

    embedded = _NAMED_HREF.sub(replace, svg) if images else svg
    left = _IMAGE_BY_NAME.search(embedded)
    if left is not None:
        raise KeyError(f"the SVG references the picture {left.group(1)!r} and images has no such entry; pass "
                       "the images page_svg gave")
    return embedded
