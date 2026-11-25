"""docx2svg -- render Word documents to SVG and PNG.

Typical use::

    from docx2svg import convert_docx_to_svg, convert_docx_to_png

    svgs = convert_docx_to_svg("report.docx")          # one SVG string per page
    pngs = convert_docx_to_png("report.docx")
    both = convert_docx("report.docx")                  # the layout and the SVGs, laid out once

A thin layer over the Rust core (``ooxml_core``): the document is laid out and drawn by the
same engine docx-agent's ``render`` tool draws with (:mod:`docx2svg.rust` says what it is
made with), so a page is the same SVG and PNG either way.  What the layout cannot lay out
yet it refuses: :class:`LayoutError` (a :class:`NotImplementedError`) says what and where,
and what to do instead.  An option the layout does not take is refused the same way
(``ValueError``), never ignored.

Every drawn element's ``data-docx-path`` names where it came from; :func:`resolve_path`
finds the element it names in the part's XML.  ``ConvertOptions.warnings`` and
``ConvertOptions.coverage`` say what is not drawn as Word draws it, and how much of the
document was laid out by measured rules.
"""

from __future__ import annotations

import os
import tempfile
from contextlib import contextmanager
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Iterator, Sequence

from ooxml_common.locales import EN_US, Locale

from . import rust
from .errors import LayoutError
from .paths import resolve_path
from .png import RasterizerNotAvailable, available_backends, svg_to_png

if TYPE_CHECKING:
    from .coverage import Coverage

__version__ = "0.15.0"

__all__ = [
    "Conversion",
    "ConvertOptions",
    "LayoutError",
    "RasterizerNotAvailable",
    "Warning",
    "available_backends",
    "convert_docx",
    "convert_docx_to_png",
    "convert_docx_to_svg",
    "embed_glyphs",
    "glyph_sheet",
    "resolve_path",
    "svg_to_png",
    "user_font_dirs",
    "__version__",
]

#: The names withdrawn when the Python layout went (lane svg-thin), each with what to use
#: instead: asking for one says so (:func:`__getattr__`), for one release.
WITHDRAWN = {
    **dict.fromkeys(
        ("lay_out_docx", "convert_docx_to_layout", "page_svgs", "parsed_document", "drawn_faces"),
        "lay the document out with the Rust engine and draw its pages from it: "
        "docx2svg.rust.word_pages(options).lay_out(data) gives an ooxml_core.LaidPages "
        "(.svg(page), .pngs(pages, raster), .pages(), .page(n), .coverage_summary()); "
        "WordPages.update(parts) lays a changed version out from the last one; "
        "convert_docx returns the LaidPages as Conversion.layout"),
    **dict.fromkeys(
        ("warm", "scan_fonts"),
        "nothing: the Rust engine reads the font folders once per process and set of options "
        "(docx2svg.rust.word_fonts)"),
    **dict.fromkeys(
        ("Document", "Paragraph", "Run", "Section", "PageMargins", "PageSize", "PageInfo", "Package",
         "parse_document", "parse_package"),
        "read the document with docx-agent (word_read, word_inspect), or its pages from "
        "ooxml_core.LaidPages.pages()"),
}


def __getattr__(name: str) -> Any:
    instead = WITHDRAWN.get(name)
    if instead is not None:
        raise AttributeError(f"docx2svg.{name} was withdrawn with docx2svg's Python layout; instead: {instead}")
    raise AttributeError(f"module 'docx2svg' has no attribute {name!r}")


@dataclass(frozen=True)
class Warning:  # noqa: A001 -- the sibling project's name for the same thing
    """Something the output does not show faithfully, with a stable ``code`` a caller can
    fail a build on (``layout-placeholder:drawing``, ``layout-approximate:table``,
    ``font-substituted``...)."""

    code: str
    message: str
    page: int | None = None

    def __str__(self) -> str:
        where = f" (page {self.page})" if self.page else ""
        return f"[{self.code}]{where} {self.message}"


#: A page drawn, but not yet as Word draws it: its warning's code.
NOT_AS_WORD = "drawn-not-as-word"


