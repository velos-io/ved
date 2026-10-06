use std::path::Path;

/// Actionable semantic validation categories, independent of diagnostic text.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationReason {
    InvalidIdentifier,
    InvalidPort,
    InvalidCommand,
    InvalidMount,
    MalformedDependency,
    InvalidDependencyCondition,
    UnknownDependency,
    InvalidExternalHost,
    DuplicateWorkload,
    DependencyCycle,
}

/// Structured context for a rejected specification field.
#[derive(thiserror::Error)]
#[error("{context}: invalid {field}: {detail}", context = self.context())]
pub struct ValidationError {
    reason: ValidationReason,
    profile: Option<String>,
    kind: Option<String>,
    workload: Option<String>,
    field: String,
    detail: String,
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}
impl std::fmt::Debug for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidationError")
            .field("reason", &self.reason)
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}
impl ValidationError {
    pub fn new(
        reason: ValidationReason,
        profile: Option<String>,
        kind: Option<String>,
        workload: Option<String>,
        field: impl Into<String>,
    ) -> Self {
        Self {
            reason,
            profile,
            kind,
            workload,
            field: field.into(),
            detail: format!("{reason:?}"),
            source: None,
        }
    }
    /// Supply a safe explanation without raw configuration values.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }
    pub fn reason(&self) -> ValidationReason {
        self.reason
    }
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }
    pub fn workload_kind(&self) -> Option<&str> {
        self.kind.as_deref()
    }
    pub fn workload(&self) -> Option<&str> {
        self.workload.as_deref()
    }
    pub fn field(&self) -> &str {
        &self.field
    }
    fn context(&self) -> String {
        let workload = match (&self.kind, &self.workload) {
            (Some(k), Some(n)) => format!("{k} '{n}'"),
            _ => "Specification".into(),
        };
        match &self.profile {
            Some(p) => format!("Profile '{p}' {workload}"),
            None => workload,
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecLoadPhase {
    Read,
    Parse,
    Validate,
    Evaluate,
    Convert,
}

#[derive(Debug, thiserror::Error)]
pub enum StarlarkError {
    #[error("Starlark result binding '{0}' was not defined")]
    MissingResult(String),

    #[error("{0}")]
    Evaluation(String),
}

impl StarlarkError {
    pub fn evaluation(msg: impl std::fmt::Display) -> Self {
        Self::Evaluation(msg.to_string())
    }
}

/// File context with a retained, potentially sensitive programmatic source.
/// Display and Debug intentionally omit the upstream diagnostic.
pub struct SpecLoadError {
    path: std::path::PathBuf,
    phase: SpecLoadPhase,
    source: Box<dyn std::error::Error + Send + Sync>,
}
impl SpecLoadError {
    pub fn new(
        path: impl AsRef<Path>,
        phase: SpecLoadPhase,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            phase,
            source: Box::new(source),
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn phase(&self) -> SpecLoadPhase {
        self.phase
    }
    pub fn validation_error(&self) -> Option<&ValidationError> {
        self.source.downcast_ref::<ValidationError>().or_else(|| {
            self.source
                .downcast_ref::<VedError>()
                .and_then(VedError::validation_error)
        })
    }
}
impl std::fmt::Display for SpecLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Specification {:?} failed in '{}'",
            self.phase,
            self.path.display()
        )?;
        let validation = self.source.downcast_ref::<ValidationError>().or_else(|| {
            self.source
                .downcast_ref::<VedError>()
                .and_then(VedError::validation_error)
        });
        if let Some(validation) = validation {
            write!(f, ": {validation}")?;
        }
        Ok(())
    }
}
impl std::fmt::Debug for SpecLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for SpecLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(thiserror::Error)]
pub enum VedError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML serialization or parsing error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON serialization or parsing error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Specification parse error in '{file}': {message}")]
    SpecParse { file: String, message: String },

    #[error("Shorthand syntax error at line {line}: {message}")]
    ShorthandSyntax { line: usize, message: String },

    #[error("Starlark evaluation error: {0}")]
    Starlark(String),

    #[error("Manifest emission error: {0}")]
    Emission(String),

    #[error("Build error in module '{module}': {message}")]
    Build { module: String, message: String },

    #[error("Failed to build {count} module(s):\n{}", format_build_failures(.failures))]
    BuildFailures {
        count: usize,
        failures: Vec<(String, VedError)>,
    },

    #[error("Stack error for '{stack}': {message}")]
    Stack { stack: String, message: String },

    #[error("Stacks catalog sync failed during '{operation}' for '{repo}': {details}")]
    StacksSyncError {
        operation: &'static str,
        repo: String,
        details: String,
    },

    #[cfg(feature = "host")]
    #[error("{operation} for '{resource}' failed: {source}")]
    ProcessOperation {
        operation: String,
        resource: String,
        #[source]
        source: ved_process::RunError,
    },

    #[error("Stack '{stack}' {operation} failed at '{path}': {source}{completed}")]
    StackIo {
        stack: String,
        operation: &'static str,
        path: String,
        #[source]
        source: std::io::Error,
        completed: String,
    },

    #[error(transparent)]
    Validation(Box<ValidationError>),

    #[error(transparent)]
    SpecLoad(Box<SpecLoadError>),

    #[cfg(feature = "host")]
    #[error(transparent)]
    WorkspaceResolution(Box<crate::WorkspaceResolutionError>),

    #[cfg(feature = "host")]
    #[error(transparent)]
    ResourceLimit(#[from] crate::ResourceLimitError),
}

