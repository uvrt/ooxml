//! Owns: part names -- the one spelling a part has in a package, as ooxml-common's
//! `normalize_part_path` makes it (`\` as `/`, no leading `/`, `.` and `..` resolved) -- and
//! the names derived from them (a part's relationships part, a relationship's target).
//!
//! Must not: touch a package.

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
}
