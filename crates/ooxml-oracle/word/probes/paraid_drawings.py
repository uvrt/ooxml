"""The ``drawings`` group of paraid_probe.py (``--drawings``): what Word 16.106 does with a
drawing's ``wp14:anchorId`` and ``wp14:editId`` -- the ids the tools stamp on every drawing
at commit, beside the paragraphs' and rows' (lane fu-paraid, #328).

Two documents, each with an inline picture (``P1``) and an anchored one (``A1``), every
paragraph with a paraId:

* **without** -- the drawings carry no ``wp14:anchorId``/``editId`` and the part does not
  declare ``wp14`` (``mc:Ignorable="w14"``), as a generator writes them;
* **with** -- the drawings carry ids of another tool's (``1A2B3D01``/``1A2B3D02`` inline,
  ``1A2B3D03``/``1A2B3D04`` anchored), the part declares ``wp14`` and ignores it.

Scenarios: ``save`` (open, save); ``edit-other`` (type into another paragraph, ``T1``);
``edit-beside`` (type into the inline picture's paragraph, next to it); ``resize-inline`` and
``resize-anchored`` (the picture made wider in Word).  For each drawing the ids before and
after (kept, added, renewed, dropped), and the part's ``wp14`` declaration and
``mc:Ignorable`` after.
"""

from __future__ import annotations

import io
import re
import zipfile

import oracle
from make_fixtures import p, r

WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
WP14 = "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing"
GRAPHIC = ('<a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData '
           'uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.'
           'openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="0" name="grey.png"/>'
           '<pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId9"/><a:stretch><a:fillRect/>'
           '</a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="457200" cy="457200"/>'
           '</a:xfrm><a:prstGeom prst="rect"/></pic:spPr></pic:pic></a:graphicData></a:graphic>')


def _ids(anchor: str | None, edit: str | None) -> str:
    return f' wp14:anchorId="{anchor}" wp14:editId="{edit}"' if anchor else ""


def inline(ids: tuple[str, str] | tuple[None, None]) -> str:
    return (f'<w:r><w:drawing><wp:inline xmlns:wp="{WP}" distT="0" distB="0" distL="0" distR="0"{_ids(*ids)}>'
            '<wp:extent cx="457200" cy="457200"/><wp:docPr id="1" name="Picture 1"/>'
            f'{GRAPHIC}</wp:inline></w:drawing></w:r>')


def anchored(ids: tuple[str, str] | tuple[None, None]) -> str:
    return (f'<w:r><w:drawing><wp:anchor xmlns:wp="{WP}" distT="0" distB="0" distL="114300" distR="114300" '
            'simplePos="0" relativeHeight="251659264" behindDoc="0" locked="0" layoutInCell="1" '
            f'allowOverlap="1"{_ids(*ids)}><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column">'
            '<wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph">'
            '<wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="457200" cy="457200"/>'
            '<wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapSquare wrapText="bothSides"/>'
            f'<wp:docPr id="2" name="Picture 2"/><wp:cNvGraphicFramePr/>{GRAPHIC}</wp:anchor></w:drawing></w:r>')


def document(with_ids: bool) -> bytes:
    """The probe document (paraid_probe.package), with or without the drawings' ids."""
    from paraid_probe import package

    pairs = (("1A2B3D01", "1A2B3D02"), ("1A2B3D03", "1A2B3D04")) if with_ids else ((None, None), (None, None))
    body = "".join([
        p("T1 Word types into this other paragraph", "1A2B3D11"),
        p(inner=inline(pairs[0]) + r(" P1 an inline picture"), para="1A2B3D12"),
        p(inner=anchored(pairs[1]) + r("A1 an anchored picture"), para="1A2B3D13"),
        p("T2 the last paragraph", "1A2B3D14"),
    ])
    data = package(body, picture=True)
    if not with_ids:
        return data
    # the part declares wp14 and ignores it, as Word writes a part with these ids
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            content = source.read(info.filename)
            if info.filename == "word/document.xml":
                content = content.replace(b'mc:Ignorable="w14"',
                                          f'xmlns:wp14="{WP14}" mc:Ignorable="w14 wp14"'.encode())
            target.writestr(info, content)
    return out.getvalue()


