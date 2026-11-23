"""The regional settings a document is drawn and edited in, through the tool layer
``Toolbox(locale=...)`` -- to every layout and render (in the worker process too), to new
charts, to the language of new chart text where the document states none, and to a new
document's language and separators.  en-US unless given, never the machine's.
"""

from __future__ import annotations

import datetime as dt
import json

import pytest
from ooxml_common.locales import EN_US, Locale
from ooxml_edit.tools import Toolbox

from docx_agent.rust import engines, reads
from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs
from pixels import pixels

DUTCH = Locale(decimal_separator=",", thousands_separator=".", date_order="dmy", language="nl-NL",
               list_separator=";")
W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
C = "{http://schemas.openxmlformats.org/drawingml/2006/chart}"


def _box(**options) -> Toolbox:
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, **options)


def _settings(data: bytes) -> tuple[str, str]:
    root = word_docs.tree(data, "word/settings.xml")
    return root.find(W + "decimalSymbol").get(W + "val"), root.find(W + "listSeparator").get(W + "val")


def _blank(locale=None, language: str | None = None) -> bytes:
    """A new blank document as one made in ``locale`` is (its language and separators)."""
    locale = locale or EN_US
    return word_docs.blank(language=language or locale.language,
                           separators=(locale.decimal_separator, locale.list_separator))


def _with_chart(box, data: bytes, chart_type: str = "column", **session_options):
    """A session holding ``data`` with a chart added after its first paragraph."""
    session = box.session(**session_options)
    session.open(data, "chart.docx")
    first = word_docs.paragraph_ids(box, session, "d1")[0]
    if chart_type == "column":
        arguments = {"categories": ["Q1", "Q2"], "data": [{"name": "North", "values": [0.4, 1.3]}]}
    else:
        arguments = {"categories": ["a", "b"], "data": [{"name": "s", "values": [1, 2]}]}
    word_docs.call(box, session, "edit_chart", doc="d1", target=first, action="add", chart_type=chart_type,
                   **arguments)
    return session


def _chart_lang(session) -> str:
    data = word_docs.saved(session, "d1")
    return word_docs.tree(data, word_docs.chart_part(data, "d:1")).find(f"{C}lang").get("val")


# -- a new document: new_document ---------------------------------------------------------------

#: Every document here is made at this time (the session's clock), so none depends on the day.
CREATED = dt.datetime(2026, 10, 7, 12, 0, tzinfo=dt.timezone.utc)
#: The separators of the Word the new document was measured on, with en-US's language.
COMMA = Locale(decimal_separator=",", thousands_separator=".", date_order="dmy", language="en-US",
               list_separator=";")


def _made(locale=None, **arguments) -> bytes:
    """``new_document``'s package in a session with ``locale`` (none: the toolbox's, en-US)."""
    with _box(workers=0) as box:
        session = box.session(clock=lambda: CREATED, **({"locale": locale} if locale is not None else {}))
        made = box.dispatch(session, "new_document", {"kind": "docx", **arguments})
        assert made.ok, made.to_json()
        return word_docs.saved(session, made.created[0])


def _language(data: bytes) -> tuple[str, str]:
    """The default language (``w:docDefaults``) and the theme's (``w:themeFontLang``)."""
    styles = word_docs.tree(data, "word/styles.xml")
    default = styles.find(f"{W}docDefaults/{W}rPrDefault/{W}rPr/{W}lang").get(W + "val")
    return default, word_docs.tree(data, "word/settings.xml").find(W + "themeFontLang").get(W + "val")


def _page(data: bytes) -> tuple[str, str]:
    size = word_docs.tree(data).find(f"{W}body/{W}sectPr/{W}pgSz")
    return size.get(W + "w"), size.get(W + "h")


def test_without_a_locale_a_new_document_is_en_us():
    data = _made()
    assert _settings(data) == (".", ",") and _language(data) == ("en-US", "en-US")
    assert _page(data) == ("11906", "16838")                                  # A4
    assert _made(EN_US) == data
    letter = _made(size="Letter")
    assert _settings(letter) == (".", ",") and _page(letter) == ("12240", "15840")


@pytest.mark.parametrize("language", ["en-US", "nl-NL"])
def test_a_comma_locale_writes_its_separators_and_language(language):
    data = _made(Locale(decimal_separator=",", thousands_separator=".", date_order="dmy", language=language,
                        list_separator=";"))
    assert _settings(data) == (",", ";") and _language(data) == (language, language)


def test_a_new_document_takes_the_locales_language_and_separators():
    data = _made(DUTCH)
    assert _settings(data) == (",", ";") and _language(data) == ("nl-NL", "nl-NL")


def test_the_properties_are_the_calls_and_the_date_the_sessions_clock():
    core = word_docs.part(_made(DUTCH, title="Verslag", author="Schrijver A"), "docProps/core.xml")
    assert "<dc:title>Verslag</dc:title>" in core and "<dc:creator>Schrijver A</dc:creator>" in core
    assert "2026-10-07T12:00:00Z" in core


