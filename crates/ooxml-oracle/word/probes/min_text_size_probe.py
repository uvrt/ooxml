#!/usr/bin/env python3
"""Measure what Word makes of the kinds of text a per-kind minimum text size tells apart (round 60,
R140: ``check`` with the application's ``min_text_size`` by kind, docx-agent 0.1's
``docx_format(min_text_size={...})``).

The document is ``cases/min_text_size_cases.py``'s: a paragraph of each kind, named by its first
word, each at one size. Word -- through ``crates/ooxml-oracle/word/oracle.py``'s machine-wide lock --

* ``word-kinds``: opens the document and reads, for every paragraph of the main text, the
  footnote, the endnote and section 1's primary header and footer, the story's type,
  the paragraph's text, its font size and its style (Word's built-in style where it is one); it
  writes them as lines of text at the document's end and saves, and the lines are read back;
* ``word-caption``: inserts its own caption below the table (``insert caption``, label Table) and saves:
  the style it gives a caption, by its id and name in the saved styles part;
* ``tools-opened``: opens the document again and saves it: whether it opens without a prompt.

Recorded per key: ``story``, ``size`` and ``style`` as Word says them; for the caption, its style
id and name.

    python crates/ooxml-oracle/word/probes/min_text_size_probe.py   # writes observations/min-text-size-word.json

Nothing Word writes is committed: only the facts read from it.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

from lxml import etree

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402  (the driver, the fixture makers and the cases on the path)

import e3_probe  # noqa: E402
import oracle  # noqa: E402

from min_text_size_cases import document  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "min-text-size-word.json"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
#: What starts each line of facts Word writes (the story, size and style, then the paragraph's
#: text, whose mark ends the line).
MARK = "FACT|"

_READ = '''set theFacts to {{}}
      set thePieces to {{}}
      repeat with i from 1 to count of paragraphs of d
        set end of thePieces to text object of paragraph i of d
      end repeat
      set end of thePieces to text object of footnote 1 of d
      set end of thePieces to text object of endnote 1 of d
      set end of thePieces to text object of (get header section 1 of d index header footer primary)
      set end of thePieces to text object of (get footer section 1 of d index header footer primary)
      repeat with t in thePieces
        set k to (story type of t) as text
        try
          set s to name local of (style of t)
        on error
          set s to (style of t) as text
        end try
        set z to (font size of font object of t) as text
        set end of theFacts to "{mark}" & k & "|" & z & "|" & s & "|" & (content of t)
      end repeat
      set AppleScript's text item delimiters to return
      insert text (return & (theFacts as text)) at end of text object of d'''


def kinds_script() -> str:
    return e3_probe._script([_READ.format(mark=MARK)])


def caption_script() -> str:
    return e3_probe._script(["insert caption at (text object of table 1 of d) caption label caption table "
                             'title ": Word\'s own" caption position caption position below'])


def facts(data: bytes) -> dict[str, dict]:
    """The facts Word wrote, by key (the text's first word): story, size, style."""
    root = etree.fromstring(e3_probe.parts(data)["word/document.xml"])
    out = {}
    for paragraph in root.iter(_W + "p"):
        text = "".join(t.text or "" for t in paragraph.iter(_W + "t"))
        if not text.startswith(MARK):
            continue
        _, story, size, style, content = text.split("|", 4)
        found = re.match(r"\W*(\w+)", content)
        if found:
            out[found.group(1)] = {"story": story, "size": float(size.replace(",", ".")), "style": style}
    return out


def caption(data: bytes) -> dict:
    """The caption Word inserted: its text and its style's id and name."""
    parts = e3_probe.parts(data)
    root = etree.fromstring(parts["word/document.xml"])
    names = {s.get(_W + "styleId"): s.find(_W + "name").get(_W + "val")
             for s in etree.fromstring(parts["word/styles.xml"]).iter(_W + "style") if s.find(_W + "name") is not None}
    for paragraph in root.iter(_W + "p"):
        text = "".join(t.text or "" for t in paragraph.iter(_W + "t"))
        if "Word's own" in text:
            node = paragraph.find(f"{_W}pPr/{_W}pStyle")
            style = node.get(_W + "val") if node is not None else None
            return {"text": text, "style_id": style, "style_name": names.get(style)}
    return {}


def main() -> None:
    out: dict = {}
    data = document()
    with oracle.session() as session:
        session.recover()
        made = session.run_script(kinds_script(), data, name="min-text-size", tag="kinds", timeout=200)
        if not made:
            raise SystemExit(f"Word did not read the kinds: {made.outcome} {made.detail}")
        out["word-kinds"] = facts(made.path.read_bytes())
        captioned = session.run_script(caption_script(), data, name="min-text-size", tag="caption", timeout=200)
        if not captioned:
            raise SystemExit(f"Word did not insert its caption: {captioned.outcome} {captioned.detail}")
        out["word-caption"] = caption(captioned.path.read_bytes())
        opened = session.resave(data, name="min-text-size", timeout=200)
        out["tools-opened"] = {"outcome": opened.outcome, "detail": opened.detail[:200]}
    OBSERVATIONS.write_text(json.dumps({
        "_about": ("What Word 16.106 for Mac read and wrote for crates/ooxml-oracle/word/probes/min_text_size_probe.py "
                   "(the document in cases/min_text_size_cases.py): 'word-kinds', per paragraph key the story "
                   "type, font size and style Word gives it (its built-in style's name where it is one); "
                   "'word-caption', the caption Word's own insert caption made (its style id and name); "
                   "'tools-opened', whether Word opened the document and saved it. "
                   "Regenerate with python crates/ooxml-oracle/word/probes/min_text_size_probe.py."),
        "word": "16.106 (macOS), Dutch interface",
        **out}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS}")


if __name__ == "__main__":
    main()
