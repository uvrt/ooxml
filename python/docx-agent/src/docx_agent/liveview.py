"""An application's live view of a document, from the layout the tools share (round 35, R27).

A page that shows a document as it is written -- each page as SVG, redrawn when an edit
changes it -- needs to know which pages to draw again and what each page shows.  It asks
the layout the tools already made of that version (:mod:`docx_agent.layout`: one per
version, incremental, shared by ``check``, ``render`` and the Python API), never a layout
of its own::

    from docx_agent.liveview import page_svg, view

    shown = view(session, "d1")                          # first call: every page changed
    for number in shown.changed:
        drawn = page_svg(session, "d1", number, shown.token)
        put(number, drawn.svg, drawn.images)             # pictures by name: img/3fa1....png
    ...
    shown = view(session, "d1", since=shown.token)       # after edits: the pages they changed

* :func:`view` -- the pages in order, each with the ids of the body's top-level blocks it
  shows and its size; the pages whose drawing changed since ``since`` (every page without
  one); a new ``token``, naming this version's layout.  ``stale`` (with a ``note``) when
  ``since`` names no version kept for the document: every page is listed then.
* :func:`page_svg` -- one page of the version a token names, as SVG, drawn from that
  version's layout even when the document moved on since: ``text="outlines"`` draws text
  as glyph paths (``glyphs``: one set kept across pages, docx2svg's
  ``ConvertOptions.glyphs``); ``images="external"`` gives pictures back by name
  (:func:`externalise_images`, the names pptx2svg's ``liveview`` gives), ``"embedded"``
  keeps them in the SVG.

Pages changed are those whose drawing differs from the same page of the token's version
(docx2svg's ``relayout.drawing``: the SVG less its ``data-docx-*`` attributes), and those
it did not have: what docx2svg's ``Layout.changed_pages`` says for a layout made from the
one before.  Pages equal in the layout (and every part but the document's text the same)
are not drawn to tell.

**Where the layouts are** (round 58, Q2).  A document a toolbox's calls laid out has its
layouts in that toolbox's workers (:func:`docx_agent.layout.remember_runner`), and the view
asks there: :func:`view_bytes` and :func:`page_bytes` run in the worker holding the
document's layouts (the pool's affinity, as ``check``'s facts do), the versions a view was
given kept with them (``extra["views"]`` of :class:`~docx_agent.layout._Held`, each laid out
again from the newest where the worker no longer has it).  This process keeps, per token,
the version's bytes and its pages -- never a layout -- and the worker sends back the pages,
the pages changed, one page's SVG.  A document no toolbox laid out (``view(document)``, an
application's own) is laid out here, the same tasks run in this process.

**Locks** (the order is :mod:`docx_agent.layout`'s; round 34's rule: never wait for a
layout in progress holding the document's lock).  ``view(session, doc)`` takes the
document's lock to read its bytes and version, and lets it go while its task runs; the task
waits for a layout of the document in progress where it is held.  Called by a thread that
holds the lock already, it lays out on its own instead of waiting.  ``view(document)`` (a
document outside a session: no lock) never waits; either way the version's layout is the
one kept for all.  :func:`page_svg` takes no document lock.
"""

from __future__ import annotations

import base64
import hashlib
import io
import re
import secrets
import zipfile
from collections import OrderedDict
from contextlib import contextmanager
from dataclasses import dataclass, field
from typing import Any, Iterator, Mapping, MutableMapping

import docx2svg

from . import layout as _layout
from .layout import (_MEASURES, _PASSED_ON, PAGE_COST, _Held, _failed, _keeper, _kept_failure, _let_go_held,
                     _put_held, _tag, _take_held, cache_key, held_layout, keep_layout, kept_layout, layout_key,
                     layout_options, made_facts, note_made, opened_layout, runner_of)

__all__ = ["PageSvg", "UnknownToken", "View", "ViewPage", "embed_images", "externalise_images",
           "page_bytes", "page_svg", "view", "view_bytes"]

