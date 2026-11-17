#!/usr/bin/env python3
"""Records every call the Python renderers make to resvg-py, for the PNG differential (test
support; phase P3, lane P3d).

docx2svg and pptx2svg draw a page by handing resvg-py's ``svg_to_bytes`` an SVG document,
the font files and folders, whether the system's fonts take part, the generic families and
a size -- once per page, or once per pass where a page is drawn in passes.  This wraps that
one function, renders each document given to PNG the way the package does
(``convert_docx_to_png``, ``convert_pptx_to_png``), and writes each call as three files in
``OUT``: ``NNNN.json`` (the arguments, the document and page, resvg-py's time), ``NNNN.svg``
(the document, as handed over) and ``NNNN.png`` (resvg-py's answer).  The Rust
``raster-diff`` binary draws each call again through ``ooxml-raster`` and compares the
pixels.

    python crates/ooxml-oracle/python/raster_record.py OUT a.docx b.pptx ...
    python crates/ooxml-oracle/python/raster_record.py OUT --tree DIR    # every .docx/.pptx below DIR

Python's behaviour is not changed: the wrapper calls resvg-py with what it was given and
returns its answer.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import tempfile
import time
from pathlib import Path

#: The arguments the renderers hand resvg-py, and the only ones ``raster-diff`` replays.
KNOWN = {"svg_string", "width", "height", "zoom", "background", "font_files", "font_dirs", "skip_system_fonts",
         "font_family", "serif_family", "sans_serif_family", "cursive_family", "fantasy_family",
         "monospace_family"}
NAMED = {"white": (255, 255, 255, 255), "black": (0, 0, 0, 255), "transparent": (0, 0, 0, 0)}


def colour(value: str | None) -> list[int] | None:
    """A background as ``[r, g, b, a]``: the spellings the renderers pass."""
    if value is None:
        return None
    text = value.strip().lower()
    if text in NAMED:
        return list(NAMED[text])
    if text.startswith("#") and len(text) in (4, 5):
        text = "#" + "".join(ch * 2 for ch in text[1:])
    if text.startswith("#") and len(text) in (7, 9):
        channels = [int(text[k:k + 2], 16) for k in range(1, len(text), 2)]
        return channels + [255] * (4 - len(channels))
    raise ValueError(f"a background raster_record cannot read: {value!r}")


class Recorder:
    """``resvg_py.svg_to_bytes`` with every call written to ``out``."""

    def __init__(self, out: Path, original) -> None:
        self.out, self.original, self.count, self.where = out, original, 0, ("", 0)

    def __call__(self, **options):
        unknown = set(options) - KNOWN
        if unknown:
            raise ValueError(f"resvg-py called with arguments raster-diff does not replay: {sorted(unknown)}")
        start = time.perf_counter()
        png = self.original(**options)
        seconds = time.perf_counter() - start
        number = self.count
        self.count += 1
        stem = self.out / f"{number:04d}"
        stem.with_suffix(".svg").write_text(options["svg_string"], encoding="utf-8")
        stem.with_suffix(".png").write_bytes(bytes(png))
        case = {key: value for key, value in options.items() if key not in ("svg_string", "background")}
        # A document's embedded faces live in a temporary folder only while it is drawn.
        for key in ("font_files", "font_dirs"):
            if case.get(key):
                case[key] = [self.kept(path, stem) for path in case[key]]
        case["background"] = colour(options.get("background"))
        case["document"], case["call"] = self.where
        case["python_seconds"] = seconds
        stem.with_suffix(".json").write_text(json.dumps(case, indent=1), encoding="utf-8")
        self.where = (self.where[0], self.where[1] + 1)
        return png

    def kept(self, path: str, stem: Path) -> str:
        """``path`` itself, or a copy of it in ``stem``'s folder where it is temporary."""
        if not Path(path).resolve().is_relative_to(Path(tempfile.gettempdir()).resolve()):
            return path
        copy = Path(f"{stem}-fonts") / Path(path).name
        copy.parent.mkdir(exist_ok=True)
        if Path(path).is_dir():
            shutil.copytree(path, copy, dirs_exist_ok=True)
        else:
            shutil.copy2(path, copy)
        return str(copy.resolve())


def documents(paths: list[str], tree: str | None) -> list[Path]:
    found = [Path(path) for path in paths]
    if tree:
        found += sorted(path for path in Path(tree).rglob("*") if path.suffix in (".docx", ".pptx"))
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("out")
    parser.add_argument("documents", nargs="*")
    parser.add_argument("--tree")
    args = parser.parse_args()
    import resvg_py

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    recorder = Recorder(out, resvg_py.svg_to_bytes)
    resvg_py.svg_to_bytes = recorder
    failed = 0
    for path in documents(args.documents, args.tree):
        recorder.where = (str(path), 0)
        try:
            if path.suffix == ".docx":
                import docx2svg
                docx2svg.convert_docx_to_png(str(path))
            else:
                import pptx2svg
                pptx2svg.convert_pptx_to_png(str(path), threads=1)
        except Exception as error:  # a document the renderer refuses is reported, not recorded
            failed += 1
            print(f"{path}: {type(error).__name__}: {error}", file=sys.stderr)
        print(f"{path}: {recorder.where[1]} calls", flush=True)
    print(f"{recorder.count} calls recorded, {failed} documents refused")
    return 0


if __name__ == "__main__":
    sys.exit(main())
