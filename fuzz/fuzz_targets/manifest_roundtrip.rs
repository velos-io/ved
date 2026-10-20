#![no_main]
use libfuzzer_sys::fuzz_target;
use ved_core::{ManifestDocBuilder, YamlNode};
fuzz_target!(|text: &str| {
    let yaml = ManifestDocBuilder::new("v1", "ConfigMap", "sample")
        .with_field(
            "data",
            YamlNode::mapping(vec![("value", YamlNode::scalar(text))]),
        )
        .render()
        .unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(parsed["data"]["value"].as_str(), Some(text));
    let again: serde_yaml::Value =
        serde_yaml::from_str(&serde_yaml::to_string(&parsed).unwrap()).unwrap();
    assert_eq!(parsed, again);
});
