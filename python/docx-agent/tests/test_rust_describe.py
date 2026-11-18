"""``describe`` served by the Rust session (lane P0d) against today's.

The Rust definition (``crates/ooxml-tools-word``, golden ``tests/golden/describe.json``
there) equals today's byte for byte and renders the same for every provider; the reply of
``ooxml_core.Session.call("describe", ...)`` equals today's envelope, and its data equals
today's in every field it gives (``RUST_DESCRIBED``), on every fixture and on a made-up
document with threads of comments, resolved ones, footnotes beside separators and content
controls under ``mc:Fallback``.  The fields it does not give yet are listed
(``NOT_YET``); a new field of today's ``describe`` fails here until it is placed.
"""
from __future__ import annotations

import io
import json
import zipfile
from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Session"):          # an ooxml-core before 0.9.0
    pytest.skip("ooxml_core has no Session", allow_module_level=True)

from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.adapters import (anthropic_tool_result, openai_function_call_output,
                                       to_anthropic, to_openai_chat, to_openai_responses)

from docx_agent.rust.session import RUST_DESCRIBED, RustTools
from docx_agent.tools import FORMAT, GROUPS, TOOLS

HERE = Path(__file__).resolve().parent
GOLDEN = HERE.parents[2] / "crates" / "ooxml-tools-word" / "tests" / "golden" / "describe.json"
FIXTURES = sorted((HERE / "fixtures").rglob("*.docx"))

#: Today's fields the Rust session does not give yet (ooxml_tools_word::NOT_DESCRIBED).
NOT_YET = {"pages", "problems"}

_W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
_W14 = 'xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"'
_R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"
_CT = "application/vnd.openxmlformats-officedocument.wordprocessingml."


def _rels(*items: tuple[str, str, str]) -> str:
    body = "".join(f'<Relationship Id="{i}" Type="{t}" Target="{target}"/>' for i, t, target in items)
    return f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>'


def made_up() -> bytes:
    """The made-up document of ``crates/ooxml-tools-word/tests/describe.rs``, with content
    types so that today's library opens it."""
    overrides = {"/word/document.xml": _CT + "document.main+xml",
                 "/word/settings.xml": _CT + "settings+xml",
                 "/word/comments.xml": _CT + "comments+xml",
                 "/word/commentsExtended.xml": _CT + "commentsExtended+xml",
                 "/word/commentsIds.xml": _CT + "commentsIds+xml",
                 "/word/footnotes.xml": _CT + "footnotes+xml",
                 "/word/header1.xml": _CT + "header+xml",
                 "/docProps/core.xml": "application/vnd.openxmlformats-package.core-properties+xml"}
    types = ('<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
             '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
             '<Default Extension="xml" ContentType="application/xml"/>'
             + "".join(f'<Override PartName="{p}" ContentType="{t}"/>' for p, t in overrides.items())
             + "</Types>")
    parts = {
        "[Content_Types].xml": types,
        "_rels/.rels": _rels(("rId1", _R + "officeDocument", "word/document.xml"),
                             ("rId2", "http://schemas.openxmlformats.org/package/2006/relationships/"
                                      "metadata/core-properties", "docProps/core.xml")),
        "docProps/core.xml": '<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/'
                             'metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/">'
                             "<dc:creator>Ann</dc:creator><dc:title>Plan<!-- draft --> two</dc:title>"
                             "<dc:description>x</dc:description></cp:coreProperties>",
        "word/_rels/document.xml.rels": _rels(
            ("rId1", _R + "settings", "settings.xml"), ("rId2", _R + "comments", "comments.xml"),
            ("rId3", "http://schemas.microsoft.com/office/2011/relationships/commentsExtended",
             "commentsExtended.xml"),
            ("rId4", "http://schemas.microsoft.com/office/2016/09/relationships/commentsIds", "commentsIds.xml"),
            ("rId5", _R + "footnotes", "footnotes.xml"), ("rId6", _R + "header", "header1.xml")),
        "word/document.xml": f'<w:document {_W} xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">'
                             "<w:body><w:sdt><w:sdtContent><w:p/></w:sdtContent></w:sdt><mc:AlternateContent>"
                             '<mc:Choice Requires="w14"><w:sdt/></mc:Choice><mc:Fallback><w:sdt/></mc:Fallback>'
                             "</mc:AlternateContent><w:p/></w:body></w:document>",
        "word/settings.xml": f'<w:settings {_W}><w:compat><w:compatSetting w:name="other" w:val="1"/>'
                             '<w:compatSetting w:name="compatibilityMode" w:val=" 14 "/></w:compat></w:settings>',
        "word/comments.xml": f'<w:comments {_W} {_W14}><w:comment w:id="0" w:author="Ann"><w:p w14:paraId="0000000a"/>'
                             '</w:comment><w:comment w:id="1" w:author="Bob"><w:p w14:paraId="0000000B"/></w:comment>'
                             '<w:comment w:id="2"><w:p/><w:p w14:paraId="0000000C"/></w:comment></w:comments>',
        "word/commentsExtended.xml": '<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml">'
                                     '<w15:commentEx w15:paraId="0000000A" w15:done="0"/><w15:commentEx '
                                     'w15:paraId="0000000B" w15:paraIdParent="0000000a" w15:done="0"/>'
                                     '<w15:commentEx w15:paraId="0000000C" w15:done="1"/></w15:commentsEx>',
        "word/commentsIds.xml": '<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/'
                                'wordml/cid"><w16cid:commentId w16cid:paraId="0000000A" w16cid:durableId="1A2B3C4D"/>'
                                "</w16cid:commentsIds>",
        "word/footnotes.xml": f'<w:footnotes {_W}><w:footnote w:type="separator" w:id="-1"/><w:footnote '
                              'w:type="continuationSeparator" w:id="0"/><w:footnote w:id="1"/>'
                              '<w:footnote w:type="normal" w:id="2"/></w:footnotes>',
        "word/header1.xml": f"<w:hdr {_W}><w:sdt/></w:hdr>",
    }
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, text in parts.items():
            archive.writestr(name, text)
    return out.getvalue()