def drawings(data: bytes) -> dict:
    """The body's drawings (``inline``, ``anchored``: their ids) and the part's root: whether
    it declares wp14 and what it ignores."""
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        xml = archive.read("word/document.xml").decode("utf-8")
    root = re.search(r"<w:document\b[^>]*>", xml).group(0)
    out: dict = {"declares wp14": f'"{WP14}"' in root,
                 "mc:Ignorable": (re.search(r'mc:Ignorable="([^"]*)"', root) or [None, None])[1]}
    for kind in ("inline", "anchor"):
        found = re.findall(rf"<wp:{kind}\b[^>]*>", xml)
        out[kind] = [[(re.search(r'wp14:anchorId="([^"]*)"', tag) or [None, None])[1],
                      (re.search(r'wp14:editId="([^"]*)"', tag) or [None, None])[1]] for tag in found]
    return out


def _verdict(before: str | None, after: str | None, renewed: str) -> str:
    if before is None:
        return "added" if after else "none"
    return "kept" if after == before else ("dropped" if after is None else renewed)


def compare(before: dict, after: dict) -> dict:
    out = {"declares wp14": after["declares wp14"], "mc:Ignorable": after["mc:Ignorable"]}
    for kind in ("inline", "anchor"):
        pairs = list(zip(before[kind], after[kind]))
        out[kind] = [{"anchorId": _verdict(b[0], a[0], "replaced"), "editId": _verdict(b[1], a[1], "renewed"),
                      "after": a} for b, a in pairs]
        if len(before[kind]) != len(after[kind]):
            out[kind + " count"] = [len(before[kind]), len(after[kind])]
    return out


EDIT = ['set s to start of content of (text object of paragraph {n} of d)',
        'insert text "Edited " at (create range d start s end s)']
#: The picture made wider: the inline one, and the first shape (the anchored one).
RESIZE_INLINE = ['set width of inline picture 1 of d to 72']
RESIZE_ANCHORED = ['set width of shape 1 of d to 72']


def run(session: oracle.Session) -> dict:
    """Every scenario on both documents (the ``drawings`` observations)."""
    results: dict = {}
    for name, with_ids in (("without ids", False), ("with ids", True)):
        data = document(with_ids)
        before = drawings(data)
        scenarios = {
            "save": lambda: session.resave(data, name=f"drawings-{with_ids}"),
            "edit-other": lambda: session.run_script(oracle.word_script([line.format(n=1) for line in EDIT]), data,
                                                     name=f"drawings-{with_ids}", tag="edit-other"),
            "edit-beside": lambda: session.run_script(oracle.word_script([line.format(n=2) for line in EDIT]), data,
                                                      name=f"drawings-{with_ids}", tag="edit-beside"),
            "resize-inline": lambda: session.run_script(oracle.word_script(RESIZE_INLINE), data,
                                                        name=f"drawings-{with_ids}", tag="resize-inline"),
            "resize-anchored": lambda: session.run_script(oracle.word_script(RESIZE_ANCHORED), data,
                                                          name=f"drawings-{with_ids}", tag="resize-anchored"),
        }
        results[name] = {"input": before}
        for scenario, action in scenarios.items():
            outcome = action()
            results[name][scenario] = (compare(before, drawings(outcome.path.read_bytes())) if outcome else
                                       {"outcome": outcome.outcome, "detail": outcome.detail})
    return results


def report(results: dict, out) -> None:
    for name, scenarios in results.items():
        print(f"\n## drawings: {name} -- input {scenarios['input']}", file=out)
        for scenario, result in scenarios.items():
            if scenario != "input":
                print(f"- {scenario}: {result}", file=out)
