//! `check`'s `reflow` (lane P5b): laid out by the host's page engine handed to the call, its
//! pages, coverage and warnings read there, and `since_last_check` measured from what the
//! document's last check saw. Its equality with docx-agent's is
//! `crates/ooxml-oracle/python/word_reads_diff.py check`'s and docx-agent's
//! `test_rust_check_reflow.py`'s.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{engine, Never, Still, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::StoreConfig;
use ooxml_tools::ArgType;
use ooxml_tools_word::{check, Check, EnginePages, LastCheck, Others, Owned, Version, CHECK_NO_LAYOUT};
use ooxml_word_edit::Editor;
use ooxml_word_model::docx::Versions;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../python/docx-agent/tests/fixtures/generated/trial/safety-bulletin.docx"
);

/// The fixture, and a version of it whose first body text is much longer.
fn versions() -> (Package, Package) {
    let package = Package::open(Arc::from(std::fs::read(FIXTURE).unwrap()), &Limits::default()).unwrap();
    let xml = String::from_utf8(package.read("word/document.xml").unwrap().unwrap().to_vec()).unwrap();
    let at = xml.find("<w:t>").or_else(|| xml.find("<w:t ")).unwrap();
    let close = at + xml[at..].find('>').unwrap() + 1;
    let longer = format!("{}{}{}", &xml[..close], "longer text ".repeat(400), &xml[close..]);
    let edited = package.with_bytes("word/document.xml", Arc::from(longer.into_bytes()));
    (package, edited)
}

/// `check` with `arguments` of `package` (version `number`), `layout` handed to it.
fn checked(package: &Package, number: u64, arguments: Value, last: &LastCheck, layout: Option<&EnginePages>) -> Value {
    let editor = Editor::open(package.clone(), StoreConfig::default(), Arc::new(Still), Arc::new(Unlimited)).unwrap();
    let doc = editor.store().snapshot().doc().clone();
    let versions = Versions::new();
    let version = Version {
        name: "a.docx",
        package,
        number,
        versions: &versions,
        placer: None,
        owned: Some(Owned {
            doc: &doc,
            baseline: editor.baseline(),
            last_check: last,
            layout: layout.map(|l| l as &dyn ooxml_tools_word::WordLayout),
        }),
        others: Others::default(),
    };
    let args = Check::from_arg(&arguments, "").unwrap();
    let done = check(&args, &version, &Never).unwrap();
    if let Some(why) = done.unported.first() {
        return json!({"unported": why});
    }
    json!({"summary": done.summary, "data": done.data})
}

#[test]
fn a_default_check_reports_the_reflow_since_the_last() {
    let (first, edited) = versions();
    let layout = EnginePages::new(Arc::new(Mutex::new(engine())));
    let last = LastCheck::default();
    let reply = checked(&first, 1, json!({"doc": "d1"}), &last, Some(&layout));
    assert_eq!(reply["summary"], "Checked validate, reflow, fields");
    let data = &reply["data"];
    let parts: Vec<&String> = data["timing"].as_object().unwrap().keys().collect();
    assert_eq!(parts, ["validate", "layout", "facts", "fields"]);
    let reflow = &data["reflow"];
    let pages = reflow["pages"].as_u64().unwrap();
    assert!(pages >= 1);
    assert_eq!(reflow["complete"], true);
    assert_eq!(reflow["coverage"]["pages"], pages);
    assert!(reflow.get("since_last_check").is_none(), "{reflow}");
    // the version the last check saw: nothing since
    let again = checked(&first, 1, json!({"doc": "d1", "include": ["reflow"]}), &last, Some(&layout));
    assert!(again["data"]["reflow"].get("since_last_check").is_none());
    // a later version: its pages since
    let later = checked(&edited, 2, json!({"doc": "d1", "include": ["reflow"]}), &last, Some(&layout));
    let since = &later["data"]["reflow"]["since_last_check"];
    let now = later["data"]["reflow"]["pages"].as_u64().unwrap();
    assert!(now > pages, "{later}");
    assert_eq!(since["pages"], json!([pages, now]));
    assert!(since["changed"].as_array().unwrap().contains(&json!(1)), "{since}");
    // the changed pages among those asked for
    let back = checked(&first, 3, json!({"doc": "d1", "include": ["reflow"], "pages": [99]}), &last, Some(&layout));
    assert_eq!(back["data"]["reflow"]["since_last_check"]["changed"], json!([]));
}

#[test]
fn a_reflow_without_the_hosts_layout_is_the_hosts() {
    let (first, _) = versions();
    let reply = checked(&first, 1, json!({"doc": "d1"}), &LastCheck::default(), None);
    assert_eq!(reply["unported"], CHECK_NO_LAYOUT);
}
