use std::path::Path;

/// Standardized output from a VCS execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcsOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

/// Errors produced while invoking a VCS driver.
#[derive(Debug, thiserror::Error)]
pub enum VcsError {
    #[error("VCS I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(feature = "host")]
    #[error("VCS process error: {0}")]
    Process(#[from] ved_process::RunError),
    #[error("VCS driver error: {0}")]
    Custom(String),
}

/// Abstract driver interface for executing version control system commands.
pub trait VcsDriver: Send + Sync {
    fn execute(
        &self,
        args: &[&str],
        cwd: Option<&Path>,
    ) -> std::result::Result<VcsOutput, VcsError>;
}

/// Dedicated host Git VCS driver using subprocess execution.
#[cfg(feature = "host")]
#[derive(Clone, Debug, Default)]
pub struct HostGitRunner {
    token: ved_process::CancellationToken,
}

#[cfg(feature = "host")]
impl HostGitRunner {
    pub fn new(token: ved_process::CancellationToken) -> Self {
        Self { token }
    }
}

#[cfg(feature = "host")]
impl VcsDriver for HostGitRunner {
    fn execute(
        &self,
        args: &[&str],
        cwd: Option<&Path>,
    ) -> std::result::Result<VcsOutput, VcsError> {
        let mut runner = ved_process::Runner::new(crate::r::bins::GIT)
            .args(args)
            .capture()
            .check_status(false)
            .cancellation_token(self.token.clone());
        if let Some(dir) = cwd {
            runner = runner.cwd(dir);
        }
        let output = runner.run().map_err(VcsError::Process)?;
        Ok(VcsOutput {
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            exit_code: output.status.code(),
        })
    }
}

/// Compatibility type alias for Git output.
pub type GitOutput = VcsOutput;

/// Compatibility type alias for Git runner error.
pub type GitRunnerError = VcsError;

/// Compatibility trait alias for Git command runners.
pub trait GitCommandRunner: VcsDriver {}
impl<T: VcsDriver + ?Sized> GitCommandRunner for T {}
