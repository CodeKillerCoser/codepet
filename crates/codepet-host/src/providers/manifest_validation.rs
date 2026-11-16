//! The published installation Schema is the sole validation contract.
//! No field validators or historical import/load exceptions belong here.
use serde_json::Value;
use std::sync::LazyLock;

static VALIDATOR: LazyLock<Result<jsonschema::Validator, String>> = LazyLock::new(|| {
    let schema: Value = serde_json::from_str(include_str!("manifest.schema.generated.json"))
        .map_err(|_| "Invalid bundled Provider manifest Schema".to_string())?;
    jsonschema::draft202012::options().should_validate_formats(true).build(&schema)
        .map_err(|_| "Cannot compile bundled Provider manifest Schema".to_string())
});

pub(super) fn validate(value: &Value) -> Result<(), String> {
    let validator = VALIDATOR.as_ref().map_err(Clone::clone)?;
    validator.validate(value).map_err(|error| {
        // Validator Display may contain field values (including Env secrets).
        // Report only the JSON Pointer and the violated standard keyword.
        let mut path = error.instance_path().to_string();
        if let jsonschema::error::ValidationErrorKind::Required { property } = error.kind() {
            path.push('/');
            path.push_str(&property.as_str().unwrap_or_default().replace('~', "~0").replace('/', "~1"));
        }
        format!("Invalid plugin manifest: {} ({})", if path.is_empty() { "/" } else { &path }, error.kind().keyword())
    })
}
