//! Owns: a host's settings read from the application's values -- ooxml-edit's `Limits`, by
//! its field names ([`HostSettings::take_limits`]), and the roles its sessions hold
//! ([`HostSettings::take_roles`]) -- for the Python facade and the replay of recorded
//! settings alike.
//!
//! Must not: hold a default of its own (the defaults are [`HostSettings::default`]'s), or
//! take a field silently that it does not read.

use crate::session::HostSettings;
use ooxml_tools::roles::RoleSet;
use ooxml_tools::{Refusal, ToolError, TryCall, WayOut};
use serde_json::{Map, Value};
use std::time::Duration;

/// The `Limits` fields the host reads (the rest are the Python host's own: its workers,
/// caches and render deadlines).
pub const LIMIT_FIELDS: &[&str] = &[
    "max_documents",
    "max_total_document_bytes",
    "max_package_bytes",
    "max_parts",
    "max_blob_bytes",
    "max_total_blob_bytes",
    "max_image_bytes",
    "max_image_pixels",
    "max_markdown_chars",
    "max_result_chars",
    "max_list_items",
    "max_images_per_call",
    "max_image_edge",
    "image_budget",
    "image_budget_per_round",
    "max_batch_ops",
    "max_read_batch_chars",
    "max_datasets",
    "max_dataset_cells",
    "layout_timeout",
    "batch_timeout",
];

impl HostSettings {
    /// The fields of `given` (ooxml-edit's `Limits`, by its field names) the host reads,
    /// taken; those it does not read returned, in the order given.
    ///
    /// # Errors
    /// A field the host reads given a value it cannot be: its name and the value.
    pub fn take_limits(&mut self, given: &Map<String, Value>) -> Result<Vec<String>, String> {
        let mut left = Vec::new();
        for (key, value) in given {
            if !self.take_limit(key, value)? {
                left.push(key.clone());
            }
        }
        Ok(left)
    }

    /// The roles named by `value` (a list of role names) as the sessions' default.
    ///
    /// # Errors
    /// A value that is not a list of role names.
    pub fn take_roles(&mut self, value: &Value) -> Result<(), String> {
        let names: Option<Vec<&str>> = value.as_array().map(|a| a.iter().filter_map(Value::as_str).collect());
        let names = names.ok_or_else(|| format!("roles {value}: a list of role names"))?;
        self.roles = RoleSet::parse(names)?;
        Ok(())
    }

    /// The refusal of an op of `within` (`batch`, `read_batch`) naming `tool`: one of the
    /// application's own tools ([`HostSettings::own_tools`], decision D1), or of a format the
    /// application's own host holds ([`HostSettings::format_tools`]), with the op alone as the
    /// way out; `None` for any other tool.
    #[must_use]
    pub fn own_tool_refused(&self, tool: &str, within: &str, retry: &TryCall) -> Option<ToolError> {
        let message = if self.own_tools.iter().any(|t| t == tool) {
            format!("{tool} is the application's own tool, which cannot run inside a {within}: call it alone")
        } else if let Some((_, kind)) = self.format_tools.iter().find(|(t, _)| t == tool) {
            format!(
                "{tool} is a tool of {kind} documents, which another host runs, so it cannot run inside this \
                 {within}: send it as its own call or in its own {within}"
            )
        } else {
            return None;
        };
        Some(ToolError::refusal(Refusal::InvalidArguments, message, WayOut::call(retry.clone())).at("tool"))
    }

    /// One `Limits` field: whether the host reads it.
    fn take_limit(&mut self, key: &str, value: &Value) -> Result<bool, String> {
        let bad = || format!("limits.{key} {value}: not a value it takes");
        let count = || value.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(bad);
        let bytes = || value.as_u64().ok_or_else(bad);
        let optional = || if value.is_null() { Ok(None) } else { count().map(Some) };
        let seconds = || {
            value
                .as_f64()
                .filter(|s| *s >= 0.0 && s.is_finite())
                .map(Duration::from_secs_f64)
                .ok_or_else(bad)
        };
        match key {
            "max_documents" => self.documents.max_documents = count()?,
            "max_total_document_bytes" => self.documents.max_total_document_bytes = bytes()?,
            "max_package_bytes" => self.blobs.max_package_bytes = bytes()?,
            "max_parts" => self.blobs.max_parts = count()?,
            "max_blob_bytes" => self.blobs.max_blob_bytes = bytes()?,
            "max_total_blob_bytes" => self.blobs.max_total_blob_bytes = bytes()?,
            "max_image_bytes" => self.blobs.max_image_bytes = bytes()?,
            "max_image_pixels" => self.blobs.max_image_pixels = bytes()?,
            "max_markdown_chars" => self.blobs.max_markdown_chars = count()?,
            "max_result_chars" => self.limits.result_chars = count()?,
            "max_list_items" => self.limits.list_items = count()?,
            "max_images_per_call" => self.limits.images_per_call = count()?,
            "max_image_edge" => self.limits.image_edge = value.as_i64().ok_or_else(bad)?,
            "image_budget" => self.image_budget = optional()?,
            "image_budget_per_round" => self.image_budget_per_round = optional()?,
            "max_batch_ops" => self.max_batch_ops = count()?,
            "max_read_batch_chars" => self.read_batch_chars = count()?,
            "max_datasets" => self.datasets.max_datasets = count()?,
            "max_dataset_cells" => self.datasets.max_cells = count()?,
            "layout_timeout" => self.layout_timeout = Some(seconds()?),
            "batch_timeout" => self.batch_timeout = seconds()?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn takes_the_fields_it_reads_and_names_the_rest() {
        let mut settings = HostSettings::default();
        let given = json!({"max_result_chars": 3000, "image_budget": null, "render_timeout": 20.0,
                           "batch_timeout": 0.5, "max_documents": 2});
        let left = settings.take_limits(given.as_object().unwrap()).unwrap();
        assert_eq!(left, ["render_timeout"]);
        assert_eq!(settings.limits.result_chars, 3000);
        assert_eq!(settings.image_budget, None);
        assert_eq!(settings.batch_timeout, Duration::from_millis(500));
        assert_eq!(settings.documents.max_documents, 2);
        for key in LIMIT_FIELDS {
            let mut settings = HostSettings::default();
            let one = Map::from_iter([((*key).to_owned(), json!(1))]);
            assert!(settings.take_limits(&one).unwrap().is_empty(), "{key}");
        }
    }

    #[test]
    fn refuses_a_value_a_field_cannot_be() {
        let mut settings = HostSettings::default();
        let given = json!({"max_result_chars": "many"});
        let error = settings.take_limits(given.as_object().unwrap()).unwrap_err();
        assert!(error.contains("max_result_chars"), "{error}");
        assert!(settings.take_roles(&json!(["review"])).is_ok());
        assert!(settings.take_roles(&json!(["nobody"])).is_err());
    }
}