#: Tokens -- versions -- kept per document for ``since`` and :func:`page_svg`, the least
#: recently used dropped first: here their bytes and pages, in the worker their layouts
#: (within a quarter of :data:`~docx_agent.layout.LAYOUT_MEMORY` but the two used last).
VIEW_TOKENS = 4

#: How long a view's or a page's task may take in a worker -- a long document's first
#: layout among it -- before the call reports ``timeout`` (the worker finishes it and keeps
#: it where its pool lets it, as a layout's task does).
VIEW_TIMEOUT = 600.0

#: :func:`page_svg`'s ``text`` and ``images``.
TEXT_MODES = ("text", "outlines")
IMAGE_MODES = ("external", "embedded")


@dataclass(frozen=True)
class ViewPage:
    """One page: its 1-based ``number``, the ids of the body's top-level blocks it draws
    something of, in document order (:meth:`~docx_agent.layout.DocumentLayout.blocks_on`),
    and its size in points."""

    number: int
    blocks: tuple[str, ...]
    width: float
    height: float


@dataclass(frozen=True)
class View:
    """:func:`view`'s answer."""

    #: This version's layout: ``since`` for the next view, and :func:`page_svg`'s ``token``.
    token: str
    #: Every page, in order.
    pages: list[ViewPage]
    #: The 1-based pages whose drawing changed since ``since`` -- every page without one, or
    #: with a stale one.  Pages ``since``'s version had past the last are not listed: the
    #: page count says they are gone.
    changed: list[int]
    #: The token the pages were compared with, as given.
    since: str | None = None
    #: ``since`` names no version kept for this document (another document's, one from
    #: before a restart, or older than the last :data:`VIEW_TOKENS`): every page is in
    #: ``changed``, and ``note`` says so.
    stale: bool = False
    note: str | None = None
    #: The session's version of the document laid out (``None`` outside a session).
    version: int | None = None

    @property
    def page_count(self) -> int:
        return len(self.pages)


@dataclass(frozen=True)
class PageSvg:
    """:func:`page_svg`'s answer: the page's SVG, and -- with ``images="external"`` -- the
    pictures it references, by name; ``warnings``, what was not drawn faithfully (text kept
    as text in ``outlines``, say)."""

    page: int
    token: str
    svg: str
    images: dict[str, bytes] = field(default_factory=dict)
    warnings: list[str] = field(default_factory=list)


class UnknownToken(LookupError):
    """A token :func:`page_svg` was given that names no layout kept for the document."""


@dataclass(frozen=True)
class _Shown:
    """What this process keeps of a version a view was given: its bytes (laid out again in a
    worker that no longer holds its layout), the options it was laid out with, its pages."""

    data: bytes
    options: dict
    pages: list[ViewPage]
    version: int | None


# -- view ----------------------------------------------------------------------------------------


def view(where: Any, *args: str, since: str | None = None) -> View:
    """The document's pages now, which changed since ``since``, and a new token (the module
    says how): ``view(document)`` or ``view(session, doc)`` -- ``doc`` an open Word
    document's id (``"d1"``)."""
    document, entry, _ = _target(where, args, "view", 0)
    if entry is None:
        return _view(document, document.to_bytes(), None, since, may_wait=False)
    with entry.lock:
        version = entry.version
        data = document.to_bytes()
        # Its task runs with the lock let go: it may wait there for a layout in progress.
        with _let_go(entry.lock) as released:
            return _view(document, data, version, since, may_wait=released)