@dataclass
class ConvertOptions:
    """Knobs shared by the SVG and PNG entry points."""

    #: 1-based page numbers; ``None`` renders every page.
    pages: Sequence[int] | None = None
    #: Output size in pixels.  For PNG the page is drawn at 300 dpi when both are ``None``;
    #: for SVG they replace the ``width``/``height`` attributes (in points by default).  Give
    #: one and the aspect ratio is kept.
    width: int | None = None
    height: int | None = None
    #: What glyph outlines are scaled to: ``device`` (the size rounded to whole 1/300-inch
    #: pixels, as Word draws its ink) or ``exact``.  Pen positions are the same either way.
    glyph_size: str = "device"
    #: The application's own font folders (and the folders under them), searched after the
    #: ones Word uses, for layout and drawing alike.  ``None`` (the default) reads the
    #: ``OOXML_FONT_DIRS`` environment variable (``os.pathsep``-separated, shared with
    #: pptx2svg); an empty list means none, the variable not read.
    font_dirs: Sequence[str] | None = None
    #: Not taken (the layout measures with the font files themselves): give ``font_dirs`` or
    #: ``font_substitutes`` instead.  A value other than ``None`` is refused (``ValueError``).
    advances: Any = None
    metrics: Any = None
    decorations: Any = None
    names: Any = None
    #: Lay out and draw an absent face with its open metric compatible substitute (Carlito
    #: for Calibri, Liberation Sans, Serif and Mono for Arial, Times New Roman and Courier
    #: New) where that is installed, and an absent face with no measured substitute with an
    #: approximate one.  Never in place of a face that is present; every use is a
    #: ``font-substituted`` warning and is listed in :attr:`coverage`.  ``False``: none of
    #: these.
    substitute_fonts: bool = True
    #: More substitutions, or other ones: Word's face -> the face to use when it is absent
    #: (``{"Cambria": "Caladea"}``).  One not measured compatible is reported as approximate.
    font_substitutes: dict | None = None
    #: The regional settings Word would take from its machine, which a document does not
    #: state (:class:`ooxml_common.locales.Locale`): the character a decimal tab stop aligns
    #: on and the separators of a chart's numbers.  Always
    #: :data:`~ooxml_common.locales.EN_US` unless given -- never read from the operating
    #: system, so a document draws the same on every server.
    locale: Locale = EN_US
    #: How text is written: ``"text"`` (the default) as ``<text>``, drawn by the viewer with
    #: its own fonts; ``"outlines"`` as the paths of the glyphs the PNG is drawn with
    #: (:func:`ooxml_common.outlines.outline_text`), so the SVG needs no font to be viewed.
    #: A text that cannot be drawn so (right-to-left or complex-script text, a colour face)
    #: stays text, with a ``text-not-outlined`` warning.  PNG output is the same either way.
    text: str = "text"
    #: With ``text="outlines"``: each text also kept, transparent, over its outlines, so a
    #: viewer can select, search and copy it.
    selectable_text: bool = False
    #: With ``text="outlines"``: one glyph set for the whole document.  A dict, and every
    #: glyph definition goes into it, by id, instead of into each page's ``<defs>``.  Put
    #: :func:`glyph_sheet` of it once in the HTML page that shows the pages inline;
    #: :func:`embed_glyphs` makes one page standalone again.
    glyphs: dict | None = None
    #: Collects :class:`Warning` for everything not drawn faithfully.
    warnings: list[Warning] = field(default_factory=list)
    #: Set by every conversion: how much of the document was laid out
    #: (:class:`docx2svg.coverage.Coverage`).
    coverage: Coverage | None = None


#: ``ConvertOptions.text``'s values.
TEXT_MODES = ("text", "outlines")


def _read(source) -> bytes:
    if isinstance(source, (bytes, bytearray)):
        return bytes(source)
    if isinstance(source, (str, os.PathLike)):
        with open(os.fspath(source), "rb") as handle:
            return handle.read()
    return source.read()


def user_font_dirs(options: ConvertOptions | None = None) -> list[str]:
    """The application's font folders a conversion with ``options`` uses, as strings:
    ``options.font_dirs``, else ``OOXML_FONT_DIRS``
    (:func:`ooxml_common.fonts.office.user_font_dirs`) -- what :func:`convert_docx_to_png`
    hands the rasteriser after the faces it names."""
    from ooxml_common.fonts.office import user_font_dirs as _user

    return [str(path) for path in _user((options or ConvertOptions()).font_dirs)]


