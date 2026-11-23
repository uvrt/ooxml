"""E6's new document: the tables of what Word's own new document is made of
(:func:`new_tables`, written by :func:`write_tables` for the probe), and the tools' new document
compared with Word's, part by part (:func:`compare_new`) -- ``e6_probe.py``'s, apart for length.

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

import pprint

from lxml import etree

import probe_paths
from e6_probe import _W, W, parts, word_locale, zipped

# -- the new document's tables -------------------------------------------------------------------

#: Where the measured table goes (:func:`probe_paths.measured`; docx-agent once kept it as
#: ``docx_agent/edit/word_new.py``, the frozen reference still does).
TABLES_NAME = "word_new.py"
_PREFIXES = {W: "w", "http://schemas.microsoft.com/office/word/2010/wordml": "w14",
             "http://schemas.openxmlformats.org/officeDocument/2006/math": "m"}


def _name(qname: str) -> str:
    namespace, _, local = qname[1:].partition("}") if qname.startswith("{") else ("", "", qname)
    return f"{_PREFIXES[namespace]}:{local}" if namespace else local


def node(element) -> tuple:
    """An element as ``(tag, ((attribute, value), ...), (children...))``: values, in Word's
    order, from which ``edit/blank.py`` writes the XML."""
    attributes = tuple((_name(key), value) for key, value in element.attrib.items()
                       if not key.endswith("}rsid") and "rsid" not in key)
    return (_name(element.tag), attributes, tuple(node(child) for child in element))


def english_name(name: str) -> str:
    """English Word's display name of a built-in style ("heading 1" -> "Heading 1")."""
    return " ".join(word[:1].upper() + word[1:] for word in name.split())


def new_tables(data: bytes) -> dict:
    """What a new document is made of, as values: read from the one Word made."""
    files = parts(data)
    styles = etree.fromstring(files["word/styles.xml"])
    by_id = {s.get(_W + "styleId"): s.find(_W + "name").get(_W + "val") for s in styles.findall(_W + "style")}

    def name_of(style) -> str:
        name = style.find(_W + "name").get(_W + "val")
        link = style.find(_W + "link")
        if style.get(_W + "type") == "character" and link is not None:
            # A linked style's name is the interface language's ("Kop 1 Char"); English
            # Word's is its paragraph style's English name and " Char".
            return english_name(by_id[link.get(_W + "val")]) + " Char"
        return name

    names = {sid: name_of(s) for sid, s in ((s.get(_W + "styleId"), s) for s in styles.findall(_W + "style"))}
    out_styles = []
    for style in styles.findall(_W + "style"):
        entry: dict = {"type": style.get(_W + "type"), "name": names[style.get(_W + "styleId")]}
        if style.get(_W + "default") == "1":
            entry["default"] = True
        if style.get(_W + "customStyle") == "1":
            entry["custom"] = True
        children = []
        for child in style:
            tag = _name(child.tag)
            if tag in ("w:name", "w:rsid"):
                continue
            if tag in ("w:basedOn", "w:next", "w:link"):
                children.append((tag, names[child.get(_W + "val")]))
            elif tag == "w:uiPriority":
                children.append((tag, int(child.get(_W + "val"))))
            elif tag in ("w:pPr", "w:rPr", "w:tblPr", "w:trPr", "w:tcPr"):
                children.append(node(child))
            else:
                children.append((tag,))
        entry["children"] = tuple(children)
        out_styles.append(entry)
    latent = styles.find(_W + "latentStyles")
    latent_rows = []
    for exception in latent:
        flags = "".join(flag for flag, key in (("s", "semiHidden"), ("u", "unhideWhenUsed"), ("q", "qFormat"),
                                                ("l", "locked")) if exception.get(_W + key) == "1")
        priority = exception.get(_W + "uiPriority")
        latent_rows.append((exception.get(_W + "name"), None if priority is None else int(priority), flags))
    defaults = styles.find(_W + "docDefaults")
    settings = etree.fromstring(files["word/settings.xml"])
    fonts = etree.fromstring(files["word/fontTable.xml"])
    web = etree.fromstring(files["word/webSettings.xml"])
    return {
        "DOC_DEFAULTS": node(defaults),
        "LATENT_DEFAULTS": tuple((_name(k), v) for k, v in latent.attrib.items()),
        "LATENT": tuple(latent_rows),
        "STYLES": tuple(out_styles),
        "FONTS": tuple(node(font) for font in fonts),
        "COMPAT": tuple((s.get(_W + "name"), s.get(_W + "val")) for s in settings.iter(_W + "compatSetting")),
        "MATH": node(settings.find("{http://schemas.openxmlformats.org/officeDocument/2006/math}mathPr")),
        "COLOR_MAPPING": tuple((_name(k), v) for k, v in settings.find(_W + "clrSchemeMapping").attrib.items()),
        "WEB_SETTINGS": tuple(_name(child.tag) for child in web),
    }