def _view(document: Any, data: bytes, version: int | None, since: str | None, *, may_wait: bool) -> View:
    options = layout_options(document, {})
    token = _token(document, data, options)
    with _keeper(document):
        tokens = _tokens(document)
        shown = tokens.get(token)
        old = tokens.get(since) if since is not None and since != token else None
        for used in (since, token):
            if used in tokens:
                tokens.move_to_end(used)
    stale = since is not None and since != token and old is None
    changed: list[int] | None = None
    if shown is None or old is not None:
        request = {"token": token, "since": since if old is not None else None,
                   "since_data": old.data if old is not None else None}
        result = _run(document, view_bytes, data, options, request, may_wait=may_wait, new=True)
        changed = result.get("changed")
        if shown is None:
            shown = _Shown(data, options, [ViewPage(number, tuple(blocks), width, height)
                                           for number, blocks, width, height in result["pages"]], version)
            with _keeper(document):
                tokens[token] = shown
                tokens.move_to_end(token)
                while len(tokens) > VIEW_TOKENS:
                    tokens.popitem(last=False)
    pages = shown.pages
    if since is None:
        return View(token, pages, [p.number for p in pages], version=version)
    if since == token:
        return View(token, pages, [], since, version=version)
    if stale:
        return View(token, pages, [p.number for p in pages], since, stale=True, version=version,
                    note=f"since {since!r} names no layout kept for this document (another document's, one "
                    f"from before a restart, or older than the last {VIEW_TOKENS}): every page is listed "
                    "as changed")
    assert changed is not None
    return View(token, pages, changed, since, version=version)


def _token(document: Any, data: bytes, options: dict) -> str:
    """The token of ``data`` laid out with ``options``: the document's own nonce (another
    document's tokens are not its) and a digest of what is laid out."""
    with _keeper(document):
        nonce = document.__dict__.setdefault("_view_nonce", secrets.token_hex(4))
    return f"{nonce}-{hashlib.sha256(cache_key(data, options).encode()).hexdigest()[:24]}"


def _tokens(document: Any) -> "OrderedDict[str, _Shown]":
    """The document's kept tokens (under its layouts' lock, :func:`_keeper`)."""
    tokens = document.__dict__.get("_view_tokens")
    if tokens is None:
        tokens = document.__dict__["_view_tokens"] = OrderedDict()
    return tokens


def _run(document: Any, task: Any, data: bytes, options: dict, request: dict, *, may_wait: bool,
         new: bool) -> dict:
    """``task(data, options, state, request)`` where the document's layouts are: in the
    worker of the runner that laid it out (:func:`~docx_agent.layout.runner_of`, with its
    affinity), else in this process.  ``new``: ``data`` is the document's version now, laid
    out and kept as its last (as a check's layout is); else a version the view was given."""
    key = cache_key(data, options)
    if new:
        failed = _kept_failure(document, key)
        if failed is not None:
            raise failed
        state = kept_layout(document, data, options, may_wait=may_wait)
    else:
        state = _held_state(document, data, options, may_wait)
    runner = runner_of(document)
    try:
        if runner is None:
            result = task(data, dict(options), state, request)
        else:
            from .tools._base import Engine

            result = Engine.of(runner, VIEW_TIMEOUT, document).task(task, data, dict(options), state, request)
    except _PASSED_ON:
        raise
    except Exception as error:
        if not new:
            raise
        raise _failed(document, key, error) from error
    if new:
        keep_layout(document, data, options, layout_key(data, options))
        note_made(document, result)
    return result


def _held_state(document: Any, data: bytes, options: dict, may_wait: bool) -> tuple:
    """The ``("held", ...)`` state of a task drawing ``data`` -- a version a view was given,
    perhaps not the document's last -- that takes the document's layouts where they are held
    and puts them back as they were (:func:`page_bytes`): no layout of it is started here."""
    _keeper(document)
    kept = document._layout_kept
    key = layout_key(data, options)
    token = kept[1] if kept is not None and isinstance(kept[1], tuple) and kept[0][1] == key[1] else None
    state = ("held", key, token, document._layout_lineage)
    return state if may_wait else (*state, False)


# -- in the worker holding the layouts -------------------------------------------------------------


