"""Pictures' base64 text, made once per their bytes: :class:`ImageTexts`.

A resolved picture (:class:`~pptx2svg.model.ImageElement`, an image fill, a picture
bullet) carries its bytes as the base64 text a ``data:`` URI takes.  A layout's or
master's picture is resolved again for every slide drawn on it, and a deck edited 400
times is resolved again after every edit; made afresh each time, each such text was a new
copy of the same picture.  So every text is taken from one :class:`ImageTexts`, which
keeps one string per picture, by the digest of its bytes: every element of a conversion
that shows the same picture holds the very same string.

A conversion with a :class:`~pptx2svg.cache.ModelCache` takes its texts from the cache's
store of pictures (:class:`PictureStore`), which holds each while a kept slide or a
conversion under way holds it: a picture unchanged across every version of a deck, and
across every deck made from one template, is one string in the process.

The text is made where the picture is resolved, not where the SVG is written: a slide
drawn again from the cache then costs no encoding at all, and the SVG is the same string
it always was.
"""

from __future__ import annotations

import base64
import hashlib
from collections.abc import Iterable, Mapping
from typing import Protocol

__all__ = ["ImageTexts", "PictureStore", "image_digest"]


def image_digest(data: bytes) -> bytes:
    """What a picture is known by: its bytes' BLAKE2b, 16 bytes -- the digest
    :class:`~pptx2svg.cache.ModelCache` keys parts by, so one hashing serves both."""
    return hashlib.blake2b(data, digest_size=16).digest()


def encode(data: bytes) -> str:
    """``data`` as the base64 text of a ``data:`` URI."""
    return base64.b64encode(data).decode("ascii")


class PictureStore(Protocol):
    """Pictures' texts shared by conversions (a :class:`~pptx2svg.cache.ModelCache`)."""

    def take(self, digest: bytes, data: bytes) -> str:
        """The text of the picture ``data`` (whose digest is ``digest``), the one string
        the store holds for it; held for the caller until :meth:`give_back`."""

    def give_back(self, digests: Iterable[bytes]) -> None:
        """The caller holds these no more (each once for each :meth:`take`)."""


class ImageTexts:
    """The base64 text of each picture of one conversion, one string per picture.

    ``store``: where the texts come from and are held while the conversion runs (else
    made here).  ``known`` gives the digest of a package part already hashed (the cache's
    keys hash every part a slide reaches), so a picture read from a part is not hashed
    twice.  One per conversion, in one thread; :meth:`close` it when it is done."""

    __slots__ = ("_known", "_store", "_texts")

    def __init__(self, store: PictureStore | None = None,
                 known: Mapping[str, bytes] | None = None) -> None:
        self._store = store
        self._known = known or {}
        self._texts: dict[bytes, str] = {}

    def text(self, data: bytes, part: str | None = None) -> str:
        """``data`` as base64 text: the string this conversion, or the store, already has
        for these bytes, else a new one.  ``part``: the package part ``data`` was read
        from, when it was (its digest may be known)."""
        digest = self._known.get(part) if part is not None else None
        if digest is None:
            digest = image_digest(data)
        found = self._texts.get(digest)
        if found is None:
            found = encode(data) if self._store is None else self._store.take(digest, data)
            self._texts[digest] = found
        return found

    def held(self) -> dict[int, tuple[bytes, str]]:
        """``{id(text): (digest, text)}`` of every text given out."""
        return {id(text): (digest, text) for digest, text in self._texts.items()}

    def close(self) -> None:
        """The conversion is done: the store's texts are given back."""
        if self._store is not None and self._texts:
            self._store.give_back(list(self._texts))
        self._texts = {}
