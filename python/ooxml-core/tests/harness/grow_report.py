"""A long made-up report from a short one: make_report.py's chapters repeated, renumbered and
cut to CHAPTERS, then opened and saved through docx-agent's tools (which stamp the copies'
paragraph ids) with the fields updated (the contents list every chapter).  The 3k-page
benches' document (gates.yml): ``make_report.py base.docx 6`` then ``grow_report.py base.docx
out.docx 61`` -- chapter 1's "Items in full" comes back in chapters 7, 13, ... 61, about
3,000 pages.  NOUPDATE=1 in the environment: no fields update.

    python grow_report.py base.docx out.docx CHAPTERS
"""
from __future__ import annotations

import datetime as dt
import io
import json
import os
import re
import sys
import zipfile

MAIN = "word/document.xml"
CLOCK = lambda: dt.datetime(2026, 11, 8, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
HEADING = re.compile(r'<w:p\b(?:(?!</w:p>).)*?w:pStyle w:val="Heading1"(?:(?!</w:p>).)*?>Chapter (\d+) ', re.DOTALL)
NUMBERED = re.compile(r">(Chapter |Section |End of chapter )(\d+)")


def chapters_of(body: str) -> tuple[int, list[str], int]:
    """Where the chapters start, each chapter's XML, and where the body's last section
    properties start."""
    starts = [m.start() for m in HEADING.finditer(body)]
    if not starts:
        raise SystemExit("no Heading1 'Chapter N' paragraphs in the base document")
    end = body.rfind("<w:sectPr")
    bounds = starts + [end]
    return starts[0], [body[a:b] for a, b in zip(bounds, bounds[1:])], end


def grown(body: str, count: int) -> str:
    """``body`` with its chapters repeated until there are ``count``; the copies' paragraph
    ids and bookmarks left out (the save stamps ids of their own), their numbers shifted."""
    _, chapters, end = chapters_of(body)
    if count < len(chapters):
        raise SystemExit(f"the base has {len(chapters)} chapters, more than {count}")
    plain = [re.sub(r"<w:bookmark(Start|End)\b[^>]*/>", "",
                    re.sub(r' w14:(paraId|textId)="[0-9A-F]+"', "", c)) for c in chapters]
    copies = []
    for n in range(len(chapters), count):
        shift = n - n % len(chapters)
        copies.append(NUMBERED.sub(lambda m, shift=shift: f">{m.group(1)}{int(m.group(2)) + shift}",
                                   plain[n % len(chapters)]))
    return body[:end] + "".join(copies) + body[end:]


def repackaged(data: bytes, count: int) -> bytes:
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            part = source.read(info.filename)
            if info.filename == MAIN:
                part = grown(part.decode("utf-8"), count).encode("utf-8")
            target.writestr(info, part)
    return out.getvalue()


def saved(data: bytes, update: bool) -> bytes:
    """``data`` opened and saved through docx-agent's tools, its fields updated."""
    from ooxml_edit.tools import Toolbox
    from ooxml_edit.tools.limits import Limits

    from docx_agent.tools import FORMAT, GROUPS, TOOLS

    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0,
                 limits=Limits(render_timeout=3600, layout_timeout=3600, batch_timeout=3600)) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(data, "report.docx")
        if update:
            out = box.dispatch(session, "word_fields", {"doc": doc, "action": "update"}).to_json()
            if not out.get("ok", True):
                raise SystemExit(json.dumps(out, indent=1)[:3000])
        return session.entry(doc).document.to_bytes()


def main() -> None:
    base, path, count = sys.argv[1], sys.argv[2], int(sys.argv[3])
    with open(base, "rb") as handle:
        data = saved(repackaged(handle.read(), count), update=not os.environ.get("NOUPDATE"))
    with open(path, "wb") as handle:
        handle.write(data)
    print(json.dumps({"bytes": len(data), "chapters": count}))


if __name__ == "__main__":
    main()
