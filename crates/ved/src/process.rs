use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoMode {
    /// Inherits stdout/stderr for interactive CLI streaming.
    Inherit,
    /// Captures stdout/stderr into strings.
    Capture,
}

#[derive(Debug, Clone)]
pub struct ProcessOutput {
    #[allow(dead_code)]
    pub stdout: Option<String>,
    #[allow(dead_code)]
    pub stderr: Option<String>,
    #[allow(dead_code)]
    pub exit_code: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("Executable '{0}' not found in PATH")]
    ExecutableNotFound(String),
    // The command intentionally omits arguments and stderr: both can contain credentials.
    #[error("Execution failed: {command} (exit code: {exit_code:?})")]
    ExecutionFailed {
        exit_code: Option<i32>,
        stderr: String,
        command: String,
    },
    #[error("I/O error executing {0}: {1}")]
    IoError(String, #[source] std::io::Error),
    #[error("Execution of {executable} timed out after {timeout:?}")]
    TimedOut {
        executable: String,
        timeout: Duration,
    },
    #[error("Execution of {0} was cancelled")]
    Cancelled(String),
    #[error("{stream} from {executable} exceeded the {limit}-byte capture limit")]
    OutputLimit {
        executable: String,
        stream: String,
        limit: usize,
    },
    #[error("{operation} {resource}: {source}")]
    Context {
        operation: String,
        resource: String,
        #[source]
        source: Box<ProcessError>,
    },
}

impl ProcessError {
    pub fn with_context(self, operation: impl Into<String>, resource: impl Into<String>) -> Self {
        Self::Context {
            operation: operation.into(),
            resource: resource.into(),
            source: Box::new(self),
        }
    }

    pub fn is_missing_resource(&self, resource: &str) -> bool {
        match self {
            Self::ExecutionFailed { stderr, .. } => {
                let name = resource.strip_prefix("namespace ").unwrap_or(resource);
                (stderr.contains("NotFound") || stderr.contains("not found"))
                    && (stderr.contains(&format!("namespace \"{name}\" not found"))
                        || stderr.contains(&format!("namespaces \"{name}\" not found")))
            }
            Self::Context { source, .. } => source.is_missing_resource(resource),
            _ => false,
        }
    }
}

pub struct CommandBuilder {
    exe: OsString,
    args: Vec<OsString>,
    envs: HashMap<OsString, OsString>,
    cwd: Option<PathBuf>,
    stdin_data: Option<String>,
    io_mode: IoMode,
    timeout: Option<Duration>,
    cancellation: ved_process::CancellationToken,
}

