"""The language a text is written in, next to its tag (round 49, R94: ``check``'s layout fact
``text_language``).

The fact lists what differs, folds a whole document under the wrong default into one entry
with the fix, groups consecutive paragraphs, counts what agrees, and stays capped.  The
documents are made up here, through the toolbox.
"""

from __future__ import annotations

import datetime
import json

import pytest
from lxml import etree

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs
from conftest import FIXTURE_DIR  # noqa: E402

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
CLOCK = lambda: datetime.datetime(2026, 11, 10, 12, 0)  # noqa: E731
#: The detector's: the confidence a listed mismatch has at least.
MIN_CONFIDENCE = 0.5
FR = ["Les copies de sauvegarde tournent chaque nuit et sont gardées pendant trente jours au moins.",
      "Un fichier supprimé par erreur peut être remis en place depuis le portail par chaque personne.",
      "Pour tout ce qui a plus d'un mois, il faut passer par l'équipe technique qui le fera vite."]
EN = ["The new office opens on the second floor next month, and the teams near the stairs move first.",
      "Please pack your desk before Friday and label each box with your name and the new room number.",
      "Coffee and tea are free on every floor, and the kitchen is cleaned each evening after six."]
NL = "De fietsroute begint bij het oude station en volgt het kanaal over een afstand van twaalf kilometer."


def made_up(*texts: str, language: str = "en-GB") -> bytes:
    """A blank document in ``language``, a paragraph per text (paraIds ``1A000001`` on)."""
    body = "".join(f'<w:p w14:paraId="{0x1A000001 + k:08X}" w14:textId="77777777"><w:r>'
                   f'<w:t xml:space="preserve">{text}</w:t></w:r></w:p>' for k, text in enumerate(texts))
    return word_docs.with_body(body, word_docs.blank(language=language))


def ids_of(data: bytes) -> list[str]:
    return [f"p:{p.get(word_docs.W14_PARA)}" for p in word_docs.tree(data).iter(_W + "p")]


def tag_runs(data: bytes, starts: str, language: str, *, no_proof: bool = False) -> bytes:
    """Each run of the paragraphs starting with ``starts`` tagged ``language`` (``w:lang``)."""
    root = word_docs.tree(data)
    for paragraph in root.iter(_W + "p"):
        text = "".join(t.text or "" for t in paragraph.iter(_W + "t"))
        if not text.startswith(starts):
            continue
        for run in paragraph.iter(_W + "r"):
            properties = run.find(_W + "rPr")
            if properties is None:
                properties = etree.Element(_W + "rPr")
                run.insert(0, properties)
            if no_proof:
                etree.SubElement(properties, _W + "noProof")
            etree.SubElement(properties, _W + "lang").set(_W + "val", language)
    return word_docs.replaced(data, {"word/document.xml": etree.tostring(
        root, xml_declaration=True, encoding="UTF-8", standalone=True)})


