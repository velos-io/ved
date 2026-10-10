use crate::cli::BuildArgs;
use crate::context::WorkspaceContext;
use crate::error::Result;
use crate::output::{CoreOutputAdapter, Outputter};

pub(crate) fn handle_build(
    ctx: &WorkspaceContext,
    args: BuildArgs,
    output: &dyn Outputter,
) -> Result<()> {
    let limit = args
        .concurrency
        .unwrap_or_else(ved_core::build::default_concurrency);
    let mut options = ved_core::build::BuildOptions::new()
        .concurrency(limit.max(1))?
        .cancellation_token(ctx.cancellation_token());
    if let Some(module) = args.module {
        options = options.module(module);
    }
    let sink = CoreOutputAdapter(output);
    ved_core::build::build_workspace(ctx.core(), ctx.root(), &options, &sink)?;
    Ok(())
}
