"""A Word template opened in a toolbox to edit warns ``TemplateOpened``: edits change the
template itself.  (The id state, its retired ids and versions, and its refusals:
test_id_state.py and test_retired_ids.py.)"""

from __future__ import annotations

import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS, TemplateOpened
from ooxml_edit.tools import Toolbox

from conftest import FIXTURE_DIR

TEMPLATE = FIXTURE_DIR / "generated" / "templates" / "brand.dotx"


def test_a_template_warns_that_edits_change_it():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        with pytest.warns(TemplateOpened, match=r"this file is a template \(\.dotx\)"):
            session.open(TEMPLATE.read_bytes(), "brand.dotx", kind="docx")
