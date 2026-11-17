//! Owns: `drawingml-model`'s walk -- each XML part of a package (or a bare XML file) read
//! as ooxml-common reads DrawingML, and resolved under the part's theme and colour maps, as
//! one JSON record per part, in the form `python/drawingml_diff.py` writes for the same
//! files. The rules the two follow alike: parts in name order; a part's theme is the first
//! `theme` relationship of it or of the layout or master it follows, else the package's
//! first theme part; its colour maps are its master's, its layout's and its own.
//!
//! Must not: be used by any library; it is a test instrument.

use super::model::{colour, effects, fill, format_scheme, geometry, geometry_spec, opt, outline, shape_style, xfrm};
use super::resolved;
use super::tree::{node, text_body};
use ooxml_base::Limits;
use ooxml_colour::{effective_colour_map, ColourContext, ColourMap, POWERPOINT, WORD};
use ooxml_drawingml::read::{colour::colour_node, fill as rf, geometry as rg, text, theme, tree};
use ooxml_drawingml::resolve::colour_context;
use ooxml_opc::Package;
use ooxml_schema::a;
use ooxml_xml::{Element, Tree};
use serde_json::{json, Map, Value as J};
use std::sync::Arc;

const COLOUR_ELEMENTS: [&str; 6] = ["srgbClr", "schemeClr", "sysClr", "prstClr", "scrgbClr", "hslClr"];

/// Every element of `root`, itself first, in document order.
fn preorder(root: &Element) -> Vec<&Element> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(e) = stack.pop() {
        out.push(e);
        let children: Vec<&Element> = e.child_elements().map(|c| &**c).collect();
        stack.extend(children.into_iter().rev());
    }
    out
}

struct Source {
    package: Option<Package>,
    names: Vec<String>,
}

impl Source {
    fn tree(&self, name: &str) -> Option<Tree> {
        self.package.as_ref()?.tree(name).ok().flatten()
    }

    fn related(&self, part: &str, suffixes: &[&str]) -> Option<String> {
        let rels = self.package.as_ref()?.relationships(part).ok()?;
        let found = rels
            .iter()
            .filter(|r| suffixes.iter().any(|s| r.rel_type.ends_with(s)))
            .filter_map(|r| r.target_part.clone())
            .find(|t| self.names.contains(t));
        found
    }

    fn parent(&self, part: &str) -> Option<String> {
        self.related(part, &["/slideLayout", "/slideMaster"])
    }

    fn theme_part(&self, part: &str, depth: usize) -> Option<String> {
        if let Some(t) = self.related(part, &["/theme"]) {
            return Some(t);
        }
        match (depth < 4).then(|| self.parent(part)).flatten() {
            Some(parent) => self.theme_part(&parent, depth + 1),
            None => self
                .names
                .iter()
                .find(|n| self.root_local(n).as_deref() == Some("theme"))
                .cloned(),
        }
    }

    fn root_local(&self, name: &str) -> Option<String> {
        self.tree(name).map(|t| t.root().name().local().to_owned())
    }

    fn map_layers(&self, part: &str, depth: usize) -> Vec<Option<ColourMap>> {
        let mut layers = match (depth < 4).then(|| self.parent(part)).flatten() {
            Some(parent) => self.map_layers(&parent, depth + 1),
            None => Vec::new(),
        };
        if let Some(t) = self.tree(part) {
            let root = t.root();
            let own = ooxml_xml::view::child(root, "clrMap")
                .and_then(|m| theme::colour_map(Some(m)))
                .or_else(|| theme::colour_map_override(ooxml_xml::view::child(root, "clrMapOvr")));
            layers.push(own);
        }
        layers
    }
}

fn map_json(m: &ColourMap) -> J {
    J::Array(m.iter().map(|(k, v)| json!([k, v])).collect())
}

