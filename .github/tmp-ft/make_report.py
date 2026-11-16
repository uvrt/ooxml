"""TEMPORARY (lane L): a made-up report of CHAPTERS chapters (about 49 pages each) -- a
title, a table of contents, per chapter a heading, sections, long paragraphs, a bullet and a
numbered list and small tables, an end paragraph -- written with docx-agent's own tools.

    python make_report.py CHAPTERS out.docx
    python make_report.py grow base.docx TIMES out.docx
"""
import datetime as dt
import random
import sys

from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.limits import Limits

from docx_agent import Document
from docx_agent.tools import FORMAT, GROUPS, TOOLS

WORDS = ("lantern meadow river stone harbour quiet orchard copper valley signal morning ledger "
         "window garden timber bridge market evening canvas thread pebble summit").split()


def sentence(rng, n=12):
    return " ".join(rng.choice(WORDS) for _ in range(n)).capitalize() + "."


def chapter(rng, n):
    parts = [f"# Chapter {n} {sentence(rng, 2)[:-1]}"]
    for s in range(14):
        parts.append(f"## Section {n}.{s + 1} {sentence(rng, 2)[:-1]}")
        for _ in range(70):
            parts.append(" ".join(sentence(rng, rng.randint(8, 14)) for _ in range(rng.randint(1, 2))))
        if s % 3 == 0:
            parts.append("\n".join(f"- {sentence(rng, rng.randint(4, 9))}" for _ in range(4)))
        if s % 3 == 1:
            parts.append("\n".join(f"{k + 1}. {sentence(rng, rng.randint(4, 9))}" for k in range(3)))
        if s % 2 == 0:
            parts.append("| Item | Value | State |\n|---|---|---|\n" + "\n".join(
                f"| {sentence(rng, 2)[:-1]} | {k} | {rng.choice(['open', 'closed'])} |" for k in range(4)))
    parts.append(f"End of chapter {n}.")
    return "\n\n".join(parts)


def main():
    if sys.argv[1] == "grow":
        return grow(sys.argv[2], int(sys.argv[3]), sys.argv[4])
    chapters, out = int(sys.argv[1]), sys.argv[2]
    rng = random.Random(58)
    clock = lambda: dt.datetime(2026, 11, 16, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0, block_edits="off",
                 limits=Limits(render_timeout=3600, layout_timeout=3600, batch_timeout=3600))  as box:
        session = box.session(clock=clock)
        document = Document.new()
        document.paragraphs()[0].set_text("Annual review of made-up things")
        doc = session.open(document.to_bytes(), "report.docx")
        first = session.entry(doc).document.paragraphs()[0].id
        for n in range(chapters):
            found = box.dispatch(session, "word_insert_markdown", {"doc": doc, "at": "end", "markdown": chapter(rng, n + 1)})
            assert found.ok, found.to_json()
        found = box.dispatch(session, "word_fields", {"doc": doc, "action": "insert_toc", "after": first, "levels": "1-2"})
        assert found.ok, found.to_json()
        data = session.entry(doc).document.to_bytes()
    with open(out, "wb") as f:
        f.write(data)
    print(out, len(data))


if __name__ == "__main__" and not globals().get("NOMAIN"):
    main()


def grow(base: str, times: int, out: str) -> None:
    """A longer report from ``base``'s: its chapters again ``times`` times over (renumbered;
    their paragraph ids and bookmarks left out, so docx-agent stamps ids of their own), the
    table of contents as it was."""
    import re
    import zipfile

    with zipfile.ZipFile(base) as z:
        items = [(info, z.read(info.filename)) for info in z.infolist()]
    body = dict((i.filename, d) for i, d in items)["word/document.xml"].decode()
    # The first chapter's heading (not its line in the table of contents).
    start = next(s for s in (body.rfind("<w:p ", 0, m.start()) for m in re.finditer(">Chapter 1 ", body))
                 if 'w:pStyle w:val="Heading1"' in body[s:body.find("</w:p>", s)])
    end = body.rfind("<w:sectPr")
    chapters = body[start:end]
    count = len(re.findall(r">Chapter \d+ ", chapters))
    plain = re.sub(r' w14:(paraId|textId)="[0-9A-F]+"', "", chapters)
    plain = re.sub(r"<w:bookmark(Start|End)\b[^>]*/>", "", plain)
    copies = []
    for k in range(1, times):
        shift = k * count
        copy = re.sub(r">(Chapter |Section |End of chapter )(\d+)", lambda m: f">{m.group(1)}{int(m.group(2)) + shift}", plain)
        copies.append(copy)
    grown = body[:end] + "".join(copies) + body[end:]
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for info, data in items:
            z.writestr(info, grown.encode() if info.filename == "word/document.xml" else data)
    print(out, count * times, "chapters")
