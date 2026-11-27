#!/usr/bin/env python3
"""Measure what Word does with a package whose kind (document or template, macro-enabled or
not) disagrees with its file's extension: the end-to-end trial's template finding
(ROADMAP.md, "Trial findings").

Through ``crates/ooxml-oracle/word/oracle.py``'s machine-wide lock, Word, by AppleScript, opens and exports to
PDF each of these, staged under the extension named (a dialog -- Word's "cannot open" or
repair prompt -- blocks the export: ``rejected``):

* ``template-as-docx``: ``brand.dotx``'s package as the tools hold it after ``open_document``
  (its kind unchanged), as a ``.docx`` (what a save that did not set the kind once wrote);
* ``template-saved-docx``: ``open_document`` of brand.dotx, ``save_document`` as docx;
* ``template-saved-dotx``: the same saved as ``dotx`` (a template Word opens to edit);
* ``document-as-docm``: a document made from the template (``new_document`` with
  ``template_blob``), its package unchanged, as a ``.docm``;
* ``document-saved-docm``: that package with the macro-enabled document's main content type
  and no VBA project -- written here by rewriting ``[Content_Types].xml``: no tool saves a
  ``.docm`` (docx-agent's Python ``save("y.docm")`` once wrote this form);
* ``macro-type-as-docx``: that package as a ``.docx``;

and ``word-saves-docm``: Word's own Save As macro-enabled document (``format
documentME``) of a document without macros -- the main part's content type it writes, and
whether it adds a VBA project.

    python crates/ooxml-oracle/word/probes/trial_probe.py            # every probe, written to crates/ooxml-oracle/word/observations/trial-word.json
    python crates/ooxml-oracle/word/probes/trial_probe.py NAME...    # some, printed

Only outcomes and content types are recorded; nothing Word writes is committed.
"""

from __future__ import annotations

import hashlib
import io
import json
import sys
import warnings
import zipfile
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402  (the driver, the fixture makers and the cases on the path)
from probe_paths import ROOT  # noqa: E402

import oracle  # noqa: E402
import probe_tools  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "trial-word.json"
BRAND = probe_paths.FIXTURES / "generated" / "templates" / "brand.dotx"
#: Long enough for Word to open and export a one-page document on a cold start; a dialog
#: never returns.
TIMEOUT = 90

SAVE_DOCM = """
on run argv
    set inputPosixPath to item 1 of argv
    set outputPosixPath to item 2 of argv
    with timeout of 180 seconds
        tell application "Microsoft Word"
            activate
            open (POSIX file inputPosixPath)
            set d to active document
            save as d file name outputPosixPath file format format documentME
            close d saving no
            quit saving no
        end tell
    end timeout
end run
"""


#: The macro-enabled document's main content type.
DOCM_TYPE = "application/vnd.ms-word.document.macroEnabled.main+xml"


def _made() -> dict[str, bytes]:
    """Through the tools: brand.dotx opened (``template``), saved as docx and as dotx, and a
    document made from it (``document``, dated 2026-10-05 by its creator's clock)."""
    box = probe_tools.box()
    session = box.session(clock=lambda: datetime(2026, 10, 5, tzinfo=timezone.utc))
    blob = session.add_blob(BRAND.read_bytes(), "brand.dotx")
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        for tool, arguments in (("open_document", {"blob": blob}),
                                ("save_document", {"doc": "d1", "name": "probe.docx", "format": "docx"}),
                                ("save_document", {"doc": "d1", "name": "probe.dotx", "format": "dotx"}),
                                ("new_document", {"kind": "docx", "template_blob": blob})):
            result = box.dispatch(session, tool, arguments)
            if not result.ok:
                raise RuntimeError(f"{tool}: {result.to_json()}")
    saved = {output.name: output.data for output in session.take_outputs()}
    return {"template": session.save("d1").data, "template.docx": saved["probe.docx"],
            "template.dotx": saved["probe.dotx"], "document": session.save("d2").data}


def _docm(data: bytes) -> bytes:
    """``data`` with the main part's content type the macro-enabled document's."""
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            content = source.read(info.filename)
            if info.filename == "[Content_Types].xml":
                content = content.replace(main_type(data).encode(), DOCM_TYPE.encode())
            target.writestr(info, content)
    return out.getvalue()


def main_type(data: bytes) -> str:
    """The main part's content type in a package's ``[Content_Types].xml``."""
    from lxml import etree

    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        types = etree.fromstring(archive.read("[Content_Types].xml"))
    for node in types:
        if node.get("PartName") == "/word/document.xml":
            return node.get("ContentType")
    return ""


def cases() -> dict[str, tuple[bytes, str]]:
    """name -> (package bytes, the extension it is staged under)."""
    made = _made()
    docm = _docm(made["document"])
    return {
        "template-as-docx": (made["template"], ".docx"),
        "template-saved-docx": (made["template.docx"], ".docx"),
        "template-saved-dotx": (made["template.dotx"], ".dotx"),
        "document-as-docm": (made["document"], ".docm"),
        "document-saved-docm": (docm, ".docm"),
        "macro-type-as-docx": (docm, ".docx"),
    }


def measure(names: list[str]) -> dict:
    out: dict = {}
    every = cases()
    with oracle.session() as word:
        for name, (data, suffix) in every.items():
            if names and name not in names:
                continue
            outcome = word.export_pdf(data, name=f"trial-{name}", timeout=TIMEOUT, suffix=suffix)
            out[name] = {"extension": suffix, "main_content_type": main_type(data), "outcome": outcome.outcome}
            print(name, outcome.outcome, f"{outcome.seconds:.1f}s", outcome.detail[:120])
        if not names or "word-saves-docm" in names:
            source = _made()["document"]
            digest = hashlib.sha256(source + SAVE_DOCM.encode()).hexdigest()[:16]
            target = oracle.ORACLE_DIR / f"trial-word-saves-docm-{digest}.docm"
            outcome = target.exists() and oracle.Outcome(True, "done", target) or word._run(
                ["-e", SAVE_DOCM], source, target, "trial-word-saves-docm", digest, 150)
            entry = {"outcome": outcome.outcome}
            if outcome:
                data = outcome.path.read_bytes()
                with zipfile.ZipFile(io.BytesIO(data)) as saved:
                    entry["main_content_type"] = main_type(data)
                    entry["vba_project"] = any("vbaProject" in name for name in saved.namelist())
            out["word-saves-docm"] = entry
            print("word-saves-docm", entry)
    return out


def main(names: list[str]) -> None:
    observed = measure(names)
    if names:
        print(json.dumps(observed, indent=1))
        return
    OBSERVATIONS.write_text(json.dumps({
        "_about": ("What Word 16.106 for Mac did with packages whose kind and extension agree or disagree "
                   "(crates/ooxml-oracle/word/probes/trial_probe.py): the outcome of opening and exporting each (done, or rejected: a "
                   "dialog blocked it), and the content type Word's own Save As macro-enabled document writes. "
                   "Regenerate with python crates/ooxml-oracle/word/probes/trial_probe.py."),
        **observed}, indent=1) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS.relative_to(ROOT)}")


if __name__ == "__main__":
    main(sys.argv[1:])
