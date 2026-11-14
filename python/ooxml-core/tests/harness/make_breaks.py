"""A made-up document for the line breaker's rules -- justified text (mode 15's squeeze),
long words (emergency breaks), tab stops of every alignment (past the edge too), soft and
no-break hyphens, dashes, line breaks, caps, w:w, tracking, kerning, mixed faces and sizes,
indents and exact / atLeast / auto line rules.  Seeded; written on docx-agent's blank document.

    python make_breaks.py out.docx [paragraphs]
"""
import io
import random
import re
import sys
import zipfile

from docx_agent import Document

WORDS = ("lantern meadow river stone harbour quiet orchard copper valley signal morning ledger window garden "
         "timber bridge market evening canvas thread pebble summit a an of to in it is be as at so we he by or "
         "on do if me my up").split()
LONG = ("incomprehensibilities", "pneumonoultramicroscopic", "Donaudampfschifffahrtsgesellschaftskapitän",
        "characteristically", "well-established", "north–south", "state-of-the-art", "1,234.56", "12.5%",
        "AVeryLongIdentifierWithoutAnySpacesAtAllInIt")
FACES = ("Calibri", "Aptos", "Arial", "Times New Roman", "Courier New", "Cambria", "Georgia", "Verdana")


def esc(t):
    return t.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def run(rng, text, face, size):
    rpr = [f'<w:rFonts w:ascii="{face}" w:hAnsi="{face}" w:cs="{face}"/>']
    if rng.random() < 0.2:
        rpr.append("<w:b/>")
    if rng.random() < 0.15:
        rpr.append("<w:i/>")
    if rng.random() < 0.05:
        rpr.append("<w:caps/>")
    if rng.random() < 0.08:
        rpr.append(f'<w:spacing w:val="{rng.choice([-10, 4, 7, 20])}"/>')
    if rng.random() < 0.06:
        rpr.append(f'<w:w w:val="{rng.choice([80, 90, 150])}"/>')
    rpr.append(f'<w:sz w:val="{size}"/><w:szCs w:val="{size}"/>')
    if rng.random() < 0.5:
        rpr.append('<w:kern w:val="2"/>')
    return f'<w:r><w:rPr>{"".join(rpr)}</w:rPr><w:t xml:space="preserve">{esc(text)}</w:t></w:r>'


def mark(rng, kind, face, size):
    return f'<w:r><w:rPr><w:rFonts w:ascii="{face}" w:hAnsi="{face}"/><w:sz w:val="{size}"/></w:rPr><w:{kind}/></w:r>'


def paragraph(rng):
    ppr = []
    jc = rng.choice(["both", "both", "left", "center", "right", None])
    if jc:
        ppr.append(f'<w:jc w:val="{jc}"/>')
    if rng.random() < 0.5:
        ind = f'w:left="{rng.choice([0, 360, 720, 1440])}" w:right="{rng.choice([0, 0, 500])}"'
        ind += rng.choice(["", f' w:firstLine="{rng.choice([200, 567])}"', f' w:hanging="{rng.choice([200, 360])}"'])
        ppr.append(f"<w:ind {ind}/>")
    rule = rng.choice(["auto", "auto", "exact", "atLeast"])
    line = {"auto": rng.choice([240, 259, 276, 360]), "exact": rng.choice([200, 300, 400]),
            "atLeast": rng.choice([100, 300, 500])}[rule]
    ppr.append(f'<w:spacing w:after="{rng.choice([0, 120, 160])}" w:line="{line}" w:lineRule="{rule}"/>')
    tabs = rng.random() < 0.3
    if tabs:
        stops = sorted(rng.sample([1000, 2500, 4000, 5500, 7000, 9800, 11000], 3))
        ppr.append("<w:tabs>" + "".join(
            f'<w:tab w:val="{rng.choice(["left", "center", "right", "decimal"])}" w:pos="{p}"/>' for p in stops)
            + "</w:tabs>")
    body = []
    face = rng.choice(FACES)
    size = rng.choice([16, 20, 22, 24, 28, 36])
    for _ in range(rng.randint(1, 6)):
        if rng.random() < 0.3:
            face = rng.choice(FACES)
            size = rng.choice([16, 20, 22, 24, 28, 36])
        words = [rng.choice(LONG) if rng.random() < 0.06 else rng.choice(WORDS) for _ in range(rng.randint(1, 30))]
        text = " ".join(words) + rng.choice([" ", "", "  "])
        body.append(run(rng, text, face, size))
        r = rng.random()
        if tabs and r < 0.4:
            body.append(mark(rng, "tab", face, size))
        elif r < 0.45:
            body.append(mark(rng, "br", face, size))
        elif r < 0.55:
            body.append(mark(rng, "softHyphen", face, size))
        elif r < 0.6:
            body.append(mark(rng, "noBreakHyphen", face, size))
    return f'<w:p><w:pPr>{"".join(ppr)}</w:pPr>{"".join(body)}</w:p>'


def main():
    out = sys.argv[1]
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 400
    rng = random.Random(14)
    data = Document.new().to_bytes()
    zin = zipfile.ZipFile(io.BytesIO(data))
    xml = zin.read("word/document.xml").decode()
    m = re.search(r"<w:body>(.*?)(<w:sectPr.*</w:sectPr>)</w:body>", xml, re.S)
    body = "".join(paragraph(rng) for _ in range(n))
    xml = xml[:m.start(1)] + body + m.group(2) + xml[m.end(2):]
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zout:
        for name in zin.namelist():
            zout.writestr(name, xml if name == "word/document.xml" else zin.read(name))
    open(out, "wb").write(buf.getvalue())


if __name__ == "__main__":
    main()
