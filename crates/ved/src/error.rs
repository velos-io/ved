use crate::process::ProcessError;
use std::path::{Path, PathBuf};
use ved_core::VedError;

pub type Result<T, E = CliError> = std::result::Result<T, E>;

#[derive(thiserror::Error)]
pub enum CliError {
    #[error("Specification file not found: {}", _0.display())]
    SpecNotFound(PathBuf),

    #[error("Invalid {key}: '{value}'. Valid values are {expected}")]
    InvalidTarget {
        key: String,
        value: String,
        expected: &'static str,
    },

    #[error("Specified compose file does not exist: {}", _0.display())]
    ComposeFileNotFound(PathBuf),

    #[error("No docker-compose file found in {} (checked docker-compose.yml, docker-compose.yaml, compose.yml, compose.yaml)", _0.display())]
    ComposeDiscoveryFailed(PathBuf),

    #[error("Validation failed for {}: {cause}", path.display())]
    ValidationFailed {
        path: PathBuf,
        #[source]
        cause: Box<VedError>,
    },

    #[error("config directory not found: {}", _0.display())]
    ConfigDirNotFound(PathBuf),

    #[error("Permission denied: {kind} directory {} is not writable", path.display())]
    DirNotWritable { kind: &'static str, path: PathBuf },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Process(#[from] ProcessError),

    #[error("{0}")]
    Core(#[from] VedError),

    #[error("{operation} {resource}: {source}")]
    Operation {
        operation: String,
        resource: String,
        #[source]
        source: Box<CliError>,
    },

    #[error("operations failed ({failures_len}) after completing [{completed_summary}]: {}", format_failures(.failures), failures_len = failures.len(), completed_summary = completed.join(", "))]
    Aggregate {
        completed: Vec<String>,
        failures: Vec<CliError>,
    },
}

// Retained parser/process sources may carry secrets; Debug uses the safe display.
impl std::fmt::Debug for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "CliError({self})")
    }
}
fn format_failures(failures: &[CliError]) -> String {
    failures
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

pub trait IntoValidationCause {
    fn into_validation_cause(self, path: &Path) -> VedError;
}

impl IntoValidationCause for VedError {
    fn into_validation_cause(self, _path: &Path) -> VedError {
        self
    }
}

impl IntoValidationCause for ved_core::ValidationError {
    fn into_validation_cause(self, _path: &Path) -> VedError {
        VedError::from(self)
    }
}

impl IntoValidationCause for ved_core::SpecLoadError {
    fn into_validation_cause(self, _path: &Path) -> VedError {
        VedError::from(self)
    }
}

impl IntoValidationCause for serde_json::Error {
    fn into_validation_cause(self, _path: &Path) -> VedError {
        VedError::Json(self)
    }
}

impl IntoValidationCause for serde_yaml::Error {
    fn into_validation_cause(self, _path: &Path) -> VedError {
        VedError::Yaml(self)
    }
}

impl IntoValidationCause for std::io::Error {
    fn into_validation_cause(self, _path: &Path) -> VedError {
        VedError::Io(self)
    }
}

impl IntoValidationCause for &str {
    fn into_validation_cause(self, path: &Path) -> VedError {
        VedError::spec_parse(path, self)
    }
}

impl IntoValidationCause for String {
    fn into_validation_cause(self, path: &Path) -> VedError {
        VedError::spec_parse(path, self)
    }
}

impl From<ved_core::WorkspaceResolutionError> for CliError {
    fn from(err: ved_core::WorkspaceResolutionError) -> Self {
        CliError::Core(VedError::from(err))
    }
}

impl CliError {
    pub fn with_context(self, operation: impl Into<String>, resource: impl Into<String>) -> Self {
        Self::Operation {
            operation: operation.into(),
            resource: resource.into(),
            source: Box::new(self),
        }
    }

    pub fn aggregate(completed: Vec<String>, failures: Vec<CliError>) -> Self {
        Self::Aggregate {
            completed,
            failures,
        }
    }

    pub fn spec_not_found<P: AsRef<Path>>(path: P) -> Self {
        Self::SpecNotFound(path.as_ref().to_path_buf())
    }

    pub fn invalid_target<K: Into<String>, V: Into<String>>(
        key: K,
        value: V,
        expected: &'static str,
    ) -> Self {
        Self::InvalidTarget {
            key: key.into(),
            value: value.into(),
            expected,
        }
    }

    pub fn compose_file_not_found<P: AsRef<Path>>(path: P) -> Self {
        Self::ComposeFileNotFound(path.as_ref().to_path_buf())
    }

    pub fn compose_discovery_failed<P: AsRef<Path>>(dir: P) -> Self {
        Self::ComposeDiscoveryFailed(dir.as_ref().to_path_buf())
    }

    pub fn validation_failed<P: AsRef<Path>, C: IntoValidationCause>(path: P, cause: C) -> Self {
        let p = path.as_ref();
        Self::ValidationFailed {
            path: p.to_path_buf(),
            cause: Box::new(cause.into_validation_cause(p)),
        }
    }

    pub fn dir_not_writable<P: AsRef<Path>>(kind: &'static str, path: P) -> Self {
        Self::DirNotWritable {
            kind,
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::SpecNotFound(_)
            | CliError::InvalidTarget { .. }
            | CliError::ComposeFileNotFound(_)
            | CliError::ComposeDiscoveryFailed(_)
            | CliError::ValidationFailed { .. } => 2,
            CliError::Core(error) => {
                if error.is_validation_error() {
                    2
                } else {
                    1
                }
            }
            CliError::Operation { source, .. } => source.exit_code(),
            CliError::Aggregate { failures, .. } => {
                if !failures.is_empty() && failures.iter().all(|error| error.exit_code() == 2) {
                    2
                } else {
                    1
                }
            }
            CliError::ConfigDirNotFound(_)
            | CliError::DirNotWritable { .. }
            | CliError::Io(_)
            | CliError::Process(_) => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_formatting() {
        let err1 = CliError::spec_not_found("test/Vedfile");
        assert_eq!(
            err1.to_string(),
            "Specification file not found: test/Vedfile"
        );

        let err2 = CliError::invalid_target("TARGET", "unknown", "'compose', 'k8s'");
        assert_eq!(
            err2.to_string(),
            "Invalid TARGET: 'unknown'. Valid values are 'compose', 'k8s'"
        );

        let err3 = CliError::compose_file_not_found("custom-compose.yml");
        assert_eq!(
            err3.to_string(),
            "Specified compose file does not exist: custom-compose.yml"
        );

        let err4 = CliError::compose_discovery_failed("/path/to/project");
        assert_eq!(
            err4.to_string(),
            "No docker-compose file found in /path/to/project (checked docker-compose.yml, docker-compose.yaml, compose.yml, compose.yaml)"
        );

        let err5 = CliError::validation_failed("Vedfile", "syntax error at line 5");
        assert_eq!(
            err5.to_string(),
            "Validation failed for Vedfile: Specification parse error in 'Vedfile': syntax error at line 5"
        );

        let err6 = CliError::ConfigDirNotFound(PathBuf::from("/missing/config"));
        assert_eq!(
            err6.to_string(),
            "config directory not found: /missing/config"
        );

        let err7 = CliError::dir_not_writable("work", PathBuf::from("/ro/work"));
        assert_eq!(
            err7.to_string(),
            "Permission denied: work directory /ro/work is not writable"
        );
    }

    #[test]
    fn test_error_exit_codes() {
        assert_eq!(
            CliError::ConfigDirNotFound(PathBuf::from("/c")).exit_code(),
            1
        );
        assert_eq!(
            CliError::dir_not_writable("output", PathBuf::from("/o")).exit_code(),
            1
        );
        assert_eq!(CliError::spec_not_found("Vedfile").exit_code(), 2);
        assert_eq!(
            CliError::invalid_target("TARGET", "bad", "'compose'").exit_code(),
            2
        );
        assert_eq!(CliError::compose_file_not_found("c.yml").exit_code(), 2);
        assert_eq!(CliError::compose_discovery_failed(".").exit_code(), 2);
        assert_eq!(CliError::validation_failed("v.yml", "err").exit_code(), 2);
        assert_eq!(
            CliError::Core(VedError::spec_parse("f", "msg")).exit_code(),
            2
        );
        assert_eq!(
            CliError::Core(VedError::shorthand_syntax(1, "msg")).exit_code(),
            2
        );
        assert_eq!(CliError::Core(VedError::config("bad cfg")).exit_code(), 2);

        assert_eq!(
            CliError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "not found"
            ))
            .exit_code(),
            1
        );
        assert_eq!(
            CliError::Process(ProcessError::ExecutableNotFound("dock".into())).exit_code(),
            1
        );
        assert_eq!(
            CliError::Core(VedError::build("svc", "compile error")).exit_code(),
            1
        );
        assert_eq!(
            CliError::Core(VedError::stacks_sync("pull", "/path", "failed")).exit_code(),
            1
        );
        assert_eq!(
            CliError::Core(VedError::stack("web", "runtime error")).exit_code(),
            1
        );
    }

    #[test]
    fn test_from_conversions() {
        let io_err: CliError =
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied").into();
        assert!(matches!(io_err, CliError::Io(_)));

        let proc_err: CliError = ProcessError::ExecutableNotFound("kubectl".into()).into();
        assert!(matches!(proc_err, CliError::Process(_)));

        let core_err: CliError = VedError::config("missing prop").into();
        assert!(matches!(core_err, CliError::Core(_)));
    }

    #[test]
    fn aggregate_retains_completed_namespaces_and_structured_failures() {
        let failure = CliError::Process(ProcessError::ExecutionFailed {
            exit_code: Some(1),
            stderr: "secret credential from api server".into(),
            command: "kubectl".into(),
        })
        .with_context("delete namespace", "ns-failed");
        let aggregate = CliError::aggregate(vec!["ns-completed".into()], vec![failure]);

        match &aggregate {
            CliError::Aggregate {
                completed,
                failures,
            } => {
                assert_eq!(completed, &["ns-completed"]);
                assert_eq!(failures.len(), 1);
                assert!(matches!(
                    &failures[0],
                    CliError::Operation { operation, resource, source }
                        if operation == "delete namespace"
                            && resource == "ns-failed"
                            && matches!(source.as_ref(), CliError::Process(ProcessError::ExecutionFailed { .. }))
                ));
                assert!(aggregate.to_string().contains("delete namespace ns-failed"));
                assert!(!aggregate.to_string().contains("secret credential"));
            }
            other => panic!("expected aggregate error, got {other:?}"),
        }
        assert!(aggregate.to_string().contains("ns-completed"));
        assert!(aggregate.to_string().contains("ns-failed"));
        assert!(!aggregate.to_string().contains("secret credential"));
    }
    #[test]
    fn structured_validation_sources_and_exit_codes_survive_wrappers() {
        use std::error::Error;
        use ved_core::{SpecLoadPhase, ValidationError, ValidationReason};
        let reason = ValidationError::new(
            ValidationReason::InvalidPort,
            Some("tools".into()),
            Some("Service".into()),
            Some("web".into()),
            "ports[0]",
        );
        let core = VedError::spec_load("app.yaml", SpecLoadPhase::Validate, reason);
        let error = CliError::validation_failed("app.yaml", core)
            .with_context("validate", "workspace")
            .with_context("run", "invocation");
        assert_eq!(error.exit_code(), 2);
        let mut source: &(dyn Error + 'static) = &error;
        let mut found = None;
        loop {
            if let Some(reason) = source.downcast_ref::<ValidationError>() {
                found = Some(reason);
                break;
            }
            match source.source() {
                Some(next) => source = next,
                None => break,
            }
        }
        let found = found.expect("typed reason remains in standard source chain");
        assert_eq!(found.reason(), ValidationReason::InvalidPort);
        assert_eq!(found.profile(), Some("tools"));
        assert_eq!(found.workload(), Some("web"));
        assert_eq!(found.field(), "ports[0]");
        assert!(error.to_string().contains("app.yaml"));
        let read = VedError::spec_load(
            "missing.yaml",
            SpecLoadPhase::Read,
            std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
        );
        assert_eq!(
            CliError::Core(read)
                .with_context("load", "spec")
                .exit_code(),
            2
        );
        assert_eq!(
            CliError::Io(std::io::Error::other("failed"))
                .with_context("open", "state")
                .exit_code(),
            1
        );
    }

    #[test]
    fn aggregates_derive_display_and_classification_from_stored_failures() {
        let mut error = CliError::aggregate(vec![], vec![CliError::spec_not_found("a.yaml")]);
        assert_eq!(error.exit_code(), 2);
        if let CliError::Aggregate { failures, .. } = &mut error {
            failures.push(CliError::Io(std::io::Error::other("operational-marker")));
        }
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("operations failed (2)"));
        assert!(error.to_string().contains("operational-marker"));
        assert_eq!(CliError::aggregate(vec![], vec![]).exit_code(), 1);
    }

    #[test]
    fn retained_diagnostics_do_not_leak_through_cli_formatting() {
        let raw = std::io::Error::other("SECRET-UPSTREAM-CREDENTIAL");
        let core = VedError::spec_load("safe.yaml", ved_core::SpecLoadPhase::Parse, raw);
        let error = CliError::validation_failed("safe.yaml", core).with_context("load", "spec");
        assert!(!format!("{error} {error:?}").contains("SECRET-UPSTREAM-CREDENTIAL"));
    }
}
