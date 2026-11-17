"""The metafile players' differential, Python side (test support; phase P3, lane P3b1).

A pytest plugin that records every metafile ooxml-common's player draws while a test suite
runs -- each top-level ``ooxml_common.metafile._draw`` call: the EMF or WMF bytes and the
font mapping in, the drawing out -- and every device-independent bitmap
``ooxml_common.metafile.dib.dib_to_image`` decodes.  Nothing of ooxml-common changes: the
two functions are wrapped for the run only.

    pytest -p metafile_record --metafile-record=OUT_DIR tests

with this folder on ``PYTHONPATH``.  ``OUT_DIR`` gets one ``<sha>.case`` file per distinct
input (the players' differential case format: ``crates/ooxml-oracle/src/metafile.rs``);
``metafile-diff OUT_DIR`` plays each case in Rust and compares.

``python metafile_record.py bundle OUT.gz DIR [--every N]`` writes a folder's cases (every
``N``-th, by name) into one gzip stream, each case prefixed by its length: what
``crates/ooxml-oracle/tests/data/metafile-cases.gz`` holds, which ``cargo test`` plays.  Run it over
ooxml-common's, pptx2svg's and docx2svg's suites: the repository has no metafile files of
its own -- its tests build them (``ooxml-common/tests/metafile_sweep.py``,
``test_metafile_gdi.py``, ``test_metafile_emfplus.py`` and the renderers' tests).

A case file is a sequence of length-prefixed fields (``u32`` little-endian length, then the
bytes): the kind (``metafile`` or ``dib``), then for a metafile its bytes, the font mapping
as JSON (``null`` for the default) and the drawing as JSON (``null`` when the player
returned ``None``; its width and height as ``repr`` strings) with its SVG as the last
field; for a DIB its header, its bits, and the image as JSON (``null`` for none) with the
image's bytes last.
"""

from __future__ import annotations

import hashlib
import json
import struct
from pathlib import Path

_OUT: list[Path] = []


def pytest_addoption(parser):
    parser.addoption("--metafile-record", default=None,
                     help="record every metafile and DIB ooxml-common draws into this folder")


def pytest_configure(config):
    out = config.getoption("--metafile-record")
    if not out:
        return
    folder = Path(out)
    folder.mkdir(parents=True, exist_ok=True)
    _OUT.append(folder)
    from ooxml_common import metafile
    from ooxml_common.metafile import dib

    draw = metafile._draw
    to_image = dib.dib_to_image

    def recorded_draw(data, font_mapping, *, nesting: int = 0):
        result = draw(data, font_mapping, nesting=nesting)
        if nesting == 0:
            _save_metafile(bytes(data), font_mapping, result)
        return result

    def recorded_to_image(header, pixels):
        result = to_image(header, pixels)
        _save_dib(bytes(header), bytes(pixels), result)
        return result

    metafile._draw = recorded_draw
    dib.dib_to_image = recorded_to_image


def _fields(*parts: bytes) -> bytes:
    return b"".join(struct.pack("<I", len(p)) + p for p in parts)


def _write(blob: bytes) -> None:
    name = hashlib.sha256(blob).hexdigest()[:24] + ".case"
    path = _OUT[0] / name
    if not path.exists():
        path.write_bytes(blob)


def _save_metafile(data: bytes, font_mapping, drawing) -> None:
    mapping = json.dumps(font_mapping).encode() if font_mapping is not None else b"null"
    if drawing is None:
        _write(_fields(b"metafile", data, mapping, b"null", b""))
        return
    _write(_fields(b"metafile", data, mapping, meta_json(drawing), drawing.svg))


def meta_json(drawing) -> bytes:
    """A drawing's metadata as JSON; its size as ``repr`` strings, which a reader parses
    exactly (a JSON number need not be)."""
    meta = {"width": repr(drawing.width), "height": repr(drawing.height),
            "skipped": [list(s) for s in drawing.skipped], "emf_plus": drawing.emf_plus,
            "gdi_fallback": drawing.gdi_fallback, "truncated": drawing.truncated, "drawn": drawing.drawn}
    return json.dumps(meta).encode()


def _save_dib(header: bytes, pixels: bytes, image) -> None:
    if image is None:
        _write(_fields(b"dib", header, pixels, b"null", b""))
        return
    _write(_fields(b"dib", header, pixels, json.dumps({"mime": image.mime_type}).encode(), image.data))


def bundle(out: Path, folder: Path, every: int = 1) -> int:
    """Writes every ``every``-th case of ``folder`` (by name) into ``out``, gzip, each case
    prefixed by its ``u32`` length; returns how many."""
    import gzip

    cases = sorted(folder.glob("*.case"))[::every]
    with gzip.GzipFile(out, mode="ab", compresslevel=9, mtime=0) as stream:
        for case in cases:
            blob = case.read_bytes()
            stream.write(struct.pack("<I", len(blob)) + blob)
    return len(cases)


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="bundle recorded cases")
    parser.add_argument("command", choices=["bundle"])
    parser.add_argument("out", type=Path)
    parser.add_argument("folder", type=Path)
    parser.add_argument("--every", type=int, default=1)
    args = parser.parse_args()
    print(bundle(args.out, args.folder, args.every))
