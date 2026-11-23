"""S10 (review): ``edit_chart`` behaves the same in a Word document and a deck (SmartArt's
``edit_smartart`` is out of the Word tools until its port).  ooxml-edit's table (``ooxml_edit.charts.cases``) is run here through this
package's tools, and through pptx-agent's in its ``tests/test_chart_cases.py``: the same
calls, the same results, but for the chart's name in a result (here ``id``)."""

from __future__ import annotations

import pytest

from ooxml_edit.charts.cases import CHART, CHART_CASES, problems
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


@pytest.mark.parametrize("case", CHART_CASES, ids=[case["name"] for case in CHART_CASES])
def test_a_chart_case(box, case):
    session = box.session()
    session.open(word_docs.blank(), "chart.docx")
    word_docs.call(box, session, "word_insert_markdown", doc="d1", at="end", markdown="Figures.\n\nMore.")
    anchor = word_docs.paragraph_ids(box, session, "d1")[0]

    def call(**arguments):
        return box.dispatch(session, "edit_chart", {"doc": "d1", **arguments})

    made = call(target=anchor, action="add", ref="c", **CHART)
    assert made.ok, made.to_json()
    for arguments in case["before"]:
        assert call(target="$c", **arguments).ok
    assert problems(case, call(target="$c", **case["call"]), key="id") == []