impl CommandBuilder {
    pub fn new(exe: impl AsRef<OsStr>) -> Self {
        Self {
            exe: exe.as_ref().to_os_string(),
            args: Vec::new(),
            envs: HashMap::new(),
            cwd: None,
            stdin_data: None,
            io_mode: IoMode::Inherit,
            timeout: None,
            cancellation: ved_process::CancellationToken::new(),
        }
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self.args.push(arg.as_ref().to_os_string());
        }
        self
    }

    pub fn env(mut self, key: impl AsRef<OsStr>, val: impl AsRef<OsStr>) -> Self {
        self.envs
            .insert(key.as_ref().to_os_string(), val.as_ref().to_os_string());
        self
    }

    #[allow(dead_code)]
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn capture(mut self) -> Self {
        self.io_mode = IoMode::Capture;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[allow(dead_code)]
    pub fn inherit(mut self) -> Self {
        self.io_mode = IoMode::Inherit;
        self
    }

    pub fn cancellation_token(mut self, token: ved_process::CancellationToken) -> Self {
        self.cancellation = token;
        self
    }

    pub fn exec(self) -> Result<ProcessOutput, ProcessError> {
        let executable = self.exe.to_string_lossy().into_owned();
        let timeout = self.timeout.or_else(|| {
            self.envs
                .get(OsStr::new("VED_PROCESS_TIMEOUT_SECS"))
                .cloned()
                .or_else(|| std::env::var_os("VED_PROCESS_TIMEOUT_SECS"))
                .and_then(|value| value.to_string_lossy().parse::<u64>().ok())
                .filter(|seconds| *seconds > 0)
                .map(Duration::from_secs)
        });
        let mut runner = ved_process::Runner::new(&self.exe)
            .args(&self.args)
            .check_status(false)
            .cancellation_token(self.cancellation);
        if let Some(timeout) = timeout {
            runner = runner.timeout(timeout);
        }
        if let Some(cwd) = self.cwd {
            runner = runner.cwd(cwd);
        }
        for (key, value) in self.envs {
            runner = runner.env(key, value);
        }
        if let Some(input) = self.stdin_data {
            runner = runner.stdin(input.into_bytes());
        }
        if self.io_mode == IoMode::Capture {
            runner = runner.capture();
        }
        let output = runner.run().map_err(|error| match error {
            ved_process::RunError::Spawn { source, .. }
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                ProcessError::ExecutableNotFound(executable.clone())
            }
            ved_process::RunError::Spawn { source, .. }
            | ved_process::RunError::Io { source, .. } => {
                ProcessError::IoError(executable.clone(), source)
            }
            ved_process::RunError::TimedOut { timeout, .. } => ProcessError::TimedOut {
                executable: executable.clone(),
                timeout,
            },
            ved_process::RunError::Cancelled { .. } => ProcessError::Cancelled(executable.clone()),
            ved_process::RunError::OutputLimit { stream, limit, .. } => ProcessError::OutputLimit {
                executable: executable.clone(),
                stream: stream.into(),
                limit,
            },
            ved_process::RunError::Exit { status, .. } => ProcessError::ExecutionFailed {
                exit_code: status.code(),
                stderr: String::new(),
                command: executable.clone(),
            },
        })?;

        if !output.status.success() {
            return Err(ProcessError::ExecutionFailed {
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                command: executable,
            });
        }
        Ok(ProcessOutput {
            stdout: (self.io_mode == IoMode::Capture)
                .then(|| String::from_utf8_lossy(&output.stdout).into_owned()),
            stderr: (self.io_mode == IoMode::Capture)
                .then(|| String::from_utf8_lossy(&output.stderr).into_owned()),
            exit_code: output.status.code().unwrap_or(0),
        })
    }

    /// Capture child output so failures never write raw stderr to the terminal.
    /// Successful stdout is replayed; successful stderr remains suppressed.
    pub fn exec_visible(self) -> Result<ProcessOutput, ProcessError> {
        let stdout = std::io::stdout();
        self.exec_visible_to(&mut stdout.lock())
    }

    fn exec_visible_to<W: std::io::Write>(
        self,
        visible_stdout: &mut W,
    ) -> Result<ProcessOutput, ProcessError> {
        let executable = self.exe.to_string_lossy().into_owned();
        let output = self.capture().exec()?;
        if let Some(stdout) = &output.stdout {
            visible_stdout
                .write_all(stdout.as_bytes())
                .map_err(|error| ProcessError::IoError(executable.clone(), error))?;
            visible_stdout
                .flush()
                .map_err(|error| ProcessError::IoError(executable, error))?;
        }
        Ok(output)
    }
}

use crate::context::WorkspaceContext;
use std::path::Path;

pub(crate) fn default_compose_path() -> &'static str {
    ".ved/docker-compose.yml"
}

pub(crate) fn command_with_ctx(ctx: &WorkspaceContext, executable: &str) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(executable)
        .cancellation_token(ctx.cancellation_token())
        .cwd(ctx.root())
        .env("VED_OUTPUT_DIR", "");
    let child_environment = ctx.core().child_environment();
    for (key, value) in &child_environment {
        cmd = cmd.env(key, value);
    }
    if let Some(timeout) = child_environment
        .get("VED_PROCESS_TIMEOUT_SECS")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
    {
        cmd = cmd.timeout(Duration::from_secs(timeout));
    }
    cmd
}

pub(crate) fn compose_cmd_with_ctx(ctx: &WorkspaceContext) -> CommandBuilder {
    let layout = ctx.artifact_layout(
        &ctx.config()
            .get_prop(ved_core::r::keys::COMPOSE_PATH, default_compose_path()),
    );
    let mut cmd = command_with_ctx(ctx, ved_core::r::bins::DOCKER)
        .arg("compose")
        .arg("--project-directory")
        .arg(ctx.root())
        .arg("-f")
        .arg(layout.compose_file);
    let locator = ctx.config_locator();
    for path in [locator.env_file(), locator.local_env_file()] {
        if path.exists() {
            cmd = cmd.arg("--env-file").arg(path);
        }
    }
    cmd
}

#[allow(dead_code)]
pub(crate) fn compose_cmd_in(root: &Path) -> crate::error::Result<CommandBuilder> {
    Ok(compose_cmd_with_ctx(&WorkspaceContext::new(
        root.to_path_buf(),
    )?))
}
#[allow(dead_code)]
pub(crate) fn compose_cmd() -> crate::error::Result<CommandBuilder> {
    compose_cmd_in(Path::new("."))
}

impl CommandBuilder {
    pub fn stdin(mut self, input: impl Into<String>) -> Self {
        self.stdin_data = Some(input.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(unix)]
    fn fake_executable(body: &str) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ved-process-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake-kubectl");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        (dir, path)
    }

