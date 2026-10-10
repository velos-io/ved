use crate::cli::ValidateArgs;
use crate::context::WorkspaceContext;
use crate::error::{CliError, Result};
use crate::output::Outputter;

pub(crate) fn handle_validate(
    ctx: &WorkspaceContext,
    args: ValidateArgs,
    output: &dyn Outputter,
) -> Result<()> {
    let spec_path = ctx.spec_path(args.file.as_deref());
    ved_core::load_any_spec_with_context(&spec_path, ctx.core())
        .map_err(|e| CliError::validation_failed(&spec_path, e))?;
    output.println(&format!("validate: {} is valid", spec_path.display()));
    Ok(())
}
