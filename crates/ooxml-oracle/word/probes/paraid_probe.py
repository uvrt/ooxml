#!/usr/bin/env python3
"""Measure what Word does with paraIds it did not write (ROADMAP.md, risk 1; Open XML SDK
#925), and with note, comment, revision and bookmark ids.

The question decides how durable ``p:<paraId>`` is.  Each probe is a document written here
(or made by Word), edited through the tools (``probe_tools``: the tool host docx-agent serves),
then handed to Word through ``crates/ooxml-oracle/word/oracle.py``
(the machine-wide lock, docx2svg's AppleScripts) in some of these scenarios:

* ``save``          -- open it and save it, no edit;
* ``save-again``    -- open that saved file and save it again;
* ``edit-other``    -- type into another paragraph, then save;
* ``edit-stamped``  -- type into a paragraph docx-agent stamped, then save;
* ``comment``       -- add a comment to another paragraph, then save.

For every labelled paragraph (its text starts with a label, ``S1``, ``O1``...) the paraId
and textId before and after are compared: kept, replaced, dropped, added.

The probes, in four groups:

* **mixed** (modes 15, 14 and none -- Word 2007's 12): paraIds docx-agent stamped
  (paragraph stamping: one by ``set_text``, one inserted, one stamped then typed into in
  Word) beside paraIds another tool wrote -- in range with a textId, without a textId, out
  of range, repeated -- a paragraph with none, and a footnote, a comment, an insertion and a
  bookmark with ids far from Word's own;
* **factors** (mode 15, save only): a document whose every paragraph has a valid, unique
  paraId, one of ours among them -- then the same with exactly one thing changed: one
  paragraph without a paraId, one out of range, one repeated, a table row without one, a
  header paragraph without one, a picture without ``wp14:anchorId``/``editId``, and with a
  comment, an insertion, a bookmark, a footnote;
* **decided** -- docx-agent's default, document stamping, on the complete document in modes
  15, 14 and none, with a picture, and on a document Word made itself (which has no
  paraIds at all), in every scenario;
* **tools** (save only) -- the tools' outputs Word is handed in production: a fixture with a
  table row without a paraId after ``word_fields update`` (which writes no row) and after
  ``word_move``, and each factor's document (one row, header paragraph, picture or paragraph
  without its id) after ``word_fields update``: how many blocks each output holds, how many
  without an id, and how many of its ids Word's saved file keeps.

    python crates/ooxml-oracle/word/probes/paraid_probe.py                 # prints the verdicts, writes the JSON
    python crates/ooxml-oracle/word/probes/paraid_probe.py --tools         # the tools group only, into the JSON

Since docx-agent 0.2.0 the edits are the tools' (``word_set_text``, ``word_insert_markdown``),
and the tools have one way of stamping ids: what they do on a document's first edit.  The
documents the observations call *paragraph stamping* (docx-agent's Python ``stamping=
"paragraph"``, which stamped only the paragraphs it edited) are now made with the tools'
stamping too, so a re-run measures the tools' stamping under those keys; the recorded
observation is the Python one's until then.  Snapshots read the package's parts directly.

Nothing Word writes is committed; only the observations (ids and verdicts) are, in
``crates/ooxml-oracle/word/observations/paraid-durability.json``.
"""

from __future__ import annotations

import argparse
import io
import json
import re
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402  (the driver, the fixture makers and the cases on the path)

import oracle  # noqa: E402
from make_fixtures import AUTHOR, DECL, NS, REL, WML, content_types, p, r, relationships, styles  # noqa: E402

import probe_tools  # noqa: E402
from lxml import etree  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "paraid-durability.json"
SECTION = ('<w:sectPr>{refs}<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" '
           'w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>')

EDIT_SCRIPT = r'''
on run argv
  set inputPath to item 1 of argv
  set outputPath to item 2 of argv
  set action to item 3 of argv
  set n to (item 4 of argv) as integer
  with timeout of 120 seconds
    tell application "Microsoft Word"
      activate
      open (POSIX file inputPath)
      set d to active document
      if action is "edit" then
        set r to text object of paragraph n of d
        set s to start of content of r
        insert text "Edited " at (create range d start s end s)
      else if action is "comment" then
        make new Word comment at d with properties {comment text:"Probe comment", scope:(text object of paragraph n of d)}
      end if
      save as d file name outputPath file format format document
      close d saving no
      quit saving no
    end tell
  end timeout
end run
'''

