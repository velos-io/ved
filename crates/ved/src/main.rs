mod artifact_publisher;
mod assets;
mod cli;
mod commands;
mod context;
pub mod error;
pub(crate) mod orchestrator;
pub(crate) mod output;
pub(crate) mod paths;
mod process;
mod signals;
mod spec;
mod ui;
pub(crate) mod vcs;

use context::WorkspaceContext;
use paths::{preflight, CliDirs, DirNeeds, WorkspacePaths};
use std::path::PathBuf;

fn main() {
    let cli = cli::parse();
    let output = &output::TerminalOutputter;
    let code = match signals::InvocationSignals::new() {
        Ok(signals) => run(cli, output, signals.token()),
        Err(error) => {
            report_error(output, &error);
            1
        }
    };
    std::process::exit(code);
}

fn run(
    cli: cli::Cli,
    output: &dyn output::Outputter,
    cancellation: ved_process::CancellationToken,
) -> i32 {
    run_with_cwd(cli, output, cancellation, std::env::current_dir)
}

fn resolve_cwd(cwd: impl FnOnce() -> std::io::Result<PathBuf>) -> error::Result<PathBuf> {
    let path = cwd().map_err(|error| {
        error::CliError::from(error).with_context("resolve", "current directory")
    })?;
    if !path.is_absolute() {
        return Err(error::CliError::from(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "current directory must be absolute",
        ))
        .with_context("resolve", "current directory"));
    }
    Ok(path)
}

fn run_with_cwd(
    cli: cli::Cli,
    output: &dyn output::Outputter,
    cancellation: ved_process::CancellationToken,
    cwd: impl FnOnce() -> std::io::Result<PathBuf>,
) -> i32 {
    let cwd = match resolve_cwd(cwd) {
        Ok(cwd) => cwd,
        Err(error) => {
            report_error(output, &error);
            return error.exit_code();
        }
    };
    let dirs = CliDirs {
        dir: cli.dir,
        config_dir: cli.config_dir,
        work_dir: cli.work_dir,
        output_dir: cli.output_dir,
    };
    let paths = WorkspacePaths::resolve(&dirs, &cwd);
    if paths.config_dir_explicit {
        std::env::set_var("VED_CONFIG_DIR", &paths.config_dir);
    }

    let needs = match &cli.command {
        cli::Commands::Up(_)
        | cli::Commands::Sync(_)
        | cli::Commands::Build(_)
        | cli::Commands::Init(_)
        | cli::Commands::Config(_) => DirNeeds::ReadWrite,

        cli::Commands::Compile(_)
        | cli::Commands::Down(_)
        | cli::Commands::Ps(_)
        | cli::Commands::Test(_)
        | cli::Commands::Validate(_)
        | cli::Commands::Reset(_)
        | cli::Commands::Deinit(_) => DirNeeds::Read,

        cli::Commands::Version => DirNeeds::None,
    };

    if let Err(err) = preflight(&paths, needs) {
        report_error(output, &err);
        return err.exit_code();
    }

    let ctx = match WorkspaceContext::with_paths(paths) {
        Ok(context) => context.with_cancellation_token(cancellation),
        Err(error) => {
            report_error(output, &error);
            return error.exit_code();
        }
    };
    if let Err(err) = commands::dispatch_with_output(cli.command, &ctx, output) {
        report_error(output, &err);
        return err.exit_code();
    }
    0
}

fn report_error(output: &dyn output::Outputter, err: &dyn std::fmt::Display) {
    output.error(&err.to_string());
}

#[cfg(test)]
mod output_tests {
    use super::*;
    #[test]
    fn cwd_failure_preserves_typed_source() {
        let error = resolve_cwd(|| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "cwd denied",
            ))
        })
        .unwrap_err();
        let error::CliError::Operation { source, .. } = &error else {
            panic!("expected contextual cwd error");
        };
        let error::CliError::Io(source) = source.as_ref() else {
            panic!("expected retained I/O source");
        };
        assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("current directory"));
    }

    #[test]
    fn cwd_failure_stops_before_preflight() {
        let directory =
            std::env::temp_dir().join(format!("ved-cwd-failure-{}", std::process::id()));
        assert!(!directory.exists());
        let cli = cli::Cli {
            dir: Some(directory.clone()),
            config_dir: None,
            work_dir: None,
            output_dir: None,
            command: cli::Commands::Config(cli::ConfigArgs {
                action: cli::ConfigAction::Init { force: false },
            }),
        };
        let output = output::BufferOutputter::default();
        let code = run_with_cwd(cli, &output, ved_process::CancellationToken::new(), || {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cwd unavailable",
            ))
        });
        assert_ne!(code, 0);
        assert!(!directory.exists());
        assert!(output.stdout_lines().is_empty());
        assert_eq!(output.stderr_lines().len(), 1);
        assert!(output.stderr_lines()[0].contains("cwd unavailable"));
    }

    #[test]
    fn relative_cwd_provider_is_rejected() {
        assert!(resolve_cwd(|| Ok(PathBuf::from("."))).is_err());
    }

    #[test]
    fn top_level_error_uses_injected_stderr() {
        let output = output::BufferOutputter::default();
        report_error(&output, &"broken");
        assert_eq!(output.stderr_lines(), ["error: broken"]);
        assert!(output.stdout_lines().is_empty());
    }
}