def glyph_sheet(glyphs: dict) -> str:
    """The glyph set of pages outlined with ``ConvertOptions(glyphs={})`` as one ``<svg>``
    element that draws nothing and takes no room, to put once in the HTML page that shows
    them inline (:func:`ooxml_common.outlines.glyph_sheet`)."""
    from ooxml_common.outlines import glyph_sheet as _glyph_sheet

    return _glyph_sheet(glyphs)


def embed_glyphs(svg: str, glyphs: dict) -> str:
    """A page outlined with ``ConvertOptions(glyphs={})`` with the glyph definitions it uses
    put back into it: standalone again (:func:`ooxml_common.outlines.embed_glyphs`)."""
    from ooxml_common.outlines import embed_glyphs as _embed_glyphs

    return _embed_glyphs(svg, glyphs)


def _laid(source, options: ConvertOptions):
    """The document laid out by the Rust engine with ``options`` (an ``ooxml_core.LaidPages``),
    its coverage and warnings said in ``options``.  Raises :class:`LayoutError` for a document
    the layout does not lay out yet, ``ValueError`` for options it does not take."""
    if options.text not in TEXT_MODES:
        raise ValueError(f"ConvertOptions.text must be one of {', '.join(TEXT_MODES)}, not {options.text!r}")
    engine = rust.word_pages(options)
    try:
        laid = engine.lay_out(_read(source))
    except NotImplementedError as why:
        raise LayoutError(str(why)) from why
    from .coverage import coverage_from

    summary = laid.coverage_summary()
    options.coverage = coverage_from(summary)
    for code, message, page in summary["warnings"]:
        options.warnings.append(Warning(code, message, page))
    return laid


def _selected(options: ConvertOptions, count: int) -> list[int]:
    """The 0-based pages ``options.pages`` selects of ``count``."""
    if options.pages is None:
        return list(range(count))
    return [number - 1 for number in options.pages if 1 <= number <= count]


def _not_as_word(options: ConvertOptions, index: int, missing: Sequence[str]) -> None:
    """Says of page ``index`` (0-based) what it does not draw as Word draws it."""
    if missing:
        reasons = ", ".join(dict.fromkeys(missing))
        options.warnings.append(Warning(NOT_AS_WORD, f"drawn, but not yet as Word draws it ({reasons})", index + 1))


def _svgs(laid, options: ConvertOptions, indexes: list[int]) -> list[str]:
    """The pages ``indexes`` (0-based) of ``laid`` as SVG, as ``options`` asks."""
    documents = []
    for index in indexes:
        svg, missing = laid.svg(index, options.width, options.height)
        _not_as_word(options, index, missing)
        documents.append(svg)
    if options.text == "outlines":
        documents = _outlined(laid, documents, indexes, options)
    return documents


@contextmanager
def _raster_files(laid, indexes: list[int]) -> Iterator[tuple[list[str], str | None, bool]]:
    """``(files, sans-serif family, skip system fonts)`` the rasteriser is handed for pages
    ``indexes`` of ``laid``, for as long as the block runs: a face the document embeds is
    written, de-obfuscated, to a temporary folder meanwhile."""
    found, sans, skip, _ = laid.raster_files(indexes)
    embedded = [face for face in found if isinstance(face, bytes)]
    if not embedded:
        yield list(found), sans, skip
        return
    with tempfile.TemporaryDirectory(prefix="docx2svg-embedded-") as directory:
        files = []
        for number, face in enumerate(found):
            if isinstance(face, bytes):
                path = os.path.join(directory, f"embedded-{number}.ttf")
                with open(path, "wb") as handle:
                    handle.write(face)
                files.append(path)
            else:
                files.append(face)
        yield files, sans, skip