NEW_DOCUMENT_SCRIPT = r'''
on run argv
  set outputPath to item 2 of argv
  with timeout of 120 seconds
    tell application "Microsoft Word"
      activate
      set d to make new document
      insert text "W1 Word's first paragraph" & return & "W2 Word's second paragraph" & return & "W3 Word types into this other paragraph" & return & "W4 Word's last paragraph" at end of text object of d
      save as d file name outputPath file format format document
      close d saving no
      quit saving no
    end tell
  end timeout
end run
'''


# -- documents --------------------------------------------------------------------------------


def png() -> bytes:
    """A 2x2 grey PNG, written here."""
    import struct
    import zlib

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    rows = b"".join(b"\x00" + b"\x80\x80\x80" * 2 for _ in range(2))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


#: An inline picture as a generator writes one: no wp14:anchorId or wp14:editId.
PICTURE = p(inner='<w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/'
            'wordprocessingDrawing"><wp:extent cx="457200" cy="457200"/><wp:docPr id="1" name="Picture 1"/>'
            '<a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData '
            'uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.'
            'openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="0" name="grey.png"/>'
            '<pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId9"/><a:stretch><a:fillRect/>'
            '</a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="457200" cy="457200"/>'
            '</a:xfrm><a:prstGeom prst="rect"/></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline>'
            '</w:drawing></w:r>' + r(" P1 a picture"), para="1A2B3C12")


def package(body: str, *, mode: int | None = 15, header: str | None = None,
            footnotes: str | None = None, comments: str | None = None, picture: bool = False) -> bytes:
    """A small Word package around ``body``: styles, settings in ``mode`` (``None``: no
    compatibility setting) and the optional story parts."""
    types = {"word/document.xml": WML + ".document.main+xml", "word/styles.xml": WML + ".styles+xml",
             "word/settings.xml": WML + ".settings+xml"}
    rels = [("rId1", REL + "styles", "styles.xml", False), ("rId2", REL + "settings", "settings.xml", False)]
    parts: dict[str, str] = {}
    refs = ""
    if header is not None:
        types["word/header1.xml"] = WML + ".header+xml"
        rels.append(("rId3", REL + "header", "header1.xml", False))
        parts["word/header1.xml"] = f"{DECL}<w:hdr {NS}>{header}</w:hdr>"
        refs = '<w:headerReference w:type="default" r:id="rId3"/>'
    if footnotes is not None:
        types["word/footnotes.xml"] = WML + ".footnotes+xml"
        rels.append(("rId4", REL + "footnotes", "footnotes.xml", False))
        parts["word/footnotes.xml"] = f"{DECL}<w:footnotes {NS}>{footnotes}</w:footnotes>"
    media: dict[str, bytes] = {}
    if picture:
        rels.append(("rId9", REL + "image", "media/grey.png", False))
        media["word/media/grey.png"] = png()
    if comments is not None:
        types["word/comments.xml"] = WML + ".comments+xml"
        rels.append(("rId5", REL + "comments", "comments.xml", False))
        parts["word/comments.xml"] = f"{DECL}<w:comments {NS}>{comments}</w:comments>"
    compat = ("" if mode is None else '<w:compat><w:compatSetting w:name="compatibilityMode" '
              f'w:uri="http://schemas.microsoft.com/office/word" w:val="{mode}"/></w:compat>')
    notes = '<w:footnotePr><w:footnote w:id="-1"/><w:footnote w:id="0"/></w:footnotePr>' if footnotes else ""
    settings = (f'{DECL}<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
                f'<w:defaultTabStop w:val="708"/>{notes}{compat}</w:settings>')
    parts = {
        "[Content_Types].xml": content_types(types).replace(
            '<Default Extension="xml"', '<Default Extension="png" ContentType="image/png"/><Default Extension="xml"'),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships(rels),
        "word/document.xml": f"{DECL}<w:document {NS}><w:body>{body}{SECTION.format(refs=refs)}</w:body></w:document>",
        "word/styles.xml": styles(),
        "word/settings.xml": settings,
        **parts,
    }
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, data in parts.items():
            archive.writestr(zipfile.ZipInfo(name, date_time=(2026, 10, 3, 12, 0, 0)), data.encode())
        for name, data in media.items():
            archive.writestr(zipfile.ZipInfo(name, date_time=(2026, 10, 3, 12, 0, 0)), data)
    return buffer.getvalue()