impl From<ValidationError> for VedError {
    fn from(err: ValidationError) -> Self {
        Self::Validation(Box::new(err))
    }
}

impl From<SpecLoadError> for VedError {
    fn from(err: SpecLoadError) -> Self {
        Self::SpecLoad(Box::new(err))
    }
}

#[cfg(feature = "host")]
impl From<crate::WorkspaceResolutionError> for VedError {
    fn from(err: crate::WorkspaceResolutionError) -> Self {
        Self::WorkspaceResolution(Box::new(err))
    }
}

impl VedError {
    pub fn spec_load(
        path: impl AsRef<Path>,
        phase: SpecLoadPhase,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::SpecLoad(Box::new(SpecLoadError::new(path, phase, source)))
    }
    pub fn validation_error(&self) -> Option<&ValidationError> {
        match self {
            Self::Validation(err) => Some(err.as_ref()),
            Self::SpecLoad(err) => err.validation_error(),
            _ => None,
        }
    }
    pub fn spec_load_error(&self) -> Option<&SpecLoadError> {
        match self {
            Self::SpecLoad(err) => Some(err.as_ref()),
            _ => None,
        }
    }
    #[cfg(feature = "host")]
    pub fn resource_limit_error(&self) -> Option<&crate::ResourceLimitError> {
        match self {
            Self::ResourceLimit(err) => Some(err),
            Self::SpecLoad(err) => err
                .source
                .downcast_ref::<crate::ResourceLimitError>()
                .or_else(|| {
                    err.source
                        .downcast_ref::<VedError>()
                        .and_then(VedError::resource_limit_error)
                }),
            _ => None,
        }
    }
    pub fn is_validation_error(&self) -> bool {
        #[cfg(feature = "host")]
        if self.resource_limit_error().is_some() {
            return true;
        }
        self.validation_error().is_some()
            || self.spec_load_error().is_some()
            || matches!(
                self,
                Self::SpecParse { .. } | Self::ShorthandSyntax { .. } | Self::Config(_)
            )
    }
    /// Returns the ordered typed causes when this error aggregates module build failures.
    pub fn build_failure_sources(&self) -> Option<impl Iterator<Item = (&str, &VedError)>> {
        match self {
            Self::BuildFailures { failures, .. } => Some(
                failures
                    .iter()
                    .map(|(module, error)| (module.as_str(), error)),
            ),
            _ => None,
        }
    }
}

