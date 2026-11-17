//! The table equals ooxml-common's `stylenames.json`, its one source.

use serde_json::Value;

#[test]
fn the_table_is_ooxml_commons() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../python/ooxml-common/src/ooxml_common/stylenames.json"
    );
    let Ok(text) = std::fs::read_to_string(path) else {
        return; // Built outside the repository: nothing to compare with.
    };
    let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let languages = json
        .get("languages")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let ours = ooxml_style_names::languages();
    assert_eq!(ours.len(), languages.len());
    for (language, (code, entry)) in ours.iter().zip(languages.iter()) {
        assert_eq!(&language.code, code);
        assert_eq!(
            Some(language.region.as_str()),
            entry.get("region").and_then(Value::as_str)
        );
        assert_eq!(
            Some(language.list_separator.as_str()),
            entry.get("list_separator").and_then(Value::as_str)
        );
        let names: Vec<(String, String)> = entry
            .get("names")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
            .collect();
        assert_eq!(language.names, names, "{code}");
    }
}
