use crate::error::Result;
use serde_yaml::{Mapping, Value};

#[cfg(any(feature = "host", test))]
use crate::model::AppSpec;
use crate::validated::ValidatedSpec;

#[cfg(feature = "host")]
pub fn generate_helm_values(spec: &AppSpec) -> Result<String> {
    generate_helm_values_validated(&ValidatedSpec::try_from(spec)?)
}

#[cfg(feature = "host")]
pub fn generate_helm_values_validated(validated: &ValidatedSpec) -> Result<String> {
    generate_helm_values_validated_with_context(
        validated,
        &crate::workspace::WorkspaceContext::new("."),
    )
}

/// Compile values using the resolved workspace configuration.
pub fn generate_helm_values_validated_with_context(
    validated: &ValidatedSpec,
    context: &crate::workspace::WorkspaceContext,
) -> Result<String> {
    let spec = validated.as_spec();
    let mut root = Mapping::new();
    let mut global = Mapping::new();

    global.insert(
        Value::String("appVersion".to_string()),
        Value::String(spec.version.clone()),
    );

    let mut k8s = Mapping::new();
    k8s.insert(
        Value::String("ingress_suffix".to_string()),
        Value::String(
            context
                .config()
                .resolve_ingress_suffix(spec.ingress.as_deref()),
        ),
    );
    global.insert(Value::String("kubernetes".to_string()), Value::Mapping(k8s));

    root.insert(Value::String("global".to_string()), Value::Mapping(global));

    let yaml_str = serde_yaml::to_string(&Value::Mapping(root))?;
    Ok(yaml_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helm_rejects_invalid_unused_workload_fields() {
        let mut spec = AppSpec::default();
        spec.services.insert(
            "web".into(),
            crate::model::ServiceSpec {
                command: Some(serde_json::json!(["echo", 7])),
                ..Default::default()
            },
        );
        let error = ValidatedSpec::try_from(&spec).unwrap_err();
        assert!(error.to_string().contains("command"));
    }
}
