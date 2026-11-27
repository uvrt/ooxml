"""Resolved slides kept by what they were resolved from: :class:`ModelCache`.

A program that converts the same deck again and again -- after every edit, to measure or
draw one slide -- resolves mostly the same slides each time.  A slide's resolution reads
its own part and what that part's relationships reach (its layout, the master, the
master's other layouts, the theme, pictures, charts, diagrams), the presentation's own
settings (slide size, default text style, table styles, embedded font list, first slide
number, the application version) and the slide's place in the deck -- nothing of another
slide.  So a resolved slide is kept under a key made of exactly those: the bytes of every
part in that reach (and of their relationships), their content types, the presentation's
settings as read, the slide's number, id and part, and the conversion's locale and font
choice.  Equal key, equal slide: a conversion handed ``ConvertOptions(cache=...)``
resolves only the slides whose key it has not seen, and gives back for the others the
very objects it resolved before -- which therefore must be treated as read-only.

Not kept:

* a slide on which a date field (``a:fld type="datetime…"``) was drawn: it shows the
  conversion's clock (``ConvertOptions.now``), so it is resolved every time;
* every slide of a conversion with a ``metafile_converter`` (a callable; its answers are
  not known to depend on the bytes alone) or with ``templates`` (layout and master
  views): such a conversion runs as without a cache.

Layouts, masters and themes are parsed once per their bytes, too (:meth:`ModelCache.parsed`),
so decks made from one template share those parses.

**What it holds, and its bounds.**  A resolved slide's pictures are base64 text, and a
picture is most of what a slide with one weighs.  The texts are kept apart from the slides,
once per picture (by the digest of its bytes; :mod:`pptx2svg.images`), for as long as a
kept slide holds them: a layout's picture under every slide, or a picture unchanged across
400 versions of a deck, is one string.  Each slide kept is weighed (what its objects take,
its pictures' texts left out), and the cache keeps the slides' weights and their pictures'
texts together within ``budget`` bytes and at most ``slides`` slides, dropping the least
recently used; a slide that alone would not fit is given back but not kept.  Parsed parts
are bounded by count (``parts``).  :meth:`ModelCache.stats` says what is held; every store
is safe to share between threads.
"""

from __future__ import annotations

import gc
import hashlib
import sys
import threading
from collections import OrderedDict
from dataclasses import dataclass
from datetime import datetime
from enum import Enum
from types import BuiltinFunctionType, FunctionType, MethodType, ModuleType
from typing import Callable, Iterable, NamedTuple

from . import model as m
from .opc import REL_SLIDE, OpcPackage, rels_path_for
from .images import ImageTexts, encode
from .parse.parts import read_presentation, slide_entries

__all__ = ["CacheStats", "ModelCache"]

#: What :class:`ModelCache` keeps by default: resolved slides, and parsed layouts, masters
#: and themes, by count; and the bytes the resolved slides and their pictures take.
SLIDES = 512
PARTS = 256
BUDGET = 256 * 2 ** 20


def _digest(data: bytes | None) -> bytes:
    return b"-" if data is None else hashlib.blake2b(data, digest_size=16).digest()


@dataclass(frozen=True)
class CacheStats:
    """What a :class:`ModelCache` holds, and how it has been used."""

    #: Resolved slides kept, and the bytes they take, their pictures left out.
    entries: int
    entry_bytes: int
    #: Pictures the kept slides hold (each once), and the bytes of their base64 text.
    images: int
    image_bytes: int
    #: Parsed layouts, masters and themes kept.
    parts: int
    #: Slides given back from the cache, and slides resolved, since it was made.
    hits: int
    misses: int
    #: The bounds: bytes (``entry_bytes + image_bytes``) and resolved slides.
    budget: int
    limit: int

    @property
    def bytes(self) -> int:
        """All the cache holds for its slides: ``entry_bytes + image_bytes``."""
        return self.entry_bytes + self.image_bytes