def test_the_page_size_and_the_locale_are_separate():
    data = _made(DUTCH, size="Letter")
    root = word_docs.tree(data, "word/settings.xml")
    assert root.find(W + "defaultTabStop").get(W + "val") == "720" and _settings(data) == (",", ";")
    assert _page(data) == ("12240", "15840")


def test_from_a_template_the_locale_is_kept_and_the_settings_are_the_templates():
    """A new document from a template made without a locale: the template's separators kept
    (en-US's), the session's locale the document's."""
    template = _made()
    with _box(workers=0, locale=DUTCH) as box:
        session = box.session(clock=lambda: CREATED)
        blob = session.add_blob(template, "template.dotx")
        made = box.dispatch(session, "new_document", {"kind": "docx", "template_blob": blob})
        assert made.ok, made.to_json()
        data = word_docs.saved(session, made.created[0])
        assert _settings(data) == (".", ",") and _language(data) == ("en-US", "en-US")
        assert engines.locale_of(reads.owned(session.entry(made.created[0]))) == DUTCH


# -- charts ----------------------------------------------------------------------------------------


def test_a_new_chart_takes_the_locales_language_where_the_document_states_none():
    def unstated(xml: str) -> str:
        from lxml import etree

        root = etree.fromstring(xml.encode("utf-8"))
        node = root.find(f"{W}docDefaults/{W}rPrDefault/{W}rPr/{W}lang")
        node.getparent().remove(node)
        return etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True).decode()

    with _box(workers=0, locale=DUTCH) as box:
        session = _with_chart(box, word_docs.replaced(_blank(), {"word/styles.xml": unstated}), "pie")
        assert _chart_lang(session) == "nl-NL"
        # A document that states its language keeps it.
        assert _chart_lang(_with_chart(box, _blank(language="de-DE"), "pie")) == "de-DE"


# -- the tool layer ------------------------------------------------------------------------------


def _render(box: Toolbox, data: bytes, **session_options) -> tuple:
    session = box.session(**session_options)
    session.open(data, "chart.docx")
    result = box.dispatch(session, "render", {"doc": "d1", "pages": [1], "width": 320})
    assert result.ok, result.error
    return pixels(result.images[0].data)


@pytest.fixture(scope="module")
def chart_bytes() -> bytes:
    with _box(workers=0) as box:
        return word_docs.saved(_with_chart(box, _blank()), "d1")


@pytest.fixture(scope="module")
def drawn(chart_bytes) -> tuple:
    """The chart's page drawn in-process in English and in Dutch."""
    with _box(workers=0) as english, _box(workers=0, locale=DUTCH) as dutch:
        return _render(english, chart_bytes), _render(dutch, chart_bytes)


def test_the_toolbox_locale_reaches_the_document_and_the_worker(chart_bytes, drawn):
    english, dutch = drawn
    assert english != dutch                                            # 0.2 against 0,2 on the axis
    with _box(workers=1) as plain:
        assert _render(plain, chart_bytes) == english
    with _box(workers=1, locale=DUTCH) as configured:
        assert _render(configured, chart_bytes) == dutch               # in the worker, in Dutch
        assert _render(configured, chart_bytes, locale=EN_US) == english


def test_the_render_cache_is_keyed_by_the_locale(chart_bytes):
    with _box(workers=0) as box:
        first = _render(box, chart_bytes)
        second = _render(box, chart_bytes, locale=DUTCH)
    assert first != second


def test_a_subprocess_worker_gets_the_locale_from_its_arguments(chart_bytes, drawn):
    """What a daemonic process renders with: the locale crosses as an argument."""
    from ooxml_edit.tools import SubprocessPool

    with _box(runner=SubprocessPool(1), locale=DUTCH) as box:
        assert _render(box, chart_bytes) == drawn[1]


def test_the_layout_takes_the_locale(chart_bytes):
    """``check``'s layout is the document's page engine's, made for its options: the
    session's locale among them."""
    with _box(workers=1, locale=DUTCH) as box:
        session = box.session()
        session.open(chart_bytes, "chart.docx")
        result = box.dispatch(session, "check", {"doc": "d1", "include": ["reflow"]})
        assert result.ok, result.error
        assert engines.locale_of(reads.owned(session.entry("d1"))) == DUTCH


def test_new_document_takes_the_session_locale():
    with _box(locale=DUTCH) as box:
        session = box.session()
        made = box.dispatch(session, "new_document", {"kind": "docx"})
        assert made.ok, made.error
        assert _settings(word_docs.saved(session, "d1")) == (",", ";")
    with _box() as box:
        session = box.session()
        assert box.dispatch(session, "new_document", {"kind": "docx"}).ok
        assert _settings(word_docs.saved(session, "d1")) == (".", ",")


def test_the_definitions_and_prompt_do_not_change():
    with _box() as plain, _box(locale=DUTCH) as configured:
        for provider in ("anthropic", "openai-responses"):
            assert json.dumps(plain.definitions(provider)) == json.dumps(configured.definitions(provider))
        assert plain.system_prompt() == configured.system_prompt()
