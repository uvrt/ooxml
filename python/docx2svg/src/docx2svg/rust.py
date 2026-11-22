"""docx2svg's layout in Rust (``ooxml_core``, phase PL, lane L4): the same pages, held in Rust.

:class:`Engine` lays the versions of one document out through ``ooxml_core.WordPages``,
each from the one before, with the interpreter lock let go; :meth:`Engine.lay_out` answers a
:class:`RustLayout`, which reads as docx2svg's :class:`~docx2svg.layout.Layout` does --
``pages`` (each a :class:`RustPage`: its frame, ``info`` and ``blocks`` at once; its
``lines``, ``rules``, ``pictures`` and ``floats`` painted in Rust the first time they are
read, a few pages kept), ``warnings``, ``coverage`` -- and draws its pages as SVG in Rust,
byte for byte :func:`docx2svg.page_svgs`'.  Nothing of the layout is a Python object but
the pages a caller reads, while it reads them.

**Falling back.**  A version the Rust layout does not lay out as docx2svg does yet (its
reasons: ``ooxml_word_pages::support``), or options it does not take (a measurer of the
caller's own, ``text="outlines"``), is not laid out here: :meth:`Engine.lay_out` answers
``None`` and counts the reason (:func:`fallbacks`), and the caller lays it out with
docx2svg's Python layout, frozen as that fallback until phase P4 deletes it.  A page whose
SVG the Rust paint does not draw as docx2svg does (what a page's paint lists as not drawn)
is the same: :meth:`RustLayout.page_svg` answers ``None`` for it, counted.  Pictures, shapes,
groups and charts -- floating or inline -- are painted in Rust (lane L4c).

**Memory** (lane L4b).  Every engine counts in the process's ``ooxml_core.MemoryBudget``
(:func:`memory_budget`): its memos as held (swept once a version is laid out, so they stay at
what the newest version needs), its pages' paint within the limit, kept in Rust, the least
recently used let go first; a host reserves the layouts it keeps from the same budget.  A
:class:`RustLayout`'s ``weight`` is what its version alone holds in Rust now.

Its ``warnings``, ``approximations``, ``coverage`` and ``relaid_cause`` are docx2svg's, in
docx2svg's order: the layout's own computed in Rust (``ooxml_word_pages::warnings``), the
faces' in the order first met.
"""

from __future__ import annotations

import hashlib
import threading
from collections import Counter, OrderedDict
from fractions import Fraction
from pathlib import Path
from typing import TYPE_CHECKING, Any, Sequence

from ooxml_common.locales import EN_US

if TYPE_CHECKING:
    from .coverage import Coverage

#: Why versions and pages were not laid out or drawn in Rust, counted by reason
#: (``layout_fallbacks{reason}``): read by :func:`fallbacks`; never in a result.
_FALLBACKS: Counter = Counter()
_FALLBACKS_LOCK = threading.Lock()

#: How many pages a layout keeps as docx2svg's objects for its readers (lane L4b: four; the
#: pages' paint is kept in Rust, within the process's memory budget, and read from there
#: again).
PAINTED_PAGES = 4


def count_fallback(reason: str) -> None:
    """Count one fallback for ``reason``."""
    with _FALLBACKS_LOCK:
        _FALLBACKS[reason] += 1


def fallbacks() -> dict[str, int]:
    """``layout_fallbacks{reason}``: the versions and pages laid out or drawn by docx2svg's
    Python layout instead, by reason, since the process started."""
    with _FALLBACKS_LOCK:
        return dict(_FALLBACKS)


def available() -> bool:
    """Whether ``ooxml_core`` has the Rust layout."""
    try:
        ooxml_core = _core()
    except ImportError:
        return False
    return hasattr(ooxml_core, "WordPages")


def _core() -> Any:
    """``ooxml_core``, the Rust core's module: an optional dependency of docx2svg."""
    import importlib

    return importlib.import_module("ooxml_core")


# -- fonts ---------------------------------------------------------------------------------

_FONTS: dict = {}
_FONTS_LOCK = threading.Lock()


