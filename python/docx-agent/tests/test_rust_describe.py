"""``describe`` on the Rust host (lane P0d): its reply renders for every provider, on
a made-up document with threads of comments, resolved ones, footnotes beside separators and
content controls under ``mc:Fallback``; a refusal keeps its way out.
"""
from __future__ import annotations

import io
import json
import zipfile

from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.adapters import anthropic_tool_result, openai_function_call_output, results_for

from docx_agent.tools import FORMAT, GROUPS, TOOLS

_W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
_W14 = 'xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"'
_R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"
_CT = "application/vnd.openxmlformats-officedocument.wordprocessingml."


def _rels(*items: tuple[str, str, str]) -> str:
    body = "".join(f'<Relationship Id="{i}" Type="{t}" Target="{target}"/>' for i, t, target in items)
    return f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>'


def made_up() -> bytes:
    """The made-up document of ``crates/ooxml-tools-word/tests/integration/describe.rs``, with
    content types."""
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


def test_the_rust_reply_renders_for_every_provider():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        session = toolbox.session()
        doc = session.open(made_up(), "plan.docx")
        result = toolbox.dispatch(session, "describe", {"doc": doc})
        text = result.to_text()
        assert anthropic_tool_result(result, "call_1")["content"] == [{"type": "text", "text": text}]
        assert openai_function_call_output(result, "call_1")["output"] == text
        for provider in ("anthropic", "openai-responses", "openai-chat"):
            assert results_for(provider, [("call_1", result)])
        short = json.loads(result.to_text(80))
        assert short["truncated"].startswith("the result was ")
        # a refusal keeps Rust's way out
        refused = toolbox.dispatch(session, "describe", {"doc": "d9"})
    assert not refused.ok and refused.error.code == "not_found"
    assert anthropic_tool_result(refused, "call_2")["is_error"] is True
    assert json.loads(refused.to_text())["error"]["valid_options"] == [doc]