FOOTNOTES = ('<w:footnote w:type="separator" w:id="-1">' + p(inner="<w:r><w:separator/></w:r>", para="5E6F7001")
             + '</w:footnote><w:footnote w:type="continuationSeparator" w:id="0">'
             + p(inner="<w:r><w:continuationSeparator/></w:r>", para="5E6F7002") + "</w:footnote>"
             '<w:footnote w:id="5"><w:p w14:paraId="5E6F7003" w14:textId="77777777"><w:pPr>'
             '<w:pStyle w:val="FootnoteText"/></w:pPr><w:r><w:rPr><w:rStyle w:val="FootnoteReference"/>'
             '</w:rPr><w:footnoteRef/></w:r>' + r(" The footnote.") + "</w:p></w:footnote>")
COMMENTS = ('<w:comment w:id="7" w:author="Reviewer" w:date="2026-10-01T09:00:00Z" w:initials="RV">'
            + p(inner="<w:r><w:annotationRef/></w:r>" + r("The comment."), para="6F708001") + "</w:comment>")
FOOTNOTE = p(inner=r("F1 a footnote, id 5") + '<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr>'
             '<w:footnoteReference w:id="5"/></w:r>', para="1A2B3C0F")
COMMENT = p(inner='<w:commentRangeStart w:id="7"/>' + r("C1 a comment, id 7") + '<w:commentRangeEnd w:id="7"/>'
            '<w:r><w:commentReference w:id="7"/></w:r>', para="1A2B3C10")
INSERTION = p(inner=r("I1 an insertion, id 42: ") + f'<w:ins w:id="42" {AUTHOR}>' + r("inserted") + "</w:ins>",
              para="1A2B3C11")
BOOKMARK = p(inner='<w:bookmarkStart w:id="9" w:name="probeMark"/>' + r("B1 a bookmark, id 9")
             + '<w:bookmarkEnd w:id="9"/>', para="1A2B3C0E")


def mixed(mode: int | None) -> bytes:
    body = "".join([
        p("S1 stamped by docx-agent through set_text"),
        p("O1 another tool's paraId, in range, with a textId", "1A2B3C04"),
        BOOKMARK,
        p("O2 another tool's paraId without a textId", "1A2B3C05", None),
        p("N1 no paraId"),
        p("R1 another tool's paraId, out of range", "80000006"),
        p("D1 a repeated paraId, first", "2B2B2B2B"),
        p("D2 a repeated paraId, second", "2B2B2B2B"),
        p("T1 Word types into this other paragraph", "1A2B3C09"),
        p("S2 stamped by docx-agent, then Word types into it"),
        FOOTNOTE, COMMENT, INSERTION,
    ])
    data = package(body, mode=mode, footnotes=FOOTNOTES, comments=COMMENTS)
    document = probe_tools.Session(data)
    _stamp_by_editing(document, "S1")
    document.call("word_insert_markdown", at=f"after:{by_label(document, 'S1')}",
                  markdown="S3 inserted by docx-agent\n")
    _stamp_by_editing(document, "S2")
    return document.data()


def complete(mode: int | None = 15, *, change: str | None = None) -> bytes:
    """Every paragraph with a valid, unique paraId, S2 stamped by docx-agent among them --
    then, with ``change``, exactly one thing different."""
    document = probe_tools.Session(unstamped(mode, change=change))
    _stamp_by_editing(document, "S2")
    return document.data()