def _fonts_key(options) -> tuple:
    from . import _font_dirs
    from .fonts import SUBSTITUTES, WORD_FONTS

    folders = tuple(("bundle" if Path(d) == WORD_FONTS else "folder", str(d)) for d in _font_dirs(options))
    substitutes = dict(SUBSTITUTES) if options.substitute_fonts else {}
    substitutes.update({name.lower(): value for name, value in (options.font_substitutes or {}).items()})
    return folders, tuple(substitutes.items()), bool(options.substitute_fonts)


def word_fonts(options) -> Any:
    """The machine's fonts as ``options`` find them (docx2svg's ``_fonts``: Word's folders,
    then the application's), scanned once per set of folders and substitutes."""
    ooxml_core = _core()

    key = _fonts_key(options)
    with _FONTS_LOCK:
        found = _FONTS.get(key)
    if found is not None:
        return found
    folders, substitutes, substitute = key
    made = ooxml_core.WordFonts(list(folders), substitutes=list(substitutes), recorded_faces=substitute,
                                fallbacks=substitute)
    with _FONTS_LOCK:
        return _FONTS.setdefault(key, made)


def unsupported_options(options) -> str | None:
    """Why ``options`` are not the Rust layout's to lay out with, or ``None``."""
    if options.advances is not None or options.metrics is not None or options.decorations is not None \
            or options.names is not None:
        return "custom measurement"
    if options.text != "text":
        return "outlined text"
    return None


# -- the memory budget (lane L4b) ------------------------------------------------------------

#: What the process's engines keep that can be made again (their pages' paint) and what a
#: host's own caches reserve (layouts kept for readers) by default, bytes.
DEFAULT_MEMORY = 256 * 2 ** 20

_BUDGET: list = []
_BUDGET_LOCK = threading.Lock()


def memory_budget(limit: int | None = None) -> Any:
    """The process's ``ooxml_core.MemoryBudget``: every engine counts in it (its memos as
    held, its pages' paint within the limit), and a host reserves what it keeps of the
    layouts from it (``memory_budget().account()``).  Made on first use with ``limit``
    bytes (default :data:`DEFAULT_MEMORY`); a later ``limit`` is ignored."""
    with _BUDGET_LOCK:
        if not _BUDGET:
            _BUDGET.append(_core().MemoryBudget(int(limit) if limit else DEFAULT_MEMORY))
        return _BUDGET[0]


# -- the engine ------------------------------------------------------------------------------


def _engine_key(options) -> tuple:
    """What an engine is made for: the fonts, the decimal separator, the glyph size, and the
    locale's language, thousands separator, date order and list separator (a chart's
    numbers and dates are written in them)."""
    locale = options.locale or EN_US
    return (_fonts_key(options), locale.decimal_separator, options.glyph_size, locale.language,
            locale.thousands_separator, locale.date_order, locale.list_separator)


