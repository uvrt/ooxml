//! The session's blob, output and image-budget stores: ooxml-edit's
//! `test_tools_dispatch.py` store tests ported (the store's part: what a toy render counts
//! is the host's), each named after the one it ports. Every byte here is made up.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ooxml_session::blobs::{check_declared_type, check_package, image_size, sniff, BlobLimits, BlobStore};
use ooxml_session::outputs::{ImageBudgets, Output, OutputStore};
use ooxml_tools::TryCall;
use ooxml_zip::{Method, NewEntry, Writer};
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex};

/// A PNG's signature and header: what is read of it (`tools_toys.png`'s first bytes).
fn png(width: u32, height: u32) -> Arc<[u8]> {
    let mut data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    data.extend(width.to_be_bytes());
    data.extend(height.to_be_bytes());
    data.extend([8, 6, 0, 0, 0]);
    data.into()
}

fn zip(entries: &[(&str, Vec<u8>)]) -> Arc<[u8]> {
    let mut writer = Writer::new();
    for (name, data) in entries {
        let entry = NewEntry {
            name: (*name).to_owned(),
            method: Method::Deflated,
            dos_time: 0,
            dos_date: 33,
            external_attr: 0,
            create_system: 3,
        };
        writer.add(&entry, data).unwrap();
    }
    writer.finish(b"").unwrap().into()
}

fn retry() -> TryCall {
    TryCall {
        tool: "read_blob".into(),
        arguments: Map::new(),
        fill: Vec::new(),
    }
}

/// `test_the_image_budget_is_a_limit`.
#[test]
fn the_image_budget_is_a_limit() {
    let mut budgets = ImageBudgets::new(Some(1), None);
    budgets.reserve(1).unwrap();
    let again = budgets.reserve(1).unwrap_err().to_json();
    assert_eq!(
        (&again["code"], &again["details"]["budget"]),
        (&json!("limit"), &json!("session"))
    );
    budgets.new_round(); // a round does not refill the session's budget
    assert_eq!(budgets.remaining(), Some(0));
    assert!(budgets.check(1).is_err());
}

/// `test_a_per_round_image_budget_starts_again_each_round`.
#[test]
fn a_per_round_image_budget_starts_again_each_round() {
    let mut budgets = ImageBudgets::new(None, Some(2));
    budgets.reserve(1).unwrap();
    budgets.reserve(1).unwrap();
    assert_eq!(budgets.remaining(), Some(0));
    let refused = budgets.reserve(1).unwrap_err().to_json();
    assert_eq!(refused["code"], "limit");
    assert_eq!(
        refused["details"],
        json!({"budget": "round", "limit": 2, "remaining": 0})
    );
    assert_eq!(
        refused["message"],
        "this round's image budget is 2 and 2 are used; render again next round"
    );
    for _ in 0..30 {
        // no session cap: rounds go on
        budgets.new_round();
        assert!(budgets.reserve(1).is_ok() && budgets.reserve(1).is_ok() && budgets.reserve(1).is_err());
    }
    assert_eq!(budgets.rounds(), 31);
    let mut both = ImageBudgets::new(Some(3), Some(2));
    assert_eq!(both.remaining(), Some(2));
    both.reserve(2).unwrap();
    both.new_round();
    assert_eq!(both.remaining(), Some(1));
    assert_eq!(ImageBudgets::new(None, None).remaining(), None);
    both.release(2); // what the model is never sent is given back
    assert_eq!(both.remaining(), Some(2));
}