def unstamped(mode: int | None = 15, *, change: str | None = None) -> bytes:
    """:func:`complete`'s document as written, before the tools stamped S2."""
    body = [
        p("O1 another tool's paraId, in range", "1A2B3C01"),
        p("T1 Word types into this other paragraph", "1A2B3C02"),
        p("S2 stamped by docx-agent, then Word types into it"),
        p("O4 another tool's paraId, last", "1A2B3C04"),
    ]
    header = footnotes = comments = None
    picture = change == "picture"
    if picture:
        body.insert(1, PICTURE)
    if change == "missing":
        body.insert(1, p("N1 no paraId"))
    elif change == "out-of-range":
        body.insert(1, p("R1 out of range", "80000005"))
    elif change == "repeat":
        body.insert(1, p("D1 a repeat of O1", "1A2B3C01"))
    elif change == "row":
        body.insert(1, '<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/>'
                       '</w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/></w:tcPr>'
                       + p("X1 in a row without a paraId", "1A2B3C05") + "</w:tc></w:tr></w:tbl>")
    elif change == "header":
        header = p("H1 a header paragraph without a paraId")
    elif change == "comment":
        body.insert(1, COMMENT)
        comments = COMMENTS
    elif change == "insertion":
        body.insert(1, INSERTION)
    elif change == "bookmark":
        body.insert(1, BOOKMARK)
    elif change == "footnote":
        body.insert(1, FOOTNOTE)
        footnotes = FOOTNOTES
    return package("".join(body), mode=mode, header=header, footnotes=footnotes, comments=comments,
                   picture=picture)


def decided(data: bytes) -> bytes:
    """The tools' stamping on the document's first edit, then a paragraph inserted."""
    document = probe_tools.Session(data)
    label = "S2" if _has(document, "S2") else "W2"
    _stamp_by_editing(document, label)
    document.call("word_insert_markdown", at=f"after:{document.paragraph_ids()[0]}",
                  markdown="S3 inserted by docx-agent\n")
    return document.data()


#: The tools' writes the ``tools`` group hands to Word: on a fixture as opened, or on a factor's
#: document as written (one block without its id), one call each.
TOOL_WRITES: dict[str, tuple[str, str, dict]] = {
    "toc-numbered, word_fields update": ("generated/toc-numbered.docx", "word_fields", {"action": "update"}),
    "toc-numbered, word_move": ("generated/toc-numbered.docx", "word_move",
                                {"section_heading": "$p12", "before": "$p6"}),
    **{f"factor {change}, word_fields update": (f"factor:{change}", "word_fields", {"action": "update"})
       for change in ("row", "header", "picture", "missing")},
}

#: A field for the factors' documents, so that ``word_fields update`` has something to write.
DATE_FIELD = p(inner='<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> '
                     'NUMPAGES </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>'
                     + r("9") + '<w:r><w:fldChar w:fldCharType="end"/></w:r>', para="1A2B3C20")


def tool_write(name: str) -> tuple[bytes, bytes]:
    """The document a :data:`TOOL_WRITES` case starts from, and the tools' output."""
    source, tool, arguments = TOOL_WRITES[name]
    if source.startswith("factor:"):
        data = unstamped(change=source.removeprefix("factor:"))
        data = _with_field(data)
    else:
        data = (probe_paths.FIXTURES / source).read_bytes()
    document = probe_tools.Session(data)
    if "$p" in json.dumps(arguments):
        ids = document.paragraph_ids()
        arguments = {k: re.sub(r"\$p(\d+)", lambda m: ids[int(m.group(1)) - 1], v) if isinstance(v, str) else v
                     for k, v in arguments.items()}
    document.call(tool, **arguments)
    return data, document.data()


def _with_field(data: bytes) -> bytes:
    """``data`` with :data:`DATE_FIELD` as the body's first paragraph."""
    parts = _parts(data)
    parts["word/document.xml"] = parts["word/document.xml"].replace(b"<w:body>", b"<w:body>" + DATE_FIELD.encode(), 1)
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, content in parts.items():
            archive.writestr(zipfile.ZipInfo(name, date_time=(2026, 10, 3, 12, 0, 0)), content)
    return buffer.getvalue()


