//! A tool definition's own checks: the common strict subset's limits the types cannot say
//! (`schema.py`'s `subset_problems`) and the consistency of its declarations (`registry.py`'s
//! `definition_problems`).
//!
//! Owns: [`ToolSpec::problems`]. The Python checker stays the authority on the exported
//! schemas (`tests/golden/`, checked by `python/docx-agent/tests/test_rust_tool_specs.py`);
//! this catches a bad definition in `cargo test`, before any export.
//!
//! Must not: change a definition.

use crate::schema::{Kind, ObjectNode, Property, Schema};
use crate::spec::{Holding, ToolSpec};

/// At most this many enum values per property.
pub const MAX_ENUM: usize = 50;
/// At most this many enum values in one tool.
pub const MAX_ENUM_TOTAL: usize = 1000;
/// How deep an object may sit below the root (an array's items count as a level).
pub const MAX_DEPTH: usize = 6;
/// Words that make a property name a file-system path.
pub const PATH_WORDS: [&str; 11] = [
    "path",
    "paths",
    "file",
    "files",
    "filename",
    "filepath",
    "dir",
    "dirs",
    "directory",
    "folder",
    "filesystem",
];

/// Whether `name` is lower snake case of at most 64 characters (`^[a-z][a-z0-9_]{0,63}$`).
#[must_use]
pub fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl ToolSpec {
    /// Every way the definition is wrong; empty when it is right.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !is_name(&self.name) {
            problems.push(format!(
                "the name {:?} is not lower snake case of at most 64",
                self.name
            ));
        }
        if self.description.trim().is_empty() {
            problems.push("no description".into());
        }
        let mut enums = 0usize;
        self.object_problems(&self.arguments, "$", 0, &mut enums, &mut problems);
        if enums > MAX_ENUM_TOTAL {
            problems.push(format!("{enums} enum values in total; at most {MAX_ENUM_TOTAL}"));
        }
        self.declaration_problems(&mut problems);
        problems
    }

    fn object_problems(&self, object: &ObjectNode, at: &str, depth: usize, enums: &mut usize, out: &mut Vec<String>) {
        if depth > MAX_DEPTH {
            out.push(format!("{at}: nested deeper than {MAX_DEPTH} levels"));
        }
        for property in object.properties() {
            let where_ = format!("{at}.{}", property.name);
            if !is_name(&property.name) {
                out.push(format!("{where_}: the name is not lower snake case"));
            }
            if property.name.split('_').any(|word| PATH_WORDS.contains(&word)) {
                out.push(format!("{where_}: a path parameter; take a blob handle instead"));
            }
            if depth == 0
                && property
                    .schema
                    .description
                    .as_deref()
                    .is_none_or(|d| d.trim().is_empty())
            {
                out.push(format!("{where_}: no description"));
            }
            for alias in &property.intent.aliases {
                if object.property(alias).is_some() {
                    out.push(format!("{where_}: the alias {alias:?} is a parameter"));
                }
            }
            self.node_problems(&property.schema, &where_, depth + 1, enums, out);
        }
    }

    fn node_problems(&self, schema: &Schema, at: &str, depth: usize, enums: &mut usize, out: &mut Vec<String>) {
        match &schema.kind {
            Kind::String(node) => {
                if node.values.len() > MAX_ENUM {
                    out.push(format!("{at}: {} enum values; at most {MAX_ENUM}", node.values.len()));
                }
                let mut seen = node.values.clone();
                seen.sort_unstable();
                seen.dedup();
                if seen.len() != node.values.len() {
                    out.push(format!("{at}: repeated enum values"));
                }
                *enums += node.values.len();
                for (alias, value) in &node.value_aliases {
                    if !node.values.is_empty() && !node.values.contains(value) {
                        out.push(format!(
                            "{at}: the value alias {alias:?} reads as {value:?}, not in the enum"
                        ));
                    }
                    if node.values.contains(alias) {
                        out.push(format!("{at}: the value alias {alias:?} is an enum value"));
                    }
                }
                if node.or_number && node.or_integer.is_some() {
                    out.push(format!("{at}: orNumber and orInteger together"));
                }
            }
            Kind::Array(array) => {
                if array.min_items.is_some_and(|n| n > 1) {
                    out.push(format!("{at}: minItems may only be 0 or 1"));
                }
                if array.map_as_pairs && !is_pair(&array.items) {
                    out.push(format!(
                        "{at}: map_as_pairs needs items of two required string properties"
                    ));
                }
                self.node_problems(&array.items, &format!("{at}[]"), depth + 1, enums, out);
            }
            Kind::Object(object) => self.object_problems(object, at, depth, enums, out),
            Kind::Free if self.flags.strict => out.push(format!("{at}: a free-form object in a strict tool")),
            Kind::Integer(_) | Kind::Number(_) | Kind::Boolean | Kind::Free => {}
        }
    }

    fn declaration_problems(&self, out: &mut Vec<String>) {
        let optional = |name: &str| self.arguments.property(name).is_some_and(|p| !p.required);
        for group in &self.exactly_one {
            for name in group.iter().filter(|name| !optional(name)) {
                out.push(format!(
                    "exactly_one names {name:?}, which is not an optional parameter"
                ));
            }
        }
        if let Some(route) = self.route.as_deref().filter(|r| self.arguments.property(r).is_none()) {
            out.push(format!("route names {route:?}, which is not a parameter"));
        }
        for name in &self.documents {
            if name.contains('.') {
                out.extend(crate::documents::path_problem(&self.arguments, name));
            } else if self
                .arguments
                .property(name)
                .is_some_and(|p| p.schema.type_name() != "string")
            {
                out.push(format!("document argument {name:?} is not a string"));
            }
        }
        for property in self.arguments.properties() {
            kind_problems(property, out);
            if let Some(name) = property.intent.names_as.as_deref() {
                if self.arguments.property(name).is_none() || name == property.name {
                    out.push(format!(
                        "{} names_as {name:?}, which is not another parameter",
                        property.name
                    ));
                }
            }
        }
        let holding = [Holding::Refs, Holding::Datasets, Holding::Sources, Holding::Facts];
        for path in &self.paths_first {
            if !holding.iter().any(|what| self.paths(*what).contains(path)) {
                out.push(format!("paths_first names {path:?}, which holds nothing"));
            }
        }
    }
}

fn kind_problems(property: &Property, out: &mut Vec<String>) {
    let intent = &property.intent;
    let name = &property.name;
    if !intent.kind_neutral.is_empty() && intent.kinds.is_empty() {
        out.push(format!("{name}: kind_neutral without kinds"));
    }
    let values = match &property.schema.kind {
        Kind::String(node) => node.values.as_slice(),
        Kind::Array(array) => match &array.items.kind {
            Kind::String(node) => node.values.as_slice(),
            _ => &[],
        },
        _ => &[],
    };
    for (value, _) in &intent.kind_values {
        if !values.contains(value) {
            out.push(format!("{name}: kind_value {value:?} is not in its enum"));
        }
    }
}

fn is_pair(items: &Schema) -> bool {
    items.as_object().is_some_and(|object| {
        matches!(object.properties(), [a, b] if a.required && b.required
            && a.schema.type_name() == "string" && b.schema.type_name() == "string")
    })
}