def _outlined(laid, documents: list[str], indexes: list[int], options: ConvertOptions) -> list[str]:
    """The pages with their text as outlines, drawn with the files :func:`convert_docx_to_png`
    hands the rasteriser; every kind of text kept as text reported once, as
    ``text-not-outlined``."""
    from .png import outline_svg

    out = []
    reported: set = set()
    with _raster_files(laid, indexes) as (files, sans, skip):
        dirs = user_font_dirs(options)
        for document, index in zip(documents, indexes):
            outlined = outline_svg(document, font_dirs=dirs, font_files=files, skip_system_fonts=skip,
                                   sans_serif_family=sans, selectable=options.selectable_text, glyphs=options.glyphs)
            out.append(outlined.svg)
            for kept in outlined.kept:
                if (kept.reason, kept.family) not in reported:
                    reported.add((kept.reason, kept.family))
                    options.warnings.append(Warning("text-not-outlined", kept.message(), index + 1))
    return out


def convert_docx_to_svg(source, options: ConvertOptions | None = None) -> list[str]:
    """Render a document to one SVG document per page (those ``options.pages`` selects)."""
    return convert_docx(source, options).svgs


@dataclass
class Conversion:
    """A document laid out once and drawn: what :func:`convert_docx` returns."""

    #: Every page laid out, whatever ``pages`` selects: an ``ooxml_core.LaidPages``
    #: (``.page_count``, ``.pages()``, ``.page(n)``, ``.svg(n)``, ``.coverage_summary()``).
    layout: Any
    #: The selected pages' SVG documents, exactly as :func:`convert_docx_to_svg` writes them.
    svgs: list[str]
    #: The 1-based number of the page each SVG is of.
    page_numbers: list[int]


def convert_docx(source, options: ConvertOptions | None = None) -> Conversion:
    """Lay a document out once and return both the layout and the selected pages' SVGs.
    Warnings are collected in ``options.warnings`` as by :func:`convert_docx_to_svg`."""
    options = options or ConvertOptions()
    laid = _laid(source, options)
    indexes = _selected(options, laid.page_count)
    return Conversion(laid, _svgs(laid, options, indexes), [index + 1 for index in indexes])


#: The rasterisers :func:`convert_docx_to_png` draws with: the Rust core's resvg.
BACKENDS = ("auto", "resvg")


def convert_docx_to_png(source, options: ConvertOptions | None = None, *, backend: str = "auto",
                        font_dirs: Sequence[str] | None = None, font_files: Sequence[str] | None = None,
                        skip_system_fonts: bool | None = None) -> list[bytes]:
    """Render a document to one PNG per page, as docx-agent's ``render`` tool draws it.

    The rasteriser is given the files Word draws every face with, each read in place, and
    the faces the document embeds, then ``font_files`` and the folders ``font_dirs`` and
    :func:`user_font_dirs` name.  ``skip_system_fonts`` defaults to on when every face drawn
    was found.  Without ``width``/``height`` a page is drawn at 300 dpi, Word's export grid.
    ``backend``: ``"auto"`` or ``"resvg"`` (the Rust core's); another is refused."""
    options = options or ConvertOptions()
    _check_backend(backend)
    laid = _laid(source, options)
    return _pngs(laid, options, _selected(options, laid.page_count), font_dirs=font_dirs, font_files=font_files,
                 skip_system_fonts=skip_system_fonts)


def _check_backend(backend: str) -> None:
    if backend not in BACKENDS:
        raise ValueError(f"backend {backend!r}: pages are drawn by the Rust core's resvg; give backend='auto' "
                         "(or 'resvg'), or rasterise convert_docx_to_svg's pages with a rasteriser of your own")


def _pngs(laid, options: ConvertOptions, indexes: list[int], *, font_dirs: Sequence[str] | None = None,
          font_files: Sequence[str] | None = None, skip_system_fonts: bool | None = None) -> list[bytes]:
    """The pages ``indexes`` (0-based) of ``laid`` as PNG, as :func:`convert_docx_to_png` draws them."""
    dirs = [str(path) for path in font_dirs or ()] + user_font_dirs(options)
    raster = rust.page_raster([str(path) for path in font_files or ()], dirs,
                              skip_system_fonts=skip_system_fonts is True)
    system = None if skip_system_fonts is None else not skip_system_fonts
    drawn = laid.pngs(indexes, raster, width=options.width, height=options.height, system=system)
    for index, (_, missing) in zip(indexes, drawn):
        _not_as_word(options, index, missing)
    return [png for png, _ in drawn]
