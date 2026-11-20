#!/usr/bin/env python3
"""Which fixture documents a branch's fixtures gate runs (tools/gates.py ``--fixtures auto``).

A change runs the fixtures that hold the Word features it touches, plus CANARIES, a fixed
handful of mixed documents run every time.  Two maps make the choice:

* **fixture -> features** (FEATURES_FILE, generated): each fixture's features, read from its
  package's XML by DETECT.  ``python tools/gates_fixtures.py write`` makes it again after a
  fixture is added or changed; tools/test_gates.py fails while it is out of date.
* **path -> features** (``path_features``): a changed file's features, from the words in its
  path (FEATURE_WORDS: ``crates/ooxml-word-edit/src/tables/merge.rs`` is ``tables``).  The
  foundations every document goes through (FOUNDATIONS: the XML writer, the package, the
  store, the session, the layout, the harness, the known differences) and any path whose
  words name no feature count for every fixture -- conservative, as is a fixture changed
  itself (it runs; ``all`` when its features cannot be read).

Paths the gates do not run at all (tools/gates.py ``gate_effects``) select nothing.
"""

from __future__ import annotations

import json
import re
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "python" / "docx-agent" / "tests" / "fixtures"
FEATURES_FILE = Path(__file__).resolve().parent / "gates_fixtures.json"

#: Every fixture, whatever it holds.
ALL = "all"

#: Each feature and the XML that shows a document holds it: (part name pattern, content pattern).
DETECT: dict[str, list[tuple[str, str]]] = {
    "tables": [(r"word/(document|header\d*|footer\d*)\.xml", r"<w:tbl>")],
    "notes": [(r"word/(footnotes|endnotes)\.xml", r'<w:(footnote|endnote) (?![^>]*w:type="(separator|continuationSeparator|continuationNotice)")')],
    "comments": [(r"word/comments\.xml", r"<w:comment ")],
    "controls": [(r"word/.*\.xml", r"<w:sdt>")],
    "drawings": [(r"word/.*\.xml", r"<w:drawing>|<w:pict>|<w:object")],
    "fields": [(r"word/.*\.xml", r"<w:fldChar |<w:fldSimple ")],
    "lists": [(r"word/(document|footnotes|endnotes|comments)\.xml", r"<w:numPr>")],
    "styles": [(r"word/styles\.xml", r'w:customStyle="1"')],
    "tracked": [(r"word/.*\.xml", r"<w:(ins|del|moveFrom|moveTo|rPrChange|pPrChange|tblPrChange|trPrChange|tcPrChange|sectPrChange|numberingChange) ")],
    "sections": [(r"word/(header|footer)\d*\.xml", r""), (r"word/document\.xml", r"<w:sectPr[ >][\s\S]*<w:sectPr[ >]")],
    "charts": [(r"word/charts/chart\d*\.xml", r"")],
    "smartart": [(r"word/diagrams/.*\.xml", r"")],
    "links": [(r"word/.*\.xml", r"<w:hyperlink |<w:bookmarkStart ")],
    "textboxes": [(r"word/.*\.xml", r"<w:txbxContent>")],
    "legacy": [(r"word/settings\.xml", r'w:name="compatibilityMode" w:uri="[^"]*" w:val="1[1-4]"')],
}

#: The words of a changed path that name a feature (matched against its folder and file
#: names, split at ``/``, ``_``, ``-`` and ``.``).
FEATURE_WORDS: dict[str, tuple[str, ...]] = {
    "tables": ("table", "tables", "cell", "cells", "row", "rows", "tbl"),
    "notes": ("note", "notes", "footnote", "footnotes", "endnote", "endnotes"),
    "comments": ("comment", "comments"),
    "controls": ("control", "controls", "sdt"),
    "drawings": ("drawing", "drawings", "picture", "pictures", "image", "images", "drawingml", "scene",
                 "metafile", "raster", "render", "shape", "shapes"),
    "fields": ("field", "fields", "toc"),
    "lists": ("list", "lists", "numbering", "numbers"),
    "styles": ("style", "styles", "format", "formatting", "template", "design"),
    "tracked": ("review", "revision", "revisions", "track", "tracked", "tracking", "changes", "moving",
                "accept", "reject"),
    "sections": ("section", "sections", "header", "headers", "footer", "footers"),
    "charts": ("chart", "charts"),
    "smartart": ("smartart", "diagram", "diagrams"),
    "links": ("link", "links", "hyperlink", "hyperlinks", "bookmark", "bookmarks"),
    "textboxes": ("textbox", "textboxes", "txbx"),
    "legacy": ("compat", "compatibility", "legacy", "upgrade"),
}

