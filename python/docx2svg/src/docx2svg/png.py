"""SVG to PNG, drawn by the Rust core (``ooxml_core.PageRaster``: resvg, in process), and an
SVG's text as glyph outlines (:func:`outline_svg`).

**Fonts.**  The SVG names its faces and embeds none.  Word lays a document out with
Microsoft's faces, so a raster is only faithful where the rasteriser draws with the same
files: the host's installed fonts, plus any ``font_dirs`` / ``font_files`` given (a
document's embedded faces among them).  A face the host lacks is reported by the layout,
not here.
"""

from __future__ import annotations

import html
from dataclasses import dataclass
from typing import Sequence

from ooxml_common.raster import RasterizerNotAvailable  # noqa: F401  (this module's public name, kept)

from . import rust

#: The rasterisers :func:`svg_to_png` takes: the Rust core's resvg.
BACKENDS = ("auto", "resvg")


def available_backends() -> list[str]:
    """The rasterisers installed: the Rust core's resvg, always (``ooxml_core`` is a
    dependency)."""
    return ["resvg"]


def svg_to_png(
    svg: str,
    *,
    width: int | None = None,
    height: int | None = None,
    scale: float | None = None,
    background: str | None = None,
    backend: str = "auto",
    font_dirs: Sequence[str] | None = None,
    font_files: Sequence[str] | None = None,
    skip_system_fonts: bool = False,
    sans_serif_family: str | None = None,
) -> bytes:
    """Rasterise one SVG document to PNG bytes (``ooxml_core.PageRaster``).

    ``width``/``height`` set the output size in pixels (give one and the aspect ratio is
    kept); ``scale`` multiplies the SVG's own size instead.  ``font_files`` and the files
    under ``font_dirs`` are handed over after the host's fonts, which ``skip_system_fonts``
    leaves out; ``sans_serif_family`` names the face the generic ``sans-serif`` means (the
    placeholders' labels use it).  ``font_dirs`` left out reads ``OOXML_FONT_DIRS``
    (``os.pathsep``-separated); an empty list means none.  ``backend``: ``"auto"`` or
    ``"resvg"``; another is refused."""
    from ooxml_common.fonts.office import user_font_dirs

    if backend not in BACKENDS:
        raise ValueError(f"backend {backend!r}: SVG is drawn by the Rust core's resvg; give backend='auto' "
                         "(or 'resvg')")
    dirs = [str(path) for path in user_font_dirs(font_dirs)]
    files = [str(path) for path in font_files or ()]
    raster = rust.page_raster(files, dirs, skip_system_fonts=skip_system_fonts,
                              sans_serif_family=sans_serif_family)
    return raster.png(svg, width=width, height=height, scale=scale, background=background)


@dataclass(frozen=True)
class DrawingPlan:
    """How an SVG whose text asks for two faces the rasteriser files under one name is
    drawn (``ooxml_core.drawing_plan``): in passes, each over the last.  ``stacks`` gives each
    ``font-family`` value drawn after the first pass its pass; ``withheld`` per pass, the
    files left out of it."""

    stacks: dict
    withheld: list

    @property
    def passes(self) -> int:
        return len(self.withheld)


def drawing_plan(svg: str, font_files: Sequence[str] = (), font_dirs: Sequence[str] = ()) -> DrawingPlan | None:
    """The passes ``svg`` is drawn in with ``font_files`` and the files under ``font_dirs``
    (``ooxml_core.drawing_plan``), or ``None``: one pass draws every text in its own face."""
    found = rust.core().drawing_plan(svg, list(font_files), list(font_dirs))
    if found is None:
        return None
    stacks, withheld = found
    return DrawingPlan(dict(stacks), [set(files) for files in withheld])


def _pass_fonts(plan: DrawingPlan, number: int, font_files, font_dirs) -> tuple:
    """``(font_files, font_dirs)`` handed over for pass ``number`` of ``plan``: the files and
    folders as given, or, where the pass leaves files out, the files of both in the order
    resvg loads them, without those."""
    from ooxml_common.fonts.rasterdb import load_order

    withheld = plan.withheld[number]
    if not withheld:
        return font_files, font_dirs
    files = [path for path in load_order([str(path) for path in font_files or ()],
                                         [str(path) for path in font_dirs or ()]) if path not in withheld]
    return files, []


def outline_svg(svg: str, *, font_dirs: Sequence[str] | None = None, font_files: Sequence[str] | None = None,
                skip_system_fonts: bool = False, sans_serif_family: str | None = None, selectable: bool = False,
                glyphs: dict | None = None):
    """``svg`` with its text drawn as glyph outlines: the faces, fallbacks, shaping and
    positions resvg draws it with when :func:`svg_to_png` is given the same arguments,
    passes included (:func:`ooxml_common.outlines.outline_text`), so the SVG needs no font
    to be viewed.  ``selectable`` keeps each text, transparent, over its outlines;
    ``glyphs``, a dict shared by a document's pages, takes the glyph definitions in place of
    the SVG's own ``<defs>``.  Returns an :class:`ooxml_common.outlines.Outlined`: the SVG
    and the texts kept as text, with why.  ``font_dirs`` left out reads
    ``OOXML_FONT_DIRS``."""
    from ooxml_common.fonts.office import user_font_dirs
    from ooxml_common.metafile import inline_metafile_images
    from ooxml_common.outlines import FontSetup, outline_text

    # A metafile drawn as SVG goes inline, as svg_to_png draws it: its text is outlined too.
    svg = inline_metafile_images(svg)
    dirs = [str(path) for path in user_font_dirs(font_dirs)]
    files = [str(path) for path in font_files or ()]
    families = {"sans_serif_family": sans_serif_family} if sans_serif_family else None
    plan = drawing_plan(svg, files, dirs)
    if plan is None:
        return outline_text(svg, FontSetup.of(files, dirs, not skip_system_fonts, families), selectable=selectable,
                            glyphs=glyphs)
    setups = []
    for number in range(plan.passes):
        pass_files, pass_dirs = _pass_fonts(plan, number, files, dirs)
        setups.append(FontSetup.of(pass_files, pass_dirs, not skip_system_fonts, families))
    stacks = {html.unescape(value): number for value, number in plan.stacks.items()}
    return outline_text(svg, lambda value: setups[stacks.get(value or "", 0)], selectable=selectable,
                        glyphs=glyphs)