class Engine:
    """One document's layouts in Rust (``ooxml_core.WordPages``), each version laid out
    from the one before; safe to share between threads (the Rust engine locks itself)."""

    def __init__(self, options) -> None:
        ooxml_core = _core()

        self.key = _engine_key(options)
        _, decimal, _, language, thousands, date_order, list_separator = self.key
        self._pages = ooxml_core.WordPages(word_fonts(options), locale=language, decimal=decimal,
                                           glyph_size=options.glyph_size,
                                           numbers=(thousands, date_order, list_separator), budget=memory_budget())
        self._lock = threading.Lock()
        #: The last version asked for, by the digest of its bytes, and its layout (``None``: a
        #: fallback): asked again, the same answer.
        self._last: tuple | None = None

    def fits(self, options) -> bool:
        """Whether this engine lays out with ``options`` (the same fonts, decimal, glyphs and
        locale)."""
        return self.key == _engine_key(options)

    def lay_out(self, data: bytes, options) -> "RustLayout | None":
        """``data`` laid out in Rust, or ``None`` (counted) where docx2svg's Python layout
        lays it out instead."""
        ooxml_core = _core()

        why = unsupported_options(options)
        if why is not None:
            count_fallback(why)
            return None
        digest = hashlib.sha256(data).digest()
        with self._lock:
            if self._last is not None and self._last[0] == digest:
                return self._last[1]          # (a fallback is counted once per version)
        try:
            laid = self._pages.lay_out(data)
        except ooxml_core.LayoutFallback as fallback:
            count_fallback(fallback.args[0])
            with self._lock:
                self._last = (digest, None)
            return None
        made = RustLayout(laid, data, options)
        with self._lock:
            self._last = (digest, made)
        return made

    def known(self, data: bytes) -> "RustLayout | None":
        """The layout of ``data`` where it is the last version this engine laid out, else
        ``None``: nothing is laid out."""
        digest = hashlib.sha256(data).digest()
        with self._lock:
            if self._last is not None and self._last[0] == digest:
                return self._last[1]
        return None

    def forget(self) -> None:
        """Let go of the last version and the memos."""
        with self._lock:
            self._last = None
        self._pages.forget()

    def stats(self) -> dict:
        return dict(self._pages.stats())


# -- the layout, read as docx2svg's ---------------------------------------------------------


def _q(pair) -> Fraction:
    return Fraction(*pair)


