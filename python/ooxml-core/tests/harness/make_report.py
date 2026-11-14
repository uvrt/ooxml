"""A made-up report for the differential tests, written through docx-agent's tools: a table
of contents, chapters of sections with paragraphs, lists and three-column tables, bookmarks
and cross references (text and page), and a footer with "page X of Y", its fields updated
once (the contents' entries).  Seeded.  PLAIN=1 in the environment: no contents, footer or
update.

    python make_report.py out.docx [chapters=1] [sections=62]

One chapter of 62 sections is about 50 pages.
"""
from __future__ import annotations

import datetime as dt
import json
import random
import sys

from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.limits import Limits

from docx_agent import Document
from docx_agent.tools import FORMAT, GROUPS, TOOLS

CLOCK = lambda: dt.datetime(2026, 11, 8, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
WORDS = ("lantern meadow river stone harbour quiet orchard copper valley signal morning ledger "
         "window garden timber bridge market evening canvas thread pebble summit").split()
FILL_COLUMNS = ("code", "name", "area", "state", "note")
SECTIONS = 62  # per chapter; the third argument changes it


def sentence(rng, n=12):
    return " ".join(rng.choice(WORDS) for _ in range(n)).capitalize() + "."


def para(rng):
    return " ".join(sentence(rng, rng.randint(10, 22)) for _ in range(4))


def chapter_markdown(rng, n):
    parts = [f"# Chapter {n + 1} {sentence(rng, 2)[:-1]}"]
    for sub in range(SECTIONS):
        parts.append(f"## Section {n + 1}.{sub + 1} {sentence(rng, 2)[:-1]}")
        for _ in range(3):
            parts.append(para(rng))
        if sub % 3 == 1:
            parts.append(f"- {sentence(rng, 6)}\n- {sentence(rng, 8)}\n  - {sentence(rng, 5)}\n- {sentence(rng, 7)}")
        if sub % 3 == 2:
            parts.append(f"1. {sentence(rng, 6)}\n2. {sentence(rng, 8)}\n3. {sentence(rng, 5)}")
        if sub % 3 == 0:
            parts.append("| Item | Value | Comment |\n|---|---|---|\n" + "\n".join(
                f"| {sentence(rng, 2)[:-1]} | {rng.randint(1, 999)} | {sentence(rng, 5)} |" for _ in range(10)))
    if n == 0:
        parts.append("Items in full")
        parts.append("| " + " | ".join(FILL_COLUMNS) + " |\n|" + "---|" * len(FILL_COLUMNS)
                     + "\n| " + " | ".join(["x"] * len(FILL_COLUMNS)) + " |")
    parts.append(f"End of chapter {n + 1}.")
    return "\n\n".join(parts)


def ok(result):
    out = result.to_json()
    if not out.get("ok", True):
        raise SystemExit(json.dumps(out, indent=1)[:3000])
    return out


def main():
    path = sys.argv[1]
    chapters = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    global SECTIONS
    if len(sys.argv) > 3:
        SECTIONS = int(sys.argv[3])
    rng = random.Random(57)
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0,
                 limits=Limits(render_timeout=900, layout_timeout=900, batch_timeout=1800)) as box:
        session = box.session(clock=CLOCK)
        d = Document.new()
        d.paragraphs()[0].set_text("Annual report of made-up things")
        first = d.paragraphs()[0].id
        doc = session.open(d.to_bytes(), "report.docx")

        def c(tool, **a):
            return ok(box.dispatch(session, tool, {"doc": doc, **a}))

        import os
        plain = os.environ.get("PLAIN")
        if not plain:
            c("word_fields", action="insert_toc", after=first)
        for n in range(chapters):
            c("word_insert_markdown", at="end", markdown=chapter_markdown(rng, n))
        if not plain:
          c("word_headers_footers", section="s:body", which="footer", action="set", text="Annual report",
          page_number="page_x_of_y")
        document = session.entry(doc).document
        heads = [p for p in document.paragraphs() if p.text.startswith("Section ")]
        for i, p in enumerate(heads[: 2 * chapters]):
            c("word_links", action="add_bookmark", target=p.id, bookmark=f"section_{i}")
        ends = [p for p in document.paragraphs() if p.text.startswith("End of chapter ")]
        for i, end in enumerate(ends):
            # one cross reference per chapter, in the paragraph before its end anchor
            body = [p for p in session.entry(doc).document.paragraphs()]
            before = body[[q.id for q in body].index(end.id) - 1]
            if before.text:
                c("word_fields", action="insert_cross_reference", at=f"{before.id}@{len(before.text)}",
                  bookmark=f"section_{(2 * i + 1) % (2 * chapters)}", kind="text" if i % 2 else "page")
        if not plain:
            # the contents' entries and page numbers, as a finished report has them
            c("word_fields", action="update")
        data = session.entry(doc).document.to_bytes()
        open(path, "wb").write(data)
        print(json.dumps({"bytes": len(data)}))


if __name__ == "__main__":
    main()