class _Version:
    """In the process holding a document's layouts: a version a view was given -- its layout
    ``laid`` (``(layout, fonts, warnings)``), its bytes and options -- and what was read of
    it: its pages' blocks and sizes, the layout mapped to ids (``layout``; kept for the
    lineage's newest only), a digest of its parts but the text, and of each page's drawing."""

    __slots__ = ("laid", "data", "options", "layout", "pages", "parts", "digests")

    def __init__(self, laid: tuple, data: bytes, options: dict) -> None:
        self.laid = laid
        self.data = data
        self.options = options
        self.layout: Any = None
        self.pages: list | None = None
        self.parts: bytes | None = None
        #: Per 0-based page, the digest of what it draws (docx2svg's ``relayout.drawing``).
        self.digests: dict[int, bytes] = {}

    def mapped(self, held: "_Held | None") -> Any:
        """The layout mapped to ids on a copy opened from the bytes -- the lineage's own
        where ``held`` is this version's (:func:`~docx_agent.layout.opened_layout`, which a
        check's facts read too)."""
        if self.layout is None:
            if held is None or held.laid is not self.laid:
                held = _Held(self.laid)
            self.layout = opened_layout(held, self.data, self.options)
        return self.layout

    def page_list(self, held: "_Held | None") -> list:
        if self.pages is None:
            layout = self.mapped(held)
            self.pages = [(page.number + 1, tuple(layout.blocks_on(page.number + 1)), page.width_pt, page.height_pt)
                          for page in self.laid[0].pages]
        return self.pages

    def parts_digest(self) -> bytes:
        if self.parts is None:
            self.parts = _parts(self.data)
        return self.parts

    def digest(self, index: int) -> bytes:
        """What page ``index`` draws, digested (docx2svg's ``relayout.drawing``), made once:
        the layout's own where docx2svg made it when it compared versions."""
        found = self.digests.get(index)
        if found is None:
            from docx2svg import relayout

            raw = self.laid[0]
            state = getattr(raw, "state", None)
            found = (getattr(state, "drawings", None) or {}).get(index)
            if found is None:
                found = relayout.drawing(raw.pages[index], self.data, self.laid[1])
            self.digests[index] = found
        return found


def _versions(held: _Held) -> "OrderedDict[str, _Version]":
    """The versions a view was given, kept with the lineage's newest layout ``held``."""
    views = held.extra.get("views")
    if views is None:
        views = held.extra["views"] = OrderedDict()
    return views


def _lay_out_version(laid: "tuple | None", data: bytes, options: dict) -> _Version:
    """A version this process no longer holds, laid out again -- from ``laid``, a layout of
    the same document, where there is one (the pages it reaches only)."""
    convert_options = docx2svg.ConvertOptions(**options)
    layout, _, fonts = docx2svg.lay_out_docx(data, convert_options, previous=laid[0] if laid else None,
                                             measures=_MEASURES, changed_pages=False)
    return _Version((layout, fonts, list(convert_options.warnings)), data, options)


def _trim(views: "OrderedDict[str, _Version]", held: _Held) -> None:
    """At most :data:`VIEW_TOKENS` versions, the least recently used dropped first while
    those not the newest layout's own pages cost more than a quarter of
    :data:`~docx_agent.layout.LAYOUT_MEMORY` (the two used last kept whatever they cost);
    only the newest keeps its layout mapped to ids."""
    while len(views) > VIEW_TOKENS:
        views.popitem(last=False)
    while len(views) > 2 and _views_cost(views, held) > _layout.LAYOUT_MEMORY // 4:
        views.popitem(last=False)
    for version in views.values():
        if version.laid is not held.laid:
            version.layout = None


def _views_cost(views: "OrderedDict[str, _Version]", held: _Held) -> int:
    counted = {id(page) for page in held.laid[0].pages}
    cost = 0
    for version in views.values():
        for page in version.laid[0].pages:
            if id(page) not in counted:
                counted.add(id(page))
                cost += PAGE_COST
    return cost


def _changed(old: _Version, new: _Version) -> list[int]:
    """The 1-based pages of ``new`` whose drawing differs from ``old``'s page of the same
    number, and those ``old`` did not have."""
    if old.laid is new.laid:
        return []
    before, after = old.laid[0].pages, new.laid[0].pages
    same_parts = old.parts_digest() == new.parts_digest()
    out = []
    for index, page in enumerate(after):
        if index >= len(before):
            out.append(index + 1)
        elif same_parts and page == before[index]:
            continue
        elif old.digest(index) != new.digest(index):
            out.append(index + 1)
    return out


def _parts(data: bytes) -> bytes:
    digest = hashlib.sha256()
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        for info in sorted(package.infolist(), key=lambda i: i.filename):
            name = info.filename
            if name == "word/document.xml" or name.startswith("docProps/"):
                continue
            digest.update(name.encode() + b"\0" + package.read(info) + b"\0")
    return digest.digest()