def _python_tool():
    return {tool.name: tool for tool in TOOLS}["describe"]


def test_the_rust_definition_is_todays_byte_for_byte():
    from test_rust_tool_specs import export, text

    assert GOLDEN.read_text(encoding="utf-8") == text(export(_python_tool()))
    rust = RustTools()
    assert [tool.name for tool in rust.tools] == ["describe", "word_read", "word_inspect", "find_text",
                                                  "read_batch"]
    assert json.loads(ooxml_core.tool_specs("docx"))[0] == json.loads(GOLDEN.read_text(encoding="utf-8"))


def test_the_rust_definition_renders_as_todays_for_every_provider():
    named = {tool.name: tool for tool in TOOLS}
    rust = RustTools().tools
    today = [named[tool.name] for tool in rust]
    assert to_anthropic(rust) == to_anthropic(today)
    assert to_openai_responses(rust) == to_openai_responses(today)
    assert to_openai_chat(rust) == to_openai_chat(today)


@pytest.fixture(scope="module")
def toolbox():
    """Today's tools, every read answered by the Python path (lane P1c: the toolbox's reads
    are the Rust session's by default; this compares the two)."""
    from docx_agent.rust import reads

    before = reads.use_session(None)
    try:
        yield Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)
    finally:
        reads.use_session(before)


def _both(toolbox, data: bytes, name: str):
    session = toolbox.session()
    doc = session.open(data, name=name)
    # as the model reads it: JSON (an author-less comment's key is "null")
    today = json.loads(json.dumps(toolbox.dispatch(session, "describe", {"doc": doc}).to_json()))
    rust = RustTools()
    rust_doc = rust.open(data, name)
    assert rust_doc == doc
    return today, rust.dispatch("describe", {"doc": rust_doc})


def _assert_equal(today: dict, result) -> None:
    rust = result.to_json()
    assert today["ok"] and rust["ok"], (today, rust)
    assert {k: v for k, v in rust.items() if k != "data"} == {k: v for k, v in today.items() if k != "data"}
    assert list(rust["data"]) == [k for k in RUST_DESCRIBED if k in rust["data"]]
    assert set(RUST_DESCRIBED) - set(rust["data"]) <= {"tagged", "headings_note"}
    for key in rust["data"]:
        assert rust["data"][key] == today["data"][key], key
    assert set(today["data"]) - set(RUST_DESCRIBED) <= NOT_YET, set(today["data"]) - set(RUST_DESCRIBED) - NOT_YET


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: str(p.relative_to(HERE / "fixtures")))
def test_describe_in_rust_equals_todays_on_the_fixtures(toolbox, path):
    # Round 59: every fixture now, the chart in a group of chart-places.docx among them.
    today, rust = _both(toolbox, path.read_bytes(), path.name)
    _assert_equal(today, rust)


def test_describe_in_rust_equals_todays_on_comments_notes_and_controls(toolbox):
    today, rust = _both(toolbox, made_up(), "plan.docx")
    _assert_equal(today, rust)
    assert rust.data["comments"] == {"threads": 2, "open": 1, "by_author": {"Ann": 1, "Bob": 1, "null": 1}}
    assert (rust.data["notes"], rust.data["controls"]) == ({"footnote": 2}, 3)


def test_the_rust_reply_renders_for_every_provider():
    rust = RustTools()
    doc = rust.open(made_up(), "plan.docx")
    result = rust.dispatch("describe", {"doc": doc})
    text = json.dumps(result.envelope, ensure_ascii=False, separators=(",", ":"))
    assert anthropic_tool_result(result, "call_1")["content"] == [{"type": "text", "text": text}]
    assert openai_function_call_output(result, "call_1")["output"] == text
    for provider in ("anthropic", "openai-responses", "openai-chat"):
        assert rust.results(provider, [("call_1", result)])
    short = json.loads(result.to_text(80))
    assert short["truncated"].startswith("the result was ")
    # a refusal keeps Rust's way out
    refused = rust.dispatch("describe", {"doc": "d9"})
    assert not refused.ok and refused.error.code == "not_found"
    assert anthropic_tool_result(refused, "call_2")["is_error"] is True
    assert json.loads(refused.to_text())["error"]["valid_options"] == [doc]