class RustLayout:
    """A version laid out in Rust, read as docx2svg's ``Layout``: ``pages`` (lazy),
    ``warnings``, ``coverage``; ``state`` is ``None`` (nothing of it is pickled or handed
    to another process: a version is laid out from the one before in Rust)."""

    state = None
    #: The document lineage whose engine laid it out, where its caller names one.
    lineage: str | None = None
    relaid: str | None = None
    relaid_cause: dict | None = None
    changed_pages = None

    def __init__(self, laid: Any, data: bytes, options) -> None:
        self.laid = laid
        self.data = data
        self.options = options
        # Laid out whole although a version before was laid out: why (docx2svg's
        # ``relaid`` and ``relaid_cause``, ``relayout.FALLBACKS``' words).
        cause = laid.relaid_cause
        self.relaid = cause["reason"] if cause else None
        self.relaid_cause = cause
        self._lock = threading.Lock()
        self._painted: OrderedDict[int, dict] = OrderedDict()
        self._boxes: OrderedDict[int, dict] = OrderedDict()
        self._texts: OrderedDict[int, list] = OrderedDict()
        frames = laid.pages()
        self.pages = [RustPage(self, index, row) for index, row in enumerate(frames)]
        self._blocks: list | None = None
        self._coverage: Coverage | None = None
        self._warnings: list | None = None

    def __reduce__(self):
        raise TypeError("a layout held in Rust is not pickled: lay the bytes out where they are needed "
                        "(docx2svg.rust.Engine)")

    # -- what docx2svg's layout has -------------------------------------------------------

    @property
    def weight(self) -> int:
        """What the Rust layout holds, bytes, roughly."""
        return self.laid.weight

    @property
    def warnings(self) -> list:
        if self._warnings is None:
            self._coverage_and_warnings()
        return self._warnings or []

    @property
    def coverage(self) -> "Coverage | None":
        if self._coverage is None:
            self._coverage_and_warnings()
        return self._coverage

    @coverage.setter
    def coverage(self, value) -> None:
        self._coverage = value

    @property
    def approximations(self) -> list:
        """docx2svg's ``(code, message, 1-based page, path)`` of each place laid out by a rule
        no probe measured, found in Rust."""
        return list(self.laid.approximations())

    @property
    def placeholders(self) -> list:
        return []

    def blocks_on(self, page: int) -> list[str]:
        from .paths import block_keys

        if not 1 <= page <= len(self.pages):
            return []
        return list(dict.fromkeys(block_keys(key)[0] for key in self.pages[page - 1].blocks))

    # -- what Rust answers ---------------------------------------------------------------

    def page_blocks(self, index: int) -> list[str]:
        if self._blocks is None:
            self._blocks = self.laid.blocks()
        return self._blocks[index]

    def body_lines(self) -> list:
        """Per page, its body's lines without their spans: ``(path, number, top, pitch,
        baseline, column, id)``, exact -- what an index of where blocks are drawn reads, with
        no page painted; ``id`` the paragraph's as docx-agent names it in these bytes
        (lane L4b: found in Rust, ``None`` where no body paragraph is the line's)."""
        return [[(path, number, _q(top), _q(pitch), baseline, column, found)
                 for path, number, top, pitch, baseline, column, found in page] for page in self.laid.body_lines()]

    def ids_of(self, paths: list) -> list:
        """The ids, as docx-agent names them in these bytes, of the body paragraphs, tables
        and rows at ``paths`` (``None`` where a path names none)."""
        return self.laid.ids_of(list(paths))

    def painted(self, index: int) -> dict:
        """Page ``index`` (0-based) painted, as docx2svg's objects: kept for the next reader
        among the last :data:`PAINTED_PAGES` read."""
        with self._lock:
            found = self._painted.get(index)
            if found is not None:
                self._painted.move_to_end(index)
                return found
        made = _paint(self.laid.page(index))
        with self._lock:
            self._painted[index] = made
            while len(self._painted) > PAINTED_PAGES:
                self._painted.popitem(last=False)
        return made

    def boxes(self, index: int) -> dict:
        """Page ``index``'s rules, pictures and floats as docx2svg's objects, without its
        lines (lane L4b: a reader of these alone reads no line into Python); kept among the
        last :data:`PAINTED_PAGES` read, or taken from the page painted whole."""
        with self._lock:
            found = self._painted.get(index) or self._boxes.get(index)
            if found is not None:
                return found
        made = _paint(self.laid.page(index, False))
        with self._lock:
            self._boxes[index] = made
            while len(self._boxes) > PAINTED_PAGES:
                self._boxes.popitem(last=False)
        return made

    def line_texts(self, index: int) -> list:
        """Page ``index``'s lines' text as a check reads it, measured in Rust (lane L4b):
        per line ``(path, story, part, number, inked, sizes, ink, widest)`` -- ``inked`` its
        characters that are not white space, ``sizes`` ``[(half_points, chars)]`` of its text
        not raised or lowered, ``ink`` ``(left, right)`` and ``widest`` exact device px, or
        ``None`` for a line of no ink.  Kept among the last :data:`PAINTED_PAGES` read."""
        with self._lock:
            found = self._texts.get(index)
            if found is not None:
                self._texts.move_to_end(index)
                return found
        made = [(path, story, part, number, inked, sizes,
                 None if ink is None else (_q(ink[0]), _q(ink[1])), None if widest is None else _q(widest))
                for path, story, part, number, inked, sizes, ink, widest in self.laid.line_texts(index)]
        with self._lock:
            self._texts[index] = made
            while len(self._texts) > PAINTED_PAGES * 64:
                self._texts.popitem(last=False)
        return made

    def page_svg(self, index: int, *, width: int | None = None, height: int | None = None) -> str | None:
        """Page ``index`` (0-based) as SVG, byte for byte docx2svg's, its drawings painted in
        Rust; ``None`` (counted) where the Rust paint does not draw it as docx2svg does yet."""
        svg, missing = self.laid.svg(index)
        if missing:
            for reason in dict.fromkeys(missing):
                count_fallback(reason)
            return None
        if width is None and height is None:
            return svg
        return _sized(svg, self.pages[index], width, height)

    def _coverage_and_warnings(self) -> None:
        from . import Warning, _PARSER_WARNINGS
        from .coverage import Coverage
        from .fonts import RECORDED, Substitution

        grades, glyph_fallbacks, read_warnings = self.laid.coverage()
        substituted: list = []
        missing: list[str] = []
        for family, grade, substitute, compatible, kind, face_class in grades:
            if grade == "substituted":
                substituted.append(Substitution(family, substitute, compatible, kind, face_class))
            elif grade == "recorded":
                substituted.append(Substitution(family, RECORDED, True))
            elif grade == "missing":
                missing.append(family)
        # In the order docx2svg says them (``in_layout_order``): the faces the body's lines
        # draw first, in drawing order -- every page painted to read them, where two or more
        # are substituted -- then the rest as first looked up.
        from .coverage import in_layout_order

        substituted = in_layout_order(self, {s.family: s for s in substituted})
        warnings = [Warning(code, _PARSER_WARNINGS.get(code, "a body element is not read")) for code in read_warnings]
        warnings += [Warning("font-substituted", s.message()) for s in substituted]
        grouped: dict = {}
        for family, char, used in glyph_fallbacks:
            grouped.setdefault((family, used), []).append(char)
        for (family, used), chars in grouped.items():
            source = "its missing glyph (.notdef)" if used == ".notdef" else repr(used)
            warnings.append(Warning(
                "font-glyph-fallback",
                f"{family!r} has no glyph for {chars[0]!r} (U+{ord(chars[0]):04X})"
                + (f" and {len(chars) - 1} more character(s)" if len(chars) > 1 else "")
                + f": measured with {source}, as an approximation (Word's own font fallback is not measured)"))
        from .coverage import StopInfo

        approximations = [StopInfo(code, code.split(":", 1)[1], message, page, path)
                          for code, message, page, path in self.approximations]
        approximations += [StopInfo("layout-approximate:glyph", f"{family!r} has no glyph for {char!r}: measured with "
                                   f"{'its missing glyph' if used == '.notdef' else repr(used)}", None, None)
                          for family, char, used in glyph_fallbacks]
        warnings += [Warning(code, message, page) for code, message, page in self.laid.warnings()]
        count = self.laid.body_blocks
        pages = len(self.pages)
        self._warnings = warnings
        self._coverage = Coverage(not missing, pages, pages, "layout", count, count, 0, None, [],
                                  [s.as_dict() for s in substituted], sorted(missing), approximations, [])


