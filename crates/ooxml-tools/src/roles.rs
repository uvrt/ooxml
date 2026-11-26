//! Owns: the roles a call may need (ooxml-edit's `registry.py` roles, as values): [`Role`],
//! the four additive toolsets an application loads in any combination; [`Needs`], what one
//! call or action needs (a role, or any); [`RoleSet`], the roles a session or a call holds;
//! and [`HostFacts`], what a host holds of a tool beside its spec -- the roles of its
//! actions, the role a call's arguments settle, the calls of a changing tool that only read,
//! whether it is a reviewer's main tool, and the optional fields that apply only to some
//! actions.
//!
//! Must not: check a call (the host does), or know a tool: a format declares its tools'
//! facts beside their registration.

use serde_json::{Map, Value};
use std::fmt;

/// One additive toolset an application loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Changes nothing in a document but its comments: reading, finding, rendering,
    /// checking, comparing; adding and replying to comments.
    Review,
    /// Closes, reopens and deletes comments.
    Resolve,
    /// Accepts and rejects tracked changes.
    Accept,
    /// Makes every other change.
    Edit,
}

impl Role {
    /// Every role, in today's order (`ROLES`).
    pub const ALL: [Role; 4] = [Role::Review, Role::Resolve, Role::Accept, Role::Edit];

    /// Its name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Review => "review",
            Role::Resolve => "resolve",
            Role::Accept => "accept",
            Role::Edit => "edit",
        }
    }

    /// The role `name` names.
    #[must_use]
    pub fn parse(name: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|role| role.as_str() == name)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a call or an action needs: a role, or none in particular (`ANY_ROLE`: in every
/// role's toolset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Needs {
    /// This role.
    Role(Role),
    /// Any role.
    Any,
}

impl Needs {
    /// Its name as a reply says it (`any` for [`Needs::Any`]).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Needs::Role(role) => role.as_str(),
            Needs::Any => "any",
        }
    }

    /// What `name` names: a role, or `any`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Needs> {
        if name == "any" {
            return Some(Needs::Any);
        }
        Role::parse(name).map(Needs::Role)
    }
}

/// The roles a session, a view or a call holds, in today's order, never empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoleSet(u8);

impl Default for RoleSet {
    fn default() -> Self {
        RoleSet::all()
    }
}

impl RoleSet {
    /// All four, the whole toolbox.
    #[must_use]
    pub fn all() -> RoleSet {
        RoleSet(0b1111)
    }

    fn bit(role: Role) -> u8 {
        match role {
            Role::Review => 1,
            Role::Resolve => 2,
            Role::Accept => 4,
            Role::Edit => 8,
        }
    }

    /// The roles `roles` names; `None` when it names none.
    #[must_use]
    pub fn of(roles: impl IntoIterator<Item = Role>) -> Option<RoleSet> {
        let bits = roles.into_iter().fold(0, |bits, role| bits | RoleSet::bit(role));
        (bits != 0).then_some(RoleSet(bits))
    }

    /// The roles `names` names, as `role_set` reads them: unknown ones, or none, refused in
    /// today's words.
    ///
    /// # Errors
    /// The message of the application's mistake (`unknown role(s) [...]`, `no roles: ...`).
    pub fn parse<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<RoleSet, String> {
        let mut unknown: Vec<&str> = Vec::new();
        let mut found = Vec::new();
        for name in names {
            match Role::parse(name) {
                Some(role) => found.push(role),
                None => unknown.push(name),
            }
        }
        let all = Role::ALL.map(Role::as_str).join(", ");
        if !unknown.is_empty() {
            unknown.sort_unstable();
            unknown.dedup();
            let listed: Vec<String> = unknown.iter().map(|n| crate::pyfmt::repr_str(n)).collect();
            return Err(format!("unknown role(s) [{}]; the roles are {all}", listed.join(", ")));
        }
        RoleSet::of(found).ok_or_else(|| format!("no roles: give one or more of {all}, or None for all"))
    }

    /// Whether it holds `role`.
    #[must_use]
    pub fn has(self, role: Role) -> bool {
        self.0 & RoleSet::bit(role) != 0
    }

    /// Whether it may make a call that needs `needs`.
    #[must_use]
    pub fn allows(self, needs: Needs) -> bool {
        match needs {
            Needs::Any => true,
            Needs::Role(role) => self.has(role),
        }
    }

    /// Its roles, in today's order.
    #[must_use]
    pub fn roles(self) -> Vec<Role> {
        Role::ALL.into_iter().filter(|role| self.has(*role)).collect()
    }

    /// The roles it shares with `other`; `None` when none.
    #[must_use]
    pub fn and(self, other: RoleSet) -> Option<RoleSet> {
        let bits = self.0 & other.0;
        (bits != 0).then_some(RoleSet(bits))
    }

    /// Whether every role of `other` is one of its own.
    #[must_use]
    pub fn covers(self, other: RoleSet) -> bool {
        other.0 & !self.0 == 0
    }

    /// Its roles' names joined as a reply says them (`review, edit`).
    #[must_use]
    pub fn listed(self) -> String {
        self.roles().iter().map(|r| r.as_str()).collect::<Vec<_>>().join(", ")
    }
}

/// A role a call's arguments settle where its action does not (a comment reply that also
/// resolves its thread needs `resolve`); `None`: its action's.
pub type CallRole = fn(&Map<String, Value>) -> Option<Needs>;

/// Whether a call of a changing tool only reads (`word_changes`' `list`).
pub type ReadsCall = fn(&Map<String, Value>) -> bool;

/// What a host holds of a tool beside its spec (ooxml-edit's `Tool` fields that are not the
/// definition): every field optional; the default is a tool without actions of other roles.
#[derive(Debug, Clone, Default)]
pub struct HostFacts {
    /// For a tool with an `action` argument: the role each action needs where it is not the
    /// tool's (`{"list": review}`).
    pub action_roles: Vec<(String, Needs)>,
    /// The role a call's arguments settle where its action does not.
    pub call_role: Option<CallRole>,
    /// The calls of a changing tool that only read.
    pub reads: Option<ReadsCall>,
    /// A reviewer's main tool: in the core of a role set without `edit`.
    pub review_core: bool,
    /// The optional fields that apply only to some actions (`ids`: accept, reject, preview).
    pub action_fields: Vec<(String, Vec<String>)>,
}

impl HostFacts {
    /// The role `action` needs here, where it names one of its own.
    #[must_use]
    pub fn action_role(&self, action: &str) -> Option<Needs> {
        self.action_roles
            .iter()
            .find(|(a, _)| a == action)
            .map(|(_, needs)| *needs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_set_keeps_todays_order_and_refuses_what_is_no_role() {
        let set = RoleSet::parse(["edit", "review"]).unwrap_or_default();
        assert_eq!(set.roles(), vec![Role::Review, Role::Edit]);
        assert_eq!(set.listed(), "review, edit");
        assert_eq!(
            RoleSet::parse(["author", "edit", "author"]),
            Err("unknown role(s) ['author']; the roles are review, resolve, accept, edit".to_owned())
        );
        assert_eq!(
            RoleSet::parse([]),
            Err("no roles: give one or more of review, resolve, accept, edit, or None for all".to_owned())
        );
        assert!(set.allows(Needs::Any) && !set.allows(Needs::Role(Role::Accept)));
    }
}
