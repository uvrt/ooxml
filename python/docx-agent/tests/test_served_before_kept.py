"""A served call's reply is built from the version its step went from as the session handed
it over, held: never looked up again after the step is served.  Other writers may make more
steps meanwhile than the session keeps for undo (``history_depth``), and that version is let
go from its history; the reply's ``touched`` and ``changed_parts``, and the change the step
counts, are still the step's own."""

import dataclasses
import threading

import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import DEPTH
from ooxml_edit.tools.served import Served

import word_docs

# The Python host's own serving of a Word document (its adapters, owner and mirror), which Phase 3 E2
# deletes with these tests; their calls' answers are the host goldens' (replayed by the Rust host).
pytestmark = pytest.mark.python_host

#: More steps than the session's undo keeps: the step's "before" state is let go meanwhile.
OTHERS = DEPTH + 2


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as box:
        yield box


def _with_steps_between(toolbox, session, doc, tool: str):
    """The session's docx format, its ``serve`` changed so that, once, after the session
    served a ``tool`` call and before the tool layer keeps its step and builds its reply,
    another writer makes :data:`OTHERS` steps: the ``Served`` that call got."""
    fmt = session.formats["docx"]
    served: list[Served] = []

    def serve(call, arguments):
        made = fmt.serve(call, arguments)
        if call.tool.name == tool and not served and isinstance(made, Served):
            served.append(made)
            failures: list[str] = []

            def other() -> None:
                view = session.view(context={"agent": "other"})
                for n in range(OTHERS):
                    result = toolbox.dispatch(view, "word_insert_markdown",
                                              {"doc": doc, "markdown": f"Other {n}.", "at": "end"})
                    if not result.ok:
                        failures.append(repr(result.to_json()))

            thread = threading.Thread(target=other)
            thread.start()
            thread.join(120)
            assert not thread.is_alive() and failures == []
        return made

    session.formats["docx"] = dataclasses.replace(fmt, serve=serve)
    return served


@pytest.mark.parametrize("tool", ["word_delete", "word_set_text"])
def test_a_served_reply_reads_the_state_it_went_from_held(toolbox, tool):
    session = toolbox.session()
    doc = session.open(word_docs.blank(), name="kept.docx")
    entry = session.entry(doc)
    made = toolbox.dispatch(session, "word_insert_markdown",
                            {"doc": doc, "markdown": "First.\n\nSecond.\n\nThird.", "at": "end"})
    assert made.ok, made.to_json()
    target = made.data["blocks"][1]
    served = _with_steps_between(toolbox, session, doc, tool)
    arguments = {"doc": doc, "target": target}
    if tool == "word_set_text":
        arguments["text"] = "Changed."
    start = entry.version

    result = toolbox.dispatch(session, tool, arguments)

    assert result.ok, result.to_json()
    assert served, "the call was not served by the session"
    # (the state the step went from is no longer in the session's history)
    assert entry.history.head(start) is None
    assert entry.version == start + 1 + OTHERS
    assert result.changed_parts == [{"doc": doc, "part": "word/document.xml"}]
    assert result.touched and all(item["doc"] == doc for item in result.touched)
    assert result.version == start + 1
    # the step's change is counted with its parts, as every other step's is
    (change,) = [change for before, end, change in entry.changes_kept if (before, end) == (start, start + 1)]
    assert change.parts == {"word/document.xml"}
    session.close(doc)