def _sized(svg: str, page, width: int | None, height: int | None) -> str:
    """``svg`` with the size ``width`` and ``height`` ask for (``svg.render_page``'s)."""
    from . import svg as _svg

    natural = (f' width="{_svg.number(Fraction(page.width_pt).limit_denominator(1000))}pt"'
               f' height="{_svg.number(Fraction(page.height_pt).limit_denominator(1000))}pt"')
    if width is None and height is not None:
        width = round(Fraction(height * page.width_px, page.height_px))
    if height is None and width is not None:
        height = round(Fraction(width * page.height_px, page.width_px))
    return svg.replace(natural, f' width="{width}" height="{height}"', 1)


class RustPage:
    """A page of a :class:`RustLayout`, read as docx2svg's ``Page``: its frame, ``info`` and
    ``blocks`` at once; what it draws painted when first read."""

    stop = None
    defs = None
    body_end = None

    def __init__(self, layout: RustLayout, number: int, row) -> None:
        from .paginate import PageInfo
        from .units import DEVICE_DPI, POINTS_PER_INCH

        (page_number, section, section_first, story, blank), (width_px, height_px, _, _, box) = row
        self._layout = layout
        self.number = number
        self.width_px = width_px
        self.height_px = height_px
        # As docx2svg's ``device_page_extent_pt`` computes them (in floating point).
        self.width_pt = width_px / DEVICE_DPI * POINTS_PER_INCH
        self.height_pt = height_px / DEVICE_DPI * POINTS_PER_INCH
        self.column = tuple(_q(v) for v in box)
        self.info = PageInfo(page_number, section, section_first, story, blank)

    @property
    def blocks(self) -> list[str]:
        return self._layout.page_blocks(self.number)

    @property
    def lines(self) -> list:
        return self._layout.painted(self.number)["lines"]

    @property
    def rules(self) -> list:
        return self._layout.boxes(self.number)["rules"]

    @property
    def pictures(self) -> list:
        return self._layout.boxes(self.number)["pictures"]

    @property
    def floats(self) -> list:
        return self._layout.boxes(self.number)["floats"]

    @property
    def boxes(self) -> list:
        return []

    def layers(self) -> tuple[list, list]:
        ordered = sorted(self.floats, key=lambda f: f.order)
        return [f for f in ordered if f.behind], [f for f in ordered if not f.behind]

    def text_lines(self) -> list:
        """Every line of text the page draws, in paint order (docx2svg's ``Page.paint``), text
        boxes included: its lines and its drawings' text from one paint of the page."""
        painted = self._layout.painted(self.number)
        lines = painted["lines"]
        ordered = sorted(painted["floats"], key=lambda f: f.order)
        behind, front = [f for f in ordered if f.behind], [f for f in ordered if not f.behind]
        story_behind, story_front = [f for f in behind if f.story], [f for f in front if f.story]
        body_behind, body_front = [f for f in behind if not f.story], [f for f in front if not f.story]

        def boxed(floats: list) -> list:
            return [line for placed in floats for line in placed.lines]

        if not (story_behind or story_front or (body_behind and any(line.story for line in lines))):
            return boxed(body_behind) + list(lines) + boxed(body_front)
        return (boxed(story_behind) + [line for line in lines if line.story] + boxed(story_front)
                + boxed(body_behind) + [line for line in lines if not line.story] + boxed(body_front))