struct Context {
    rules: Vec<(&'static str, ColourContext)>,
    formats: ooxml_drawingml::source::FormatScheme,
    maps: J,
}

fn context(source: &Source, part: &str) -> Context {
    let theme_tree = source.theme_part(part, 0).and_then(|t| source.tree(&t));
    let theme = theme_tree.as_ref().map(|t| theme::theme(t.root()));
    let layers = source.map_layers(part, 0);
    let refs: Vec<Option<&ColourMap>> = layers.iter().map(Option::as_ref).collect();
    let map = effective_colour_map(&refs);
    let maps = json!({"layers": layers.iter().map(|l| opt(l.as_ref(), map_json)).collect::<Vec<_>>(),
                      "effective": map_json(&map)});
    let rules = vec![
        ("powerpoint", colour_context(theme.as_ref(), map.clone(), POWERPOINT)),
        ("word", colour_context(theme.as_ref(), map, WORD)),
    ];
    Context {
        rules,
        formats: theme.map(|t| t.formats).unwrap_or_default(),
        maps,
    }
}

fn sp_pr(e: &Element, ctx: &Context) -> (J, J) {
    let view = a::ShapeProperties::new(e);
    let stated_fill = rf::fill(Some(e));
    let stated_line = rf::line(view.line());
    let stated_effects = rf::effects(view.effects());
    let model = json!({
        "fill": opt(stated_fill.as_ref(), fill),
        "outline": opt(stated_line.as_ref(), outline),
        "geometry": opt(rg::geometry(Some(view)).as_ref(), geometry),
        "spec": opt(rg::geometry_spec(Some(view)).as_ref(), geometry_spec),
        "text_rect": opt(rg::text_rect(Some(view)).as_ref(), |r| json!(r)),
        "xfrm": opt(rg::xfrm(view.xfrm()).as_ref(), xfrm),
        "effects": opt(stated_effects.as_ref(), effects),
    });
    let per_rules: Map<String, J> = ctx
        .rules
        .iter()
        .map(|(name, c)| {
            let v = json!({
                "fill": opt(stated_fill.as_ref(), |f| resolved::fill(c, f, None)),
                "outline": opt(stated_line.as_ref(), |o| resolved::outline(c, o, None)),
                "effects": opt(stated_effects.as_ref(), |x| resolved::effects(c, x, None)),
            });
            ((*name).to_owned(), v)
        })
        .collect();
    (model, J::Object(per_rules))
}

fn style(e: &Element, ctx: &Context) -> (J, J) {
    let stated = rf::shape_style(Some(a::ShapeStyle::new(e)));
    let per_rules: Map<String, J> = ctx
        .rules
        .iter()
        .map(|(name, c)| {
            let v = opt(stated.as_ref(), |s| resolved::style_refs(c, s, &ctx.formats));
            ((*name).to_owned(), v)
        })
        .collect();
    (opt(stated.as_ref(), shape_style), J::Object(per_rules))
}

/// One part's record.
fn part(source: &Source, name: &str, root: &Element) -> J {
    let ctx = context(source, name);
    let mut record = Map::new();
    if root.name().local() == "theme" {
        let t = theme::theme(root);
        let colours: Vec<J> = t.colours.iter().map(|(k, c)| json!([k, colour(c)])).collect();
        record.insert(
            "theme".into(),
            json!({"colours": colours, "formats": format_scheme(&t.formats)}),
        );
    }
    let (mut trees, mut sppr, mut bodies, mut styles, mut colours) = (vec![], vec![], vec![], vec![], vec![]);
    for e in preorder(root) {
        match e.name().local() {
            "spTree" => trees.push(J::Array(tree::shape_tree(Some(e)).iter().map(node).collect())),
            "spPr" => {
                let (model, res) = sp_pr(e, &ctx);
                sppr.push(json!({"model": model, "resolved": res}));
            }
            "txBody" | "rich" | "txPr" => bodies.push(opt(text::text_body(Some(e)).as_ref(), text_body)),
            "style" => {
                let (model, res) = style(e, &ctx);
                styles.push(json!({"model": model, "resolved": res}));
            }
            local if COLOUR_ELEMENTS.contains(&local) => {
                let source_colour = colour_node(e);
                colours.push(json!({"source": opt(source_colour.as_ref(), colour),
                                    "resolved": resolved::maybe(&ctx.rules, source_colour.as_ref())}));
            }
            _ => {}
        }
    }
    record.insert("maps".into(), ctx.maps);
    record.insert("trees".into(), J::Array(trees));
    record.insert("sppr".into(), J::Array(sppr));
    record.insert("bodies".into(), J::Array(bodies));
    record.insert("styles".into(), J::Array(styles));
    record.insert("colours".into(), J::Array(colours));
    J::Object(record)
}

/// The record of one file: a package's XML parts, or a bare XML file as one part.
pub fn file(path: &str) -> Result<J, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let mut parts = Map::new();
    if path.ends_with(".xml") {
        let source = Source {
            package: None,
            names: Vec::new(),
        };
        let tree = ooxml_xml::parse(&bytes, &Limits::default().xml).map_err(|e| e.to_string())?;
        parts.insert(path.to_owned(), part(&source, path, tree.root()));
        return Ok(json!({"file": path, "parts": parts}));
    }
    let package = Package::open(Arc::from(bytes), &Limits::default()).map_err(|e| e.to_string())?;
    let mut names: Vec<String> = package.part_names();
    names.sort();
    let source = Source {
        package: Some(package),
        names,
    };
    for name in source
        .names
        .iter()
        .filter(|n| n.ends_with(".xml") && *n != "[Content_Types].xml")
    {
        let record = match source.package.as_ref().map(|p| p.tree(name)) {
            Some(Ok(Some(t))) => part(&source, name, t.root()),
            _ => json!({"error": "parse"}),
        };
        parts.insert(name.clone(), record);
    }
    Ok(json!({"file": path, "parts": parts}))
}