def write_tables(data: bytes) -> None:

    values = new_tables(data)
    lines = ['"""What Word 16.106 makes a new blank document of, as values: the document defaults, the',
             "latent styles, the styles a new document has, the font table, the compatibility settings,",
             "the math settings and the web settings.",
             "",
             "Generated by ``crates/ooxml-oracle/word/probes/e6_probe.py`` from the new document Word made (``make new document``)",
             "and saved; do not edit by hand.  Nothing here is Word's XML: :mod:`docx_agent.edit.blank`",
             "writes every part from these values.  An element is ``(tag, ((attribute, value), ...),",
             "(children...))``; a style is its type, its English name (the interface's own names of",
             "linked character styles, \"Kop 1 Char\", are given as English Word names them, \"Heading 1",
             'Char"), and its children in order, where ``basedOn``, ``next`` and ``link`` name a style',
             "and ``uiPriority`` is a number.  A latent style is ``(name, uiPriority, flags)``: ``s``",
             "semi-hidden, ``u`` unhidden when used, ``q`` quick style, ``l`` locked.",
             '"""', "", "# fmt: off"]
    for key, value in values.items():
        lines.append(f"{key} = " + pprint.pformat(value, width=110, sort_dicts=False))
        lines.append("")
    probe_paths.measured(TABLES_NAME).write_text("\n".join(lines), encoding="utf-8")


# -- new_document against Word's own new document ---------------------------------------------


def _canonical_new(files: dict[str, bytes], part: str, *, word: bool, english_ids: dict[str, str]) -> str:
    """A part of a new document in a form both sides can be compared in: rsids, the
    document id and the window's view dropped; style ids and the linked styles' localised
    names as English Word writes them; the theme's localised name; dates and authors as
    placeholders."""
    data = files[part]
    if part.endswith(".rels") or part == "[Content_Types].xml":
        root = etree.fromstring(data)
        children = sorted(root, key=lambda n: (n.get("PartName") or n.get("Extension") or n.get("Target") or ""))
        for child in children:
            root.remove(child)
            child.attrib.pop("Id", None)
            root.append(child)
        return etree.tostring(root, method="c14n").decode()
    root = etree.fromstring(data)
    for node in list(root.iter()):
        if not isinstance(node.tag, str):
            continue
        for key in list(node.attrib):
            if "rsid" in key:
                del node.attrib[key]
        name = etree.QName(node).localname
        if name in ("rsids", "rsid", "docId", "view") and node.getparent() is not None:
            node.getparent().remove(node)
    if part == "word/styles.xml":
        tables = new_tables(_zip(files)) if word else None
        names = {}
        for style in root.findall(_W + "style"):
            names[style.get(_W + "styleId")] = style.find(_W + "name").get(_W + "val")
        if word:
            renamed = {}
            for style, entry in zip(root.findall(_W + "style"), tables["STYLES"]):
                # English Word's id for the name: the one our new document gives it (a style
                # ours lacks keeps Word's id, and shows as a difference).
                renamed[style.get(_W + "styleId")] = english_ids.get(entry["name"], style.get(_W + "styleId"))
                style.find(_W + "name").set(_W + "val", entry["name"])
            for node in root.iter(_W + "basedOn", _W + "next", _W + "link"):
                node.set(_W + "val", renamed[node.get(_W + "val")])
            for style in root.findall(_W + "style"):
                style.set(_W + "styleId", renamed[style.get(_W + "styleId")])
        del names
    if part == "word/theme/theme1.xml":
        root.set("name", "{theme}")
    if part == "docProps/core.xml":
        for node in root:
            if etree.QName(node).localname in ("created", "modified"):
                node.text = "{date}"
            if etree.QName(node).localname in ("creator", "lastModifiedBy"):
                node.text = "{author}"
    if part == "docProps/app.xml":
        for node in root:
            if etree.QName(node).localname in ("Application", "AppVersion"):
                root.remove(node)
    if part == "word/document.xml":
        for node in root.iter(_W + "p"):
            for key in list(node.attrib):
                del node.attrib[key]
    return etree.tostring(root, method="c14n").decode()


def _zip(files: dict[str, bytes]) -> bytes:
    return zipped(dict(files))


def compare_new(word: bytes) -> dict:
    """The tools' new document (``new_document``: the Rust core's), built with this Word's
    language and separators, against the new document Word made: per part, ``same`` or where
    they differ."""
    import ooxml_core

    locale = word_locale()
    ours = parts(bytes(ooxml_core.new_document(page=("A4", False), language=locale.language,
                                               separators=(locale.decimal_separator, locale.list_separator),
                                               properties=(None, "Word's user", "2026-10-04T00:00:00Z"))))
    styles = etree.fromstring(ours["word/styles.xml"])
    english_ids = {s.find(_W + "name").get(_W + "val"): s.get(_W + "styleId") for s in styles.findall(_W + "style")}
    theirs = parts(word)
    out: dict = {}
    for part in sorted(set(ours) | set(theirs)):
        if part not in ours or part not in theirs:
            out[part] = "only in " + ("Word's" if part in theirs else "ours")
            continue
        a = _canonical_new(ours, part, word=False, english_ids=english_ids)
        b = _canonical_new(theirs, part, word=True, english_ids=english_ids)
        if a == b:
            out[part] = "same"
        else:
            at = next((k for k in range(min(len(a), len(b))) if a[k] != b[k]), min(len(a), len(b)))
            out[part] = {"ours": a[max(0, at - 80):at + 160], "Word's": b[max(0, at - 80):at + 160]}
    return out
