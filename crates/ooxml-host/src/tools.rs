//! Owns: the session tools the host runs through its seam ([`HostTools`], ooxml-edit's
//! `shared.SESSION_TOOLS` handlers and docx-agent's `new_document`, ported exactly):
//! `open_document` (a blob opened as the document its bytes are, with its format's summary
//! and the problems it opened with counted; a blob that is text, an image or no document
//! refused with where it belongs), `new_document` (the kind's format makes the bytes, the
//! session opens them), `close_document` (the datasets derived from it said) and `read_blob`
//! (a text blob a page at a time, its lines and characters counted). `batch` waits for its
//! port (lane D2): refused as [`NotYet`] refuses it.
//!
//! Must not: know a format (each answers through [`ooxml_session::Format`] and
//! [`ooxml_session::FormatDoc`]), or hold the stores' lock while a document opens.

use crate::catalogue::Own;
use crate::documents::{OpenRefused, Opening};
use crate::own::{NotYet, OwnCall, SessionTools};
use ooxml_session::blobs::Blob;
use ooxml_session::{revision_date, DocId, DocKind, NewDocument};
use ooxml_tools::paging::page_text;
use ooxml_tools::{Done, Failure, Refusal, ToolError, ToolResult, WayOut};
use serde_json::{Map, Value};

/// Text a blob may hold, by type; anything else that decodes as UTF-8 is read too.
const TEXT_TYPES: [&str; 4] = ["text/", "application/json", "application/csv", "application/xml"];

/// The session tools the host runs itself: `open_document`, `new_document`,
/// `close_document` and `read_blob`; `batch` not yet (lane D2).
#[derive(Debug, Clone, Copy, Default)]
pub struct HostTools;

impl SessionTools for HostTools {
    fn run(&self, call: &OwnCall<'_>) -> ToolResult<Value> {
        let answered = match call.own {
            Own::OpenDocument => open_document(call),
            Own::NewDocument => new_document(call),
            Own::CloseDocument => close_document(call),
            Own::ReadBlob => read_blob(call),
            _ => return NotYet.run(call),
        };
        answered.unwrap_or_else(ToolResult::failed)
    }
}

/// The text argument `name` of the call, where it gives one.
fn text<'a>(call: &'a OwnCall<'_>, name: &str) -> Option<&'a str> {
    call.arguments.get(name).and_then(Value::as_str)
}

/// The blob the call names at `field`.
fn blob(call: &OwnCall<'_>, field: &str) -> Result<Blob, ToolError> {
    let handle = text(call, field).unwrap_or_default();
    call.session.stores().blobs.blob(handle, call.retry).cloned()
}

/// A result for the document `id`, at its version.
fn of_document(call: &OwnCall<'_>, id: &DocId, done: Done<Value>) -> ToolResult<Value> {
    let mut result = ToolResult::done(done);
    result.doc = Some(id.to_string());
    result.version = call.session.session().snapshot(id).ok().map(|o| o.number());
    result
}

/// `open_document`: the blob opened as a document.
fn open_document(call: &OwnCall<'_>) -> Result<ToolResult<Value>, ToolError> {
    let found = blob(call, "blob")?;
    let how = Opening {
        source: Some(&found.handle),
        ..Opening::default()
    };
    let id = call
        .session
        .open_with(found.data.clone(), &found.name, &how)
        .map_err(|refused| not_opened(&found, refused, call))?;
    let session = call.session.session();
    let opened = session
        .snapshot(&id)
        .map_err(|e| ToolError::internal(e.to_string(), "open_document"))?;
    let format = session
        .format_doc(&id)
        .map_err(|e| ToolError::internal(e.to_string(), "open_document"))?;
    let mut data = call.session.described(&id);
    data.insert("baseline_problems".into(), Value::from(format.baseline().len()));
    if let Some(summary) = format.summary(opened.any()) {
        data.insert("summary".into(), Value::Object(summary));
    }
    let mut done = Done::new(format!("Opened {} as {id} ({})", opened.name(), opened.kind().as_str()));
    done.created.push(id.to_string());
    done.data = Some(Value::Object(data));
    Ok(of_document(call, &id, done))
}

