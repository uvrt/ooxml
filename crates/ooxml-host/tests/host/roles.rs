//! Roles: a tool or an action outside the session's roles refused in today's words
//! (ooxml-edit's `test_roles`).

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::toy::{call, call_as, session_with};
use ooxml_host::{CallAs, HostSettings, NotRun};
use ooxml_tools::roles::{Role, RoleSet};
use serde_json::json;

fn reviewer() -> ooxml_host::HostSession {
    session_with(HostSettings {
        roles: RoleSet::of([Role::Review]).unwrap(),
        ..HostSettings::default()
    })
}

#[test]
fn a_tool_outside_the_roles_is_refused_with_the_way_to_report_it() {
    let s = reviewer();
    let reply = call(&s, "toy_set", json!({"doc": "d1", "target": "a", "text": "x"}));
    assert_eq!(reply["error"]["code"], "refused");
    assert_eq!(
        reply["error"]["message"],
        "toy_set needs the edit role; this session has review. Do not make the change: report it with \
         the call that would make it (tool and arguments)."
    );
    assert_eq!(reply["error"]["details"], json!({"role": "edit", "roles": ["review"]}));
}

#[test]
fn an_action_outside_the_roles_is_refused_at_action_with_the_actions_left() {
    let s = reviewer();
    assert_eq!(
        call(&s, "toy_notes", json!({"doc": "d1", "action": "list"}))["ok"],
        true
    );
    let reply = call(&s, "toy_notes", json!({"doc": "d1", "action": "add", "text": "x"}));
    assert_eq!(reply["error"]["field"], "action");
    assert_eq!(reply["error"]["valid_options"], json!(["list"]));
    assert!(reply["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("toy_notes add needs the edit role"));
}

#[test]
fn a_call_narrows_its_sessions_roles_and_never_widens_them() {
    let s = crate::toy::session();
    let narrowed = CallAs {
        roles: RoleSet::of([Role::Review]),
        context: None,
        ..CallAs::default()
    };
    let reply = call_as(
        &s,
        "toy_set",
        json!({"doc": "d1", "target": "a", "text": "x"}),
        &narrowed,
    );
    assert_eq!(reply["error"]["code"], "refused");
    let r = reviewer();
    let widened = CallAs {
        roles: Some(RoleSet::all()),
        context: None,
        ..CallAs::default()
    };
    assert!(matches!(r.call("toy_read", json!({}), &widened), Err(NotRun::Roles(_))));
}
