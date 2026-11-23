import os
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

import pytest

# 0.117.0 (lane G): every kept serialization of an edited part is compared with the tree
# written out again, at every use (ooxml_edit.opc.OpcPackage.keep_serialized).
os.environ.setdefault("OOXML_EDIT_VERIFY_SERIALIZED", "1")

#: The repository's Word fixtures: the gates' documents too, so they live with the oracle
#: (crates/ooxml-oracle/fixtures), apart from docx-agent's tests.
ORACLE = Path(__file__).resolve().parents[3] / "crates" / "ooxml-oracle"
FIXTURE_DIR = ORACLE / "fixtures"
#: Word's recorded answers to the probes (crates/ooxml-oracle/word/observations), which the
#: tests holding the tools to Word read; the probes' made-up documents (word/cases) are
#: importable as modules (rows_cases, ...).
OBSERVATIONS = ORACLE / "word" / "observations"
sys.path.insert(0, str(ORACLE / "word" / "cases"))
#: A new blank document, written where the corpus is read from (``new/document``): every gate
#: and every phase's edit set holds for a document docx-agent made, as for one it opened.
NEW_DOCUMENT = Path(tempfile.gettempdir()) / "docx-agent-tests" / "new" / "document.docx"
#: Its creation date: fixed, so its bytes -- and the oracle's cache -- are the same each run.
NEW_CREATED = datetime(2026, 10, 4, tzinfo=timezone.utc)


def blank_document(created: datetime = NEW_CREATED) -> bytes:
    """A new blank document's package as the Rust core makes it (:func:`word_docs.blank`)."""
    from word_docs import blank

    return blank(created=created)


def new_document() -> Path:
    """:func:`blank_document`, written (atomically: xdist's workers each collect) when it is
    not there or differs."""
    data = blank_document()
    if not NEW_DOCUMENT.exists() or NEW_DOCUMENT.read_bytes() != data:
        NEW_DOCUMENT.parent.mkdir(parents=True, exist_ok=True)
        staged = NEW_DOCUMENT.with_name(f".{os.getpid()}.docx")
        staged.write_bytes(data)
        os.replace(staged, NEW_DOCUMENT)
    return NEW_DOCUMENT


def fixture_paths() -> list[Path]:
    return sorted(FIXTURE_DIR.glob("*/*.docx")) + [new_document()]


def fixture_id(path: Path) -> str:
    return f"{path.parent.name}/{path.stem}"


@pytest.fixture(params=fixture_paths(), ids=fixture_id)
def docx_path(request) -> Path:
    """Every fixture in turn -- the corpus a test must hold for, not one happy document."""
    return request.param


@pytest.fixture(scope="session")
def markup_doc() -> Path:
    """docx-agent's own fixture: every id edge case and the markup text reading walks."""
    return FIXTURE_DIR / "generated" / "ids-and-markup.docx"


@pytest.fixture(scope="session")
def long_doc() -> Path:
    """36 pages in docx2svg's layout, no paraIds: the multi-page reflow fixture."""
    return FIXTURE_DIR / "samplelib" / "sample-long.docx"


MARKDOWN_DIR = FIXTURE_DIR / "generated" / "markdown"
#: The chart and SmartArt documents Word saved (their ``PROVENANCE.md``).
CHARTS_DIR = FIXTURE_DIR / "generated" / "charts"


def reading_paths() -> list[Path]:
    """The corpus, E2's own fixtures and the chart documents (``generated/markdown`` and
    ``generated/charts``, one level below the corpus so the other phases' suites and the Word
    oracle keep theirs): what every reader is held to."""
    return fixture_paths() + sorted(MARKDOWN_DIR.glob("*.docx")) + sorted(CHARTS_DIR.glob("*.docx"))


@pytest.fixture(params=reading_paths(), ids=fixture_id)
def reading_path(request) -> Path:
    return request.param


@pytest.fixture(scope="session")
def constructs_doc() -> Path:
    return MARKDOWN_DIR / "constructs.docx"


@pytest.fixture(scope="session")
def review_doc() -> Path:
    return MARKDOWN_DIR / "review.docx"


@pytest.fixture(scope="session")
def dutch_doc() -> Path:
    return MARKDOWN_DIR / "dutch-template.docx"


