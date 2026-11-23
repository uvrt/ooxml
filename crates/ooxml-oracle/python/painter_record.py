#!/usr/bin/env python3
"""Records every call docx2svg's drawing painter makes to ooxml-common's DrawingML pieces,
for the painter-pieces differential (test support; phase P3, lane P3d).

docx2svg paints a Word drawing by calling ooxml-common's pieces directly (``drawing.py``,
``svg.py``): ``fill.render_fill_attrs``, ``render_outline_attrs`` and
``render_arrowheads``, ``effect.render_effects``, and ``geometry.preset_path_data``,
``spec_path_data``, ``path_ends``, ``trim_path`` and ``text_rect``; and what a symbol face's
codes stand for (``text.symbol_fonts.to_unicode``, every code).  This wraps each
(the outermost call only: one piece calling another is the first's answer), lays each
document given out to SVG with ``docx2svg.convert_docx_to_svg``, and writes one JSON line
a call to ``OUT``: the function, its arguments (model values in the scene fixtures' form,
``goldens.to_data``), how many ids the page's resources had already taken, and the
answer -- the attribute text or result, and the resources the call declared.  The Rust
``painter-diff`` binary makes each call through ``ooxml-render`` and compares.

    PYTHONPATH=python/ooxml-common/src:python/ooxml-common/tests:python/docx2svg/src \\
        python crates/ooxml-oracle/python/painter_record.py OUT.jsonl a.docx b.docx ...

Python's behaviour is not changed: each wrapper calls the function with what it was given
and returns its answer.  docx2svg's Python painter is the frozen reference's (docx2svg draws
with the Rust core since lane svg-thin): this runs only where the gates' reference is
installed (``GATES_REFERENCE_SHA`` set, ``reference_packages.py``), and refuses otherwise.
"""

from __future__ import annotations

import functools
import json
import sys
from pathlib import Path

import goldens
from ooxml_common.drawingml import effect, fill, geometry

#: (module, function name) for every piece recorded.
PIECES = [
    (fill, "render_fill_attrs"), (fill, "render_outline_attrs"), (fill, "render_arrowheads"),
    (effect, "render_effects"), (geometry, "preset_path_data"), (geometry, "spec_path_data"),
    (geometry, "path_ends"), (geometry, "trim_path"), (geometry, "text_rect"),
]


def answer(value):
    """A result as JSON: path lists as ``[d, fill, stroke]``, tuples as lists."""
    if isinstance(value, geometry.GeometryPath):
        return [value.d, value.fill, value.stroke]
    if isinstance(value, (list, tuple)):
        return [answer(item) for item in value]
    return value


class Recorder:
    def __init__(self, out) -> None:
        self.out, self.depth, self.count = out, 0, 0

    def wrap(self, module, name: str) -> None:
        original = getattr(module, name)

        @functools.wraps(original)
        def recorded(*args, **kwargs):
            if self.depth:
                return original(*args, **kwargs)
            defs = next((a for a in list(args) + list(kwargs.values()) if hasattr(a, "new_id")), None)
            before = (getattr(defs, "_next_id", None), len(getattr(defs, "defs", [])))
            self.depth += 1
            try:
                result = original(*args, **kwargs)
            finally:
                self.depth -= 1
            try:
                record = {
                    "fn": name,
                    "args": [goldens.to_data(a) if a is not defs else None for a in args],
                    "kwargs": {k: v.name if k == "rules" else goldens.to_data(v)
                               for k, v in kwargs.items() if v is not defs},
                    "ids_before": before[0],
                    "result": answer(result),
                    "defs": list(defs.defs[before[1]:]) if defs is not None and hasattr(defs, "defs") else [],
                }
            except TypeError as error:  # an argument the fixtures' form cannot hold: listed, not compared
                record = {"fn": name, "skipped": str(error)}
            self.out.write(json.dumps(record) + "\n")
            self.count += 1
            return result

        setattr(module, name, recorded)


def main() -> int:
    out_path, documents = Path(sys.argv[1]), sys.argv[2:]
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from reference_packages import check_frozen

    if check_frozen() is None:
        print("painter_record: docx2svg's Python painter is the frozen reference's: run this where the gates' "
              "reference is installed (GATES_REFERENCE_SHA set; crates/ooxml-oracle/python/reference_packages.py)",
              file=sys.stderr)
        return 2
    import docx2svg

    with out_path.open("w", encoding="utf-8") as out:
        recorder = Recorder(out)
        for module, name in PIECES:
            recorder.wrap(module, name)
        for path in documents:
            before = recorder.count
            try:
                docx2svg.convert_docx_to_svg(path)
            except Exception as error:  # a document the renderer refuses is reported, not recorded
                print(f"{path}: {type(error).__name__}: {error}", file=sys.stderr)
            print(f"{path}: {recorder.count - before} calls", flush=True)
        # What every code of every symbol face stands for (``text.symbol_fonts``), both spellings.
        from ooxml_common.text import symbol_fonts
        for table in sorted(set(symbol_fonts.SYMBOL_FACES.values())):
            for code in range(0x20, 0x100):
                for char in (chr(code), chr(0xF000 + code)):
                    out.write(json.dumps({"fn": "to_unicode", "args": [table, char],
                                          "result": symbol_fonts.to_unicode(table, char), "defs": []}) + "\n")
    print(f"{recorder.count} calls recorded")
    return 0


if __name__ == "__main__":
    sys.exit(main())