#: The detector's (``crates/ooxml-word-edit/src/facts/language``): the fewest words it judges,
#: and the tag each language's text is written under.
MIN_WORDS = 8
TAGS = {"en": "en-US", "fr": "fr-FR", "nl": "nl-NL", "de": "de-DE", "es": "es-ES", "it": "it-IT", "pt": "pt-PT"}
SAMPLES = json.loads((FIXTURE_DIR / "language-samples.json").read_text(encoding="utf-8"))


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _ok(box, session, tool, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, json.dumps(result.to_json(), default=str)[:800]
    return result


def fact(box, data: bytes, **arguments) -> tuple[dict, object]:
    session = box.session(clock=CLOCK)
    session.open(data, "made-up.docx")
    found = _ok(box, session, "check", doc="d1", include=["layout"], **arguments).data["layout"]
    return found.get("text_language"), session


# -- the detector, as measured ------------------------------------------------------------------


def _windows(text: str, n: int, step: int = 3) -> list[str]:
    words = text.split()
    return [" ".join(words[i:i + n]) for i in range(0, len(words) - n + 1, step)]


def test_the_measured_accuracy_holds_at_the_minimum_length(box):
    """The fresh samples (written after the detector's last change) cut into windows of
    MIN_WORDS words from every third word, each a paragraph of a document tagged its own
    language: check lists none as another language (no false positive), and judges more than
    150 of them."""
    from xml.sax.saxutils import escape

    judged = listed = 0
    for language, paragraphs in SAMPLES["fresh"].items():
        windows = [w for paragraph in paragraphs for w in _windows(paragraph, MIN_WORDS)]
        data = made_up(*(escape(w) for w in windows), language=TAGS[language])
        found, _ = fact(box, data)
        assert found is not None, language
        judged += found["document"]["paragraphs"]
        listed += sum(entry.get("paragraphs", 1) for entry in found.get("mismatched", []))
        listed += found.get("mismatched_left_out", 0)
        assert found["document"]["detected"] == language, (language, found["document"])
    assert listed == 0
    assert judged > 150


# -- the fact -----------------------------------------------------------------------------------


def test_a_document_in_another_language_than_its_default_is_one_entry_with_the_fix(box):
    """Rule 2 (the way out) and rule 3: the fix made, the check has nothing left to say."""
    found, session = fact(box, made_up(*FR, "Résumé", *FR))
    document = found["document"]
    assert {k: document[k] for k in ("tag", "source", "detected", "share", "paragraphs", "mismatched")} == {
        "tag": "en-GB", "source": "default", "detected": "fr", "share": 1.0, "paragraphs": 6, "mismatched": 6}
    assert document["fix"] == {"tool": "word_styles", "calls": [{"action": "set_language", "language": "fr-FR"}]}
    assert "mismatched" not in found and found["too_short"] == 1             # folded, not listed
    _ok(box, session, "word_styles", doc="d1", **document["fix"]["calls"][0])
    after = _ok(box, session, "check", doc="d1", include=["layout"]).data["layout"]["text_language"]
    assert after["document"]["tag"] == "fr-FR" and after["document"]["detected"] == "fr"
    assert "fix" not in after["document"] and "mismatched" not in after


def test_a_passage_in_another_language_is_listed_and_consecutive_paragraphs_are_one_range(box):
    document = made_up(*EN, *FR[:2], "Annexe", FR[2], *EN, NL)
    found, _ = fact(box, document)
    ids = ids_of(document)
    assert found["document"] == {"tag": "en-GB", "source": "default", "detected": "en",
                                 "share": found["document"]["share"], "words": found["document"]["words"],
                                 "paragraphs": 10}
    first, second = found["mismatched"]
    assert first == {"range": f"{ids[3]}..{ids[6]}", "paragraphs": 4, "tag": "en-GB", "source": "default",
                     "detected": "fr", "confidence": first["confidence"], "words": first["words"]}
    assert first["confidence"] >= MIN_CONFIDENCE
    assert second["para"] == ids[10] and second["detected"] == "nl"
    assert found["too_short"] == 1                                         # "Annexe"


def test_text_tagged_its_own_language_agrees_and_a_wrong_run_tag_says_run(box):
    document = tag_runs(made_up(*EN, FR[0], EN[0] + " Again."), FR[0][:12], "fr-FR")
    found, _ = fact(box, document)
    assert "mismatched" not in found                                       # its runs say fr-FR
    document = tag_runs(made_up(*EN, FR[0]), EN[1][:12], "de-DE")
    found, _ = fact(box, document)
    run, french = found["mismatched"]
    assert (run["tag"], run["source"], run["detected"]) == ("de-DE", "run", "en")
    assert (french["tag"], french["source"]) == ("en-GB", "default")


def test_text_word_does_not_proof_is_not_judged(box):
    document = tag_runs(made_up(*EN, FR[0]), FR[0][:12], "en-GB", no_proof=True)
    found, _ = fact(box, document)
    assert "mismatched" not in found and found["document"]["paragraphs"] == 3


def test_the_list_is_capped_and_says_what_was_left_out(box):
    """Rule 4: at most CAP entries, the rest counted."""
    texts = []
    for i in range(25):
        texts += [EN[i % 3], FR[i % 3]]
    found, _ = fact(box, made_up(*texts))
    assert len(found["mismatched"]) == 20 and found["mismatched_left_out"] == 5


def test_a_range_says_what_its_words_are_and_proposes_no_document_fix(box):
    document = made_up(*EN, *FR, *EN)
    ids = ids_of(document)
    found, _ = fact(box, document, range=f"{ids[3]}..{ids[5]}")
    assert "document" not in found and found["checked"]["detected"] == "fr"
    assert "fix" not in found["checked"] and "mismatched" not in found["checked"]
    assert found["mismatched"] == [{"range": f"{ids[3]}..{ids[5]}", "paragraphs": 3, "tag": "en-GB",
                                    "source": "default", "detected": "fr",
                                    "confidence": found["mismatched"][0]["confidence"],
                                    "words": found["mismatched"][0]["words"]}]


def test_a_document_too_short_to_judge_has_no_fact(box):
    found, _ = fact(box, made_up("Titre", "Résumé court"))
    assert found is None


def test_the_fix_takes_a_tag_the_document_already_names(box):
    """A style no paragraph uses names fr-BE: the fix sets that tag, not fr-FR."""
    style = ('<w:style w:type="paragraph" w:customStyle="1" w:styleId="NoteBelge"><w:name w:val="Note belge"/>'
             '<w:basedOn w:val="Normal"/><w:rPr><w:lang w:val="fr-BE"/></w:rPr></w:style>')
    data = word_docs.replaced(made_up(*FR), {"word/styles.xml": lambda xml: xml.replace("</w:styles>",
                                                                                          style + "</w:styles>")})
    found, _ = fact(box, data)
    assert found["document"]["fix"]["calls"] == [{"action": "set_language", "language": "fr-BE"}]
