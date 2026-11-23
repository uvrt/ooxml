"""``STYLEREF`` results the tools' field update stores (production round 21, T2), held to what
Word stored for the same made-up documents after updating every field and saving
(``probes/styleref_probe.py``, ``observations/styleref-word.json``).

The result is the text docx2svg draws for the field: in the body, the last text of its style
before it, else the first after it; in a header or footer, the text on the first page that
shows it.  Word stores, for a header, the text of the last page **its window** drew -- page 2
for a header shown on pages 1 and 2 -- and for a header it has not drawn yet something else
again: neither can be known from the file, so those differ (:data:`WINDOW`).
"""

from __future__ import annotations

import io
import re
import zipfile

import pytest
from recorded import Opened, observed  # noqa: I001  (puts the probes on the path)

import styleref_probe  # noqa: E402

OBSERVED = observed("styleref-word.json")
#: The observation's keys these tests compare with (``source`` is its note).
READS = {"documents"}
LABEL = re.compile(r"(\w+)=\[([^\]]*)\]")

#: Header and footer fields whose text Word stored from its window (see the module), and the
#: header ``PAGE`` fields Word stored as they were.
WINDOW = {
    "base": {"P"}, "firsteven": {"P", "O"}, "firsteven2": {"O"}, "sections": {"S", "SL"},
    "extra": {"A", "AL", "FT", "FTL", "S", "SL"}, "latent": {"l7"},
}
#: Not computed (left as they were): the paragraph number switches, the default character
#: style, a switch Word does not know.
NOT_COMPUTED = {
    "switches": {"s1", "s2", "s3", "s5", "s7", "s8", "s9", "sa", "sb", "se"},
    "latent": {"l8", "lc"},
}
#: ``STYLEREF "Kop 1"`` in a Dutch document: the English Word measured gave its error; the
#: tools read a document without a locale in its own language, as TOC ``\\t`` does, where
#: "Kop 1" is heading 1 (feedback-round21, measurements-D).
DOCUMENT_LANGUAGE = {"nl": {"n4": "Part one heading"}}


def _stored(data: bytes) -> dict:
    out = {}
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for name in archive.namelist():
            if name.startswith("word/") and name.endswith(".xml"):
                xml = re.sub(r"<w:instrText[^>]*>.*?</w:instrText>", "", archive.read(name).decode())
                out.update(LABEL.findall("".join(re.findall(r"<w:t(?: [^>]*)?>([^<]*)</w:t>", xml))))
    return out


@pytest.mark.parametrize("case", sorted(OBSERVED["documents"]))
def test_an_update_stores_what_word_stores(box, case):
    document = Opened(box, styleref_probe.case_document(case))
    document.call("word_fields", action="update")
    ours = _stored(document.data())
    word = OBSERVED["documents"][case]
    window, cached = WINDOW.get(case, set()), NOT_COMPUTED.get(case, set())
    for label, text in word.items():
        if label in DOCUMENT_LANGUAGE.get(case, {}):
            assert text.startswith("Error!") and ours[label] == DOCUMENT_LANGUAGE[case][label]
        elif label in cached:
            assert ours[label] == "?", label
        elif label not in window:
            assert ours[label] == text, (label, ours[label], text)