def ids_of(data: bytes) -> dict[str, list[str | None]]:
    """Every paragraph's and table row's paraId, of every story, in order."""
    out: dict[str, list[str | None]] = {"paragraphs": [], "rows": []}
    for name, content in sorted(_parts(data).items(), key=lambda item: item[0] != "word/document.xml"):
        if not _STORIES.match(name):
            continue
        root = etree.fromstring(content)
        out["paragraphs"] += [e.get(_W14 + "paraId") for e in root.iter(_W + "p")]
        out["rows"] += [e.get(_W14 + "paraId") for e in root.iter(_W + "tr")]
    return out


def durability(output: bytes, saved: bytes) -> dict:
    """What Word kept of the tools' output's ids on saving it: per kind, how many blocks the
    output had, how many without an id, and how many of its ids Word's file still has."""
    before, after = ids_of(output), ids_of(saved)
    out = {}
    for kind in ("paragraphs", "rows"):
        held = [i for i in before[kind] if i]
        out[kind] = {"blocks": len(before[kind]), "without an id": len(before[kind]) - len(held),
                     "ids kept by Word": len(set(held) & {i for i in after[kind] if i})}
    return out


def run_tools(session: oracle.Session) -> dict:
    """The ``tools`` group: each :data:`TOOL_WRITES` output, as written and saved by Word."""
    results: dict = {}
    for k, name in enumerate(TOOL_WRITES):
        source, output = tool_write(name)
        outcome = session.resave(output, name=f"tools-{k}")
        if not outcome:
            results[name] = {"outcome": outcome.outcome, "detail": outcome.detail}
            continue
        results[name] = {"input": {kind: {"blocks": v["blocks"], "without an id": v["without an id"]}
                                   for kind, v in durability(source, source).items()},
                         "save": durability(output, outcome.path.read_bytes())}
    return results


def _has(document: probe_tools.Session, label: str) -> bool:
    try:
        by_label(document, label)
        return True
    except KeyError:
        return False


def _stamp_by_editing(document: probe_tools.Session, label: str) -> None:
    block = by_label(document, label)
    text = next(t for b, t in document.blocks() if b == block)
    document.call("word_set_text", target=block, text=text + ".")


def by_label(document: probe_tools.Session, label: str) -> str:
    """The body block whose text starts with ``label``: its id."""
    for block, text in document.blocks():
        if text.removeprefix("Edited ").startswith(label + " "):
            return block
    raise KeyError(label)


# -- observing --------------------------------------------------------------------------------


_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
_W14 = "{http://schemas.microsoft.com/office/word/2010/wordml}"
#: The story parts, in the order their paragraphs are read.
_STORIES = re.compile(r"word/(document|header\d+|footer\d+|footnotes|endnotes|comments)\.xml$")


def _parts(data: bytes) -> dict[str, bytes]:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return {name: archive.read(name) for name in archive.namelist()}


def _paragraphs(data: bytes, body_only: bool = False) -> list[tuple[str, str | None, str | None]]:
    """Each paragraph's text, paraId and textId: the body's, then (unless ``body_only``) the
    other stories'."""
    out = []
    for name, content in sorted(_parts(data).items(), key=lambda item: item[0] != "word/document.xml"):
        if not _STORIES.match(name) or (body_only and name != "word/document.xml"):
            continue
        for paragraph in etree.fromstring(content).iter(_W + "p"):
            text = "".join(t.text or "" for t in paragraph.iter(_W + "t"))
            out.append((text, paragraph.get(_W14 + "paraId"), paragraph.get(_W14 + "textId")))
    return out


def _mode(data: bytes) -> int:
    """The compatibility mode (12, Word 2007's, where the settings name none)."""
    found = re.search(rb'w:name="compatibilityMode"[^>]*w:val="(\d+)"', _parts(data).get("word/settings.xml", b""))
    return int(found.group(1)) if found else 12


