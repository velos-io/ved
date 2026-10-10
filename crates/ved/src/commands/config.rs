use std::fs;
use std::path::Path;

use ved_core::{default_config_env, default_local_env};

use crate::cli::{ConfigAction, ConfigArgs};
use crate::context::WorkspaceContext;
use crate::error::Result;
use crate::output::Outputter;

pub(crate) fn handle_config(
    ctx: &WorkspaceContext,
    args: ConfigArgs,
    output: &dyn Outputter,
) -> Result<()> {
    match args.action {
        ConfigAction::Init { force } => run_config_init_with_output(ctx, force, output),
    }
}

pub(crate) fn run_config_init_with_ctx(ctx: &WorkspaceContext, force: bool) -> Result<()> {
    run_config_init_with_output(ctx, force, &crate::output::TerminalOutputter)
}

pub(crate) fn run_config_init_with_output(
    ctx: &WorkspaceContext,
    force: bool,
    output: &dyn Outputter,
) -> Result<()> {
    let config_dir = ctx.config_dir();
    fs::create_dir_all(config_dir)?;
    let env_path = config_dir.join(".env");
    let local_env_path = config_dir.join(".local.env");

    let env_is_empty = fs::read_to_string(&env_path)
        .map(|c| c.trim().is_empty())
        .unwrap_or(false);
    if !env_path.exists() || force || env_is_empty {
        fs::write(&env_path, default_config_env())?;
        output.println(&format!("config: wrote {}", env_path.display()));
    } else {
        output.println(&format!(
            "config: {} already exists (use --force to overwrite)",
            env_path.display()
        ));
    }

    let local_is_empty = fs::read_to_string(&local_env_path)
        .map(|c| c.trim().is_empty())
        .unwrap_or(false);
    if !local_env_path.exists() || force || local_is_empty {
        fs::write(&local_env_path, default_local_env())?;
        output.println(&format!("config: wrote {}", local_env_path.display()));
    } else {
        output.println(&format!(
            "config: {} already exists (use --force to overwrite)",
            local_env_path.display()
        ));
    }

    Ok(())
}

pub(crate) fn run_config_init_in(root: &Path, force: bool) -> Result<()> {
    let ctx = WorkspaceContext::new(root.to_path_buf())?;
    run_config_init_with_ctx(&ctx, force)
}

#[allow(dead_code)]
pub(crate) fn run_config_init(force: bool) -> Result<()> {
    run_config_init_in(Path::new("."), force)
}
