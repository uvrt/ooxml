"""An existing document's language (round 35, R24: ``word_styles`` ``set_language``; measured
in ``crates/ooxml-oracle/word/observations/language-word.json``).

Setting it writes what a new document in that language has -- ``w:docDefaults``' ``w:lang``
and ``w:themeFontLang`` -- and the styles that stated the old default; text written after
it gets the language's spacing; ``check`` no longer offers to delete French spaces in text
tagged another language.  The documents are made up here, through the toolbox.
"""

from __future__ import annotations

import datetime
import json
from pathlib import Path

import pytest
from lxml import etree

from ooxml_edit.tools import Toolbox

from docx_agent.tools import GROUPS, TOOLS, docx_format

import word_docs
from conftest import OBSERVATIONS  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
OBSERVED = json.loads((OBSERVATIONS / "language-word.json").read_text(encoding="utf-8"))
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
NBSP = "\u00a0"
CLOCK = lambda: datetime.datetime(2026, 11, 6, 12, 0)  # noqa: E731


def _xml(root) -> bytes:
    return etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True)


def _lang(style, **attributes: str) -> None:
    rpr = style.find(_W + "rPr")
    if rpr is None:
        rpr = etree.SubElement(style, _W + "rPr")
    lang = rpr.find(_W + "lang")
    if lang is None:
        lang = etree.SubElement(rpr, _W + "lang")
    for slot, value in attributes.items():
        lang.set(_W + slot, value)


def _style(style_id: str, name: str, kind: str = "paragraph", based_on: str | None = "Normal",
           rpr: str = "") -> str:
    based = f'<w:basedOn w:val="{based_on}"/>' if based_on else ""
    return (f'<w:style w:type="{kind}" w:customStyle="1" w:styleId="{style_id}"><w:name w:val="{name}"/>'
            f"{based}<w:rPr>{rpr}</w:rPr></w:style>")


def made(*texts, language: str = "en-US", styles: str = "", edit_styles=None, run_tags=(),
         paragraph_styles=()) -> bytes:
    """A blank document in ``language``: ``styles`` added, ``edit_styles(root)`` run on the
    style sheet, a paragraph per text (paraIds ``3C000001`` on), its run tagged ``run_tags[k]``
    and styled ``paragraph_styles[k]`` where given."""
    body = []
    for k, text in enumerate(texts or ("",)):
        style = paragraph_styles[k] if k < len(paragraph_styles) else None
        tag = run_tags[k] if k < len(run_tags) else None
        ppr = f'<w:pPr><w:pStyle w:val="{style}"/></w:pPr>' if style else ""
        rpr = f'<w:rPr><w:lang w:val="{tag}"/></w:rPr>' if tag else ""
        body.append(f'<w:p w14:paraId="{0x3C000001 + k:08X}" w14:textId="77777777">{ppr}<w:r>{rpr}'
                    f'<w:t xml:space="preserve">{text}</w:t></w:r></w:p>')
    data = word_docs.with_body("".join(body), word_docs.blank(language=language))
    root = word_docs.tree(data, "word/styles.xml")
    if styles:
        for node in etree.fromstring(f'<w:styles xmlns:w="{W}">{styles}</w:styles>'):
            root.append(node)
    if edit_styles is not None:
        edit_styles(root)
    return word_docs.replaced(data, {"word/styles.xml": _xml(root)})


def template(*texts: str, language: str = "en-US", normal: str | None = None,
             heading: str | None = "en-GB", paragraph_styles=(), run_tags=()) -> bytes:
    """A made-up report template: its default language, and styles that state their own."""
    def edit(root):
        if normal:
            _lang(next(s for s in root.iter(_W + "style") if s.get(_W + "styleId") == "Normal"), val=normal)

    styles = _style("Chaptertitle", "Chapter title", rpr=f'<w:b/><w:lang w:val="{heading}"/>') if heading else ""
    return made(*texts, language=language, styles=styles, edit_styles=edit, paragraph_styles=paragraph_styles,
                run_tags=run_tags)