def snapshot(data: bytes) -> dict:
    """Per label, ``[paraId, textId, typed into by Word]``; the table rows' paraIds; the
    note, comment, insertion and bookmark ids; and the compatibility mode."""
    out: dict = {"paragraphs": {}, "mode": _mode(data)}
    for text, para_id, text_id in _paragraphs(data):
        match = re.match(r"([A-Z]\d) ", text.removeprefix("Edited "))
        if match:
            out["paragraphs"][match.group(1)] = [para_id, text_id, text.startswith("Edited ")]
    xml = b"".join(content for name, content in _parts(data).items()
                   if name.startswith("word/") and name.endswith(".xml"))
    out["rows"] = [m.decode() for m in re.findall(rb'<w:tr\b[^>]*?w14:paraId="([0-9A-Fa-f]+)"', xml)]
    for key, pattern in (("footnote ids", rb'<w:footnoteReference w:id="(-?\d+)"'),
                         ("comment ids", rb'<w:commentReference w:id="(\d+)"'),
                         ("insertion ids", rb'<w:ins w:id="(\d+)"'),
                         ("bookmark ids", rb'<w:bookmarkStart w:id="(\d+)" w:name="probeMark"')):
        out[key] = sorted({m.decode() for m in re.findall(pattern, xml)})
    return out


def verdict(before: list | None, after: list | None) -> str:
    if before is None or after is None:
        return "paragraph missing"
    (para0, text0, _), (para1, text1, edited) = before, after
    if para0 is None:
        para = "added" if para1 else "none"
    else:
        para = "kept" if para1 == para0 else ("dropped" if para1 is None else "replaced")
    if text0 is None:
        text = "added" if text1 else "none"
    else:
        text = "kept" if text1 == text0 else ("dropped" if text1 is None else "renewed")
    return f"paraId {para}, textId {text}" + (" (typed into in Word)" if edited else "")


def paragraph_index(data: bytes, label: str) -> int:
    """The 1-based index Word's ``paragraph n of document`` has for a label."""
    for number, (text, _, _) in enumerate(_paragraphs(data, body_only=True), start=1):
        if text.removeprefix("Edited ").startswith(label + " "):
            return number
    raise KeyError(label)


def compare(before: dict, after: dict) -> dict:
    out = {"mode": after["mode"],
           "paragraphs": {label: verdict(before["paragraphs"].get(label), after["paragraphs"].get(label))
                          for label in before["paragraphs"]}}
    if before["rows"] or after["rows"]:
        out["rows"] = [before["rows"], after["rows"]]
    ids = {key: [before[key], after[key]] for key in ("footnote ids", "comment ids", "insertion ids", "bookmark ids")
           if before[key] or after[key]}
    if ids:
        out["ids"] = ids
    return out