/// The refusal of a blob no format opened, saying where it belongs.
fn not_opened(blob: &Blob, refused: OpenRefused, call: &OwnCall<'_>) -> ToolError {
    match refused {
        OpenRefused::Limit(message) => ToolError::failure(Failure::Limit, message),
        OpenRefused::Argument(message) | OpenRefused::Unreadable(message) => {
            if is_text(blob) {
                return ToolError::refusal(
                    Refusal::InvalidArguments,
                    format!(
                        "{} is text ({}), not a document: read it with read_blob",
                        blob.name, blob.mime
                    ),
                    WayOut::one(Value::from("read_blob")),
                )
                .at("blob");
            }
            if blob.mime.starts_with("image/") {
                return ToolError::refusal(
                    Refusal::InvalidArguments,
                    format!(
                        "{} is an image ({}), not a document: place it with a picture tool",
                        blob.name, blob.mime
                    ),
                    WayOut::call(call.retry.clone()),
                )
                .at("blob");
            }
            let mut kinds: Vec<&str> = call
                .session
                .session()
                .formats()
                .iter()
                .map(|f| f.kind().as_str())
                .collect();
            kinds.sort_unstable();
            ToolError::refusal(
                Refusal::InvalidArguments,
                message,
                WayOut::names(kinds).unwrap_or_else(|| WayOut::call(call.retry.clone())),
            )
            .at("blob")
        }
    }
}

/// Whether a blob holds text (CSV, Markdown, plain text, JSON): what `read_blob` reads.
fn is_text(blob: &Blob) -> bool {
    if blob.data.starts_with(b"PK") || blob.mime.starts_with("image/") {
        return false;
    }
    let head = blob.data.get(..blob.data.len().min(65_536)).unwrap_or_default();
    let head = head.strip_prefix(b"\xef\xbb\xbf".as_slice()).unwrap_or(head);
    // (not final: a cut through a multi-byte character at the end is still text)
    let text = match std::str::from_utf8(head) {
        Ok(text) => text,
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(head.get(..error.valid_up_to()).unwrap_or_default()).unwrap_or_default()
        }
        Err(_) => return false,
    };
    !text.contains('\0')
}

/// `new_document`: the bytes the kind's format makes, opened by the session.
fn new_document(call: &OwnCall<'_>) -> Result<ToolResult<Value>, ToolError> {
    let kind_named = text(call, "kind").unwrap_or_default();
    let session = call.session.session();
    let format = DocKind::parse(kind_named)
        .and_then(|kind| session.formats().iter().find(|f| f.kind() == kind))
        .ok_or_else(|| {
            let mut kinds: Vec<&str> = session.formats().iter().map(|f| f.kind().as_str()).collect();
            kinds.sort_unstable();
            ToolError::refusal(
                Refusal::InvalidArguments,
                format!("new_document cannot make a {kind_named} document here: no library for it is loaded"),
                WayOut::names(kinds).unwrap_or_else(|| WayOut::call(call.retry.clone())),
            )
            .at("kind")
        })?;
    let template = text(call, "template_blob")
        .filter(|t| !t.is_empty())
        .map(|_| blob(call, "template_blob"))
        .transpose()?;
    let created = revision_date(session.context().clock.now());
    let made = NewDocument {
        size: text(call, "size"),
        title: text(call, "title"),
        author: text(call, "author"),
        template: template.as_ref().map(|t| &*t.data),
        created: &created,
        locale: session.config().locale.as_ref(),
        retry: call.retry,
    };
    let bytes = format.new_document(&made, &session.open_env())?;
    let name = text(call, "name").map_or_else(|| format!("new.{}", format.kind().as_str()), str::to_owned);
    let how = Opening {
        kind: Some(format.kind()),
        source: Some(template.as_ref().map_or("new", |t| t.handle.as_str())),
        ..Opening::default()
    };
    let id = call
        .session
        .open_with(bytes.into(), &name, &how)
        .map_err(|refused| match refused {
            OpenRefused::Limit(message) => ToolError::failure(Failure::Limit, message),
            other => ToolError::internal(other.to_string(), "new_document:open"),
        })?;
    let mut data = call.session.described(&id);
    let opened = session
        .snapshot(&id)
        .map_err(|e| ToolError::internal(e.to_string(), "new_document"))?;
    if let Some(summary) = session.format_doc(&id).ok().and_then(|f| f.summary(opened.any())) {
        data.insert("summary".into(), Value::Object(summary));
    }
    let mut done = Done::new(format!("Made {} as {id}", opened.name()));
    done.created.push(id.to_string());
    done.data = Some(Value::Object(data));
    Ok(of_document(call, &id, done))
}

