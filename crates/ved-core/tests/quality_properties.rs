use proptest::prelude::*;
use ved_core::{AppSpec, ManifestDocBuilder, ServiceSpec, ValidatedSpec, YamlNode};

proptest! {
    #[test]
    #[cfg(feature = "host")]
    fn generated_shorthand_preserves_name_and_port(name in "[a-z][a-z0-9]{0,20}", port in 1u16..=65535) {
        let text = format!("App sample 1.0.0\nService {name} nginx {port}\n");
        let parsed = ved_core::parse_shorthand(&text).unwrap();
        let validated = ValidatedSpec::try_from(parsed).unwrap();
        prop_assert_eq!(validated.workload(None, &name).unwrap().ports()[0].container, port);
    }

    #[test]
    fn malformed_ports_are_rejected(port in 65536u32..u32::MAX) {
        let mut spec = AppSpec::default();
        spec.services.insert("web".into(), ServiceSpec {
            image: Some("nginx".into()),
            ports: vec![serde_json::json!(port.to_string())],
            ..Default::default()
        });
        prop_assert!(ValidatedSpec::try_from(spec).is_err());
    }

    #[test]
    fn validated_identifiers_cannot_escape(name in ".{0,80}") {
        let safe = name.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
        let mut spec = AppSpec::default();
        spec.services.insert(name.clone(), ServiceSpec { image: Some("nginx".into()), ..Default::default() });
        prop_assert_eq!(ValidatedSpec::try_from(spec).is_ok(), safe);
        if safe {
            prop_assert_eq!(std::path::Path::new(&name).components().count(), 1);
        }
    }

    #[test]
    fn manifest_strings_preserve_values_and_types(value in proptest::collection::vec(any::<char>(), 0..128).prop_map(|chars| chars.into_iter().collect::<String>()), count in any::<i32>(), enabled in any::<bool>()) {
        let yaml = ManifestDocBuilder::new("v1", "ConfigMap", "sample")
            .with_field("data", YamlNode::mapping(vec![("value", YamlNode::scalar(value.clone()))]))
            .with_field("count", YamlNode::integer(count))
            .with_field("enabled", YamlNode::boolean(enabled))
            .render().unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        prop_assert_eq!(parsed["data"]["value"].as_str(), Some(value.as_str()));
        prop_assert_eq!(parsed["count"].as_i64(), Some(i64::from(count)));
        prop_assert_eq!(parsed["enabled"].as_bool(), Some(enabled));
        let reparsed: serde_yaml::Value = serde_yaml::from_str(&serde_yaml::to_string(&parsed).unwrap()).unwrap();
        prop_assert_eq!(parsed, reparsed);
    }
}
