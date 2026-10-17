//! YAML load for manifests. Merge keys are applied; duplicate keys are errors.

use serde_json::Value;
use serde_saphyr::{DuplicateKeyPolicy, MergeKeyPolicy, Options, from_str_with_options};

use super::error::{ManifestCode, ManifestError};

/// Parse one YAML document into JSON, applying merge keys.
///
/// # Errors
///
/// Returns [`ManifestError`] with code `manifest_parse_error` when the text
/// is not a single YAML document.
pub fn load_yaml(text: &str, source_path: &str) -> Result<Value, ManifestError> {
    let mut options = Options::default();
    options.duplicate_keys = DuplicateKeyPolicy::Error;
    options.merge_keys = MergeKeyPolicy::Merge;
    if let Some(budget) = options.budget.as_mut() {
        budget.max_documents = 1;
        budget.max_depth = 64;
        budget.max_anchors = 64;
        budget.max_aliases = 64;
    }
    from_str_with_options(text, options).map_err(|err| {
        ManifestError::new(
            ManifestCode::Parse,
            format!("manifest at {source_path} is not valid YAML: {err}"),
        )
    })
}
