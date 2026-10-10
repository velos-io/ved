use std::fs;
use std::path::{Path, PathBuf};

use ved_core::{default_config_env, default_local_env};

use crate::cli::InitArgs;
use crate::context::WorkspaceContext;
use crate::error::{CliError, Result};
use crate::output::Outputter;

const VEW_SCRIPT: &str = include_str!("../../assets/vew");

pub(crate) fn handle_init(
    ctx: &WorkspaceContext,
    args: InitArgs,
    output: &dyn Outputter,
) -> Result<()> {
    run_init_with_output(ctx, args.compose, args.from_compose, args.project, output)
}

pub(crate) fn run_init_with_ctx(
    ctx: &WorkspaceContext,
    compose: bool,
    from_compose: Option<PathBuf>,
    project: bool,
) -> Result<()> {
    run_init_with_output(
        ctx,
        compose,
        from_compose,
        project,
        &crate::output::TerminalOutputter,
    )
}

pub(crate) fn run_init_with_output(
    ctx: &WorkspaceContext,
    compose: bool,
    from_compose: Option<PathBuf>,
    project: bool,
    output: &dyn Outputter,
) -> Result<()> {
    let dir = ctx.root();
    fs::create_dir_all(dir)?;

    let vew_path = dir.join(ved_core::r::bins::VEW);
    fs::write(&vew_path, VEW_SCRIPT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&vew_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&vew_path, perms)?;
    }
    output.println(&format!("init: wrote {}", vew_path.display()));

    let ved_dir = ctx.work_dir();
    fs::create_dir_all(ved_dir)?;

    let ved_gitignore = ved_dir.join(".gitignore");
    if !ved_gitignore.exists() {
        let gitignore_content = "bin/\ndocker-compose.yml\nstate.json\n";
        fs::write(&ved_gitignore, gitignore_content)?;
        output.println(&format!("init: wrote {}", ved_gitignore.display()));
    }

    let version_path = ved_dir.join(ved_core::r::files::VED_VERSION);
    if !version_path.exists() {
        fs::write(&version_path, concat!(env!("CARGO_PKG_VERSION"), "\n"))?;
        output.println(&format!("init: wrote {}", version_path.display()));
    }

    let legacy_version = dir.join(ved_core::r::files::VED_VERSION);
    if legacy_version.exists() && legacy_version != version_path {
        let _ = fs::remove_file(&legacy_version);
    }

    let state = ved_core::scan_workspace(dir);
    ved_core::save_workspace_state(&state, ved_dir)?;
    output.println(&format!(
        "init: cached workspace state ({} detected modules)",
        state.modules.len()
    ));

    let vedfile_path = dir.join(ved_core::r::files::VEDFILE);
    let yaml_path = dir.join(ved_core::r::files::VED_YAML);
    let target_spec_path = vedfile_path.clone();
    let app_name = dir
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "my-app".to_string());

    if compose || from_compose.is_some() {
        let compose_file = if let Some(cf) = from_compose {
            if !cf.exists() {
                return Err(CliError::compose_file_not_found(cf));
            }
            cf
        } else {
            let candidates = [
                dir.join(ved_core::r::files::COMPOSE_FILE),
                dir.join("docker-compose.yaml"),
                dir.join("compose.yml"),
                dir.join("compose.yaml"),
            ];
            candidates
                .into_iter()
                .find(|p| p.exists())
                .ok_or_else(|| CliError::compose_discovery_failed(dir))?
        };

        let starlark = ved_core::convert_compose_to_ved(&compose_file, &app_name)?;
        fs::write(&target_spec_path, starlark)?;
        output.println(&format!(
            "init: converted {} to {}",
            compose_file.display(),
            target_spec_path.display()
        ));
    } else if project {
        let (starlark, dfs) = ved_core::scaffold_from_dockerfiles(dir, &app_name)?;
        if dfs.is_empty() {
            let starter_dockerfile = dir.join("Dockerfile");
            if !starter_dockerfile.exists() {
                let df_content = "FROM alpine:latest\nWORKDIR /app\nEXPOSE 8080\nCMD [\"sh\", \"-c\", \"while true; do sleep 3600; done\"]\n";
                fs::write(&starter_dockerfile, df_content)?;
                output.println(&format!(
                    "init: wrote starter Dockerfile to {}",
                    starter_dockerfile.display()
                ));
            }
        }
        fs::write(&target_spec_path, starlark)?;
        output.println(&format!(
            "init: generated {} from Dockerfile occurrences",
            target_spec_path.display()
        ));
    } else if vedfile_path.exists() {
        let content = fs::read_to_string(&vedfile_path)?;
        let trimmed = content.trim();

        if ved_core::is_only_comments(&content) {
            let expanded = ved_core::shorthand_to_starlark(&content)?;
            fs::write(&vedfile_path, expanded)?;
            output.println(&format!(
                "init: expanded shorthand docstring in {}",
                vedfile_path.display()
            ));
        } else if trimmed.is_empty() {
            let starter_vedfile = ved_core::generate_scaffold_vedfile(&app_name, &state);
            fs::write(&vedfile_path, starter_vedfile)?;
            output.println(&format!("init: wrote {}", vedfile_path.display()));
        } else {
            output.println(&format!(
                "init: {} already exists (skipping scaffolding)",
                vedfile_path.display()
            ));
        }
    } else if yaml_path.exists() {
        output.println(&format!(
            "init: {} already exists (skipping scaffolding)",
            yaml_path.display()
        ));
    } else if dir.join("spec.ved").exists() {
        let spec_path = dir.join("spec.ved");
        let content = fs::read_to_string(&spec_path)?;
        let starter_vedfile = ved_core::shorthand_to_starlark(&content)?;
        fs::write(&target_spec_path, starter_vedfile)?;
        output.println(&format!(
            "init: wrote {} (from {})",
            target_spec_path.display(),
            spec_path.display()
        ));
    } else {
        let starter_vedfile = ved_core::generate_scaffold_vedfile(&app_name, &state);
        fs::write(&target_spec_path, starter_vedfile)?;
        output.println(&format!("init: wrote {}", target_spec_path.display()));
    }

    let config_dir = ctx.config_dir();
    fs::create_dir_all(config_dir)?;
    let env_path = config_dir.join(".env");
    let local_env_path = config_dir.join(".local.env");
    if !env_path.exists() {
        fs::write(&env_path, default_config_env())?;
        output.println(&format!("init: wrote {}", env_path.display()));
    }
    if !local_env_path.exists() {
        fs::write(&local_env_path, default_local_env())?;
        output.println(&format!("init: wrote {}", local_env_path.display()));
    }

    output.println("\nProject initialized! Run './vew up' to launch the stack.");
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn run_init(
    dir: &Path,
    compose: bool,
    from_compose: Option<PathBuf>,
    project: bool,
) -> Result<()> {
    let ctx = WorkspaceContext::new(dir.to_path_buf())?;
    run_init_with_ctx(&ctx, compose, from_compose, project)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vew_asset_integrity() {
        assert!(
            !std::hint::black_box(VEW_SCRIPT).is_empty(),
            "VEW_SCRIPT must not be empty"
        );
        assert!(
            VEW_SCRIPT.starts_with("#!/bin/sh"),
            "VEW_SCRIPT must start with #!/bin/sh"
        );
        assert!(
            VEW_SCRIPT.contains("VED_HOME"),
            "VEW_SCRIPT must contain VED_HOME resolution"
        );
    }
}
