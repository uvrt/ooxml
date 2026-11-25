//! Owns: part names -- the one spelling a part has in a package, as ooxml-common's
//! `normalize_part_path` makes it (`\` as `/`, no leading `/`, `.` and `..` resolved) -- and
//! the names derived from them (a part's relationships part, a relationship's target).
//!
//! Must not: touch a package.

use std::sync::Arc;

/// `path` as a part name: backslashes as slashes, no leading slash, `.`, `..` and empty
/// segments resolved as `posixpath.normpath` resolves them.
pub fn normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return String::new();
    }
    let mut segments: Vec<&str> = Vec::new();
    for s in path.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                if segments.last().is_some_and(|l| *l != "..") {
                    segments.pop();
                } else {
                    segments.push("..");
                }
            }
            s => segments.push(s),
        }
    }
    if segments.is_empty() {
        ".".to_owned()
    } else {
        segments.join("/")
    }
}

/// The directory of a part name (`word` of `word/document.xml`; empty at the top).
pub fn directory(part: &str) -> &str {
    part.rsplit_once('/').map_or("", |(d, _)| d)
}

/// The relationships part of `part` (`word/_rels/document.xml.rels`); `_rels/.rels` for the
/// package (an empty name).
pub fn rels_for(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// A relationship's target resolved against its source part's directory.
pub fn resolve_target(base_dir: &str, target: &str) -> String {
    if target.starts_with('/') || base_dir.is_empty() {
        normalize(target)
    } else {
        normalize(&format!("{base_dir}/{target}"))
    }
}

/// A part's name in its package spelling ([`normalize`]'s form): what a store key, a change
/// and an edit's outcome name a part by, kept apart from ids and addresses.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PartName(Arc<str>);

impl PartName {
    /// `path` normalised ([`normalize`]).
    #[must_use]
    pub fn normalize(path: &str) -> Self {
        Self(Arc::from(normalize(path)))
    }

    /// A name already in its package spelling: one a package lists, one [`normalize`] made,
    /// or a known constant (`word/document.xml`).
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        debug_assert!(
            name.is_empty() || normalize(&name) == *name,
            "part name {name:?} is not in its package spelling"
        );
        Self(name)
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name, shared.
    #[must_use]
    pub fn as_arc(&self) -> &Arc<str> {
        &self.0
    }
}

impl std::ops::Deref for PartName {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for PartName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for PartName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PartName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Debug for PartName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&*self.0, f)
    }
}

impl serde::Serialize for PartName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl From<&str> for PartName {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<String> for PartName {
    fn from(name: String) -> Self {
        Self::new(name)
    }
}

impl From<&String> for PartName {
    fn from(name: &String) -> Self {
        Self::new(name.as_str())
    }
}

impl From<Arc<str>> for PartName {
    fn from(name: Arc<str>) -> Self {
        Self::new(name)
    }
}

impl From<PartName> for String {
    fn from(name: PartName) -> Self {
        name.0.to_string()
    }
}

impl PartialEq<str> for PartName {
    fn eq(&self, other: &str) -> bool {
        *self.0 == *other
    }
}

impl PartialEq<&str> for PartName {
    fn eq(&self, other: &&str) -> bool {
        *self.0 == **other
    }
}

impl PartialEq<String> for PartName {
    fn eq(&self, other: &String) -> bool {
        *self.0 == **other
    }
}

impl PartialEq<PartName> for str {
    fn eq(&self, other: &PartName) -> bool {
        *self == *other.0
    }
}

impl PartialEq<PartName> for &str {
    fn eq(&self, other: &PartName) -> bool {
        **self == *other.0
    }
}

impl PartialEq<PartName> for String {
    fn eq(&self, other: &PartName) -> bool {
        **self == *other.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_as_python_spells_them() {
        assert_eq!(normalize("/word/document.xml"), "word/document.xml");
        assert_eq!(normalize("word\\media\\a.png"), "word/media/a.png");
        assert_eq!(normalize("word/./x/../a.xml"), "word/a.xml");
        assert_eq!(normalize("a//b"), "a/b");
        assert_eq!(normalize("../a"), "../a");
        assert_eq!(rels_for("word/document.xml"), "word/_rels/document.xml.rels");
        assert_eq!(rels_for(""), "_rels/.rels");
        assert_eq!(resolve_target("word", "media/a.png"), "word/media/a.png");
        assert_eq!(resolve_target("word", "../customXml/item1.xml"), "customXml/item1.xml");
        assert_eq!(resolve_target("word", "/word/styles.xml"), "word/styles.xml");
        assert_eq!(resolve_target("", "word/document.xml"), "word/document.xml");
    }

    #[test]
    fn a_part_name_reads_as_its_spelling() {
        let name = PartName::normalize("/word\\document.xml");
        assert_eq!(name, "word/document.xml");
        assert_eq!(name.to_string(), "word/document.xml");
        assert_eq!(format!("{name:?}"), "\"word/document.xml\"");
        assert_eq!(PartName::new("word/styles.xml").as_str(), "word/styles.xml");
    }
}