def view_bytes(data: bytes, options: dict, state: tuple, request: dict) -> dict:
    """In the worker holding the document's layouts (round 58, Q2): the layout of ``data``
    -- the one held, or laid out from the one found (:func:`~docx_agent.layout.held_layout`,
    kept as the lineage's newest, as a check's is) -- and what :func:`view` answers of it:
    ``pages`` (each ``(number, blocks, width, height)``) and, with ``request["since"]``,
    ``changed`` -- against that token's version, kept here, else laid out again from
    ``since_data``.  ``request["token"]`` names this version among those kept.  Plus how the
    layout was found or made (:func:`~docx_agent.layout.made_facts`)."""
    held, how, seconds = held_layout(data, options, state)
    try:
        out = made_facts(held, how, seconds)
        views = _versions(held)
        token = request["token"]
        new = views.get(token)
        if new is None or new.laid is not held.laid:
            new = views[token] = _Version(held.laid, data, options)
        out["pages"] = new.page_list(held)
        since = request.get("since")
        if since is not None:
            old = views.get(since)
            if old is None:
                old = views[since] = _lay_out_version(held.laid, request["since_data"], options)
            views.move_to_end(since)
            out["changed"] = _changed(old, new)
        views.move_to_end(token)
        _trim(views, held)
    finally:
        _put_held(state[1], held, state[3])       # a layout of these bytes either way
    return out


# A task ooxml-edit's workers may run (the mark, as the layout's tasks set it).
view_bytes.__ooxml_edit_task__ = True  # type: ignore[attr-defined]


def page_bytes(data: bytes, options: dict, state: tuple, request: dict) -> dict:
    """In the worker holding the document's layouts (round 58, Q2): page ``request["page"]``
    of the version ``request["token"]`` names -- kept with the lineage's layouts, else laid
    out again from the newest of them -- drawn as :func:`page_svg` says (``text``; with
    ``glyphs`` true, the glyph definitions apart) and tagged with that version's ids:
    ``{"svg", "glyphs", "warnings"}``.  The lineage's layouts are put back as they were: a
    page of an older version never becomes the newest."""
    lineage = state[3]
    found, _ = _take_held(state[1], state[2], lineage, state[4] if len(state) > 4 else True)
    try:
        token = request["token"]
        version = _versions(found).get(token) if found is not None else None
        if version is None:
            version = _lay_out_version(found.laid if found is not None else None, data, options)
        if found is None:
            # Nothing of the document here: what was laid out is kept as its layouts.
            found = _Held(version.laid)
            found.token = state[1]
        views = _versions(found)
        views[token] = version
        views.move_to_end(token)
        layout = version.mapped(found)
        drawing = docx2svg.ConvertOptions(**{**version.options, "pages": [request["page"]],
                                             "text": request["text"],
                                             "glyphs": {} if request["glyphs"] else None})
        svg = docx2svg.page_svgs(version.laid[0], version.data, drawing, fonts=version.laid[1])[0]
        out = {"svg": _tag(layout._document, layout, svg), "glyphs": drawing.glyphs or {},
               "warnings": [str(w) for w in drawing.warnings]}
        _trim(views, found)
    except BaseException:
        if found is not None and found.token is not None:
            _put_held(found.token, found, lineage)
        else:
            _let_go_held(lineage)
        raise
    _put_held(found.token, found, lineage)
    return out


page_bytes.__ooxml_edit_task__ = True  # type: ignore[attr-defined]


# -- one page --------------------------------------------------------------------------------------