class _Xs(list):
    """A span's pens as exact lengths, each made a :class:`~fractions.Fraction` when the list
    is first read (lane L4b: most of a page's spans are never read for their pens, and their
    Fractions were most of what painting a page into Python cost).  A list in every other
    way: any read of it makes it first."""

    __slots__ = ("_raw",)

    def __init__(self, raw: Sequence) -> None:
        super().__init__()
        self._raw: "Sequence | None" = raw

    def _made(self) -> "_Xs":
        raw = object.__getattribute__(self, "_raw")
        if raw is not None:
            self._raw = None
            list.extend(self, (_q(x) for x in raw))
        return self

    def __len__(self) -> int:
        raw = object.__getattribute__(self, "_raw")
        return len(raw) if raw is not None else list.__len__(self)

    def __reduce_ex__(self, protocol):
        return (list, ([*list.__iter__(self._made())],))

    def __copy__(self) -> list:
        return [*list.__iter__(self._made())]

    __hash__ = None  # type: ignore[assignment]


def _made_first(name: str):
    method = getattr(list, name)

    def call(self, *args):
        return method(self._made(), *args)

    call.__name__ = name
    return call


for _name in ("__getitem__", "__iter__", "__reversed__", "__contains__", "__eq__", "__ne__", "__lt__",
              "__le__", "__gt__", "__ge__", "__add__", "__mul__", "__rmul__", "__iadd__", "__imul__",
              "__setitem__", "__delitem__", "__repr__", "__str__", "append", "extend", "insert", "pop", "remove",
              "index", "count", "copy", "sort", "reverse", "clear"):
    if hasattr(list, _name):
        setattr(_Xs, _name, _made_first(_name))


