use std::fs;
use std::path::Path;

use crate::cli::DeinitArgs;
use crate::context::WorkspaceContext;
use crate::error::Result;
use crate::output::Outputter;

pub(crate) fn handle_deinit(
    ctx: &WorkspaceContext,
    args: DeinitArgs,
    output: &dyn Outputter,
) -> Result<()> {
    let target_dir = ctx.effective_dir(&args.dir);
    run_deinit_with_output(&target_dir, args.force, output)
}

#[allow(dead_code)]
pub(crate) fn run_deinit(dir: &Path, force: bool) -> Result<()> {
    run_deinit_with_output(dir, force, &crate::output::TerminalOutputter)
}

pub(crate) fn run_deinit_with_output(
    dir: &Path,
    force: bool,
    output: &dyn Outputter,
) -> Result<()> {
    let targets = [
        ved_core::r::bins::VEW,
        ved_core::r::dirs::VED_DIR,
        ved_core::r::files::VEDFILE,
    ];
    let mut existing = Vec::new();
    for name in &targets {
        let p = dir.join(name);
        if p.exists() {
            existing.push((*name, p));
        }
    }

    if existing.is_empty() {
        output.println(&format!("deinit: no Ved files found in {}", dir.display()));
        return Ok(());
    }

    if !force {
        output.println(&format!(
            "warning: this will remove the following files and directories from {}:",
            dir.display()
        ));
        for (name, _) in &existing {
            output.println(&format!("  - {name}"));
        }
        if !crate::ui::confirm_interactive("Are you sure you want to proceed? [y/N]: ")? {
            output.println("deinit: aborted");
            return Ok(());
        }
    }

    for (_, p) in existing {
        if p.is_dir() {
            fs::remove_dir_all(&p)?;
        } else {
            fs::remove_file(&p)?;
        }
        output.println(&format!("deinit: removed {}", p.display()));
    }

    output.println("deinit: completed");
    Ok(())
}
