"""Word's Rust layout against docx2svg's, on made-up documents and docx2svg's own samples.

* measure + paginate (`word-check check`): every paragraph's pieces, lines and line heights,
  every table row's height, and every page start and page's info equal docx2svg's;
* incremental (`word-check edits`): each of 15 edits re-measured and re-paginated from the
  base equals docx2svg's full layout of the edited document;
* the owner (ooxml_core): opened on the report, each edit committed as a change set, its
  snapshot's page starts, page numbers and computed fields (contents, PAGEREF, PAGE,
  NUMPAGES) equal docx2svg's;
* the registry (open_document, commit, call, close_document) round trip.
"""
from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path

import pytest

from conftest import ROOT, export_json

SAMPLES = [
    ROOT / "python/docx2svg/tests/fixtures/wordto" / name
    for name in ("sample-1page.docx", "sample-5pages.docx", "sample-10pages.docx", "sample-with-images.docx",
                 "sample-with-table.docx")
] + [
    ROOT / "python/docx2svg/tests/fixtures/samplelib" / name
    for name in ("sample-long.docx", "sample-resume.docx", "sample-simple.docx")
]


def run_check(word_check: Path, *args: Path | str) -> dict:
    out = subprocess.run([str(word_check), *map(str, args)], capture_output=True, text=True, check=True)
    return json.loads(out.stdout)


def assert_check_equal(result: dict) -> None:
    m = result["measure"]
    assert m["examples"] == [], m["examples"]
    for key in ("pieces_equal", "lines_equal", "heights_equal"):
        assert m[key] == m["paragraphs"], (key, m)
    assert m.get("rows_equal", 0) == m.get("rows", 0), m
    for key in ("paginate_docx2svg_heights", "paginate_rust_heights"):
        if key in result:
            assert result[key]["all_equal"], (key, result[key])


def test_breaks(work, word_check):
    """The line breaker's rules: justified lines, emergency breaks, tabs, hyphens, faces."""
    pytest.importorskip("docx_agent")
    path = work / "breaks.docx"
    if not path.exists():
        import sys

        subprocess.run([sys.executable, str(ROOT / "python/ooxml-core/tests/harness/make_breaks.py"), str(path), "120"],
                       check=True, stdout=subprocess.DEVNULL)
    out = work / "breaks.json"
    if not out.exists():
        export_json(path.read_bytes(), out)
    assert_check_equal(run_check(word_check, "check", out))


@pytest.mark.parametrize("sample", SAMPLES, ids=lambda p: p.stem)
def test_samples(work, word_check, sample):
    """docx2svg's samples: the whole layout where the exporter takes the document, else (a
    drawing, a picture) line breaking of every paragraph it can take, as lane S's corpus."""
    out = work / f"{sample.stem}.json"
    if not out.exists():
        from docx_agent.rust.exporter import Unsupported

        try:
            export_json(sample.read_bytes(), out)
        except (SystemExit, Unsupported):
            export_json(sample.read_bytes(), out, lines_only=True)
    assert_check_equal(run_check(word_check, "check", out))


def test_report(word_check, report_export):
    result = run_check(word_check, "check", report_export)
    assert_check_equal(result)
    assert result["paginate_docx2svg_heights"]["pages_rust"] >= 2


def test_report_edits(word_check, report_export, edited_exports):
    result = run_check(word_check, "edits", report_export, *edited_exports.values())
    assert result["base_pagination_equal"]
    assert len(result["rows"]) == len(edited_exports)
    for row in result["rows"]:
        assert row["changed_heights_equal"], row
        assert row["incremental_vs_docx2svg"]["all_equal"], row
        assert row["full_rust_vs_docx2svg"]["all_equal"], row


# -- the owner, through the Python module ---------------------------------------------------

def _compare(snap, export: dict) -> dict:
    want = [(a, b, list(c)) for a, b, c in (tuple(p["start"]) for p in export["pages"])]
    got = [(a, b, list(c)) for a, b, c in snap.starts()]
    differ = []
    fields = snap.field_texts()
    for item, para, run, keyword, text in fields:
        it = export["items"][item]
        if it["t"] == "p":
            want_text = it["measure"]["runs"][run]["text"]
        else:
            ps = [p for r in it["rows"] for c in r["cells"] for p in c["paragraphs"]]
            want_text = ps[para]["measure"]["runs"][run]["text"]
        if want_text != text:
            differ.append([item, keyword, text, want_text])
    return {"starts_equal": got == want, "pages": [len(got), len(want)],
            "numbers_equal": [i[0] for i in snap.info()] == [i[0] for i in export["info"]],
            "fields": len(fields), "fields_differ": differ[:5]}