def run(session: oracle.Session, name: str, data: bytes, *, other: str | None = None,
        stamped: str | None = None) -> dict:
    before = snapshot(data)
    results: dict = {}
    scenarios = {"save": lambda: session.resave(data, name=name)}
    if other:
        scenarios["edit-other"] = lambda: session.run_script(
            EDIT_SCRIPT, data, name=name, tag="edit-other", args=["edit", str(paragraph_index(data, other))])
        scenarios["comment"] = lambda: session.run_script(
            EDIT_SCRIPT, data, name=name, tag="comment", args=["comment", str(paragraph_index(data, other))])
    if stamped:
        scenarios["edit-stamped"] = lambda: session.run_script(
            EDIT_SCRIPT, data, name=name, tag="edit-stamped", args=["edit", str(paragraph_index(data, stamped))])
    saved = None
    for scenario, action in scenarios.items():
        outcome = action()
        if not outcome:
            results[scenario] = {"outcome": outcome.outcome, "detail": outcome.detail}
            continue
        written = outcome.path.read_bytes()
        if scenario == "save":
            saved = written
        results[scenario] = compare(before, snapshot(written))
    if saved is not None and other:
        outcome = session.resave(saved, name=name + "-again")
        if outcome:
            results["save-again"] = compare(snapshot(saved), snapshot(outcome.path.read_bytes()))
    return results


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--json", type=Path, default=OBSERVATIONS)
    parser.add_argument("--tools", action="store_true",
                        help="measure the tools group only, into the observations as they are")
    args = parser.parse_args()
    if args.tools:
        observations = json.loads(args.json.read_text(encoding="utf-8"))
        if word_version() != observations["word"]:
            raise SystemExit(f"Word {word_version()} is not the observations' {observations['word']}: run them all")
        with oracle.session() as session:
            observations["tools"] = run_tools(session)
        args.json.write_text(json.dumps(observations, indent=1) + "\n")
        report_tools(observations["tools"])
        return
    observations: dict = {"word": word_version(), "mixed": {}, "factors": {}, "decided": {}}
    with oracle.session() as session:
        for mode in (15, 14, None):
            label = f"mode {mode or 'none'}"
            observations["mixed"][label] = run(session, f"mixed{mode or 0}", mixed(mode), other="T1", stamped="S2")
        for change in (None, "missing", "out-of-range", "repeat", "row", "header", "picture", "comment",
                       "insertion", "bookmark", "footnote"):
            observations["factors"][change or "complete"] = run(session, f"factor-{change or 'complete'}",
                                                                complete(change=change))
        for mode in (15, 14, None):
            observations["decided"][f"generated, mode {mode or 'none'}"] = run(
                session, f"decided{mode or 0}", decided(complete(mode)), other="T1", stamped="S3")
        observations["decided"]["generated, a picture, mode 15"] = run(
            session, "decided-picture", decided(complete(15, change="picture")), other="T1", stamped="S3")
        created = session.run_script(NEW_DOCUMENT_SCRIPT, None, name="word-created", tag="new")
        if created:
            made = created.path.read_bytes()
            observations["word-created"] = {
                "as made": snapshot(made)["paragraphs"],
                "paragraph stamping": run(session, "created-paragraph", _paragraph_stamped(made)),
            }
            observations["decided"]["Word-created, mode 15"] = run(session, "created-decided", decided(made),
                                                                    other="W3", stamped="S3")
        else:
            observations["word-created"] = {"outcome": created.outcome, "detail": created.detail}
        observations["tools"] = run_tools(session)
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(observations, indent=1) + "\n")
    report(observations)


def _paragraph_stamped(data: bytes) -> bytes:
    document = probe_tools.Session(data)
    _stamp_by_editing(document, "W2")
    return document.data()


def word_version() -> str | None:
    import plistlib

    try:
        with open(oracle.WORD_APP / "Contents" / "Info.plist", "rb") as handle:
            return plistlib.load(handle).get("CFBundleShortVersionString")
    except OSError:
        return None


def report(observations: dict, out=sys.stdout) -> None:
    print(f"Word {observations['word']}", file=out)
    for group in ("mixed", "factors", "decided"):
        for probe, scenarios in observations[group].items():
            print(f"\n## {group}: {probe}", file=out)
            _scenarios(scenarios, out)
    created = observations.get("word-created", {})
    if "as made" in created:
        print("\n## Word-created, as Word made it", file=out)
        print("    " + ", ".join(f"{k}: {v[0]}" for k, v in created["as made"].items()), file=out)
        print("## Word-created, paragraph stamping", file=out)
        _scenarios(created["paragraph stamping"], out)
    if "tools" in observations:
        print("\n## tools: their outputs, saved by Word", file=out)
        report_tools(observations["tools"], out)


def report_tools(tools: dict, out=sys.stdout) -> None:
    for name, result in tools.items():
        if "save" not in result:
            print(f"- {name}: {result.get('outcome')} {result.get('detail', '')}", file=out)
            continue
        print(f"- {name}: input {result['input']}; output and Word's save {result['save']}", file=out)


def _scenarios(scenarios: dict, out) -> None:
    for scenario, result in scenarios.items():
        if "paragraphs" not in result:
            print(f"- {scenario}: {result.get('outcome')} {result.get('detail', '')}", file=out)
            continue
        verdicts: dict[str, list[str]] = {}
        for label, text in result["paragraphs"].items():
            verdicts.setdefault(text, []).append(label)
        print(f"- {scenario} (mode {result['mode']}): "
              + "; ".join(f"{' '.join(labels)}: {text}" for text, labels in verdicts.items()), file=out)
        if "rows" in result:
            print(f"    rows: {result['rows'][0]} -> {result['rows'][1]}", file=out)
        for key, (before, after) in result.get("ids", {}).items():
            print(f"    {key}: {before} -> {after}", file=out)


if __name__ == "__main__":
    main()