/// `close_document`: the document let go, the datasets derived from it said.
fn close_document(call: &OwnCall<'_>) -> Result<ToolResult<Value>, ToolError> {
    let doc = text(call, "doc").unwrap_or_default();
    let id = DocId::parse(doc).ok_or_else(|| not_open(call, doc))?;
    let version = call.session.session().snapshot(&id).ok().map(|o| o.number());
    let kept = call.session.close(&id).map_err(|e| e.to_tool_error())?;
    let mut done = if kept.is_empty() {
        Done::new(format!("Closed {doc}"))
    } else {
        let mut done = Done::new(format!(
            "Closed {doc}; the datasets derived from it stay: {}",
            kept.join(", ")
        ));
        let mut data = Map::new();
        data.insert("datasets".into(), Value::from(kept));
        done.data = Some(Value::Object(data));
        done
    };
    done.removed.push(doc.to_owned());
    let mut result = ToolResult::done(done);
    result.doc = Some(doc.to_owned());
    result.version = version;
    Ok(result)
}

/// The refusal of `doc`, which names no open document.
fn not_open(call: &OwnCall<'_>, doc: &str) -> ToolError {
    let open = call
        .session
        .session()
        .documents()
        .iter()
        .map(ToString::to_string)
        .collect();
    ooxml_session::SessionError::NotOpen {
        id: doc.to_owned(),
        open,
    }
    .to_tool_error()
}

/// `read_blob`: a page of a text blob.
fn read_blob(call: &OwnCall<'_>) -> Result<ToolResult<Value>, ToolError> {
    let found = blob(call, "blob")?;
    let handle = found.handle.as_str();
    let mime = if found.mime.is_empty() {
        "application/octet-stream"
    } else {
        found.mime.as_str()
    };
    let refused = |message: String| {
        ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(call.retry.clone())).at("blob")
    };
    if mime.starts_with("image/") {
        return Err(refused(format!(
            "{handle} is an image ({mime}); read_blob reads text. Use a picture tool to place it."
        )));
    }
    if found.data.starts_with(b"PK") {
        return Err(refused(format!(
            "{handle} is a package ({}); open it with open_document.",
            found.name
        )));
    }
    let bytes = found
        .data
        .strip_prefix(b"\xef\xbb\xbf".as_slice())
        .unwrap_or(&found.data);
    let Ok(decoded) = std::str::from_utf8(bytes) else {
        return Err(refused(format!("{handle} ({mime}) is not UTF-8 text")));
    };
    if !TEXT_TYPES.iter().any(|t| mime.starts_with(t)) && decoded.contains('\0') {
        return Err(refused(format!("{handle} ({mime}) is binary, not text")));
    }
    let text = decoded.replace("\r\n", "\n").replace('\r', "\n");
    let cursor = call.arguments.get("cursor").and_then(Value::as_str);
    let limit = call
        .session
        .host()
        .settings()
        .limits
        .result_chars
        .saturating_sub(2000)
        .max(1000);
    let mut first = call.retry.clone();
    first.arguments.shift_remove("cursor");
    let (page, next_cursor) = page_text(&text, cursor, limit, first)?;
    let start = cursor
        .and_then(|c| c.get(1..))
        .and_then(|c| c.parse::<usize>().ok())
        .unwrap_or(0);
    let chars = text.chars().count();
    let lines = text.matches('\n').count() + usize::from(!text.is_empty() && !text.ends_with('\n'));
    let mut data = Map::new();
    data.insert("blob".into(), Value::from(handle));
    data.insert("name".into(), Value::from(found.name.as_str()));
    data.insert("mime".into(), Value::from(mime));
    data.insert("chars".into(), Value::from(chars));
    data.insert("lines".into(), Value::from(lines));
    data.insert("from".into(), Value::from(start));
    data.insert("text".into(), Value::from(page.as_str()));
    let mut done = Done::new(format!(
        "Read {} of {} characters of {}",
        thousands(page.chars().count()),
        thousands(chars),
        found.name
    ));
    done.data = Some(Value::Object(data));
    done.next_cursor = next_cursor;
    Ok(ToolResult::done(done))
}

/// `n` with commas between thousands (Python's `f"{n:,}"`).
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
