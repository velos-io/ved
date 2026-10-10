use std::path::Path;

use crate::cli::SyncArgs;
use crate::context::WorkspaceContext;
use crate::error::Result;
use crate::output::Outputter;

pub(crate) fn handle_sync(
    ctx: &WorkspaceContext,
    args: SyncArgs,
    output: &dyn Outputter,
) -> Result<()> {
    let target_dir = ctx.effective_dir(&args.dir);
    run_sync_with_output(ctx, &target_dir, output)
}

pub(crate) fn run_sync_with_ctx(ctx: &WorkspaceContext, dir: &Path) -> Result<()> {
    run_sync_with_output(ctx, dir, &crate::output::TerminalOutputter)
}

pub(crate) fn run_sync_with_output(
    ctx: &WorkspaceContext,
    dir: &Path,
    output: &dyn Outputter,
) -> Result<()> {
    output.println(&format!("sync: scanning workspace at {}...", dir.display()));
    let state = ved_core::scan_workspace(dir);
    let ved_dir = &ctx.paths.work_dir;
    ved_core::save_workspace_state(&state, ved_dir)?;

    output.println(&format!(
        "── Detected Modules ({}) ───────────────────────────────",
        state.modules.len()
    ));
    if state.modules.is_empty() {
        output.println("  (no modules detected; standard container stack)");
    } else {
        for m in &state.modules {
            let kinds: Vec<String> = m.kinds.iter().map(|k| format!("{k:?}")).collect();
            output.println(&format!(
                "  {:<16} path={:<12} kinds={:<18} build=\"{}\"",
                m.name,
                m.path,
                kinds.join(","),
                m.build_cmd
            ));
        }
    }
    let stacks_dir = crate::vcs::resolve_host_stacks_dir(ctx.core());
    if stacks_dir.exists() && stacks_dir.join(".git").exists() {
        output.println(&format!(
            "sync: updating stacks catalog at {}...",
            stacks_dir.display()
        ));
        if let Err(e) = crate::vcs::sync_stacks_catalog(ctx.core(), ctx.cancellation_token()) {
            output.eprintln(&format!(
                "sync: warning: failed to update stacks catalog: {e}"
            ));
        }
    }

    output.println(&format!(
        "\nsync: updated {}",
        ctx.paths.state_file().display()
    ));
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn run_sync(dir: &Path) -> Result<()> {
    let ctx = WorkspaceContext::new(dir.to_path_buf())?;
    run_sync_with_ctx(&ctx, dir)
}