def languages(data: bytes) -> dict:
    """The languages the document states: the default, the theme's, each style's own and the
    runs' tags counted."""
    def attrs(node) -> dict:
        return {} if node is None else {k[len(_W):]: v for k, v in node.attrib.items() if k.startswith(_W)}

    styles = word_docs.tree(data, "word/styles.xml")
    out = {"default": attrs(styles.find(f"{_W}docDefaults/{_W}rPrDefault/{_W}rPr/{_W}lang")),
           "theme": attrs(word_docs.tree(data, "word/settings.xml").find(_W + "themeFontLang")),
           "styles": {}, "runs": {}}
    for style in styles.iter(_W + "style"):
        lang = style.find(f"{_W}rPr/{_W}lang")
        if lang is not None:
            out["styles"][style.find(_W + "name").get(_W + "val")] = attrs(lang)
    for lang in word_docs.tree(data).iter(_W + "lang"):
        if lang.getparent().getparent().tag == _W + "r":
            tag = lang.get(_W + "val")
            out["runs"][tag] = out["runs"].get(tag, 0) + 1
    return out


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[docx_format()], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _session(box, data: bytes):
    session = box.session(clock=CLOCK)
    session.open(data, "rapport.docx")
    return session


def _ok(box, session, tool, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, json.dumps(result.to_json(), default=str)[:800]
    return result


def set_language(box, session, language: str, **arguments):
    return _ok(box, session, "word_styles", doc="d1", action="set_language", language=language, **arguments)


def refused(box, session, language: str, **arguments) -> dict:
    result = box.dispatch(session, "word_styles", {"doc": "d1", "action": "set_language", "language": language,
                                                   **arguments})
    assert not result.ok, result.to_json()
    return result.to_json()["error"]


def now(session) -> dict:
    return languages(word_docs.saved(session, "d1"))


def texts(session) -> list[str]:
    return [t for _, t in word_docs.paragraphs(session, "d1")]


def _spacing(box, session) -> list[dict]:
    return _ok(box, session, "check", doc="d1", include=["layout"]).data["layout"]["punctuation_spacing"]


# -- the measurements --------------------------------------------------------------------------


def test_what_word_writes_when_it_sets_a_language():
    actions = OBSERVED["actions"]
    before = actions["nothing"]
    # None of Word's language commands rewrites the defaults, the theme or another style.
    for action in ("Normal style", "all text", "normal template"):
        assert actions[action]["docDefaults"] == before["docDefaults"]
        assert actions[action]["themeFontLang"] == before["themeFontLang"]
        for name, lang in before["styles"].items():
            assert actions[action]["styles"][name] == lang
    assert actions["Normal style"]["styles"]["Normal"] == {"val": "fr-BE"}
    assert {lang["val"] for lang in actions["all text"]["runs"]} == {"fr-BE"}
    assert actions["normal template"]["Normal.dotm changed"] and not actions["normal template"]["runs"]
    assert actions["normal template"]["proofed as"] == before["proofed as"]       # the open document as it was


def test_tags_are_read_as_word_reads_them(box):
    reads = OBSERVED["reads"]
    for given in ("fr-BE", "FR-be", "fr_BE", "fr-Latn-BE"):
        assert reads[given].startswith("belgian french")
        session = _session(box, template())
        set_language(box, session, given)
        assert now(session)["default"]["val"] == "fr-BE", given
    assert reads["xx-YY"] == reads["zz"] == "belgian dutch"     # the measuring Word's own language
    for given, slot in (("ja-JP", "eastAsia"), ("he-IL", "bidi")):
        session = _session(box, template())
        set_language(box, session, given)
        assert now(session)["default"][slot] == given


@pytest.mark.parametrize("given, wanted", [("fr", "fr-BE"), ("fr-XX", "fr-FR"), ("xx-YY", "en-US"),
                                           ("", "fr-FR"), ("fr-be-x-test", "fr-BE")])
def test_a_tag_word_does_not_know_is_refused_with_the_ones_that_would_do(box, given, wanted):
    error = refused(box, _session(box, template()), given)
    assert wanted in error.get("valid_options", []) or wanted in error["message"], error


def test_a_bare_language_lists_its_own_country_first(box):
    assert refused(box, _session(box, template()), "nl")["valid_options"][:2] == ["nl-NL", "nl-BE"]


# -- set_language ------------------------------------------------------------------------------


def test_it_writes_what_a_new_document_in_the_language_has(box):
    session = _session(box, template())
    before = now(session)
    set_language(box, session, "fr-BE")
    assert now(session) != before
    blank = languages(word_docs.blank(language="fr-BE"))
    assert (now(session)["default"], now(session)["theme"]) == (blank["default"], blank["theme"])
    assert word_docs.validation(box, session, "d1")["new"] == []


def test_styles_stating_the_old_default_follow_and_others_keep_theirs(box):
    session = _session(box, template(normal="en-US", heading="en-GB"))
    result = set_language(box, session, "fr-BE")
    styles = now(session)["styles"]
    assert styles["Normal"] == {"val": "fr-BE"} and styles["Chapter title"] == {"val": "en-GB"}
    assert any("Chapter title (en-GB)" in w for w in result.warnings)


def test_a_style_alone(box):
    session = _session(box, template(heading="en-GB"))
    set_language(box, session, "fr-FR", name="Chapter title")
    assert now(session)["default"]["val"] == "en-US"
    assert now(session)["styles"]["Chapter title"] == {"val": "fr-FR"}


def test_runs_keep_their_own_tags(box):
    session = _session(box, template("Bonjour.", "Guten Tag.", "Encore.", run_tags=("en-US", "de-DE", "en-US")))
    result = set_language(box, session, "fr-BE")
    assert now(session)["runs"] == {"en-US": 2, "de-DE": 1}
    assert any("also sets the 2 in en-US" in w for w in result.warnings), result.warnings


def test_east_asian_and_complex_script_languages_go_where_word_writes_them(box):
    session = _session(box, template())
    set_language(box, session, "ja-JP")
    assert now(session)["default"] == {"val": "en-US", "eastAsia": "ja-JP", "bidi": "ar-SA"}
    set_language(box, session, "he-IL")
    assert now(session)["default"]["bidi"] == "he-IL"
    assert now(session)["theme"] == {"val": "en-US", "eastAsia": "ja-JP", "bidi": "he-IL"}


def test_one_undo_step_and_a_no_op_is_not_an_error(box):
    session = _session(box, template(normal="en-US"))
    before = word_docs.saved(session, "d1")
    set_language(box, session, "nl-BE")
    _ok(box, session, "undo", doc="d1")
    assert word_docs.saved(session, "d1") == before
    set_language(box, session, "nl-BE")
    again = set_language(box, session, "nl_BE")
    assert again.summary.startswith("No change") and any("nl-BE already" in w for w in again.warnings), \
        again.to_json()


def test_a_document_without_a_theme_language_gets_one(box):
    data = template()
    settings = word_docs.tree(data, "word/settings.xml")
    settings.remove(settings.find(_W + "themeFontLang"))
    session = _session(box, word_docs.replaced(data, {"word/settings.xml": _xml(settings)}))
    assert now(session)["theme"] == {}
    set_language(box, session, "fr-BE")
    assert now(session)["theme"] == {"val": "fr-BE"} and word_docs.validation(box, session, "d1")["new"] == []


# -- writing and check -------------------------------------------------------------------------


def test_an_english_template_set_to_belgian_french_is_written_with_french_spacing(box):
    data = template("Titre")
    session = _session(box, data)
    result = set_language(box, session, "fr-BE")
    assert result.data["default"]["val"] == "fr-BE" and "fr-BE" in result.summary
    first = word_docs.paragraph_ids(box, session, "d1")[0]
    _ok(box, session, "word_set_text", doc="d1", target=first, text="Résumé : les points clés!")
    _ok(box, session, "word_insert_markdown", doc="d1", at="end",
        markdown="Il a dit « oui » : pourquoi?\n\n| Mesure | Valeur |\n|---|---|\n| Délai : | court! |\n")
    written = texts(session)
    assert written[0] == f"Résumé{NBSP}: les points clés{NBSP}!"
    assert f"Il a dit «{NBSP}oui{NBSP}»{NBSP}: pourquoi{NBSP}?" in written
    assert f"Délai{NBSP}:" in written and f"court{NBSP}!" in written
    assert _spacing(box, session) == []
    assert word_docs.validation(box, session, "d1")["new"] == []


def test_belgian_dutch_gets_no_french_rules(box):
    session = _session(box, template("Titel"))
    set_language(box, session, "nl-BE")
    first = word_docs.paragraph_ids(box, session, "d1")[0]
    _ok(box, session, "word_set_text", doc="d1", target=first, text="Besluit: goed! Waarom?")
    assert texts(session)[0] == "Besluit: goed! Waarom?"
    assert _spacing(box, session) == []


def test_french_spacing_in_english_text_is_a_language_mismatch_never_a_deletion(box):
    session = _session(box, template("Bonjour : le monde ! Pourquoi ?", "Fine: yes."))
    found = _spacing(box, session)
    assert len(found) == 1
    entry = found[0]
    assert entry["kind"] == "language_mismatch" and entry["language"] == "en-US" and entry["source"] == "default"
    assert [m["kind"] for m in entry["marks"]] == ["space_before"] * 3
    assert entry["fix"] == {"tool": "word_styles", "calls": [{"action": "set_language", "language": "fr-FR"}]}
    assert "replace_text" not in json.dumps(found)
    # Following the fix: the text is French now, its ordinary spaces the ones to make no-break.
    for call in entry["fix"]["calls"]:
        _ok(box, session, "word_styles", doc="d1", **call)
    after = _spacing(box, session)
    assert after and all(e.get("kind") != "language_mismatch" and e["fix"]["tool"] == "replace_text"
                         for e in after)
    for item in after:
        for call in item["fix"]["calls"]:
            assert NBSP in call["replace"] and call["replace"].replace(NBSP, "") == call["find"].replace(" ", "")
            _ok(box, session, "replace_text", doc="d1", **call)
    assert _spacing(box, session) == []


def test_the_fix_names_the_style_the_tag_comes_from_and_a_french_tag_the_document_has(box):
    # Written as French text is written (the no-break space), under a style stating English.
    session = _session(box, template(f"Chapitre{NBSP}: un", language="fr-BE", heading="en-US",
                                     paragraph_styles=("Chaptertitle",)))
    entry = _spacing(box, session)[0]
    assert entry["source"] == "style:Chapter title"
    assert entry["fix"]["calls"] == [{"action": "set_language", "language": "fr-BE", "name": "Chapter title"}]
    _ok(box, session, "word_styles", doc="d1", **entry["fix"]["calls"][0])
    assert _spacing(box, session) == []        # French now: the no-break space it was written with


def test_a_runs_own_tag_gets_no_fix(box):
    entry = _spacing(box, _session(box, template("Bonjour : le monde", run_tags=("en-GB",))))[0]
    assert entry["source"] == "run" and "fix" not in entry and "not a spacing fault" in entry["note"]


def test_french_text_keeps_its_replace_text_fixes(box):
    entry = _spacing(box, _session(box, made("Bonjour : le monde!", language="fr-FR")))[0]
    assert "kind" not in entry and entry["fix"]["tool"] == "replace_text"


def test_the_tool_refuses_with_the_way_out(box):
    session = _session(box, template())
    error = refused(box, session, "fr")
    assert error["field"] == "language" and "fr-BE" in error["valid_options"]
    assert refused(box, session, "fr-BE", name="No such style")["code"] == "not_found"
    no_op = set_language(box, session, "en_us")
    assert no_op.summary.startswith("No change")
    wrong = box.dispatch(session, "word_styles", {"doc": "d1", "action": "remove", "name": "Normal",
                                                  "language": "fr-BE"})
    assert not wrong.ok and wrong.to_json()["error"]["field"] == "language"


def test_undo_through_the_session(box):
    session = _session(box, template())
    set_language(box, session, "fr-BE")
    _ok(box, session, "undo", doc="d1")
    assert now(session)["default"]["val"] == "en-US"


# -- Word ----------------------------------------------------------------------------------------


@pytest.mark.oracle
def test_word_proofs_the_text_in_the_language_set(box):
    import oracle

    session = _session(box, template("Plain text.", "Chapter text.", normal="en-US", heading="en-GB",
                                     paragraph_styles=(None, "Chaptertitle")))
    set_language(box, session, "fr-BE")
    script = '''
on run argv
  set out to ""
  with timeout of 120 seconds
    tell application "Microsoft Word"
      open (POSIX file (item 1 of argv))
      set d to active document
      repeat with i from 1 to 2
        set out to out & ((language ID of text object of paragraph i of d) as string) & linefeed
      end repeat
      save as d file name (item 2 of argv) file format format document
      close d saving no
      quit saving no
    end tell
  end timeout
  return out
end run
'''
    try:
        with oracle.session() as word:
            outcome = word.run_script(script, word_docs.saved(session, "d1"), name="set-language",
                                      tag="proofed")
    except oracle.WordBusy as busy:
        pytest.skip(str(busy))
    assert outcome, outcome.detail
    if outcome.path is not None:
        outcome.path.unlink(missing_ok=True)
    assert outcome.detail.split() == ["belgian", "french", "english", "uk"]


# -- the template's own language (round 35, R25) -------------------------------------------------


ENGLISH = ("Normal", "Heading1", "Heading2", "Heading1Char", "Heading2Char", "Title", "TitleChar",
           "PageFooter", "PageFooterChar", "ReportDate")


def english_template(*texts: str, default: str | None = "nl-BE", paragraph_styles=(), run_tags=(),
                     normal_east_asia: str | None = None) -> bytes:
    """A made-up report template made in an English Word: the default ``default`` (or none),
    Normal, the headings, their character styles, footer and date styles in en-US; one
    paragraph style in German (with a child stating nothing) and one character style in
    Italian, on purpose."""
    styles = (_style("PageFooter", "Page footer") + _style("PageFooterChar", "Page footer Char", "character", None)
              + _style("ReportDate", "Report date", based_on="PageFooter")
              + _style("QuotationGerman", "Quotation German", rpr="<w:i/>")
              + _style("QuotationGermanchild", "Quotation German child", based_on="QuotationGerman")
              + _style("TermItalian", "Term Italian", "character", None, rpr="<w:i/>"))

    def edit(root):
        by_id = {s.get(_W + "styleId"): s for s in root.iter(_W + "style")}
        for identifier in ENGLISH:
            _lang(by_id[identifier], val="en-US")
        _lang(by_id["QuotationGerman"], val="de-DE")
        _lang(by_id["TermItalian"], val="it-IT")
        if normal_east_asia:
            _lang(by_id["Normal"], eastAsia=normal_east_asia)
        lang = root.find(f"{_W}docDefaults/{_W}rPrDefault/{_W}rPr/{_W}lang")
        if default is None:
            del lang.attrib[_W + "val"]
        else:
            lang.set(_W + "val", default)

    return made(*texts, styles=styles, edit_styles=edit, paragraph_styles=paragraph_styles, run_tags=run_tags)


FRENCH = f"Résumé{NBSP}: les points clés{NBSP}! Pourquoi{NBSP}?"


@pytest.mark.parametrize("default", ["nl-BE", None, "en-US"])
def test_the_template_s_own_language_follows_and_other_languages_stay_named(box, default):
    session = _session(box, english_template(FRENCH, default=default))
    before = now(session)["default"]
    result = set_language(box, session, "fr-BE")
    styles = now(session)["styles"]
    english = [name for name, lang in styles.items() if name not in ("Quotation German", "Term Italian")]
    assert len(english) == len(ENGLISH) and all(styles[name] == {"val": "fr-BE"} for name in english), styles
    assert styles["Quotation German"] == {"val": "de-DE"} and styles["Term Italian"] == {"val": "it-IT"}
    assert "Quotation German child" not in styles         # states nothing: German through its parent
    kept = next(w for w in result.warnings if w.startswith("styles keeping"))
    assert "Quotation German (de-DE)" in kept and "Term Italian (it-IT)" in kept and "en-US" not in kept
    own = [w for w in result.warnings if w.startswith("the template's own language")]
    if default == "en-US":
        assert own == []                                     # the default is all there was
    else:
        assert own and "en-US (Normal's)" in own[0] and (default is None or "nl-BE (the default)" in own[0])
    after = now(session)["default"]
    assert after["val"] == "fr-BE" and {k: v for k, v in after.items() if k != "val"} == \
        {k: v for k, v in before.items() if k != "val"}      # eastAsia and bidi as they were
    assert word_docs.validation(box, session, "d1")["new"] == []


def test_french_in_the_english_template_is_one_call_away_and_then_clean(box):
    session = _session(box, english_template(FRENCH, f"Il a dit «{NBSP}oui{NBSP}»."))
    entry = _spacing(box, session)[0]
    assert entry["kind"] == "language_mismatch" and entry["source"] == "style:Normal"
    # Normal's language is the template's own: the fix is the document's, not Normal's alone.
    assert entry["fix"]["calls"] == [{"action": "set_language", "language": "fr-FR"}]
    result = set_language(box, session, "fr-BE")
    assert "Quotation German (de-DE)" in json.dumps(result.to_json())
    assert _spacing(box, session) == []
    first = word_docs.paragraph_ids(box, session, "d1")[0]
    _ok(box, session, "word_set_text", doc="d1", target=first, text="Conclusion : tout va bien!")
    _ok(box, session, "word_insert_markdown", doc="d1", at="end", markdown="# Chapitre : deux\n\nPourquoi?\n")
    written = texts(session)
    assert f"Conclusion{NBSP}: tout va bien{NBSP}!" in written
    assert f"Chapitre{NBSP}: deux" in written and f"Pourquoi{NBSP}?" in written
    assert _spacing(box, session) == []


def test_a_style_in_another_language_keeps_its_own_fix(box):
    session = _session(box, english_template(FRENCH, paragraph_styles=("QuotationGerman",)))
    entry = _spacing(box, session)[0]
    assert entry["source"] == "style:Quotation German"
    assert entry["fix"]["calls"] == [{"action": "set_language", "language": "fr-FR", "name": "Quotation German"}]


def test_runs_in_the_template_s_own_language_say_they_stay(box):
    session = _session(box, english_template("One.", "Two.", "Three.", run_tags=("en-US", "nl-BE", "de-DE")))
    plain = set_language(box, session, "fr-BE")
    assert now(session)["runs"] == {"en-US": 1, "nl-BE": 1, "de-DE": 1}
    assert any("also sets the 2 in nl-BE or en-US" in w for w in plain.warnings), plain.warnings


def _bare(*languages: str | None) -> bytes:
    """No default language, none on Normal; styles stating ``languages`` (None: nothing)."""
    styles = "".join(_style(f"Body{i}", f"Body {i}", rpr=f'<w:lang w:val="{language}"/>' if language else "")
                     for i, language in enumerate(languages))

    def edit(root):
        root.find(f"{_W}docDefaults/{_W}rPrDefault/{_W}rPr/{_W}lang").attrib.pop(_W + "val")

    return made(styles=styles, edit_styles=edit)


@pytest.mark.parametrize("languages, own", [
    (("en-US", "en-US", "de-DE"), ["en-US"]),         # more than half, at least two
    (("en-US", "de-DE"), []),                         # a tie
    (("en-US", "en-US", "de-DE", "it-IT"), []),       # two of four: not more than half
    (("de-DE", None), []),                            # a single style is an exception
    (("en_us", "EN-US", "de-DE"), ["en_us"]),         # any spelling, the first one's kept
])
def test_without_a_default_or_normal_the_language_most_styles_state(box, languages, own):
    session = _session(box, _bare(*languages))
    result = set_language(box, session, "fr-BE")
    styles = now(session)["styles"]
    for i, language in enumerate(languages):
        if language:
            assert styles[f"Body {i}"]["val"] == ("fr-BE" if own and _same_tag(language, own[0]) else language)
    if not own:
        assert any("states no language of its own" in w for w in result.warnings)


def _same_tag(a: str, b: str) -> bool:
    return a.replace("_", "-").lower() == b.replace("_", "-").lower()


def test_the_rule_reads_only_the_attribute_the_language_is_written_in(box):
    session = _session(box, english_template(normal_east_asia="zh-CN"))
    set_language(box, session, "fr-BE")
    styles = now(session)["styles"]
    assert styles["Normal"] == {"val": "fr-BE", "eastAsia": "zh-CN"}        # eastAsia not touched
    set_language(box, session, "ja-JP")
    assert now(session)["styles"]["Normal"] == {"val": "fr-BE", "eastAsia": "ja-JP"}
    assert now(session)["styles"]["heading 1"] == {"val": "fr-BE"}   # no eastAsia of its own