#: Paths every fixture goes through: a change to one runs them all.
FOUNDATIONS = ("crates/ooxml-base/", "crates/ooxml-xml/", "crates/ooxml-zip/", "crates/ooxml-opc/",
               "crates/ooxml-store/", "crates/ooxml-session/", "crates/ooxml-derive/", "crates/ooxml-tools/",
               "crates/ooxml-tools-derive/", "crates/ooxml-word-model/", "crates/ooxml-word-layout/",
               "crates/ooxml-word-pages/", "crates/ooxml-text/", "crates/ooxml-fonts/", "crates/ooxml-schema/",
               "crates/ooxml-oracle/python/", "crates/ooxml-oracle/known-differences.toml",
               "crates/ooxml-oracle/src/lib.rs", "crates/ooxml-oracle/src/edits", "crates/ooxml-oracle/src/bin/",
               "crates/ooxml-oracle/Cargo.toml", "python/ooxml-common/", "python/ooxml-edit/src/ooxml_edit/opc",
               "python/ooxml-edit/src/ooxml_edit/xml", "python/docx2svg/",
               "tools/gates.py", "tools/gates_fixtures.py", ".github/workflows/gates.yml", "Cargo.toml", "Cargo.lock")

#: A few mixed fixtures every selection runs: every feature but SmartArt between them, a
#: legacy-mode document among them.
CANARIES = ("generated/review-follow.docx", "generated/ids-and-markup.docx", "generated/charts/chart-places.docx",
            "generated/markdown/constructs.docx", "wordto/sample-with-table.docx")


def fixture_names(tree: Path = FIXTURES) -> list[str]:
    return sorted(str(f.relative_to(tree)) for f in tree.rglob("*.docx") if not f.name.startswith("~$"))


def features_of(path: Path) -> list[str]:
    """The features (DETECT) the document at ``path`` holds."""
    found = set()
    with zipfile.ZipFile(path) as package:
        parts = {name: None for name in package.namelist() if name.endswith(".xml")}
        for feature, tests in DETECT.items():
            for part_pattern, content in tests:
                for name in parts:
                    if not re.fullmatch(part_pattern, name):
                        continue
                    if not content:
                        found.add(feature)
                        break
                    if parts[name] is None:
                        parts[name] = package.read(name).decode("utf-8", "replace")
                    if re.search(content, parts[name]):
                        found.add(feature)
                        break
                if feature in found:
                    break
    return sorted(found)


def scan(tree: Path = FIXTURES) -> dict[str, list[str]]:
    return {name: features_of(tree / name) for name in fixture_names(tree)}


def load() -> dict[str, list[str]]:
    return json.loads(FEATURES_FILE.read_text())


def _words(path: str) -> set[str]:
    return {w.lower() for w in re.split(r"[/_\-.]", path) if w}


#: Words of a path no gate's result reads: a benchmark's code (its timings are its output).
NO_FEATURE_WORDS = ("bench", "benches")


def path_features(path: str) -> set[str] | str:
    """The features a change to ``path`` touches, or ALL."""
    if path.startswith(FOUNDATIONS):
        return ALL
    words = _words(path)
    if any(word.endswith(NO_FEATURE_WORDS) for word in words):
        return set()
    found = {feature for feature, names in FEATURE_WORDS.items() if words & set(names)}
    return found or ALL


def select(changed: list[str], known: dict[str, list[str]] | None = None) -> list[str] | str:
    """The fixtures (names relative to the fixtures folder) a branch changing ``changed`` runs,
    or ALL; ``changed``: the changed paths the gates run at all (tools/gates.py gate_effects)."""
    known = load() if known is None else known
    prefix = str(FIXTURES.relative_to(ROOT)) + "/"
    chosen = set(CANARIES)
    features: set[str] = set()
    for path in changed:
        if path.startswith(prefix):
            name = path[len(prefix):]
            if name.endswith(".docx"):
                chosen.add(name)
                features.update(known.get(name, []))
                if name not in known:
                    return ALL
            else:
                return ALL          # the fixtures' own tooling or data
            continue
        found = path_features(path)
        if found == ALL:
            return ALL
        features |= found
    chosen |= {name for name, held in known.items() if features & set(held)}
    return sorted(name for name in chosen if name in known or (FIXTURES / name).is_file())


def main(argv: list[str]) -> int:
    if argv[:1] == ["write"]:
        FEATURES_FILE.write_text(json.dumps(scan(), indent=1, sort_keys=True) + "\n")
        print(f"{FEATURES_FILE.relative_to(ROOT)}: {len(load())} fixtures")
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