fn format_build_failures(failures: &[(String, VedError)]) -> String {
    failures
        .iter()
        .map(|(module, error)| format!("- [{module}]: {error}"))
        .collect::<Vec<_>>()
        .join("\n")
}
impl VedError {
    pub fn config<S: Into<String>>(msg: S) -> Self {
        Self::Config(msg.into())
    }

    pub fn emission<S: Into<String>>(msg: S) -> Self {
        Self::Emission(msg.into())
    }

    pub fn starlark<S: Into<String>>(msg: S) -> Self {
        Self::Starlark(msg.into())
    }

    pub fn spec_parse<P: AsRef<Path>, S: Into<String>>(path: P, msg: S) -> Self {
        Self::SpecParse {
            file: path.as_ref().display().to_string(),
            message: msg.into(),
        }
    }

    pub fn shorthand_syntax<S: Into<String>>(line: usize, msg: S) -> Self {
        Self::ShorthandSyntax {
            line,
            message: msg.into(),
        }
    }

    pub fn build<S1: Into<String>, S2: Into<String>>(module: S1, msg: S2) -> Self {
        Self::Build {
            module: module.into(),
            message: msg.into(),
        }
    }

    pub fn stack<S1: Into<String>, S2: Into<String>>(stack: S1, msg: S2) -> Self {
        Self::Stack {
            stack: stack.into(),
            message: msg.into(),
        }
    }

    pub fn stacks_sync<R: Into<String>, D: Into<String>>(
        operation: &'static str,
        repo: R,
        details: D,
    ) -> Self {
        Self::StacksSyncError {
            operation,
            repo: repo.into(),
            details: details.into(),
        }
    }

    pub fn stack_io<S: Into<String>, P: AsRef<Path>>(
        stack: S,
        operation: &'static str,
        path: P,
        source: std::io::Error,
        completed: Vec<String>,
    ) -> Self {
        let completed = if completed.is_empty() {
            String::new()
        } else {
            format!("; completed writes: {}", completed.join(", "))
        };
        Self::StackIo {
            stack: stack.into(),
            operation,
            path: path.as_ref().display().to_string(),
            source,
            completed,
        }
    }
}

pub type Result<T, E = VedError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ved_error_display_and_helpers() {
        let err = VedError::shorthand_syntax(42, "unexpected token");
        assert_eq!(
            err.to_string(),
            "Shorthand syntax error at line 42: unexpected token"
        );

        let err2 = VedError::spec_parse("Appfile", "file not found");
        assert_eq!(
            err2.to_string(),
            "Specification parse error in 'Appfile': file not found"
        );

        let err3 = VedError::config("missing INGRESS_SUFFIX");
        assert_eq!(
            err3.to_string(),
            "Configuration error: missing INGRESS_SUFFIX"
        );

        let err5 = VedError::stacks_sync(
            "pull",
            "/home/user/.ved/stacks",
            "fatal: unable to access: Connection timed out",
        );
        assert_eq!(
            err5.to_string(),
            "Stacks catalog sync failed during 'pull' for '/home/user/.ved/stacks': fatal: unable to access: Connection timed out"
        );
    }

    #[test]
    fn test_ved_error_pattern_matching() {
        let err = VedError::build("frontend", "exit code 1");
        match err {
            VedError::Build { module, message } => {
                assert_eq!(module, "frontend");
                assert_eq!(message, "exit code 1");
            }
            _ => panic!("Expected VedError::Build variant"),
        }

        let stack_err = VedError::stack("otel", "catalog entry missing");
        match stack_err {
            VedError::Stack { stack, message } => {
                assert_eq!(stack, "otel");
                assert_eq!(message, "catalog entry missing");
            }
            _ => panic!("Expected VedError::Stack variant"),
        }

        let sync_err = VedError::stacks_sync(
            "clone",
            "https://github.com/org/stacks.git",
            "Permission denied (publickey)",
        );
        match sync_err {
            VedError::StacksSyncError {
                operation,
                repo,
                details,
            } => {
                assert_eq!(operation, "clone");
                assert_eq!(repo, "https://github.com/org/stacks.git");
                assert_eq!(details, "Permission denied (publickey)");
            }
            _ => panic!("Expected VedError::StacksSyncError variant"),
        }
    }

    #[test]
    fn typed_spec_load_preserves_source_chain() {
        let source = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = VedError::spec_load("app.yaml", SpecLoadPhase::Read, source);
        assert_eq!(error.to_string(), "Specification Read failed in 'app.yaml'");
        let chain = std::error::Error::source(&error).expect("spec load source retained");
        assert_eq!(chain.to_string(), "file not found");
    }
}