def _paint(view: dict) -> dict:
    """A page's paint as docx2svg's ``Line``, ``Span``, ``Rule``, ``Picture`` and ``Float``."""
    from .drawing import Primitive
    from .layout import Float, Line, Picture, Rule, Span

    def made(row: Sequence) -> Line:
        path, number, baseline, top, pitch, paragraph_id, band, story, part, column, spans = row
        line = Line(path, number, baseline, _q(top), _q(pitch), paragraph_id=paragraph_id, band=tuple(band),
                    story=story, part=part, column=column)
        for (span_path, chars, xs, y, end, face, bold, italic, half_points, color, underline, strike,
             double_strike, highlight, shading, kind, line_baseline, vertical_align) in spans:
            line.spans.append(Span(span_path, list(chars), _Xs(xs), y, _q(end), face, bold, italic,
                                   half_points, color=color, underline=underline, strike=strike,
                                   double_strike=double_strike, highlight=highlight, shading=shading, kind=kind,
                                   line_baseline=line_baseline, vertical_align=vertical_align))
        return line

    lines = [made(row) for row in view["lines"]]
    rules = [Rule(kind, path, _q(x), _q(y), _q(w), _q(h), color, style)
             for kind, path, x, y, w, h, color, style in view["rules"]]
    pictures = [Picture(path, relationship, _q(x), _q(y), _q(w), _q(h), part=part,
                        crop=tuple(_q(c) for c in crop) if crop is not None else None)
                for path, relationship, x, y, w, h, part, crop in view["pictures"]]
    floats = []
    for slot in view["drawings"]:
        placed = Float(slot["path"], _q(slot["x"]), _q(slot["y"]), _q(slot["width"]), _q(slot["height"]),
                       slot["behind"], tuple(slot["order"]),
                       story=slot["story"][0] if slot["story"] else None,
                       part=slot["story"][1] if slot["story"] else None)
        # What its text is drawn in (a text path's), for the rasteriser's faces (``drawn_faces``).
        placed.primitives = [Primitive("markup", slot["path"], face=tuple(face)) for face in slot["faces"]]
        # Its text boxes' lines, in page coordinates (none where the page was read without lines).
        placed.lines = [made(row) for row in slot.get("lines", ())]
        floats.append(placed)
    return {"lines": lines, "rules": rules, "pictures": pictures, "floats": floats}


def svgs(layout: RustLayout, options) -> "list[str] | None":
    """The pages ``options.pages`` selects of ``layout`` as SVG (:func:`docx2svg.page_svgs`),
    or ``None`` where one of them is not drawn in Rust as docx2svg draws it."""
    from . import _selected

    out = []
    for index in _selected(options, len(layout.pages)):
        svg = layout.page_svg(index, width=options.width, height=options.height)
        if svg is None:
            return None
        out.append(svg)
    return out


__all__: Sequence[str] = ("Engine", "RustLayout", "RustPage", "available", "count_fallback", "fallbacks",
                          "memory_budget", "svgs", "unsupported_options", "word_fonts")


# -- a page's PNG ----------------------------------------------------------------------------

#: The faces pages are drawn with (``ooxml_core.PageRaster``), read once, by what they were
#: read from: the files and folders handed over (with each file's modification time and
#: size: a font changed is read again), the system's fonts or not, the ``sans-serif`` face.
_RASTERS: OrderedDict = OrderedDict()
_RASTERS_LIMIT = 8
_RASTERS_LOCK = threading.Lock()


def page_png(svg: str, *, font_dirs: Sequence[str] | None = None, font_files: Sequence[str] | None = None,
             skip_system_fonts: bool = False, sans_serif_family: str | None = None, width: int | None = None,
             height: int | None = None, scale: float | None = None, background: str | None = None) -> bytes:
    """``svg`` (a page the Rust layout drew) as PNG, drawn in Rust as :func:`docx2svg.png.svg_to_png`
    draws it with resvg-py -- the same faces, a metafile's picture inline, the drawing plan's
    faces -- pixel for pixel.  ``font_dirs`` left out reads ``OOXML_FONT_DIRS``."""
    from ooxml_common.fonts.office import user_font_dirs
    from ooxml_common.fonts.rasterdb import file_stamps

    files = [str(path) for path in font_files or ()]
    dirs = [str(path) for path in user_font_dirs(font_dirs)]
    key = (tuple(file_stamps(files, dirs)), tuple(files), tuple(dirs), bool(skip_system_fonts), sans_serif_family)
    with _RASTERS_LOCK:
        raster = _RASTERS.get(key)
        if raster is not None:
            _RASTERS.move_to_end(key)
    if raster is None:
        raster = _core().PageRaster(files, dirs, skip_system_fonts=bool(skip_system_fonts),
                                    sans_serif_family=sans_serif_family)
        with _RASTERS_LOCK:
            raster = _RASTERS.setdefault(key, raster)
            while len(_RASTERS) > _RASTERS_LIMIT:
                _RASTERS.popitem(last=False)
    return raster.png(svg, width=width, height=height, scale=scale, background=background)
