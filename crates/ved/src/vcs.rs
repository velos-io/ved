use std::path::PathBuf;
pub use ved_core::HostGitRunner;
use ved_process::CancellationToken;

/// Locate stacks directory for the host workspace.
pub fn resolve_host_stacks_dir(context: &ved_core::WorkspaceContext) -> PathBuf {
    ved_core::resolve_stacks_dir_with_context(context)
}

/// Synchronize the stacks catalog using the host git runner.
pub fn sync_stacks_catalog(
    context: &ved_core::WorkspaceContext,
    token: CancellationToken,
) -> ved_core::Result<()> {
    let runner = HostGitRunner::new(token);
    ved_core::sync_stacks_catalog_with_context_and_driver(context, &runner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_git_runner_constructs_with_token() {
        let token = CancellationToken::new();
        let runner = HostGitRunner::new(token.clone());
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
        let _ = runner;
    }
}