def page_svg(where: Any, *args: Any, text: str = "text", images: str = "external",
             glyphs: dict | None = None) -> PageSvg:
    """One page of the version ``token`` names, as SVG: ``page_svg(document, page, token)``
    or ``page_svg(session, doc, page, token)``.  Its paragraph groups carry
    ``data-docx-agent-id`` (that version's ids), as :meth:`~docx_agent.Document.render_svg`'s.

    ``text``: ``"text"`` (``<text>``, drawn with the viewer's fonts) or ``"outlines"`` (the
    glyphs' paths: no font needed to view it); with ``glyphs`` -- a dict kept across pages
    and versions -- the glyph definitions go there, each once, and the page references them
    (docx2svg's ``glyph_sheet`` of it goes once in the page showing them).  ``images``:
    ``"external"`` -- each picture referenced by name, its bytes in the answer's
    ``images`` (:func:`externalise_images`) -- or ``"embedded"``.

    Raises :class:`UnknownToken` for a token no :func:`view` of the document gave or no
    longer kept, ``IndexError`` for a page the version does not have, ``ValueError`` for a
    ``text`` or ``images`` it does not take."""
    if text not in TEXT_MODES:
        raise ValueError(f"text must be one of {', '.join(TEXT_MODES)}, not {text!r}")
    if images not in IMAGE_MODES:
        raise ValueError(f"images must be one of {', '.join(IMAGE_MODES)}, not {images!r}")
    if glyphs is not None and text != "outlines":
        raise ValueError("glyphs is the glyph set of outlined pages: give text=\"outlines\" with it")
    document, entry, (page, token) = _target(where, args, "page_svg", 2)
    with _keeper(document):
        shown = _tokens(document).get(token)
        if shown is not None:
            _tokens(document).move_to_end(token)
    if shown is None:
        raise UnknownToken(f"token {token!r} names no layout kept for this document (another document's, "
                           f"one from before a restart, or older than the last {VIEW_TOKENS}); call view() for a "
                           "current token and the pages it changed")
    count = len(shown.pages)
    if not isinstance(page, int) or not 1 <= page <= count:
        raise IndexError(f"page {page!r}: the version {token!r} names has {count} page(s), 1 to {count}")
    # Round 34's rule: a thread holding the document's lock never waits for a layout.
    may_wait = entry is None or not _owned(entry.lock)
    drawn = _run(document, page_bytes, shown.data, shown.options,
                 {"token": token, "page": page, "text": text, "glyphs": glyphs is not None},
                 may_wait=may_wait, new=False)
    if glyphs is not None:
        for name, definition in drawn["glyphs"].items():
            glyphs.setdefault(name, definition)
    svg = drawn["svg"]
    store: dict[str, bytes] = {}
    if images == "external":
        svg = externalise_images(svg, store)
    return PageSvg(page, token, svg, store, list(drawn["warnings"]))


def _owned(lock: Any) -> bool:
    """Whether this thread holds ``lock`` (one that cannot say counts as held)."""
    owned = getattr(lock, "_is_owned", None)
    return owned is None or owned()


# -- the target ------------------------------------------------------------------------------------


def _target(where: Any, args: tuple, name: str, rest: int) -> tuple[Any, Any, tuple]:
    """``(document, entry, the other arguments)`` of ``(document, ...)`` -- a Document, no
    entry -- or ``(session, doc, ...)``, a session's open Word document; ``rest``: how many
    arguments follow."""
    from .edit.document import Document

    tail = ", page, token" if rest else ""
    if isinstance(where, Document) and len(args) == rest:
        return where, None, tuple(args)
    if isinstance(where, Document) or not callable(getattr(where, "entry", None)) or len(args) != rest + 1:
        raise TypeError(f"{name} takes (document{tail}) or (session, doc{tail})")
    entry = where.entry(args[0])            # not_found names the open documents
    if not isinstance(entry.document, Document):
        raise TypeError(f"{args[0]} is a {entry.kind} document; {name} draws a Word document's pages")
    return entry.document, entry, tuple(args[1:])


@contextmanager
def _let_go(lock: Any) -> Iterator[bool]:
    """The document's lock let go while the version is laid out (the layout's ``unlocked``):
    yields whether it was.  A thread that held it before :func:`view` took it holds it still
    after one release: it is not let go, and the layout is this caller's own (never a wait
    under the lock); a lock that cannot say whether it is held counts as held."""
    lock.release()
    try:
        owned = getattr(lock, "_is_owned", None)
        yield owned is not None and not owned()
    finally:
        lock.acquire()


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
