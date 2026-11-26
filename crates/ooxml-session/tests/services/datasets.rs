//! Datasets (`d:<id>`): defined by one call, named by later ones on any document, kept past
//! a close, and marked stale -- never refreshed -- when their source changes. ooxml-edit's
//! `test_tools_datasets.py` ported onto the toy session (`toy_session`), each test named
//! after the one it ports; the value type's own tests are `ooxml-tools`' unit tests. Every
//! name and figure is made up.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use crate::toy_session::{Ctx, Sess, Tool, NOW};
use ooxml_session::datasets::{Also, DatasetEntry, DatasetLimits, Of};
use ooxml_tools::datasets::{Cache, Dataset, DATASET_PREFIX};
use ooxml_tools::sources::Cell;
use ooxml_tools::{Refusal, ToolError, WayOut};
use serde_json::{json, Value};
use std::sync::Arc;

/// A toy deck's two pages as a dataset: page number, title, title length; each value's
/// source the page it came from (`_titles`).
fn titles(ctx: &Ctx<'_>) -> Dataset {
    let content = ctx.content();
    let rows: Vec<Vec<Cell>> = (1..=2)
        .map(|n: i64| {
            let title = content.titles.get(&n.to_string()).cloned().unwrap_or_default();
            let length = title.chars().count().to_string();
            vec![Cell::Int(n.to_string()), Cell::Text(title), Cell::Int(length)]
        })
        .collect();
    let sources = (1..=2).map(|n| vec![vec![format!("page:{n}")]; 3]).collect();
    Dataset::new(["page", "title", "length"].map(str::to_owned).to_vec(), rows)
        .unwrap()
        .with_sources(sources)
        .unwrap()
        .with_keys(1)
        .unwrap()
        .with_types(vec![Some("number".into()), Some("text".into()), Some("number".into())])
        .unwrap()
}

fn text<'a>(ctx: &'a Ctx<'_>, name: &str) -> Option<&'a str> {
    ctx.arguments.get(name).and_then(Value::as_str)
}

fn ds_derive(ctx: &mut Ctx<'_>) -> Result<Value, ToolError> {
    let id = text(ctx, "id").unwrap().to_owned();
    let address = if let Some(origin) = text(ctx, "from").map(str::to_owned) {
        let entry = ctx.datasets.dataset(&origin, None, &ctx.sess.store, &ctx.retry)?;
        let data = &entry.dataset;
        let derived = Dataset::new(data.columns().to_vec(), data.rows()[..1].to_vec())
            .unwrap()
            .with_sources(data.sources().unwrap()[..1].to_vec())
            .unwrap()
            .with_keys(data.keys())
            .unwrap();
        let store = &ctx.sess.store;
        ctx.datasets.define(
            &id,
            Arc::new(derived),
            Of::Dataset(&entry),
            None,
            &[],
            store,
            NOW,
            "id",
            &ctx.retry,
        )?
    } else {
        let mut made = titles(ctx);
        if let Some(cache) = text(ctx, "cache") {
            made = made.with_cache(Cache::new(&[(cache, 2)]).unwrap());
        }
        let meta = ctx.sess.meta(ctx.doc.as_deref().unwrap());
        let store = &ctx.sess.store;
        ctx.datasets.define(
            &id,
            Arc::new(made),
            Of::Doc(&meta),
            None,
            &[],
            store,
            NOW,
            "id",
            &ctx.retry,
        )?
    };
    if text(ctx, "fail").is_some() {
        return Err(ToolError::refusal(
            Refusal::Refused,
            "failed after defining",
            WayOut::one(Value::Null),
        ));
    }
    Ok(json!({"summary": format!("Derived {address}")}))
}

const DERIVE: Tool = Tool {
    name: "ds_derive",
    datasets: &["from"],
    sources: &[],
    run: ds_derive,
};

fn ds_read(ctx: &mut Ctx<'_>) -> Result<Value, ToolError> {
    let entry = ctx
        .datasets
        .dataset(text(ctx, "from").unwrap(), None, &ctx.sess.store, &ctx.retry)?;
    let heads = |doc: &str| ctx.sess.head(doc);
    let records: Vec<Value> = entry
        .dataset
        .records()
        .iter()
        .map(|row| Value::Object(row.iter().map(|(k, v)| ((*k).to_owned(), v.to_json())).collect()))
        .collect();
    Ok(json!({"columns": entry.dataset.columns(), "rows": records,
              "stale": ctx.datasets.is_stale(&entry, &heads), "doc": entry.doc(), "version": entry.version()}))
}