class _Picture:
    """A picture's text in the cache's store, and who holds it."""

    __slots__ = ("text", "size", "kept", "using")

    def __init__(self, text: str) -> None:
        self.text = text
        self.size = sys.getsizeof(text)
        #: Kept slides that hold it, and conversions under way that took it.
        self.kept = 0
        self.using = 0


class _Kept(NamedTuple):
    #: ``(slide, warnings, font scheme)``, as :meth:`ModelCache.resolve` reads it.
    entry: tuple
    weight: int
    #: The digests of the pictures it holds (:attr:`ModelCache._images`).
    images: tuple[bytes, ...]


class ModelCache:
    """Resolved slides, and parsed layouts, masters and themes, by what they are made of
    (see the module docstring).  Hand one to every conversion that should share it::

        cache = ModelCache()
        options = ConvertOptions(slide_numbers=[3], cache=cache)
        model = convert_pptx_to_model(deck_bytes, options)

    ``budget`` (bytes) and ``slides`` bound the resolved slides kept, with their pictures;
    ``parts`` the parsed parts (see the module docstring).  A slide given back from it is
    the object an earlier conversion returned: read it, do not change it."""

    def __init__(self, slides: int = SLIDES, parts: int = PARTS, budget: int = BUDGET) -> None:
        self._limit_slides = max(0, int(slides))
        self._limit_parts = max(0, int(parts))
        self._budget = max(0, int(budget))
        self._slides: OrderedDict[bytes, _Kept] = OrderedDict()
        self._parts: OrderedDict[tuple, object] = OrderedDict()
        #: The pictures kept slides and conversions under way hold, by digest; the kept
        #: slides' are counted in ``_kept_images`` and ``_image_bytes``.
        self._images: dict[bytes, _Picture] = {}
        self._kept_images = 0
        self._entry_bytes = 0
        self._image_bytes = 0
        self._lock = threading.Lock()
        #: Slides given back from the cache, and slides resolved, since it was made.
        self.hits = 0
        self.misses = 0

    def __len__(self) -> int:
        with self._lock:
            return len(self._slides)

    def clear(self) -> None:
        """Let go of every slide, picture and part kept (the counts of hits and misses
        stay)."""
        with self._lock:
            self._slides.clear()
            self._parts.clear()
            for digest, picture in list(self._images.items()):
                picture.kept = 0
                if not picture.using:
                    del self._images[digest]
            self._kept_images = self._entry_bytes = self._image_bytes = 0

    @property
    def budget(self) -> int:
        """The most bytes the kept slides and their pictures take; set it to change it
        (slides are let go of at once to fit; 0 keeps none)."""
        return self._budget

    @budget.setter
    def budget(self, value: int) -> None:
        with self._lock:
            self._budget = max(0, int(value))
            self._fit()

    def stats(self) -> CacheStats:
        """What the cache holds now, and its hits and misses (:class:`CacheStats`)."""
        with self._lock:
            return CacheStats(entries=len(self._slides), entry_bytes=self._entry_bytes,
                              images=self._kept_images, image_bytes=self._image_bytes,
                              parts=len(self._parts), hits=self.hits, misses=self.misses,
                              budget=self._budget, limit=self._limit_slides)

    # -- parsed parts ----------------------------------------------------------------------

    def parsed(self, package: OpcPackage, path: str, reader: Callable) -> object:
        """``reader(package, path)``, once per the part's bytes and its relationships' (a
        :data:`~pptx2svg.parse.parts.PartReader`)."""
        key = (f"{reader.__module__}.{reader.__qualname__}", path,
               _digest(package.read(path)), _digest(package.read(rels_path_for(path))))
        with self._lock:
            found = self._parts.get(key)
            if found is not None:
                self._parts.move_to_end(key)
                return found
        value = reader(package, path)
        if value is not None and self._limit_parts:
            with self._lock:
                self._parts[key] = value
                while len(self._parts) > self._limit_parts:
                    self._parts.popitem(last=False)
        return value

    # -- resolved slides -------------------------------------------------------------------

    def resolve(self, package: OpcPackage, options, *, east_asian, host_fonts: bool,
                now: datetime | None):
        """``(presentation, resolved)`` as :func:`pptx2svg.convert_pptx_to_model` reads and
        resolves them, the slides kept here taken from here."""
        from .resolve.view import ResolvedPresentation, _font_scheme, resolve_presentation

        header = read_presentation(package, (), parsed=self.parsed)
        entries = slide_entries(package)
        count = len(entries)
        if options.slide_numbers is None:
            numbers = list(range(1, count + 1))
        else:
            numbers = sorted({n for n in options.slide_numbers if 1 <= n <= count})
        # What every slide's resolution reads besides its own reach.
        settings = repr((header.part_path, header.slide_width, header.slide_height,
                         header.default_text_style, header.table_styles,
                         header.embedded_fonts, header.first_slide_number,
                         header.app_version, options.locale, bool(host_fonts)))
        digests: dict[str, tuple] = {}
        keys = {number: self._key(package, settings, number, *entries[number - 1], digests)
                for number in numbers}
        found: dict[int, tuple] = {}
        with self._lock:
            for number, key in keys.items():
                kept = self._slides.get(key) if key is not None else None
                if kept is not None:
                    self._slides.move_to_end(key)
                    found[number] = kept.entry
            self.hits += len(found)
            self.misses += len(numbers) - len(found)
        missing = [number for number in numbers if number not in found]
        if missing:
            now = now if now is not None else datetime.now()
            presentation = read_presentation(package, missing, parsed=self.parsed)
            # The pictures' texts from the store, one per picture; the digests of the parts
            # the keys took already.
            images = ImageTexts(self, {part: known[0] for part, known in digests.items()
                                       if known[0] != b"-"})
            try:
                for source in presentation.slides:
                    one = resolve_presentation(package, presentation,
                                               slide_numbers=[source.slide_number],
                                               east_asian=east_asian, now=now,
                                               locale=options.locale, images=images)
                    layout = presentation.layouts.get(source.layout_part_path or "")
                    master = (presentation.masters.get(layout.master_part_path or "")
                              if layout else None)
                    theme = (presentation.themes.get(master.theme_part_path or "")
                             if master else None)
                    entry = (one.slides[0] if one.slides else None, tuple(one.warnings),
                             _font_scheme(theme) if theme is not None else None)
                    found[source.slide_number] = entry
                    key = keys.get(source.slide_number)
                    if key is not None and self._limit_slides and self._budget \
                            and not one.dated:
                        self._keep(key, entry, images)
            finally:
                images.close()
        slides: list[m.Slide] = []
        warnings: list = []
        font_scheme = m.FontScheme()
        for number in numbers:
            entry = found.get(number)
            if entry is None or entry[0] is None:      # no such part: not drawn, as before
                continue
            slides.append(entry[0])
            warnings.extend(entry[1])
            if entry[2] is not None:
                font_scheme = entry[2]
        size = m.SlideSize(width=header.slide_width, height=header.slide_height)
        resolved = ResolvedPresentation(slide_size=size, slides=slides, warnings=warnings,
                                        font_scheme=font_scheme)
        return header, resolved

    # -- what is kept, and its bounds ------------------------------------------------------

    def take(self, digest: bytes, data: bytes) -> str:
        """The one text of the picture ``data`` (:class:`pptx2svg.images.PictureStore`),
        held for a conversion under way until it is given back."""
        with self._lock:
            picture = self._images.get(digest)
            if picture is not None:
                picture.using += 1
                return picture.text
        text = encode(data)                      # outside the lock: it takes a while
        with self._lock:
            picture = self._images.setdefault(digest, _Picture(text))
            picture.using += 1
            return picture.text

    def give_back(self, digests: Iterable[bytes]) -> None:
        """A conversion is done with these pictures."""
        with self._lock:
            for digest in digests:
                picture = self._images[digest]
                picture.using -= 1
                if not picture.using and not picture.kept:
                    del self._images[digest]

    def _keep(self, key: bytes, entry: tuple, images: ImageTexts) -> None:
        """Keep ``entry`` under ``key`` if it fits the budget alone, then let go of the
        least recently used slides until everything does.  Its pictures are the store's
        (taken, and not given back yet)."""
        weight, pictures = _weigh(entry, images.held())
        with self._lock:
            if key in self._slides:              # another thread kept it meanwhile
                self._slides.move_to_end(key)
                return
            held = [self._images[digest] for digest in pictures]
            added = sum(picture.size for picture in held if not picture.kept)
            if weight + added > self._budget:
                return                           # alone over the budget: given back only
            for picture in held:
                if not picture.kept:
                    self._kept_images += 1
                    self._image_bytes += picture.size
                picture.kept += 1
            self._slides[key] = _Kept(entry, weight, tuple(pictures))
            self._entry_bytes += weight
            self._fit()

    def _fit(self) -> None:
        """Let go of the least recently used slides until the bounds hold (lock held)."""
        while self._slides and (len(self._slides) > self._limit_slides
                                or self._entry_bytes + self._image_bytes > self._budget):
            _, kept = self._slides.popitem(last=False)
            self._entry_bytes -= kept.weight
            for digest in kept.images:
                picture = self._images[digest]
                picture.kept -= 1
                if not picture.kept:
                    self._kept_images -= 1
                    self._image_bytes -= picture.size
                    if not picture.using:
                        del self._images[digest]

    def _key(self, package: OpcPackage, settings: str, number: int, slide_id: int | None,
             path: str, digests: dict) -> bytes:
        """The slide's key: what its resolution reads."""
        reach: dict[str, tuple] = {}
        stack = [path]
        while stack:
            part = stack.pop()
            if part in reach:
                continue
            known = digests.get(part)
            if known is None:
                known = (_digest(package.read(part)),
                         _digest(package.read(rels_path_for(part))), package.content_type(part))
                digests[part] = known
            reach[part] = known
            if known[0] == b"-":
                continue
            for relationship in package.relationships(part).values():
                # Another slide is reached only as a hyperlink's target: its own bytes do
                # not draw this one.
                if relationship.is_external or relationship.target_part is None \
                        or relationship.type == REL_SLIDE:
                    continue
                stack.append(relationship.target_part)
        made = repr((settings, number, slide_id, path, sorted(reach.items())))
        return hashlib.blake2b(made.encode(), digest_size=20).digest()


#: What a slide's weight does not count: shared by every slide, or not data at all.
_UNWEIGHED = (type, ModuleType, FunctionType, BuiltinFunctionType, MethodType, Enum,
              type(None), bool)


def _weigh(entry: tuple, images: dict[int, tuple[bytes, str]]) -> tuple[int, dict[bytes, str]]:
    """The bytes ``entry``'s objects take (each counted once), and the pictures' texts it
    holds out of ``images`` (``{id(text): (digest, text)}``), which are not counted.  The
    objects an object holds are what the garbage collector finds in it
    (:func:`gc.get_referents`), a level at a time."""
    total = 0
    pictures: dict[bytes, str] = {}
    seen: set[int] = set()
    level: list = [entry]
    while level:
        walk = []
        for obj in level:
            if id(obj) in seen:
                continue
            seen.add(id(obj))
            picture = images.get(id(obj))
            if picture is not None:
                pictures[picture[0]] = picture[1]
            elif not isinstance(obj, _UNWEIGHED):
                total += sys.getsizeof(obj)
                walk.append(obj)
        level = gc.get_referents(*walk)
    return total, pictures
