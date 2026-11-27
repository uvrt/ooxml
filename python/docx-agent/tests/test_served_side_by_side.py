"""Lane P2c6: edits the Rust session serves run side by side, without the document's lock --
each its own step, numbered in the order the session made them (and counted so, whatever order
they end in), and undone one by one."""

import threading

import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox

import word_docs


WRITERS, EDITS = 8, 3


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as box:
        yield box


def test_edits_side_by_side_are_one_step_each(toolbox, constructs_doc):
    session = toolbox.session()
    doc = session.open(constructs_doc.read_bytes(), name=constructs_doc.name)

    def version() -> int:
        return toolbox.dispatch(session, "describe", {"doc": doc}).version

    paragraphs = [block for block, text in word_docs.paragraphs(session, doc)
                  if text.strip() and block != "p:None"][:WRITERS * EDITS]
    before = {block: text for block, text in word_docs.paragraphs(session, doc) if block in paragraphs}
    start = version()
    results, failures = [], []
    gate = threading.Barrier(WRITERS)

    def writer(k: int) -> None:
        view = session.view(context={"agent": f"w{k}"})
        gate.wait(10)
        for n in range(EDITS):
            target = paragraphs[k * EDITS + n]
            result = toolbox.dispatch(view, "word_set_text", {"doc": doc, "target": target,
                                                              "text": f"Writer {k} edit {n}"})
            (results if result.ok else failures).append(result)

    threads = [threading.Thread(target=writer, args=(k,)) for k in range(WRITERS)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(60)
    assert not failures, failures[0].error
    assert sorted(result.version for result in results) == list(range(start + 1, start + WRITERS * EDITS + 1))
    assert version() == start + WRITERS * EDITS
    for _ in range(WRITERS * EDITS):
        assert toolbox.dispatch(session, "undo", {"doc": doc}).ok
    assert version() == start
    assert {block: text for block, text in word_docs.paragraphs(session, doc) if block in before} == before
    session.close(doc)