impl std::fmt::Debug for VedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VedError({self})")
    }
}

#[cfg(test)]
mod structured_tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn typed_root_and_nested_sources_survive_context_and_redact() {
        let cause = ValidationError::new(
            ValidationReason::InvalidCommand,
            Some("tools".into()),
            Some("Job".into()),
            Some("worker".into()),
            "command",
        )
        .with_detail("expected command string or strings")
        .with_source(std::io::Error::other("secret-marker"));
        let validation = VedError::from(cause);
        assert_eq!(
            validation.validation_error().unwrap().reason(),
            ValidationReason::InvalidCommand
        );
        assert!(validation
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .is_some());
        let error = VedError::spec_load("secret.star", SpecLoadPhase::Validate, validation);
        let typed = error.validation_error().unwrap();
        assert_eq!(typed.reason(), ValidationReason::InvalidCommand);
        assert_eq!(typed.profile(), Some("tools"));
        assert_eq!(typed.field(), "command");
        assert_eq!(
            error.spec_load_error().unwrap().path(),
            Path::new("secret.star")
        );
        assert!(error.is_validation_error());
        assert!(!format!("{error} {error:?}").contains("secret-marker"));
        assert_eq!(typed.source().unwrap().to_string(), "secret-marker");
    }

    #[test]
    fn file_spec_load_wrapper_preserves_concrete_root_identity() {
        let validation = ValidationError::new(
            ValidationReason::InvalidCommand,
            None,
            Some("Service".into()),
            Some("web".into()),
            "command",
        );
        let error = VedError::spec_load("spec.yaml", SpecLoadPhase::Validate, validation);
        assert_eq!(
            error.validation_error().unwrap().reason(),
            ValidationReason::InvalidCommand
        );
        assert!(error.to_string().contains("command"));
        let error = VedError::spec_load(
            "spec.yaml",
            SpecLoadPhase::Read,
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "SECRET-IO"),
        );
        let mut source: &(dyn std::error::Error + 'static) = &error;
        let found = loop {
            if let Some(io) = source.downcast_ref::<std::io::Error>() {
                break Some(io);
            }
            match source.source() {
                Some(next) => source = next,
                None => break None,
            }
        };
        assert_eq!(found.unwrap().kind(), std::io::ErrorKind::PermissionDenied);
        assert!(!format!("{error} {error:?}").contains("SECRET-IO"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn typed_resource_limit_and_workspace_resolution_preserve_identity() {
        use crate::resources::{ResourceLimitError, ResourceLimitKind};
        let limit_err = ResourceLimitError::new(ResourceLimitKind::Ticks, "test.star", 1000);
        let err = VedError::from(limit_err);
        assert!(err.is_validation_error());
        let extracted = err
            .resource_limit_error()
            .expect("resource limit error extracted");
        assert_eq!(extracted.kind(), ResourceLimitKind::Ticks);
        assert_eq!(extracted.limit(), 1000);
    }
}