#: The fixtures the heaviest per-fixture suites hold by default -- Word-authored with tables
#: in mode 14, a document of every id edge case and revisions, a localised template with
#: lists, and E6's new document; ``--run-slow`` holds them on every fixture.
REPRESENTATIVE = {"wordto/sample-with-table", "generated/ids-and-markup", "generated/lists-and-styles",
                  "new/document"}
#: Those suites, by test function: each runs on :data:`REPRESENTATIVE` by default and on every
#: fixture with ``--run-slow``.
SWEPT = {
    "test_e1_operations.py::test_operation",
    "test_e3_invariant.py::test_accept_all_is_the_edit_and_reject_all_the_original",
    "test_e3_operations.py::test_tracked_operation",
    "test_e3_render.py::test_e3_edits_render_and_read",
    "test_e4_operations.py::test_operation",
    "test_e4_operations.py::test_operation_renders_and_places_what_it_made",
    "test_e4_tracked.py::test_accept_all_is_the_edit_and_reject_all_the_original",
    "test_e5_operations.py::test_operation",
    "test_e5_operations.py::test_operation_renders_and_places_what_it_made",
    "test_e5_tracked.py::test_accept_all_is_the_edit_and_reject_all_the_original",
    "test_markdown_write.py::test_the_corpus_round_trips_in_every_fixture",
    "test_markdown_write.py::test_tracked_insertion_accepts_to_the_edit_and_rejects_to_the_original",
    "test_e6_copy.py::test_every_fixture_copies_into_a_new_document_and_another",
    "test_e6_upgrade.py::test_every_fixture_upgrades_and_reports_its_reflow",
}
#: Randomised sweeps: the seeds held by default (``--run-slow``: every seed).
SEEDS = {"test_e5_table_sweep.py::test_sweep_keeps_the_grid": {"0", "1"},
         "test_e5_table_sweep.py::test_sweep_tracked_accepts_and_rejects": {"0"}}


def _sweep_kept(item) -> bool:
    """Whether the default run holds this item of a swept suite."""
    key = f"{Path(str(item.fspath)).name}::{item.originalname}"
    if key in SEEDS:
        return item.callspec.id in SEEDS[key] if hasattr(item, "callspec") else True
    if key not in SWEPT or not hasattr(item, "callspec"):
        return True
    for value in item.callspec.params.values():
        if isinstance(value, Path):
            return fixture_id(value) in REPRESENTATIVE
    return True


def pytest_addoption(parser):
    parser.addoption("--run-slow", action="store_true", default=False,
                     help="also run the exhaustive sweeps marked slow (ROADMAP.md, \"Testing strategy\")")


def pytest_collection_modifyitems(config, items):
    """The slow sweeps run with ``--run-slow``.  The Word oracle never runs under xdist's
    parallel workers (ROADMAP.md, "Word is one instance per machine"): there, its tests
    skip; run ``pytest -m oracle`` without ``-n``.  A test meeting a form whose port the user
    deferred is a strict xfail naming it (:mod:`deferred_forms`), and so is one meeting a gap
    the Rust session has yet (:mod:`rust_gaps`)."""
    from deferred_forms import DEFERRED
    from rust_gaps import GAPS

    for item in items:
        # A test's own id, or its name alone for every one of its parameters.
        keys = (f"{item.path.name}::{item.name}", f"{item.path.name}::{item.originalname}")
        form = next((DEFERRED[k] for k in keys if k in DEFERRED), None)
        if form is not None:
            item.add_marker(pytest.mark.xfail(reason=f"{form}: deferred (user)", strict=True))
        gap = next((GAPS[k] for k in keys if k in GAPS), None)
        if gap is not None:
            item.add_marker(pytest.mark.xfail(reason=gap, strict=True))
    if not config.getoption("--run-slow"):
        later = pytest.mark.skip(reason="an exhaustive sweep: run with --run-slow")
        for item in items:
            if item.get_closest_marker("slow") is not None or not _sweep_kept(item):
                item.add_marker(later)
    if not os.environ.get("PYTEST_XDIST_WORKER"):
        return
    serial = pytest.mark.skip(reason="the Word oracle runs serially: pytest -m oracle, without -n")
    for item in items:
        if item.get_closest_marker("oracle") is not None:
            item.add_marker(serial)