const READ: Tool = Tool {
    name: "ds_read",
    datasets: &["from"],
    sources: &[],
    run: ds_read,
};

fn ds_insert(ctx: &mut Ctx<'_>) -> Result<Value, ToolError> {
    let mut content = ctx.content();
    let items = ctx.arguments["items"].as_array().unwrap().clone();
    for item in &items {
        let entry = ctx
            .datasets
            .dataset(item["from"].as_str().unwrap(), None, &ctx.sess.store, &ctx.retry)?;
        for value in entry.dataset.column(item["column"].as_str().unwrap())? {
            content.items.push(match value {
                Cell::Text(t) => t.clone(),
                other => other.to_json().to_string(),
            });
        }
    }
    ctx.change = Some((None, content));
    Ok(json!({"summary": format!("Inserted {} column(s)", items.len())}))
}

const INSERT: Tool = Tool {
    name: "ds_insert",
    datasets: &["items[].from"],
    sources: &[],
    run: ds_insert,
};

fn ds_join(ctx: &mut Ctx<'_>) -> Result<Value, ToolError> {
    let id = text(ctx, "id").unwrap().to_owned();
    let also: Vec<String> = ctx.arguments["also"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect();
    let mut entries: Vec<DatasetEntry> = Vec::new();
    let mut metas = Vec::new();
    for other in &also {
        if other.starts_with(DATASET_PREFIX) {
            entries.push(ctx.datasets.dataset(other, None, &ctx.sess.store, &ctx.retry)?);
        } else {
            metas.push(ctx.sess.meta(other));
        }
    }
    let others: Vec<Also<'_>> = also
        .iter()
        .map(|a| {
            if a.starts_with(DATASET_PREFIX) {
                Also::Dataset(entries.iter().find(|e| e.address() == *a).unwrap())
            } else {
                Also::Doc(metas.iter().find(|m| m.doc == *a).unwrap())
            }
        })
        .collect();
    let made = titles(ctx);
    let meta = ctx.sess.meta(ctx.doc.as_deref().unwrap());
    let store = &ctx.sess.store;
    let address = ctx.datasets.define(
        &id,
        Arc::new(made),
        Of::Doc(&meta),
        None,
        &others,
        store,
        NOW,
        "id",
        &ctx.retry,
    )?;
    Ok(json!({"summary": format!("Derived {address}")}))
}

const JOIN: Tool = Tool {
    name: "ds_join",
    datasets: &["also[]"],
    sources: &[],
    run: ds_join,
};

fn set_title(ctx: &mut Ctx<'_>) -> Result<Value, ToolError> {
    let page = ctx.arguments["page"].to_string();
    let mut content = ctx.content();
    content
        .titles
        .insert(page.clone(), text(ctx, "text").unwrap().to_owned());
    ctx.change = Some((Some(page), content));
    Ok(json!({}))
}

const SET_TITLE: Tool = Tool {
    name: "toy_ppt_set_title",
    datasets: &[],
    sources: &[],
    run: set_title,
};

/// The session of the tests: `d1` a deck, `d2` a text.
fn session() -> Sess {
    let mut sess = Sess::new(DatasetLimits::default());
    sess.open("pptx", "deck.pptx");
    sess.open("docx", "text.docx");
    sess
}

fn retitle(sess: &mut Sess, text: &str, page: u32) {
    let reply = sess.call(SET_TITLE, json!({"doc": "d1", "page": page, "text": text}));
    assert!(reply.ok(), "{:?}", reply.error);
}

fn undo(sess: &mut Sess, doc: &str) {
    sess.format.docs.get_mut(doc).unwrap().undo().unwrap();
}

fn redo(sess: &mut Sess, doc: &str) {
    sess.format.docs.get_mut(doc).unwrap().redo().unwrap();
}

fn derive(sess: &mut Sess, doc: &str, id: &str) {
    let reply = sess.call(DERIVE, json!({"doc": doc, "id": id}));
    assert!(reply.ok(), "{:?}", reply.error);
}

fn ids(sess: &Sess) -> Vec<String> {
    sess.store.ids()
}

/// `test_a_dataset_from_invalidated_caches_is_stale_from_the_start_and_stays_so`.
#[test]
fn a_dataset_from_invalidated_caches_is_stale_from_the_start_and_stays_so() {
    let mut sess = session();
    let reply = sess.call(DERIVE, json!({"doc": "d1", "id": "titles", "cache": "invalidated"}));
    assert!(reply.ok());
    assert_eq!(
        reply.warnings,
        ["d:titles is stale: it was computed from formula values whose cache is invalidated (2 cell(s)): they are as \
          last saved, and nothing here recalculates; recalculate the source in its application, then derive it again"]
    );
    assert!(sess.stale("titles"));
    let heads = |doc: &str| sess.head(doc);
    let entry = sess.store.find("titles").unwrap();
    assert_eq!(entry.brief(entry.stale(&heads))["stale"], true);
    let read = sess.call(READ, json!({"from": "d:titles"}));
    assert!(read.data["stale"].as_bool().unwrap() && !read.warnings.is_empty());
    let fresh = sess.call(
        DERIVE,
        json!({"doc": "d1", "id": "other", "cache": "cached_unverified"}),
    );
    assert!(fresh.ok() && fresh.warnings.is_empty() && !sess.stale("other"));
    retitle(&mut sess, "Changed", 1);
    undo(&mut sess, "d1");
    assert!(sess.stale("titles")); // undo cannot make it current
}

/// `test_a_dataset_is_named_by_a_later_call_without_a_document`.
#[test]
fn a_dataset_is_named_by_a_later_call_without_a_document() {
    let mut sess = session();
    let made = sess.call(DERIVE, json!({"doc": "d1", "id": "titles"}));
    assert!(made.ok() && made.datasets == ["d:titles"]);
    let read = sess.call(READ, json!({"from": "d:titles[title]"}));
    assert!(read.ok(), "{:?}", read.error);
    assert_eq!(read.data["columns"], json!(["page", "title", "length"]));
    assert_eq!((&read.data["doc"], &read.data["version"]), (&json!("d1"), &json!(0)));
    assert_eq!(read.data["stale"], false);
    assert!(read.warnings.is_empty() && read.datasets.is_empty());
}

/// `test_a_dataset_from_one_document_fills_another`.
#[test]
fn a_dataset_from_one_document_fills_another() {
    let mut sess = session();
    retitle(&mut sess, "Revenue", 1);
    derive(&mut sess, "d1", "titles");
    let before = sess.format.doc("d2").content().items.len();
    let reply = sess.call(
        INSERT,
        json!({"doc": "d2", "items": [{"from": "d:titles", "column": "title"}]}),
    );
    assert!(reply.ok(), "{:?}", reply.error);
    assert_eq!(sess.format.doc("d2").content().items[before], "Revenue");
}

/// `test_an_unknown_dataset_is_refused_with_the_known_ids`.
#[test]
fn an_unknown_dataset_is_refused_with_the_known_ids() {
    let mut sess = session();
    derive(&mut sess, "d1", "titles");
    derive(&mut sess, "d2", "items");
    let reply = sess.call(
        INSERT,
        json!({"doc": "d2", "items": [{"from": "d:titles", "column": "title"},
                                      {"from": "d:title", "column": "title"}]}),
    );
    let error = reply.failed("not_found");
    assert_eq!(error["field"], "items[1].from");
    assert_eq!(error["valid_options"], json!(["d:titles", "d:items"]));
    assert_eq!(sess.version("d2"), 0); // refused before the handler ran
}

/// `test_a_value_that_is_not_a_dataset_address_is_left_to_the_handler` (rule 1: no
/// option, so the call as given is the way out).
#[test]
fn a_value_that_is_not_a_dataset_address_is_left_to_the_handler() {
    let mut sess = session();
    let reply = sess.call(READ, json!({"from": "t:Products"}));
    let error = reply.failed("not_found");
    assert_eq!(error["valid_options"], json!([]));
    assert_eq!(error["try_call"]["tool"], "ds_read");
}

/// `test_a_dataset_id_has_a_shape`.
#[test]
fn a_dataset_id_has_a_shape() {
    let long = "x".repeat(65);
    for bad in ["", "1st", "has space", "a/b", long.as_str()] {
        let mut sess = session();
        let reply = sess.call(DERIVE, json!({"doc": "d1", "id": bad}));
        let error = reply.failed("invalid_arguments");
        assert_eq!(error["field"], "id");
        assert_eq!(
            error["message"],
            r"a dataset id must match ^[A-Za-z_][A-Za-z0-9_.\-]{0,63}$"
        );
        assert!(sess.store.entries().is_empty());
    }
}

/// `test_redefining_an_id_replaces_it`.
#[test]
fn redefining_an_id_replaces_it() {
    let mut sess = session();
    derive(&mut sess, "d1", "t");
    derive(&mut sess, "d2", "t");
    assert_eq!(ids(&sess), ["d:t"]);
    assert_eq!(sess.store.find("t").unwrap().doc(), Some("d2"));
}

/// `test_a_failed_call_defines_nothing`.
#[test]
fn a_failed_call_defines_nothing() {
    let mut sess = session();
    let reply = sess.call(DERIVE, json!({"doc": "d1", "id": "t", "fail": "yes"}));
    assert!(!reply.ok() && reply.datasets.is_empty());
    assert!(sess.store.entries().is_empty());
}

/// `test_a_dataset_derived_from_a_dataset_keeps_its_origins_source`.
#[test]
fn a_dataset_derived_from_a_dataset_keeps_its_origins_source() {
    let mut sess = session();
    derive(&mut sess, "d1", "titles");
    let made = sess.call(DERIVE, json!({"doc": "d2", "id": "first", "from": "d:titles"}));
    assert!(made.ok(), "{:?}", made.error);
    let first = sess.store.find("first").unwrap();
    assert_eq!(
        (first.doc(), first.version(), first.dataset.rows().len()),
        (Some("d1"), Some(0), 1)
    );
    assert_eq!(first.origin.as_deref(), Some("d:titles"));
    retitle(&mut sess, "Changed", 1);
    assert!(sess.stale("first") && sess.stale("titles"));
}

/// `test_the_session_lists_its_datasets_without_their_values`.
#[test]
fn the_session_lists_its_datasets_without_their_values() {
    let mut sess = session();
    assert!(sess.describe().is_none());
    derive(&mut sess, "d1", "titles");
    assert_eq!(
        sess.describe().unwrap(),
        json!([{"id": "d:titles", "columns": ["page", "title", "length"], "rows": 2,
                "doc": "d1", "version": 0, "stale": false}])
    );
}

/// `describe`'s briefs (`DatasetEntry.brief`, `briefs`): the latest 20, where each came
/// from, its labels; never its values.
#[test]
fn describe_lists_the_latest_datasets_briefly() {
    let mut sess = session();
    for n in 0..22 {
        derive(&mut sess, "d1", &format!("t{n}"));
    }
    let heads = |doc: &str| sess.head(doc);
    let briefs = sess.store.briefs(&heads);
    assert_eq!(briefs["more_datasets"], 2);
    assert_eq!(
        briefs["datasets"][0],
        json!({"id": "d:t2", "columns": ["page", "title", "length"],
               "types": {"page": "number", "title": "text", "length": "number"}, "rows": 2, "from": "d1"})
    );
    let kept = DatasetEntry::new("x", Arc::new(Dataset::new(vec!["a".into()], vec![]).unwrap()), None);
    assert_eq!(kept.brief(false)["from"], "application");
}

/// `test_a_change_to_the_source_makes_it_stale_and_naming_it_warns`.
#[test]
fn a_change_to_the_source_makes_it_stale_and_naming_it_warns() {
    let mut sess = session();
    derive(&mut sess, "d1", "titles");
    let rows = sess.store.find("titles").unwrap().dataset.rows().to_vec();
    retitle(&mut sess, "New title", 1);
    assert!(sess.stale("titles"));
    assert_eq!(sess.store.find("titles").unwrap().dataset.rows(), rows); // never refreshed
    let read = sess.call(READ, json!({"from": "d:titles"}));
    assert_eq!(read.data["stale"], true);
    assert_eq!(
        read.warnings,
        ["d:titles is stale: d1 has changed since it was derived (version 0, now 1); derive it again for current values"]
    );
}

/// `test_a_change_to_another_document_leaves_it_current`.
#[test]
fn a_change_to_another_document_leaves_it_current() {
    let mut sess = session();
    derive(&mut sess, "d1", "titles");
    let reply = sess.call(
        INSERT,
        json!({"doc": "d2", "items": [{"from": "d:titles", "column": "page"}]}),
    );
    assert!(reply.ok());
    assert!(!sess.stale("titles"));
}

/// `test_undoing_the_change_makes_it_current_again_and_redo_stale`.
#[test]
fn undoing_the_change_makes_it_current_again_and_redo_stale() {
    let mut sess = session();
    retitle(&mut sess, "First", 1); // version 1
    derive(&mut sess, "d1", "titles");
    assert_eq!(sess.store.find("titles").unwrap().version(), Some(1));
    retitle(&mut sess, "Second", 1); // version 2
    assert!(sess.stale("titles"));
    undo(&mut sess, "d1");
    assert!(!sess.stale("titles") && sess.version("d1") == 1);
    redo(&mut sess, "d1");
    assert!(sess.stale("titles"));
    undo(&mut sess, "d1");
    undo(&mut sess, "d1"); // before it was derived
    assert!(sess.stale("titles") && sess.version("d1") == 0);
}

/// `test_a_scoped_undo_leaves_it_stale`.
#[test]
fn a_scoped_undo_leaves_it_stale() {
    let mut sess = session();
    retitle(&mut sess, "One", 1);
    retitle(&mut sess, "Two", 2);
    derive(&mut sess, "d1", "titles");
    assert!(!sess.stale("titles"));
    sess.format.docs.get_mut("d1").unwrap().undo_scope("1").unwrap();
    assert!(sess.stale("titles"));
}

/// `test_a_dataset_outlives_closing_its_source_as_it_was`.
#[test]
fn a_dataset_outlives_closing_its_source_as_it_was() {
    let mut sess = session();
    derive(&mut sess, "d1", "current");
    derive(&mut sess, "d1", "old");
    sess.store.find_mut("old").unwrap().source.version = Some(99); // as if of another state
    assert_eq!(sess.close("d1"), ["d:current", "d:old"]);
    let heads = |doc: &str| sess.head(doc);
    let current = sess.store.find("current").unwrap();
    let old = sess.store.find("old").unwrap();
    assert!(current.closed() && !current.stale(&heads) && current.source.source_version(&heads).is_none());
    assert!(old.closed() && old.stale(&heads));
    let read = sess.call(READ, json!({"from": "d:old"}));
    assert!(read.ok());
    assert_eq!(read.data["rows"][0]["page"], 1);
    assert_eq!(
        read.warnings,
        ["d:old is stale: d1 changed after it was derived, before it was closed"]
    );
    assert_eq!(sess.describe().unwrap()[0]["closed"], true);
    // A new document gets a new handle: nothing ties it to the closed one's datasets.
    sess.open("pptx", "deck.pptx");
    assert!(sess.head("d1").is_none() && !sess.stale("current"));
}

/// `test_a_batch_names_a_dataset_an_earlier_op_defined`.
#[test]
fn a_batch_names_a_dataset_an_earlier_op_defined() {
    let mut sess = session();
    let reply = sess.batch(
        &[
            (DERIVE, json!({"doc": "d1", "id": "titles"})),
            (
                INSERT,
                json!({"doc": "d2", "items": [{"from": "d:titles", "column": "title"}]}),
            ),
        ],
        false,
    );
    assert!(reply.ok() && reply.data["failed"] == json!([]), "{:?}", reply.data);
    assert_eq!(reply.datasets, ["d:titles"]);
    assert!(!sess.stale("titles"));
}

/// `test_a_dataset_derived_before_a_later_op_changed_its_source_is_stale_from_the_start`.
#[test]
fn a_dataset_derived_before_a_later_op_changed_its_source_is_stale_from_the_start() {
    let mut sess = session();
    let reply = sess.batch(
        &[
            (DERIVE, json!({"doc": "d1", "id": "before"})),
            (SET_TITLE, json!({"doc": "d1", "page": 1, "text": "X"})),
            (DERIVE, json!({"doc": "d1", "id": "after"})),
        ],
        false,
    );
    assert!(reply.ok());
    assert!(sess.stale("before") && !sess.stale("after"));
    let version = |id: &str| sess.store.find(id).unwrap().version();
    assert_eq!((version("before"), version("after")), (Some(1), Some(1)));
    assert_eq!(sess.version("d1"), 1);
    assert_eq!(
        reply.warnings,
        ["d:before is stale: d1 was changed after it was derived in the same batch; derive it again for current values"]
    );
    undo(&mut sess, "d1");
    assert!(sess.stale("before") && sess.stale("after")); // the state it came from never had a version
}

/// `test_a_failed_atomic_batch_defines_nothing`.
#[test]
fn a_failed_atomic_batch_defines_nothing() {
    let mut sess = session();
    let ops = [
        (DERIVE, json!({"doc": "d1", "id": "titles"})),
        (
            INSERT,
            json!({"doc": "d2", "items": [{"from": "d:nope", "column": "title"}]}),
        ),
    ];
    let reply = sess.batch(&ops, true);
    let error = reply.failed("not_found");
    assert_eq!(error["field"], "ops[1].arguments.items[0].from");
    assert_eq!(error["valid_options"], json!(["d:titles"]));
    assert!(sess.store.entries().is_empty());
    // By default the op that applied keeps its dataset; the failing one names what exists.
    let reply = sess.batch(&ops, false);
    assert!(reply.ok() && reply.data["failed"] == json!([1]));
    let failed = &reply.data["ops"][1]["error"];
    assert_eq!(failed["field"], "arguments.items[0].from");
    assert_eq!(failed["valid_options"], json!(["d:titles"]));
    assert_eq!(ids(&sess), ["d:titles"]);
}

/// `test_a_dataset_over_the_cell_limit_is_refused`.
#[test]
fn a_dataset_over_the_cell_limit_is_refused() {
    let mut sess = Sess::new(DatasetLimits {
        max_cells: 5,
        ..DatasetLimits::default()
    });
    sess.open("pptx", "deck.pptx");
    let reply = sess.call(DERIVE, json!({"doc": "d1", "id": "titles"})); // 2 x 3 = 6
    let error = reply.failed("limit");
    assert_eq!(
        error["message"],
        "the dataset has 6 values (2 rows of 3 columns); at most 5: group, filter or take the top rows"
    );
    assert!(sess.store.entries().is_empty());
}

/// `test_the_session_keeps_at_most_max_datasets_and_an_id_may_be_reused`.
#[test]
fn the_session_keeps_at_most_max_datasets_and_an_id_may_be_reused() {
    let mut sess = Sess::new(DatasetLimits {
        max_datasets: 2,
        ..DatasetLimits::default()
    });
    sess.open("pptx", "deck.pptx");
    derive(&mut sess, "d1", "a");
    derive(&mut sess, "d1", "b");
    let full = sess.call(DERIVE, json!({"doc": "d1", "id": "c"}));
    let error = full.failed("limit");
    assert_eq!(error["valid_options"], json!(["d:a", "d:b"]));
    assert_eq!(
        error["message"],
        "2 datasets are kept; at most 2: reuse an id to replace one"
    );
    derive(&mut sess, "d1", "a");
    let retry = ooxml_tools::TryCall {
        tool: "x".into(),
        arguments: serde_json::Map::new(),
        fill: Vec::new(),
    };
    sess.store.drop_dataset("d:a", &retry).unwrap();
    derive(&mut sess, "d1", "c");
    assert_eq!(ids(&sess), ["d:b", "d:c"]);
}

/// A batch whose op would keep over the session's limit: that op refused as it defines
/// (as today's batch refuses it, `define_dataset`'s `limit`), counting what the batch's earlier
/// ops defined; the rest applied, or none in an atomic batch.
#[test]
fn a_batch_op_over_the_dataset_limit_is_refused_and_the_batchs_rule_decides_the_rest() {
    let mut sess = Sess::new(DatasetLimits {
        max_datasets: 2,
        ..DatasetLimits::default()
    });
    sess.open("pptx", "deck.pptx");
    let ops = [
        (DERIVE, json!({"doc": "d1", "id": "a"})),
        (DERIVE, json!({"doc": "d1", "id": "b"})),
        (DERIVE, json!({"doc": "d1", "id": "c"})),
    ];
    let atomic = sess.batch(&ops, true);
    atomic.failed("limit");
    assert!(sess.store.entries().is_empty(), "an atomic batch keeps none");
    let partial = sess.batch(&ops, false);
    assert!(
        partial.ok() && partial.data["failed"] == json!([2]),
        "{:?}",
        partial.data
    );
    assert_eq!(
        partial.data["ops"][2]["error"]["message"],
        "2 datasets are kept; at most 2: reuse an id to replace one"
    );
    assert_eq!(ids(&sess), ["d:a", "d:b"], "never applied over the limit");
}

/// `test_a_dataset_entry_names_its_address`.
#[test]
fn a_dataset_entry_names_its_address() {
    let dataset = Dataset::new(vec!["a".into()], vec![vec![Cell::Int("1".into())]]).unwrap();
    let entry = DatasetEntry::new("x", Arc::new(dataset), None);
    let heads = |_: &str| None;
    assert_eq!(entry.address(), format!("{DATASET_PREFIX}x"));
    assert!(!entry.stale(&heads) && !entry.closed());
}

/// The session of the join tests: `d3` another deck (`three`).
fn three() -> Sess {
    let mut sess = session();
    sess.open("pptx", "other.pptx");
    sess
}

fn ok(sess: &mut Sess, tool: Tool, arguments: Value) {
    let reply = sess.call(tool, arguments);
    assert!(reply.ok(), "{:?}", reply.error);
}

/// `test_a_dataset_that_also_read_a_document_goes_stale_when_it_changes`.
#[test]
fn a_dataset_that_also_read_a_document_goes_stale_when_it_changes() {
    let mut sess = three();
    ok(&mut sess, JOIN, json!({"doc": "d1", "id": "joined", "also": ["d3"]}));
    let entry = sess.store.find("joined").unwrap();
    assert!(!sess.stale("joined"));
    assert_eq!(
        entry.source.also.iter().map(|o| o.doc.as_deref()).collect::<Vec<_>>(),
        [Some("d3")]
    );
    assert_eq!(entry.source.also[0].version, Some(sess.version("d3")));
    ok(&mut sess, SET_TITLE, json!({"doc": "d3", "page": 1, "text": "New"}));
    assert!(sess.stale("joined") && sess.store.find("joined").unwrap().doc() == Some("d1"));
    let read = sess.call(READ, json!({"from": "d:joined"}));
    assert_eq!(
        read.warnings,
        [
            "d:joined is stale: d3 has changed since it was derived (version 0, now 1); derive it again for current \
          values (a source it also read)"
        ]
    );
    undo(&mut sess, "d3");
    assert!(!sess.stale("joined")); // back where it was derived
                                    // what is derived from it inherits every source
    ok(
        &mut sess,
        DERIVE,
        json!({"doc": "d1", "id": "first", "from": "d:joined"}),
    );
    assert!(!sess.stale("first"));
    ok(&mut sess, SET_TITLE, json!({"doc": "d3", "page": 1, "text": "Again"}));
    assert!(sess.stale("first"));
}

/// `test_a_dataset_that_also_read_a_dataset_goes_stale_with_its_source`.
#[test]
fn a_dataset_that_also_read_a_dataset_goes_stale_with_its_source() {
    let mut sess = three();
    ok(&mut sess, DERIVE, json!({"doc": "d3", "id": "other"}));
    ok(
        &mut sess,
        JOIN,
        json!({"doc": "d1", "id": "joined", "also": ["d:other"]}),
    );
    assert!(!sess.stale("joined"));
    retitle(&mut sess, "Own change", 1); // its own source: stale
    assert!(sess.stale("joined"));
    undo(&mut sess, "d1");
    assert!(!sess.stale("joined"));
    ok(&mut sess, SET_TITLE, json!({"doc": "d3", "page": 1, "text": "New"}));
    assert!(sess.stale("joined") && sess.stale("other"));
}

/// `test_closing_another_source_keeps_its_staleness_for_good`.
#[test]
fn closing_another_source_keeps_its_staleness_for_good() {
    let mut sess = three();
    ok(&mut sess, JOIN, json!({"doc": "d1", "id": "current", "also": ["d3"]}));
    ok(&mut sess, SET_TITLE, json!({"doc": "d3", "page": 1, "text": "New"}));
    ok(&mut sess, JOIN, json!({"doc": "d1", "id": "later", "also": ["d3"]}));
    ok(&mut sess, SET_TITLE, json!({"doc": "d3", "page": 1, "text": "Newer"}));
    undo(&mut sess, "d3"); // later is current again
    assert!(sess.stale("current") && !sess.stale("later"));
    assert!(sess.close("d3").is_empty()); // neither one's own source
    assert!(sess.stale("current") && !sess.stale("later"));
    assert!(sess.store.find("later").unwrap().source.also[0].closed());
    assert!(!sess.store.find("current").unwrap().closed()); // its own source is open
}
