use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::PluginAction;
use crate::context::WorkspaceContext;
use crate::error::{CliError, Result};
use crate::output::Outputter;

pub(crate) fn plugins_bin_dir() -> PathBuf {
    let home = std::env::var("VED_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".ved")))
        .unwrap_or_else(|_| PathBuf::from(".ved"));
    home.join("plugins").join("bin")
}

pub(crate) fn handle_plugin(action: PluginAction, output: &dyn Outputter) -> Result<()> {
    match action {
        PluginAction::Install { name, path } => {
            let bin_dir = plugins_bin_dir();
            fs::create_dir_all(&bin_dir)?;

            if let Some(src_path) = path {
                if !src_path.exists() {
                    return Err(CliError::Message(format!(
                        "Local plugin binary not found: {}",
                        src_path.display()
                    )));
                }

                // If installing local path for kit or mirror or custom name
                if name == "kit" {
                    link_or_copy(&src_path, &bin_dir.join("ved-pki"))?;
                    link_or_copy(&src_path, &bin_dir.join("ved-crypt"))?;
                    link_or_copy(&src_path, &bin_dir.join("ved-crypto"))?;
                    output.println(&format!(
                        "ok: installed plugin 'kit' (ved-pki, ved-crypt) from {}",
                        src_path.display()
                    ));
                } else if name == "mirror" {
                    link_or_copy(&src_path, &bin_dir.join("ved-mirror"))?;
                    output.println(&format!(
                        "ok: installed plugin 'mirror' (ved-mirror) from {}",
                        src_path.display()
                    ));
                } else {
                    let dest = bin_dir.join(format!("ved-{name}"));
                    link_or_copy(&src_path, &dest)?;
                    output.println(&format!(
                        "ok: installed plugin '{}' ({}) from {}",
                        name,
                        dest.file_name().unwrap().to_string_lossy(),
                        src_path.display()
                    ));
                }
                return Ok(());
            }

            // Remote / standard installation via cargo or release download
            if name == "kit" {
                output.println("plugin: installing 'ved-kit' from crates.io/repo...");
                let status = Command::new("cargo")
                    .arg("install")
                    .arg("--git")
                    .arg("https://github.com/velos-io/ved-kit")
                    .arg("--root")
                    .arg(bin_dir.parent().unwrap())
                    .status();

                let kit_bin = bin_dir.join("ved-kit");
                if status.is_ok() && kit_bin.exists() {
                    link_or_copy(&kit_bin, &bin_dir.join("ved-pki"))?;
                    link_or_copy(&kit_bin, &bin_dir.join("ved-crypt"))?;
                    link_or_copy(&kit_bin, &bin_dir.join("ved-crypto"))?;
                    output.println("ok: installed plugin 'kit' (ved-pki, ved-crypt)");
                } else {
                    return Err(CliError::Message(
                        "Failed to install ved-kit via cargo. Build or specify --path".into(),
                    ));
                }
            } else if name == "mirror" {
                output.println("plugin: installing 'ved-mirror' from crates.io/repo...");
                let status = Command::new("cargo")
                    .arg("install")
                    .arg("--git")
                    .arg("https://github.com/velos-io/ved-mirror")
                    .arg("--root")
                    .arg(bin_dir.parent().unwrap())
                    .status();

                let mirror_bin = bin_dir.join("ved-mirror");
                if status.is_ok() && mirror_bin.exists() {
                    output.println("ok: installed plugin 'mirror' (ved-mirror)");
                } else {
                    return Err(CliError::Message(
                        "Failed to install ved-mirror via cargo. Build or specify --path".into(),
                    ));
                }
            } else {
                return Err(CliError::Message(format!(
                    "Unknown plugin shorthand '{name}'. Use --path <bin> or 'kit' / 'mirror'."
                )));
            }
            Ok(())
        }
        PluginAction::List => {
            let bin_dir = plugins_bin_dir();
            output.println(&format!("Installed plugins in {}:", bin_dir.display()));
            if !bin_dir.exists() {
                output.println("  (no plugins installed)");
                return Ok(());
            }

            let mut entries = Vec::new();
            for entry in fs::read_dir(&bin_dir)? {
                let entry = entry?;
                let file_name = entry.file_name().to_string_lossy().to_string();
                if let Some(cmd) = file_name.strip_prefix("ved-") {
                    entries.push(cmd.to_string());
                }
            }
            if entries.is_empty() {
                output.println("  (no plugins installed)");
            } else {
                entries.sort();
                for cmd in entries {
                    output.println(&format!("  - ved {cmd}"));
                }
            }
            Ok(())
        }
        PluginAction::Remove { name } => {
            let bin_dir = plugins_bin_dir();
            let mut removed = false;
            let target_names = if name == "kit" {
                vec!["ved-pki", "ved-crypt", "ved-crypto", "ved-kit"]
            } else if name == "mirror" {
                vec!["ved-mirror"]
            } else {
                vec![name.as_str()]
            };

            for t in target_names {
                let p = bin_dir.join(t);
                if p.exists() {
                    let _ = fs::remove_file(p);
                    removed = true;
                }
            }
            if removed {
                output.println(&format!("ok: removed plugin '{name}'"));
            } else {
                output.println(&format!("plugin '{name}' was not installed"));
            }
            Ok(())
        }
    }
}

fn find_in_path(bin_name: &str) -> Option<PathBuf> {
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(bin_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub(crate) fn handle_external(
    args: Vec<OsString>,
    ctx: &WorkspaceContext,
    _output: &dyn Outputter,
) -> Result<()> {
    if args.is_empty() {
        return Err(CliError::Message("No external command specified".into()));
    }

    let cmd_name = args[0].to_string_lossy();
    let bin_name = format!("ved-{cmd_name}");

    // 1. Search ~/.ved/plugins/bin first
    let plugin_bin = plugins_bin_dir().join(&bin_name);
    let resolved_executable = if plugin_bin.is_file() {
        plugin_bin
    } else {
        // 2. Search PATH
        match find_in_path(&bin_name) {
            Some(p) => p,
            None => {
                let hint = if cmd_name == "pki" || cmd_name == "crypt" || cmd_name == "crypto" {
                    "\nhint: install via 'ved plugin install kit'"
                } else if cmd_name == "mirror" {
                    "\nhint: install via 'ved plugin install mirror'"
                } else {
                    ""
                };
                return Err(CliError::Message(format!(
                    "Unknown command 'ved {cmd_name}'. No executable '{bin_name}' found in {} or on PATH.{hint}",
                    plugins_bin_dir().display()
                )));
            }
        }
    };

    // Forward execution
    let mut command = Command::new(resolved_executable);
    if args.len() > 1 {
        command.args(&args[1..]);
    }

    // Set contextual environment variables
    command.env("VED_WORKSPACE_ROOT", ctx.root());
    command.env("VED_CONFIG_DIR", ctx.config_dir());
    command.env("VED_WORK_DIR", ctx.work_dir());
    if let Some(out_dir) = ctx.output_dir() {
        command.env("VED_OUTPUT_DIR", out_dir);
    }

    let status = command.status().map_err(|e| CliError::Operation {
        operation: format!("execute {bin_name}"),
        resource: ctx.root().display().to_string(),
        source: Box::new(CliError::Io(e)),
    })?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        std::process::exit(code);
    }

    Ok(())
}

fn link_or_copy(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        let _ = fs::remove_file(dst);
    }
    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink(src, dst).is_ok() {
            return Ok(());
        }
    }
    fs::copy(src, dst)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(dst)?.permissions();
        perms.set_mode(0o755);
        let _ = fs::set_permissions(dst, perms);
    }
    Ok(())
}