# -- faces ----------------------------------------------------------------------------------
#
# The fixtures are laid out (by docx2svg) in the faces Word uses -- Calibri, Cambria,
# Georgia, Aptos -- which only a machine with Office has.  Without one, docx2svg lays it out
# with an approximate stand-in (round 16; before, it stopped where a paragraph could not be
# measured), and a test of a render, a reflow or a field's
# page number then says nothing about this code.  On such a machine, a test whose layout
# reports a face absent skips; where the faces are installed, nothing here applies.  CI's
# Linux and macOS runners install the open substitutes (Carlito, Liberation), which docx2svg
# lays Calibri, Arial, Times New Roman and Courier New out with, so only a document in a
# face without one (Cambria, Georgia, Aptos) skips there.

_OFFICE_FACES = ("Calibri", "Cambria", "Georgia", "Aptos")
_FACE_ABSENT = ("layout-stopped:unmeasurable", "layout-stopped:no face metrics", "line-numbers-not-drawn")


def _office_faces_absent() -> bool:
    from docx2svg.fonts import InstalledFonts, default_font_dirs

    try:  # docx2svg's approximate stand-in for an absent face (round 16) is not the face
        fonts = InstalledFonts(dirs=tuple(default_font_dirs()), fallbacks=False)
    except TypeError:
        fonts = InstalledFonts(dirs=tuple(default_font_dirs()))
    return any(fonts.face(family) is None for family in _OFFICE_FACES)


OFFICE_FACES_ABSENT = _office_faces_absent()


def _skip_if_a_check_says_a_face_is_absent(coverage, warnings) -> None:
    # A check's reflow facts (``coverage_facts``, its warnings as text).
    coverage = coverage or {}
    if coverage.get("missing_fonts"):
        pytest.skip("a face this document is laid out in is not installed here: "
                    + ", ".join(coverage["missing_fonts"]))
    approximate = [f for f in coverage.get("substituted_fonts", ()) if f.endswith("(approximate)")]
    if approximate:
        pytest.skip("a face this document is laid out in is not installed here (laid out with an "
                    "approximate substitute): " + ", ".join(approximate))
    for text in warnings or ():
        if any(code in text for code in _FACE_ABSENT) or "cannot be measured" in text:
            pytest.skip(f"a face this document is laid out in is not installed here: {text}")


def _skip_if_the_session_laid_a_face_absent(call) -> None:
    # The version the session's check laid out, laid out again by the document's page engine
    # (its pages are the engine's memo): the faces its coverage grades missing or substituted
    # by an approximate face.
    from docx_agent.rust import reads as rust_reads
    from docx_agent.rust.engines import existing_pages

    owned = rust_reads.owned(call.entry)
    pages = existing_pages(owned) if owned is not None else None
    if pages is None:
        return
    grades, _, _ = pages.lay_out(call.entry.snapshot()).coverage()
    missing = [family for family, grade, *_ in grades if grade == "missing"]
    if missing:
        pytest.skip(f"a face this document is laid out in is not installed here: {', '.join(missing)}")
    approximate = [family for family, grade, _, compatible, *_ in grades if grade == "substituted" and not compatible]
    if approximate:
        pytest.skip(f"a face this document is laid out in is not installed here (laid out with an approximate "
                    f"substitute): {', '.join(approximate)}")


@pytest.fixture(autouse=True)
def _skip_where_faces_are_absent(request, monkeypatch):
    # ``any_faces``: a test that holds without Office's faces (it leaves what they measure out).
    if not OFFICE_FACES_ABSENT or request.node.get_closest_marker("any_faces") is not None:
        return
    from docx_agent.rust import reads as rust_reads

    serve_check = rust_reads.serve_check

    def checked_serve_check(call, arguments):
        # A check lays out in the Rust session: read the coverage and the warnings its reflow
        # says, or, for a check of the layout or of ranges alone, the faces of that layout.
        served = serve_check(call, arguments)
        data = (served.data or {}) if served is not None and served.ok else {}
        reflow = data.get("reflow")
        if isinstance(reflow, dict):
            _skip_if_a_check_says_a_face_is_absent(reflow.get("coverage"), data.get("layout_warnings"))
        elif "layout" in data or "range" in data:
            _skip_if_the_session_laid_a_face_absent(call)
        return served

    monkeypatch.setattr(rust_reads, "serve_check", checked_serve_check)
