//! `check`'s `reflow` (lane P5b): laid out by the host's page engine handed to the call, its
//! pages, coverage and warnings read there, and `since_last_check` measured from what the
//! document's last check saw. Its equality with docx-agent's is
//! `crates/ooxml-oracle/python/word_reads_diff.py check`'s and docx-agent's
//! `test_rust_check_reflow.py`'s. A layout past the call's limit on laying out is said as
//! docx-agent says it, and the check goes on; the limit holds the layout only, never the
//! summary read from a version laid out.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::common::{engine, Never, Still, Unlimited};
use ooxml_base::Limits;
use ooxml_opc::Package;
use ooxml_store::StoreConfig;
use ooxml_tools::ArgType;
use ooxml_tools_word::{
    check, Check, EnginePages, LastCheck, LayoutLimit, Others, Owned, Version, WordLayout, CHECK_NO_LAYOUT,
};
use ooxml_word_edit::Editor;
use ooxml_word_model::docx::Versions;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    limited(package, number, arguments, last, layout, None)
}

/// [`checked`], laying out within `limit` where given.
fn limited(
    package: &Package,
    number: u64,
    arguments: Value,
    last: &LastCheck,
    layout: Option<&EnginePages>,
    limit: Option<LayoutLimit<'_>>,
) -> Value {
    let editor = Editor::open(
        package.clone(),
        StoreConfig::default(),
        Arc::new(Still),
        Arc::new(Unlimited),
    )
    .unwrap();
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
            layout_limit: limit,
        }),
        others: Others::default(),
        word: Some(&doc),
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
    let again = checked(
        &first,
        1,
        json!({"doc": "d1", "include": ["reflow"]}),
        &last,
        Some(&layout),
    );
    assert!(again["data"]["reflow"].get("since_last_check").is_none());
    // a later version: its pages since
    let later = checked(
        &edited,
        2,
        json!({"doc": "d1", "include": ["reflow"]}),
        &last,
        Some(&layout),
    );
    let since = &later["data"]["reflow"]["since_last_check"];
    let now = later["data"]["reflow"]["pages"].as_u64().unwrap();
    assert!(now > pages, "{later}");
    assert_eq!(since["pages"], json!([pages, now]));
    assert!(since["changed"].as_array().unwrap().contains(&json!(1)), "{since}");
    // the changed pages among those asked for
    let back = checked(
        &first,
        3,
        json!({"doc": "d1", "include": ["reflow"], "pages": [99]}),
        &last,
        Some(&layout),
    );
    assert_eq!(back["data"]["reflow"]["since_last_check"]["changed"], json!([]));
}

#[test]
fn a_reflow_without_the_hosts_layout_is_the_hosts() {
    let (first, _) = versions();
    let reply = checked(&first, 1, json!({"doc": "d1"}), &LastCheck::default(), None);
    assert_eq!(reply["unported"], CHECK_NO_LAYOUT);
}

/// A limit on laying out that has passed.
struct Past;

impl ooxml_base::Cancel for Past {
    fn is_cancelled(&self) -> bool {
        true
    }
}

#[test]
fn a_reflow_past_the_limit_on_laying_out_says_so_and_the_check_goes_on() {
    let (first, edited) = versions();
    let layout = EnginePages::new(Arc::new(Mutex::new(engine())));
    let last = LastCheck::default();
    let seen = checked(&first, 1, json!({"doc": "d1"}), &last, Some(&layout));
    let limit = LayoutLimit {
        cancel: &Past,
        timeout: Duration::from_millis(1),
    };
    let reply = limited(&edited, 2, json!({"doc": "d1"}), &last, Some(&layout), Some(limit));
    let data = &reply["data"];
    assert_eq!(
        data["reflow"],
        json!({"error": "timeout: laying the document out did not finish within 0.001 s"})
    );
    assert!(data["validate"].is_object() && data["fields"].is_object(), "{reply}");
    let parts: Vec<&String> = data["timing"].as_object().unwrap().keys().collect();
    assert_eq!(parts, ["validate", "layout", "facts", "fields"]);
    // the version a check saw last is still the first: the next check measures from it
    let later = checked(
        &edited,
        2,
        json!({"doc": "d1", "include": ["reflow"]}),
        &last,
        Some(&layout),
    );
    let pages = seen["data"]["reflow"]["pages"].clone();
    assert_eq!(
        later["data"]["reflow"]["since_last_check"]["pages"][0], pages,
        "{later}"
    );
}

#[test]
fn a_version_laid_out_is_summarised_whole_past_the_limit_on_laying_out() {
    // docx-agent's deadline holds its layout, not the facts read from it: a version the
    // engine laid out already is answered whatever the limit says, and its summary is made
    // whole however long its pages take to paint
    let (first, _) = versions();
    let layout = EnginePages::new(Arc::new(Mutex::new(engine())));
    layout.laid(&first, &Never).unwrap();
    let limit = LayoutLimit {
        cancel: &Past,
        timeout: Duration::from_millis(1),
    };
    let reply = limited(
        &first,
        1,
        json!({"doc": "d1"}),
        &LastCheck::default(),
        Some(&layout),
        Some(limit),
    );
    let reflow = &reply["data"]["reflow"];
    assert!(reflow.get("error").is_none(), "{reply}");
    assert_eq!(reflow["complete"], true);
    assert!(reflow["pages"].as_u64().unwrap() >= 1, "{reply}");
}
