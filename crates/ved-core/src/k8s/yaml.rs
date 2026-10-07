use crate::error::Result;
use serde_yaml::{Mapping, Value};

/// Structural YAML node. Text is always literal data, never YAML source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YamlNode {
    Scalar(String),
    /// Legacy spelling for a literal string. Raw YAML fragments are not interpreted.
    Raw(String),
    Integer(i64),
    Boolean(bool),
    EmptyMapping,
    Mapping(Vec<(String, YamlNode)>),
    Sequence(Vec<YamlNode>),
}

impl YamlNode {
    pub fn scalar(s: impl Into<String>) -> Self {
        Self::Scalar(s.into())
    }
    /// Construct a literal string; this legacy method no longer inserts YAML fragments.
    pub fn raw(s: impl Into<String>) -> Self {
        Self::Raw(s.into())
    }
    pub fn integer(n: impl Into<i64>) -> Self {
        Self::Integer(n.into())
    }
    pub fn boolean(b: bool) -> Self {
        Self::Boolean(b)
    }
    pub fn mapping(pairs: Vec<(impl Into<String>, YamlNode)>) -> Self {
        Self::Mapping(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
    pub fn sequence(items: Vec<YamlNode>) -> Self {
        Self::Sequence(items)
    }

    fn to_value(&self) -> Result<Value> {
        self.to_value_at("$")
    }

    fn to_value_at(&self, path: &str) -> Result<Value> {
        Ok(match self {
            Self::Scalar(s) | Self::Raw(s) => Value::String(s.clone()),
            Self::Integer(n) => Value::Number((*n).into()),
            Self::Boolean(b) => Value::Bool(*b),
            Self::EmptyMapping => Value::Mapping(Mapping::new()),
            Self::Mapping(entries) => {
                let mut mapping = Mapping::new();
                for (key, value) in entries {
                    let yaml_key = Value::String(key.clone());
                    if mapping.contains_key(&yaml_key) {
                        return Err(crate::error::VedError::emission(format!(
                            "Duplicate YAML mapping key {key:?} at {path}"
                        )));
                    }
                    mapping.insert(yaml_key, value.to_value_at(&format!("{path}[{key:?}]"))?);
                }
                Value::Mapping(mapping)
            }
            Self::Sequence(items) => Value::Sequence(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| item.to_value_at(&format!("{path}[{index}]")))
                    .collect::<Result<Vec<_>>>()?,
            ),
        })
    }
}

/// Serialises structural documents, reporting serialization failures to the caller.
#[derive(Default)]
pub struct YamlWriter {
    buffer: String,
}

impl YamlWriter {
    pub fn new() -> Self {
        Self {
            buffer: String::with_capacity(2048),
        }
    }
    pub fn into_string(self) -> String {
        self.buffer
    }
    pub fn write_doc(&mut self, node: &YamlNode) -> Result<()> {
        // Complete serialisation before changing the stream on failure.
        let document = serde_yaml::to_string(&node.to_value()?)?;
        self.write_doc_separator();
        self.buffer.push_str(&document);
        if !self.buffer.ends_with('\n') {
            self.buffer.push('\n');
        }
        Ok(())
    }
    pub fn write_doc_separator(&mut self) {
        if !self.buffer.is_empty() && !self.buffer.ends_with("\n---\n") {
            if !self.buffer.ends_with('\n') {
                self.buffer.push('\n');
            }
            self.buffer.push_str("---\n");
        }
    }
}

/// Builder for structural Kubernetes manifest documents.
pub struct ManifestDocBuilder {
    api_version: String,
    kind: String,
    name: String,
    labels: Vec<(String, String)>,
    annotations: Vec<(String, String)>,
    extra_fields: Vec<(String, YamlNode)>,
}

impl ManifestDocBuilder {
    pub fn new(api_version: &str, kind: &str, name: &str) -> Self {
        Self {
            api_version: api_version.into(),
            kind: kind.into(),
            name: name.into(),
            labels: Vec::new(),
            annotations: Vec::new(),
            extra_fields: Vec::new(),
        }
    }
    pub fn with_label(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.labels.push((key.into(), val.into()));
        self
    }
    pub fn with_annotation(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.annotations.push((key.into(), val.into()));
        self
    }
    pub fn with_field(mut self, key: impl Into<String>, node: YamlNode) -> Self {
        self.extra_fields.push((key.into(), node));
        self
    }
    pub fn to_node(self) -> YamlNode {
        let mut metadata = vec![("name".to_string(), YamlNode::scalar(self.name))];
        for (key, entries) in [("labels", self.labels), ("annotations", self.annotations)] {
            if !entries.is_empty() {
                metadata.push((
                    key.into(),
                    YamlNode::Mapping(
                        entries
                            .into_iter()
                            .map(|(k, v)| (k, YamlNode::scalar(v)))
                            .collect(),
                    ),
                ));
            }
        }
        let mut root = vec![
            ("apiVersion".into(), YamlNode::scalar(self.api_version)),
            ("kind".into(), YamlNode::scalar(self.kind)),
            ("metadata".into(), YamlNode::Mapping(metadata)),
        ];
        root.extend(self.extra_fields);
        YamlNode::Mapping(root)
    }
    pub fn render(self) -> Result<String> {
        let mut writer = YamlWriter::new();
        writer.write_doc(&self.to_node())?;
        Ok(writer.into_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn duplicate_nested_keys_fail_without_changing_stream() {
        let mut writer = YamlWriter::new();
        writer.write_doc(&YamlNode::scalar("previous")).unwrap();
        let previous = writer.buffer.clone();
        let invalid = YamlNode::mapping(vec![(
            "nested",
            YamlNode::sequence(vec![YamlNode::mapping(vec![
                ("same", YamlNode::scalar("first")),
                ("same", YamlNode::scalar("second")),
            ])]),
        )]);
        let error = writer.write_doc(&invalid).unwrap_err();
        assert!(matches!(error, crate::error::VedError::Emission(_)));
        let message = error.to_string();
        assert!(message.contains("same"));
        assert!(message.contains("$[\"nested\"][0]"));
        assert_eq!(writer.into_string(), previous);
    }

    #[test]
    fn builder_rejects_duplicate_identity_and_label_fields() {
        for key in ["apiVersion", "kind", "metadata"] {
            let result = ManifestDocBuilder::new("v1", "Secret", "original")
                .with_field(key, YamlNode::scalar("replacement"))
                .render();
            assert!(result.unwrap_err().to_string().contains(key));
        }
        let result = ManifestDocBuilder::new("v1", "Secret", "original")
            .with_label("app", "first")
            .with_label("app", "second")
            .render();
        let error = result.unwrap_err().to_string();
        assert!(error.contains("app"));
        assert!(error.contains("$[\"metadata\"][\"labels\"]"));
        let result = ManifestDocBuilder::new("v1", "Secret", "original")
            .with_annotation("owner", "first")
            .with_annotation("owner", "second")
            .render();
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn hostile_keys_and_strings_round_trip() {
        let texts = [
            "a\"b",
            "back\\slash",
            "line\nnext",
            "key: value",
            "# hash",
            "日本語🦀",
            "true",
            "null",
            "001",
            "2026-10-10",
            "",
            "\0\t\r",
            "---\nkind: Evil",
        ];
        let node = YamlNode::Mapping(
            texts
                .iter()
                .map(|s| (s.to_string(), YamlNode::scalar(*s)))
                .collect(),
        );
        let mut writer = YamlWriter::new();
        writer.write_doc(&node).unwrap();
        let parsed: Value = serde_yaml::from_str(&writer.into_string()).unwrap();
        for text in texts {
            assert_eq!(parsed[text], Value::String(text.into()));
        }
    }

    #[test]
    fn types_and_empty_collections_are_preserved() {
        let node = YamlNode::mapping(vec![
            ("integer", YamlNode::integer(8080)),
            ("boolean", YamlNode::boolean(false)),
            ("string", YamlNode::scalar("false")),
            ("map", YamlNode::Mapping(vec![])),
            ("empty", YamlNode::EmptyMapping),
            ("sequence", YamlNode::sequence(vec![])),
        ]);
        let mut writer = YamlWriter::new();
        writer.write_doc(&node).unwrap();
        let parsed: Value = serde_yaml::from_str(&writer.into_string()).unwrap();
        assert_eq!(parsed["integer"].as_i64(), Some(8080));
        assert_eq!(parsed["boolean"].as_bool(), Some(false));
        assert_eq!(parsed["string"].as_str(), Some("false"));
        assert_eq!(parsed["map"], Value::Mapping(Mapping::new()));
        assert_eq!(parsed["empty"], Value::Mapping(Mapping::new()));
        assert_eq!(parsed["sequence"], Value::Sequence(vec![]));
    }

    #[test]
    fn raw_is_literal_and_cannot_inject_documents() {
        let input = "true\n---\nkind: Injected";
        let mut writer = YamlWriter::new();
        writer.write_doc(&YamlNode::raw(input)).unwrap();
        let output = writer.into_string();
        let docs: Vec<Value> = serde_yaml::Deserializer::from_str(&output)
            .map(|doc| Value::deserialize(doc).unwrap())
            .collect();
        assert_eq!(docs, vec![Value::String(input.into())]);
    }

    #[test]
    fn explicit_and_automatic_separators_create_exact_document_count() {
        let mut writer = YamlWriter::new();
        writer.write_doc_separator();
        writer.write_doc(&YamlNode::scalar("first")).unwrap();
        writer.write_doc_separator();
        writer.write_doc_separator();
        writer.write_doc(&YamlNode::scalar("second")).unwrap();
        writer.write_doc(&YamlNode::scalar("third")).unwrap();
        let output = writer.into_string();
        let docs: Vec<Value> = serde_yaml::Deserializer::from_str(&output)
            .map(|doc| Value::deserialize(doc).unwrap())
            .collect();
        assert_eq!(
            docs,
            vec![
                Value::String("first".into()),
                Value::String("second".into()),
                Value::String("third".into())
            ]
        );
    }
}