def test_owner_fields(report, report_export, edited_exports):
    """The owner's field passes and incremental relayout from a change set, per edit."""
    ooxml_core = pytest.importorskip("ooxml_core")
    pytest.importorskip("docx_agent")
    from lxml import etree

    import edits
    from docx_agent import Document
    from mirror import Mirror, key_of

    w_p = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}p"
    w_tbl = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}tbl"
    data = report.read_bytes()

    # the base, opened whole
    mirror = Mirror(Document.open(data))
    text, _ = mirror.export()
    owner, _ = ooxml_core.Owner.open(text)
    _, snap, _, _ = owner.check(0, json.dumps({"blocks": []}))
    base = _compare(snap, json.loads(report_export.read_text()))
    owner.stop()
    assert base["starts_equal"] and base["numbers_equal"] and not base["fields_differ"], base
    assert base["fields"] > 0, "the report has computed fields"

    def digest(element) -> bytes:
        return hashlib.blake2b(etree.tostring(element), digest_size=12).digest()

    for name, ref in edited_exports.items():
        kind, position = name.split("@")
        document = Document.open(data)
        mirror = Mirror(document)
        text, _ = mirror.export()
        owner, _ = ooxml_core.Owner.open(text)
        try:
            before = {id(u.element): digest(u.element) for u in mirror.units}
            old_keys = {key_of(u.element) for u in mirror.units}
            edits.edit(document, kind, position)
            # the change set: the blocks whose XML changed or that are new (a test-only digest)
            changed = [key_of(child) for child in mirror.body if child.tag in (w_p, w_tbl)
                       and before.get(id(child)) != digest(child)]
            created = [k for k in changed if k not in old_keys]
            commit, _ = mirror.changes([{"changed": [k for k in changed if k in old_keys], "created": created}], 1)
            owner.commit(commit)
            _, snap, _, _ = owner.check(1, json.dumps({"blocks": []}))
            row = _compare(snap, json.loads(ref.read_text()))
        finally:
            owner.stop()
        assert row["starts_equal"] and row["numbers_equal"] and not row["fields_differ"], (name, row)


def test_registry(report):
    """open_document, commit, call and close_document: a document's owner by its id."""
    ooxml_core = pytest.importorskip("ooxml_core")
    pytest.importorskip("docx_agent")
    from docx_agent import Document
    from mirror import Mirror

    text, _ = Mirror(Document.open(report.read_bytes())).export()
    opened = ooxml_core.open_document("d1", "docx", text)
    assert json.loads(opened)
    assert ("d1", "docx") in ooxml_core.documents()
    answer = json.loads(ooxml_core.call("d1", 0, json.dumps({"blocks": []})))
    assert isinstance(answer, dict)
    ooxml_core.close_document("d1")
    assert ("d1", "docx") not in ooxml_core.documents()
    with pytest.raises(Exception):
        ooxml_core.call("d1", 0, json.dumps({"blocks": []}))


def test_field_pages(report):
    """field_pages: each key's first page and every page's number, section, story kind and
    blankness, of exactly one version (a fields update's reading); docx2svg's pages."""
    ooxml_core = pytest.importorskip("ooxml_core")
    pytest.importorskip("docx_agent")
    if not hasattr(ooxml_core, "field_pages"):
        pytest.skip("needs ooxml-core 0.4.0 (field_pages)")
    from docx_agent import Document
    from mirror import Mirror

    document = Document.open(report.read_bytes())
    mirror = Mirror(document)
    text, _ = mirror.export()
    ooxml_core.open_document("fp1", "docx", text)
    try:
        keys = [u.key for u in mirror.units]
        answer = json.loads(ooxml_core.field_pages("fp1", 0, keys + ["p:00000000"], True))
        layout = document.layout()
        assert answer["pages"] == len(layout.pages) == len(answer["info"])
        for row, page in zip(answer["info"], layout.pages):
            assert row == [page.info.number, page.info.section, ("default", "first", "even").index(page.info.story),
                           page.info.blank]
        expected = []
        for key in keys:
            placed = layout.where(key)
            expected.append(placed[0].page if placed else None)
        assert answer["first"] == expected + [None]
        assert "info" not in json.loads(ooxml_core.field_pages("fp1", 0, keys[:1]))
    finally:
        ooxml_core.close_document("fp1")
