#!/usr/bin/env python3
"""Measure what Word does with a strict Open XML document (phase 3 RD1a): the document it
saves as Strict, how it reads universal measures, what it draws, and whether it opens a
strict document an edit wrote transitional markup into.

Word -- through ``crates/ooxml-oracle/word/oracle.py``'s machine-wide lock -- opens
``fixtures/generated/markdown/constructs.docx`` (headings, lists, a table, notes, a header and
footer, pictures, a text box) and saves it twice: as a Word Document (``transitional.docx``)
and as a Strict Open XML Document (``strict.docx``), both written to
``tests/data/word-probes/make_strict_probe/`` for the layout tests.  Then:

* ``units``: the transitional twin with its ``w:pgMar`` written in universal measures (``in``,
  ``mm``, ``cm``, ``pt``, ``pc``, ``pi``), saved again by Word, which writes them in twips;
* ``documents``: every line of Word's PDF of each twin, ``[page, baseline in device px at
  300 dpi, text]``;
* ``edited``: whether Word opens ``edited-before-rd1a.docx`` -- what the session wrote
  before RD1a for ``word_insert_markdown`` ``at: end`` on ``strict.docx``: the paragraph
  after the body, and a second styles part (``word/styles1.xml``, related by the transitional
  type, the strict one unfound) -- ``done``, or ``rejected`` (Word stops on a dialog: the
  repair prompt).

    python crates/ooxml-oracle/word/probes/strict_probe.py   # writes crates/ooxml-oracle/word/observations/strict-word.json

Only the facts read from Word's answers are recorded, and the two documents Word made.
"""

from __future__ import annotations

import io
import json
import plistlib
import re
import shutil
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402
from probe_paths import FIXTURES, ORACLE  # noqa: E402

import oracle  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "strict-word.json"
SOURCE = FIXTURES / "generated" / "markdown" / "constructs.docx"
TWINS = ORACLE / "tests" / "data" / "word-probes" / "make_strict_probe"
#: Word's "Strict Open XML Document" file format, as its AppleScript dictionary names it.
STRICT_FORMAT = "format strict open xmldocument"
#: ``w:pgMar`` values written in universal measures, one probe document each.
UNITS = [
    {"top": "1in", "right": "25.4mm", "bottom": "2.54cm", "left": "6pc", "header": "3pi", "footer": "35.4pt",
     "gutter": "0.5in"},
    {"top": "1.27cm", "right": "12.7mm", "bottom": "0.75in", "left": "17.95pt", "header": "1.5pc",
     "footer": "2.25pi", "gutter": "1mm"},
    {"top": "1.001in", "right": "10.05mm", "bottom": "3.333cm", "left": "12.33pt", "header": "0.1pc",
     "footer": "7.7pi", "gutter": "0.03in"},
]
#: What the session wrote for an insert into ``strict.docx`` before RD1a (kept as made).
EDITED = TWINS / "edited-before-rd1a.docx"


def _rewritten(data: bytes, part: str, change) -> bytes:
    source = zipfile.ZipFile(io.BytesIO(data))
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for item in source.infolist():
            content = source.read(item.filename)
            if item.filename == part:
                content = change(content.decode("utf-8")).encode("utf-8")
            target.writestr(item, content)
    return out.getvalue()


def _margins(data: bytes, margins: dict[str, str]) -> bytes:
    written = " ".join(f'w:{side}="{value}"' for side, value in margins.items())
    return _rewritten(data, "word/document.xml", lambda xml: re.sub(r"<w:pgMar [^>]*/>", f"<w:pgMar {written}/>", xml))


def _lines(pdf: Path) -> list[list]:
    """Every line Word drew: ``[page, baseline px at 300 dpi, text]``, top to bottom."""
    import ctypes

    import pypdfium2
    import pypdfium2.raw as raw

    out = []
    document = pypdfium2.PdfDocument(str(pdf))
    try:
        for number, page in enumerate(document):
            height = page.get_height()
            textpage = page.get_textpage()
            lines: dict[int, list[tuple[float, str]]] = {}
            for index in range(textpage.count_chars()):
                code = raw.FPDFText_GetUnicode(textpage.raw, index)
                if not code or chr(code) in "\r\n":
                    continue
                x, y = ctypes.c_double(), ctypes.c_double()
                raw.FPDFText_GetCharOrigin(textpage.raw, index, x, y)
                baseline = round((height - y.value) * 300 / 72)
                lines.setdefault(baseline, []).append((x.value, chr(code)))
            for baseline in sorted(lines):
                text = "".join(c for _, c in sorted(lines[baseline], key=lambda p: p[0]))
                out.append([number, baseline, text])
    finally:
        document.close()
    return out


def main() -> None:
    source = SOURCE.read_bytes()
    strict = oracle.word_script([f"save as d file name outputPath file format {STRICT_FORMAT}",
                                 "close d saving no", "quit saving no", "return"])
    TWINS.mkdir(parents=True, exist_ok=True)
    found: dict = {"units": [], "documents": {}, "edited": {}}
    with oracle.session() as word:
        made = {"transitional.docx": word.resave(source, name="strict-twin"),
                "strict.docx": word.run_script(strict, source, name="strict-made", tag="strict")}
        for name, outcome in made.items():
            if not outcome:
                raise SystemExit(f"Word did not save {name}: {outcome.outcome} {outcome.detail}")
            shutil.copy(outcome.path, TWINS / name)
        twin = (TWINS / "transitional.docx").read_bytes()
        for k, margins in enumerate(UNITS):
            saved = word.resave(_margins(twin, margins), name=f"strict-units-{k}")
            xml = zipfile.ZipFile(saved.path).read("word/document.xml").decode("utf-8")
            read = re.search(r"<w:pgMar ([^>]*)/>", xml).group(1)
            found["units"].append({"written": margins,
                                   "word": dict(re.findall(r'w:(\w+)="([^"]*)"', read))})
        for name in made:
            data = (TWINS / name).read_bytes()
            pdf = word.export_pdf(data, name=f"strict-{Path(name).stem}")
            found["documents"][name] = _lines(pdf.path)
        found["edited"][EDITED.name] = word.export_pdf(EDITED.read_bytes(), name="strict-edited").outcome
    info = plistlib.loads((oracle.WORD_APP / "Contents" / "Info.plist").read_bytes())
    observed = {
        "_about": "Word's Save As Strict of constructs.docx beside its Save As Word Document; universal "
                  "measures as Word reads them (written, then saved by Word in twips); every line Word drew "
                  "of each twin, [page, baseline px at 300 dpi, text]; whether Word opens the file "
                  "the session wrote for an insert before RD1a (edited-before-rd1a.docx). Regenerate with "
                  "crates/ooxml-oracle/word/probes/strict_probe.py.",
        "word": f"Word {info.get('CFBundleShortVersionString')}",
        **found,
    }
    OBSERVATIONS.write_text(json.dumps(observed, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(OBSERVATIONS)


if __name__ == "__main__":
    main()
