use std::path::Path;

pub(super) fn contains_interpolation(value: &str) -> bool {
    value.contains('$')
}

pub(super) fn is_null_or_tagged_null(value: &serde_yaml::Value) -> bool {
    match value {
        serde_yaml::Value::Null => true,
        serde_yaml::Value::Tagged(value) => is_null_or_tagged_null(&value.value),
        serde_yaml::Value::Bool(_)
        | serde_yaml::Value::Number(_)
        | serde_yaml::Value::String(_)
        | serde_yaml::Value::Sequence(_)
        | serde_yaml::Value::Mapping(_) => false,
    }
}

pub(super) fn resource_is_external(
    mapping: Option<&serde_yaml::Mapping>,
    field: &str,
    name: &str,
    file: &Path,
) -> anyhow::Result<bool> {
    let Some(value) =
        mapping.and_then(|mapping| mapping.get(serde_yaml::Value::String("external".into())))
    else {
        return Ok(false);
    };
    value.as_bool().ok_or_else(|| {
        anyhow::anyhow!(
            "Compose {field} `{name}` in {} uses a non-boolean external value; use `external: true` with a top-level literal `name` if needed",
            file.display()
        )
    })
}