    #[cfg(unix)]
    #[test]
    fn fake_executable_success_captures_output() {
        let (dir, exe) = fake_executable("echo ok; exit 0");
        let out = CommandBuilder::new(exe).capture().exec().unwrap();
        assert_eq!(out.stdout.as_deref(), Some("ok\n"));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn command_builder_enforces_finite_configurable_deadline() {
        use std::time::{Duration, Instant};
        let (dir, exe) = fake_executable("while :; do sleep 1; done");
        let started = Instant::now();
        let result = CommandBuilder::new(exe)
            .timeout(Duration::from_millis(120))
            .exec();
        assert!(matches!(result, Err(ProcessError::TimedOut { .. })));
        assert!(started.elapsed() < Duration::from_secs(3));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn fake_executable_classifies_only_matching_not_found() {
        let (dir, exe) = fake_executable(
            "echo 'Error from server (NotFound): namespaces \"gone\" not found' >&2; exit 1",
        );
        let err = CommandBuilder::new(exe).capture().exec().unwrap_err();
        assert!(err.is_missing_resource("namespace gone"));
        assert!(!err.is_missing_resource("namespace other"));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn fake_executable_auth_connection_and_nonzero_are_failures() {
        for output in [
            "echo 'Error from server (Forbidden): forbidden' >&2; exit 1",
            "echo 'The connection to the server was refused' >&2; exit 1",
            "echo 'unrelated object not found' >&2; exit 1",
        ] {
            let (dir, exe) = fake_executable(output);
            let err = CommandBuilder::new(exe).capture().exec().unwrap_err();
            assert!(!err.is_missing_resource("namespace gone"));
            let diagnostic = err.to_string();
            assert!(!diagnostic.contains("forbidden"));
            assert!(!diagnostic.contains("connection to"));
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[cfg(unix)]
    #[test]
    fn fake_executable_diagnostics_redact_args_and_raw_stderr() {
        let (dir, exe) = fake_executable("echo 'secret-token' >&2; exit 7");
        let err = CommandBuilder::new(exe)
            .arg("password=secret-arg")
            .capture()
            .exec()
            .unwrap_err();
        let diagnostic = err.to_string();
        assert!(!diagnostic.contains("secret-token"));
        assert!(!diagnostic.contains("secret-arg"));
        assert!(matches!(
            err,
            ProcessError::ExecutionFailed {
                exit_code: Some(7),
                ..
            }
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn visible_execution_replays_success_and_suppresses_failure_stderr() {
        let (success_dir, success_exe) = fake_executable("printf 'visible success\\n'; exit 0");
        let mut visible = Vec::new();
        CommandBuilder::new(success_exe)
            .exec_visible_to(&mut visible)
            .unwrap();
        assert_eq!(visible, b"visible success\n");
        let _ = fs::remove_dir_all(success_dir);

        let (failure_dir, failure_exe) =
            fake_executable("echo 'secret stderr payload' >&2; exit 9");
        let mut visible = Vec::new();
        let err = CommandBuilder::new(failure_exe)
            .exec_visible_to(&mut visible)
            .unwrap_err();
        assert!(visible.is_empty());
        assert!(!err.to_string().contains("secret stderr payload"));
        let _ = fs::remove_dir_all(failure_dir);
    }

    #[cfg(unix)]
    #[test]
    fn missing_executable_is_detected_with_isolated_path() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ved-process-empty-path-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let err = CommandBuilder::new("kubectl")
            .env("PATH", &dir)
            .capture()
            .exec()
            .unwrap_err();
        assert!(matches!(err, ProcessError::ExecutableNotFound(_)));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_missing_executable() {
        let err = CommandBuilder::new("this-binary-should-not-exist-12345")
            .exec()
            .unwrap_err();
        match err {
            ProcessError::ExecutableNotFound(exe) => {
                assert_eq!(exe, "this-binary-should-not-exist-12345")
            }
            _ => panic!("Expected ExecutableNotFound"),
        }
    }

    #[test]
    fn test_capture_mode() {
        let res = CommandBuilder::new("echo")
            .arg("hello world")
            .capture()
            .exec()
            .unwrap();
        assert_eq!(res.exit_code, 0);
        assert_eq!(res.stdout.unwrap().trim(), "hello world");
    }

    #[test]
    fn test_failure_exit_code() {
        let err = CommandBuilder::new("sh")
            .arg("-c")
            .arg("exit 42")
            .capture()
            .exec()
            .unwrap_err();
        match err {
            ProcessError::ExecutionFailed { exit_code, .. } => assert_eq!(exit_code, Some(42)),
            _ => panic!("Expected ExecutionFailed"),
        }
    }
}
