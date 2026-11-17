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
SVG the Rust paint does not draw as docx2svg does (a drawing for the painters of phase P3,
line numbers...) is the same: :meth:`RustLayout.page_svg` answers ``None`` for it, counted.

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
from typing import TYPE_CHECKING, Any, Callable, Sequence

if TYPE_CHECKING:
    from .coverage import Coverage

#: Why versions and pages were not laid out or drawn in Rust, counted by reason
#: (``layout_fallbacks{reason}``): read by :func:`fallbacks`; never in a result.
_FALLBACKS: Counter = Counter()
_FALLBACKS_LOCK = threading.Lock()

#: How many pages' paint a layout keeps for its readers (the rest painted again when read).
PAINTED_PAGES = 24


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


# -- the engine ------------------------------------------------------------------------------


class Engine:
    """One document's layouts in Rust (``ooxml_core.WordPages``), each version laid out
    from the one before; safe to share between threads (the Rust engine locks itself)."""

    def __init__(self, options) -> None:
        ooxml_core = _core()

        self.key = (_fonts_key(options), options.locale.decimal_separator if options.locale else ".",
                    options.glyph_size)
        self._pages = ooxml_core.WordPages(word_fonts(options), decimal=self.key[1], glyph_size=options.glyph_size)
        self._lock = threading.Lock()
        #: The last version asked for, by the digest of its bytes, and its layout (``None``: a
        #: fallback): asked again, the same answer.
        self._last: tuple | None = None

    def fits(self, options) -> bool:
        """Whether this engine lays out with ``options`` (the same fonts, decimal, glyphs)."""
        return self.key == (_fonts_key(options), options.locale.decimal_separator if options.locale else ".",
                            options.glyph_size)

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
        baseline, column)``, exact -- what an index of where blocks are drawn reads, with
        no page painted."""
        return [[(path, number, _q(top), _q(pitch), baseline, column)
                 for path, number, top, pitch, baseline, column in page] for page in self.laid.body_lines()]

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

    def page_svg(self, index: int, *, painter: Callable | None = None, width: int | None = None,
                 height: int | None = None) -> str | None:
        """Page ``index`` (0-based) as SVG, byte for byte docx2svg's; ``None`` (counted) where
        the Rust paint does not draw it as docx2svg does yet."""
        svg, missing = self.laid.svg(index, painter)
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
        return self._layout.painted(self.number)["rules"]

    @property
    def pictures(self) -> list:
        return self._layout.painted(self.number)["pictures"]

    @property
    def floats(self) -> list:
        return self._layout.painted(self.number)["floats"]

    @property
    def boxes(self) -> list:
        return []

    def layers(self) -> tuple[list, list]:
        ordered = sorted(self.floats, key=lambda f: f.order)
        return [f for f in ordered if f.behind], [f for f in ordered if not f.behind]

    def text_lines(self) -> list:
        """Every line of text the page draws, in paint order (docx2svg's ``Page.paint``)."""
        lines = self.lines
        behind, front = self.layers()
        if not ([f for f in behind if f.story] or [f for f in front if f.story]
                or ([f for f in behind if not f.story] and any(line.story for line in lines))):
            return list(lines)
        return [line for line in lines if line.story] + [line for line in lines if not line.story]


def _paint(view: dict) -> dict:
    """A page's paint as docx2svg's ``Line``, ``Span``, ``Rule``, ``Picture`` and ``Float``."""
    from .layout import Float, Line, Picture, Rule, Span

    lines = []
    for path, number, baseline, top, pitch, paragraph_id, band, story, part, column, spans in view["lines"]:
        line = Line(path, number, baseline, _q(top), _q(pitch), paragraph_id=paragraph_id, band=tuple(band),
                    story=story, part=part, column=column)
        for (span_path, chars, xs, y, end, face, bold, italic, half_points, color, underline, strike,
             double_strike, highlight, shading, kind, line_baseline, vertical_align) in spans:
            line.spans.append(Span(span_path, list(chars), [_q(x) for x in xs], y, _q(end), face, bold, italic,
                                   half_points, color=color, underline=underline, strike=strike,
                                   double_strike=double_strike, highlight=highlight, shading=shading, kind=kind,
                                   line_baseline=line_baseline, vertical_align=vertical_align))
        lines.append(line)
    rules = [Rule(kind, path, _q(x), _q(y), _q(w), _q(h), color, style)
             for kind, path, x, y, w, h, color, style in view["rules"]]
    pictures = [Picture(path, relationship, _q(x), _q(y), _q(w), _q(h), part=part,
                        crop=tuple(_q(c) for c in crop) if crop is not None else None)
                for path, relationship, x, y, w, h, part, crop in view["pictures"]]
    floats = [Float(slot["path"], _q(slot["x"]), _q(slot["y"]), _q(slot["width"]), _q(slot["height"]),
                    slot["behind"], tuple(slot["order"]),
                    story=slot["story"][0] if slot["story"] else None,
                    part=slot["story"][1] if slot["story"] else None)
              for slot in view["drawings"]]
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
                          "svgs", "unsupported_options", "word_fonts")