/// `test_blobs_are_checked_and_handled`.
#[test]
fn blobs_are_checked_and_handled() {
    let mut blobs = BlobStore::new(BlobLimits::default());
    assert_eq!(blobs.add(png(8, 4), "logo.png", None).unwrap(), "b1");
    assert_eq!(blobs.blob("b1", &retry()).unwrap().mime, "image/png");
    assert_eq!(blobs.add(Arc::from(&b"# Title\n"[..]), "notes.md", None).unwrap(), "b2");
    assert_eq!(blobs.blob("b2", &retry()).unwrap().mime, "text/markdown");
    let declared = blobs
        .add(Arc::from(&b"MZ\x90\x00"[..]), "logo.png", Some("image/png"))
        .unwrap_err();
    assert_eq!(
        declared.0,
        "logo.png is declared image/png but its bytes are application/octet-stream"
    );
    let missing = blobs.blob("b9", &retry()).unwrap_err().to_json();
    assert_eq!(missing["code"], "not_found");
    assert_eq!(missing["message"], "no blob 'b9'");
    assert_eq!(missing["valid_options"], json!(["b1 (logo.png)", "b2 (notes.md)"]));
    let package = zip(&[("[Content_Types].xml", b"<Types/>".to_vec())]);
    assert_eq!(blobs.add(package, "in.pptx", None).unwrap(), "b3");
    assert_eq!(
        blobs.describe()[0],
        json!({"handle": "b1", "name": "logo.png", "mime": "image/png", "size": 29})
    );
    assert_eq!(image_size(&png(8, 4)), Some((8, 4)));
    assert_eq!(sniff(b"plain", "notes.TXT"), "text/plain");
    assert_eq!(
        check_declared_type(b"PK\x03\x04", "a.docx", Some("application/vnd.ms-word")).unwrap(),
        "application/vnd.ms-word"
    );
}

/// `test_an_image_over_the_pixel_limit_is_refused_from_its_header`.
#[test]
fn an_image_over_the_pixel_limit_is_refused_from_its_header() {
    let mut blobs = BlobStore::new(BlobLimits {
        max_image_pixels: 100,
        ..BlobLimits::default()
    });
    let error = blobs.add(png(20, 20), "big.png", None).unwrap_err();
    assert_eq!(error.0, "big.png is 20x20 pixels; at most 100 decoded");
}

/// `test_a_zip_bomb_is_refused_before_opening` (the package check as a blob's).
#[test]
fn a_zip_bomb_is_refused_before_opening() {
    let mb = 1024 * 1024;
    let bomb = zip(&[("a.xml", vec![0; 8 * mb])]);
    let limits = BlobLimits {
        max_uncompressed_bytes: 5 * 1024 * 1024,
        ..BlobLimits::default()
    };
    let error = check_package(&bomb, &limits, "bomb.pptx").unwrap_err();
    assert_eq!(error.0, "bomb.pptx expands to 8388608 bytes; at most 5242880");
    let ratio = zip(&[("a.xml", vec![0; 4 * mb])]);
    let error = check_package(&ratio, &BlobLimits::default(), "ratio.pptx").unwrap_err();
    assert!(
        error.0.starts_with("ratio.pptx: a.xml expands 4194304 bytes from ")
            && error.0.ends_with("over the ratio limit")
    );
    let parts = zip(&[("a", vec![1]), ("b", vec![2]), ("c", vec![3]), ("d", vec![4])]);
    let limits = BlobLimits {
        max_parts: 3,
        ..BlobLimits::default()
    };
    assert_eq!(
        check_package(&parts, &limits, "many.pptx").unwrap_err().0,
        "many.pptx has 4 parts; at most 3"
    );
    let broken: Arc<[u8]> = Arc::from(&b"PK\x03\x04 not a zip"[..]);
    let error = check_package(&broken, &BlobLimits::default(), "").unwrap_err();
    assert!(
        error.0.starts_with("the package is not a readable ZIP package: "),
        "{}",
        error.0
    );
}

/// Outputs are kept until taken, or handed to the application's callback at once; what
/// they say never holds their bytes.
#[test]
fn outputs_are_kept_until_taken() {
    let output = || Output {
        name: "report.docx".into(),
        format: "docx".into(),
        data: Arc::from(&b"PK"[..]),
        doc: Some("d1".into()),
        validate: None,
        state: None,
    };
    let mut outputs = OutputStore::new(None);
    outputs.add(output());
    let taken = outputs.take();
    assert_eq!(taken.len(), 1);
    assert!(outputs.take().is_empty());
    assert_eq!(
        Value::Object(taken[0].describe()),
        json!({"name": "report.docx", "format": "docx", "size": 2, "doc": "d1", "validate": null})
    );
    let handed = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&handed);
    let mut outputs = OutputStore::new(Some(Box::new(move |o: Output| seen.lock().unwrap().push(o.name))));
    outputs.add(output());
    assert!(outputs.take().is_empty());
    assert_eq!(*handed.lock().unwrap(), ["report.docx"]);
}
