"""An application's live view of a document the Rust session holds (round 35, R27; lane LV3).

A page that shows a document as it is written -- each page as SVG, redrawn when an edit
changes it -- asks the session for the pages and for the ones an edit changed::

    from docx_agent.liveview import page_svg, view

    shown = view(session, "d1")                          # first call: every page changed
    for number in shown.changed:
        drawn = page_svg(session, "d1", number, shown.token)
        put(number, drawn.svg, drawn.images)             # pictures by name: img/3fa1....png
    ...
    shown = view(session, "d1", since=shown.token)       # after edits: the pages they changed

The work is the Rust session's (``ooxml_core.Session.view`` and ``page_svg``): the version is
laid out by the document's page engine, the last versions views were given are kept within
the session's memory budget, and a token names one (``<session>-<doc>-<state number>``).
This module finds the document's session and engine, and gives pictures back by name.
A document outside a session has no live view: open it in one.
"""

from __future__ import annotations

import base64
import hashlib
import json
import re
from contextlib import contextmanager
from dataclasses import dataclass, field
from typing import Any, Iterator, Mapping, MutableMapping

__all__ = ["PageSvg", "UnknownToken", "View", "ViewPage", "embed_images", "externalise_images",
           "page_svg", "view"]

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


class UnknownToken(LookupError):
    """A token :func:`page_svg` was given that names no version kept for the document."""


def view(where: Any, *args: str, since: str | None = None) -> View:
    """``view(session, doc)``: the document's pages now, which changed since ``since``, and a
    new token.  Raises the session's refusal (a ``ToolError``) for a version it does not lay
    out yet, ``TimeoutError`` past the session's limit on laying out."""
    session, doc, call, _ = _target(where, args, "view", 0)
    with _raised():
        got = session.view(doc, since, **call)
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
    session, doc, call, (page, token) = _target(where, args, "page_svg", 2)
    with _raised():
        svg, warnings = session.page_svg(doc, page, token, text=text, **call)
    store: dict[str, bytes] = {}
    if images == "external":
        svg = externalise_images(svg, store)
    return PageSvg(page, token, svg, store, list(warnings))


def _target(where: Any, args: tuple, name: str, rest: int) -> tuple[Any, str, dict[str, Any], tuple]:
    """The Rust session holding the document ``(session, doc, ...)`` names, its id there,
    the call's keywords (``layout``: the document's page engine; ``context``: the host's
    limit on laying out), and the ``rest`` arguments after ``doc``."""
    import ooxml_core

    from .rust.owner import context_of
    from .rust.reads import Mirror

    tail = ", page, token" if rest else ""
    if not callable(getattr(where, "entry", None)) or len(args) != rest + 1:
        raise TypeError(f"{name} takes (session, doc{tail}): a document's live view is its session's -- "
                        f"open it in one (doc = session.open(data, name)) and call {name}(session, doc{tail})")
    limits = getattr(where, "limits", None)
    context = context_of(None, limits, getattr(limits, "layout_timeout", None))
    host = getattr(where, "rust", None)     # a session on the Rust host (Phase 3 E1)
    if host is not None:
        pages = host.pages(args[0]) if where.kind_of(args[0]) == "docx" else None
        if pages is None:
            raise TypeError(f"{args[0]} is not a Word document the session's Rust core holds; {name} draws "
                            "the pages of one")
        return host, args[0], {"layout": ooxml_core.CallLayout(pages), "context": context}, tuple(args[1:])
    entry = where.entry(args[0])            # not_found names the open documents
    owned = entry if isinstance(getattr(entry, "handle", None), Mirror) else None
    if owned is None:
        raise TypeError(f"{args[0]} is not a Word document the session's Rust core holds; {name} draws "
                        "the pages of one")
    call = {"layout": ooxml_core.CallLayout(_engine(owned)), "context": context}
    return owned.handle.session, owned.handle.doc, call, tuple(args[1:])


def _engine(owned: Any) -> Any:
    """The page engine (an ``ooxml_core.WordPages``) the document ``owned`` (its session
    entry) is laid out with: the session's own (:func:`docx_agent.rust.engines.session_pages`)
    -- the one seam where this module reaches the engine."""
    from .rust.engines import session_pages

    return session_pages(owned)


@contextmanager
def _raised() -> Iterator[None]:
    """The session's errors in this module's words: ``ooxml_core.UnknownToken`` as
    :class:`UnknownToken`, ``ooxml_core.ViewRefused`` as the session's refusal."""
    import ooxml_core

    from .rust.refusals import refusal

    try:
        yield
    except ooxml_core.UnknownToken as error:
        raise UnknownToken(str(error)) from error
    except ooxml_core.ViewRefused as error:
        raise refusal(json.loads(error.args[2])) from error


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
