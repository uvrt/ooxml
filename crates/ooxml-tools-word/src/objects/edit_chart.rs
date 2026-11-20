//! Owns: `edit_chart`'s handlers (docx-agent's `tools/shared.py`, `edit_chart`): `read` on
//! the version the call pins, every other action through the editor.
//!
//! Not here yet: every answer is `unported` -- `read`'s reply (lane P4c15's next part), the
//! data actions and the style actions (its parts 2 and 3) -- so a host that owns the
//! document answers the call its own way meanwhile.
//!
//! Must not: change the document but through a command.

use super::EditChart;
use crate::{Editing, Version};
use ooxml_base::Cancel;
use ooxml_tools::{ArgEnum, Done, ToolError, ToolResult};
use serde_json::Value;

/// `edit_chart`'s `read`: what the chart holds and how it looks, changing nothing.
///
/// # Errors
/// None yet: the reply is `unported`.
pub fn edit_chart_read(args: &EditChart, _: &Version<'_>, _: &dyn Cancel) -> Result<Done<Value>, ToolError> {
    Ok(unported(format!("Read {}", args.target), "edit_chart read"))
}

/// `edit_chart`'s actions that change the chart.
pub fn edit_chart(args: &EditChart, _: &Editing<'_>) -> ToolResult<Value> {
    let action = args.action.as_str();
    ToolResult::done(unported(
        format!("{action} on {}", args.target),
        &format!("edit_chart {action}"),
    ))
}

fn unported(summary: String, what: &str) -> Done<Value> {
    let mut done = Done::new(summary);
    done.unported.push(what.to_owned());
    done
}
