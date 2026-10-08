use crate::error::{Result, VedError};
use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::discovery::{DetectedModule, ModuleKind};

/// Look up a build toolchain binary path from environment variables or config files.
pub fn get_build_prop(key: &str) -> Option<String> {
    crate::config::get_prop_opt(key)
}

/// Resolve the program name and arguments for a detected module toolchain.
pub fn resolve_module_command(module: &DetectedModule, module_dir: &Path) -> (String, Vec<String>) {
    let context = crate::workspace::WorkspaceContext::new(".");
    resolve_module_command_with_context(module, module_dir, &context)
}

/// Resolve toolchains from the selected workspace rather than process cwd.
pub fn resolve_module_command_with_context(
    module: &DetectedModule,
    module_dir: &Path,
    context: &crate::workspace::WorkspaceContext,
) -> (String, Vec<String>) {
    let get_build_prop = |key| {
        context.config().get_prop_opt(key).map(|binary| {
            let path = Path::new(&binary);
            if path.is_relative() && path.components().count() > 1 {
                context.root().join(path).to_string_lossy().into_owned()
            } else {
                binary
            }
        })
    };
    let primary_kind = module.kinds.first().unwrap_or(&ModuleKind::Docker);

    match primary_kind {
        ModuleKind::Gradle => {
            let gradlew = module_dir.join("gradlew");
            if gradlew.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = fs::metadata(&gradlew) {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o755);
                        let _ = fs::set_permissions(&gradlew, perms);
                    }
                }
                ("./gradlew".to_string(), vec!["build".to_string()])
            } else if let Some(bin) = get_build_prop("GRADLE_BIN") {
                (bin, vec!["build".to_string()])
            } else {
                ("gradle".to_string(), vec!["build".to_string()])
            }
        }
        ModuleKind::Maven => {
            let mvnw = module_dir.join("mvnw");
            if mvnw.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = fs::metadata(&mvnw) {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o755);
                        let _ = fs::set_permissions(&mvnw, perms);
                    }
                }
                (
                    "./mvnw".to_string(),
                    vec!["package".to_string(), "-DskipTests".to_string()],
                )
            } else if let Some(bin) = get_build_prop("MAVEN_BIN") {
                (bin, vec!["package".to_string(), "-DskipTests".to_string()])
            } else {
                (
                    "mvn".to_string(),
                    vec!["package".to_string(), "-DskipTests".to_string()],
                )
            }
        }
        ModuleKind::Node => {
            if let Some(bin) = get_build_prop("NPM_BIN") {
                (bin, vec!["run".to_string(), "build".to_string()])
            } else {
                (
                    "npm".to_string(),
                    vec!["run".to_string(), "build".to_string()],
                )
            }
        }
        ModuleKind::Go => {
            if let Some(bin) = get_build_prop("GO_BIN") {
                (bin, vec!["build".to_string(), "./...".to_string()])
            } else {
                (
                    "go".to_string(),
                    vec!["build".to_string(), "./...".to_string()],
                )
            }
        }
        ModuleKind::Rust => {
            if let Some(bin) = get_build_prop("CARGO_BIN").or_else(|| get_build_prop("RUST_BIN")) {
                (bin, vec!["build".to_string(), "--release".to_string()])
            } else {
                (
                    "cargo".to_string(),
                    vec!["build".to_string(), "--release".to_string()],
                )
            }
        }
        ModuleKind::Python => {
            let venv_py = module_dir.join(".venv/bin/python");
            if venv_py.exists() {
                (
                    venv_py.to_string_lossy().to_string(),
                    vec!["-m".to_string(), "build".to_string()],
                )
            } else if let Some(bin) = get_build_prop("PYTHON_BIN") {
                (bin, vec!["-m".to_string(), "build".to_string()])
            } else {
                (
                    "python3".to_string(),
                    vec!["-m".to_string(), "build".to_string()],
                )
            }
        }
        ModuleKind::Docker => (
            crate::r::bins::DOCKER.to_string(),
            vec![
                "build".to_string(),
                "-t".to_string(),
                module.name.clone(),
                ".".to_string(),
            ],
        ),
    }
}

use crate::output::{OutputSink, StdoutSink};

/// Execute build for a single module using the default stdout sink.
pub fn build_module(root: &Path, module: &DetectedModule) -> Result<()> {
    build_module_with_sink(root, module, &StdoutSink)
}

/// Execute build for a single module using the provided output sink.
pub fn build_module_with_sink(
    root: &Path,
    module: &DetectedModule,
    sink: &impl OutputSink,
) -> Result<()> {
    let context = crate::workspace::WorkspaceContext::new(root);
    build_module_with_context_and_sink(&context, context.root(), module, sink)
}

pub fn build_module_with_context_and_sink(
    context: &crate::workspace::WorkspaceContext,
    root: &Path,
    module: &DetectedModule,
    sink: &impl OutputSink,
) -> Result<()> {
    build_module_with_context_and_sink_and_control(
        context,
        root,
        module,
        sink,
        None,
        ved_process::CancellationToken::new(),
    )
}

pub fn build_module_with_context_and_sink_and_control(
    context: &crate::workspace::WorkspaceContext,
    root: &Path,
    module: &DetectedModule,
    sink: &impl OutputSink,
    timeout: Option<Duration>,
    cancellation: ved_process::CancellationToken,
) -> Result<()> {
    let module_dir = if module.path == "." {
        root.to_path_buf()
    } else {
        root.join(&module.path)
    };

    let (prog, args) = resolve_module_command_with_context(module, &module_dir, context);
    sink.write_line(&format!(
        "build: [{}] {} in {}",
        module.name,
        prog,
        module_dir.display()
    ));

    let child_environment = context.child_environment();
    let configured_timeout = timeout.or_else(|| {
        child_environment
            .get("VED_PROCESS_TIMEOUT_SECS")
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .or_else(|| {
                std::env::var("VED_PROCESS_TIMEOUT_SECS")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|seconds| *seconds > 0)
                    .map(Duration::from_secs)
            })
    });
    let mut runner = ved_process::Runner::new(&prog)
        .args(&args)
        .cwd(&module_dir)
        .cancellation_token(cancellation)
        .check_status(false);
    if let Some(timeout) = configured_timeout {
        runner = runner.timeout(timeout);
    }
    for (key, value) in child_environment {
        runner = runner.env(key, value);
    }
    let output = runner.run().map_err(|source| VedError::ProcessOperation {
        operation: "build".into(),
        resource: format!("module '{}' using {}", module.name, prog),
        source,
    })?;

    if !output.status.success() {
        return Err(VedError::build(
            &module.name,
            format!(
                "Build command '{}' failed with exit code {:?} for module '{}'",
                prog,
                output.status.code(),
                module.name
            ),
        ));
    }

    sink.write_line(&format!("build: [{}] done", module.name));
    Ok(())
}

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

/// Default concurrency based on logical core count.
pub fn default_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Checked workspace build configuration.
///
/// Defaults select all modules, use [`default_concurrency`], inherit configured
/// per-child deadlines and create a fresh cancellation token. Cloning options
/// shares cancellation state. A cancelled token stays cancelled when reused.
///
/// ```
/// use std::time::Duration;
/// use ved_core::build::BuildOptions;
/// # fn main() -> ved_core::Result<()> {
/// let options = BuildOptions::new()
///     .module("service-a")
///     .concurrency(2)?
///     .timeout(Duration::from_secs(30))?;
/// assert_eq!(options.selected_module(), Some("service-a"));
/// assert_eq!(options.concurrency_limit(), 2);
/// # Ok(())
/// # }
/// ```
///
/// Fields are private so callers cannot bypass checked setters:
///
/// ```compile_fail
/// use ved_core::build::BuildOptions;
/// let options = BuildOptions { concurrency: 0, ..BuildOptions::default() };
/// ```
#[derive(Clone, Debug)]
pub struct BuildOptions {
    target_module: Option<String>,
    concurrency: usize,
    timeout: Option<Duration>,
    cancellation: ved_process::CancellationToken,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            target_module: None,
            concurrency: default_concurrency(),
            timeout: None,
            cancellation: ved_process::CancellationToken::new(),
        }
    }
}

impl BuildOptions {
    /// Create default options with independent cancellation state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Select a module by name or path, using existing workspace matching rules.
    /// The selection is not trimmed; an empty or missing name reports an error.
    pub fn module(mut self, module: impl Into<String>) -> Self {
        self.target_module = Some(module.into());
        self
    }

    /// Restore selection of all detected modules.
    pub fn all_modules(mut self) -> Self {
        self.target_module = None;
        self
    }

    /// Set a positive worker limit, capped by the number of selected modules.
    /// Zero is rejected before discovery or child execution.
    pub fn concurrency(mut self, concurrency: usize) -> Result<Self> {
        if concurrency == 0 {
            return Err(VedError::Config(
                "Build concurrency must be greater than zero".into(),
            ));
        }
        self.concurrency = concurrency;
        Ok(self)
    }

    /// Set a positive deadline for each child, measured from its launch.
    /// This is not a deadline for the complete workspace build. Zero is rejected.
    pub fn timeout(mut self, timeout: Duration) -> Result<Self> {
        if timeout.is_zero() {
            return Err(VedError::Config(
                "Build timeout must be greater than zero".into(),
            ));
        }
        self.timeout = Some(timeout);
        Ok(self)
    }

    /// Restore existing timeout resolution: positive context child environment
    /// `VED_PROCESS_TIMEOUT_SECS`, positive process environment value, then the
    /// process runner's default. This does not disable child deadlines.
    pub fn configured_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }

    /// Share application-owned cancellation with active and pending builds.
    pub fn cancellation_token(mut self, cancellation: ved_process::CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Return the selected module name/path, or `None` for all modules.
    pub fn selected_module(&self) -> Option<&str> {
        self.target_module.as_deref()
    }

    /// Return the positive configured concurrency limit.
    pub fn concurrency_limit(&self) -> usize {
        self.concurrency
    }

    /// Return the explicit per-child timeout, or `None` for configured resolution.
    pub fn timeout_override(&self) -> Option<Duration> {
        self.timeout
    }

    /// Return the shared cancellation token.
    pub fn cancellation(&self) -> &ved_process::CancellationToken {
        &self.cancellation
    }

    // Preserve the 0.1 wrappers' zero-concurrency normalization and immediate
    // zero-timeout deadline without exposing invalid new builder options.
    fn legacy(
        target_module: Option<&str>,
        concurrency: usize,
        timeout: Option<Duration>,
        cancellation: ved_process::CancellationToken,
    ) -> Self {
        Self {
            target_module: target_module.map(str::to_owned),
            concurrency: concurrency.max(1),
            timeout,
            cancellation,
        }
    }
}

/// Build all detected modules or a specific module in the workspace.
pub fn build_all(root: &Path, target_module: Option<&str>) -> Result<()> {
    build_all_concurrent(root, target_module, default_concurrency())
}

/// Build all detected modules using the provided output sink.
pub fn build_all_with_sink(
    root: &Path,
    target_module: Option<&str>,
    sink: &impl OutputSink,
) -> Result<()> {
    build_all_concurrent_with_sink(root, target_module, default_concurrency(), sink)
}

/// Build detected modules concurrently up to the specified concurrency limit using stdout.
pub fn build_all_concurrent(
    root: &Path,
    target_module: Option<&str>,
    concurrency: usize,
) -> Result<()> {
    build_all_concurrent_with_sink(root, target_module, concurrency, &StdoutSink)
}

/// Build detected modules concurrently up to the specified concurrency limit using the provided output sink.
pub fn build_all_concurrent_with_sink(
    root: &Path,
    target_module: Option<&str>,
    concurrency: usize,
    sink: &impl OutputSink,
) -> Result<()> {
    let context = crate::workspace::WorkspaceContext::new(root);
    build_all_concurrent_with_context_and_sink(
        &context,
        context.root(),
        target_module,
        concurrency,
        sink,
    )
}

/// Build using resolved toolchain configuration and work state.
pub fn build_all_with_context(
    context: &crate::workspace::WorkspaceContext,
    root: &Path,
    target_module: Option<&str>,
) -> Result<()> {
    build_all_concurrent_with_context(context, root, target_module, default_concurrency())
}

pub fn build_all_concurrent_with_context(
    context: &crate::workspace::WorkspaceContext,
    root: &Path,
    target_module: Option<&str>,
    concurrency: usize,
) -> Result<()> {
    build_all_concurrent_with_context_and_sink(
        context,
        root,
        target_module,
        concurrency,
        &StdoutSink,
    )
}

pub fn build_all_concurrent_with_context_and_sink(
    context: &crate::workspace::WorkspaceContext,
    root: &Path,
    target_module: Option<&str>,
    concurrency: usize,
    sink: &impl OutputSink,
) -> Result<()> {
    build_all_concurrent_with_context_and_sink_and_control(
        context,
        root,
        target_module,
        concurrency,
        sink,
        None,
        ved_process::CancellationToken::new(),
    )
}

/// Build selected modules with an optional shared child-process deadline and cancellation token.
/// Cancellation stops scheduling pending modules and terminates active child process groups.
pub fn build_all_concurrent_with_context_and_sink_and_control(
    context: &crate::workspace::WorkspaceContext,
    root: &Path,
    target_module: Option<&str>,
    concurrency: usize,
    sink: &impl OutputSink,
    timeout: Option<Duration>,
    cancellation: ved_process::CancellationToken,
) -> Result<()> {
    let options = BuildOptions::legacy(target_module, concurrency, timeout, cancellation);
    build_workspace(context, root, &options, sink)
}

/// Build selected workspace modules using checked execution options.
///
/// Reuses cached workspace discovery when available. Failures are reported in
/// module order even when children finish out of order. Cancellation stops
/// pending work and terminates active child process groups.
///
/// ```no_run
/// use ved_core::build::{build_workspace, BuildOptions};
/// use ved_core::{StdoutSink, WorkspaceContext};
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let context = WorkspaceContext::try_new(".")?;
/// let options = BuildOptions::new().concurrency(2)?;
/// build_workspace(&context, context.root(), &options, &StdoutSink)?;
/// # Ok(())
/// # }
/// ```
pub fn build_workspace(
    context: &crate::workspace::WorkspaceContext,
    root: &Path,
    options: &BuildOptions,
    sink: &impl OutputSink,
) -> Result<()> {
    let target_module = options.selected_module();
    let concurrency = options.concurrency_limit();
    let timeout = options.timeout_override();
    let cancellation = options.cancellation().clone();
    let ved_dir = context.work_dir();
    let state = if ved_dir.join("state.json").exists() {
        crate::discovery::load_workspace_state(ved_dir)?
    } else {
        crate::discovery::scan_workspace(root)
    };

    let modules: Vec<&DetectedModule> = if let Some(target) = target_module {
        state
            .modules
            .iter()
            .filter(|m| {
                m.name == target || m.path == target || m.path.trim_start_matches("./") == target
            })
            .collect()
    } else {
        state.modules.iter().collect()
    };

    if modules.is_empty() {
        if let Some(target) = target_module {
            return Err(VedError::build(
                target,
                format!("Module '{target}' not found in workspace"),
            ));
        } else {
            sink.write_line("build: no detected modules found in workspace (nothing to build)");
            return Ok(());
        }
    }

    if modules.len() == 1 {
        sink.write_line("build: building 1 module...");
        build_module_with_context_and_sink_and_control(
            context,
            root,
            modules[0],
            sink,
            timeout,
            cancellation,
        )?;
        sink.write_line("build: all modules built successfully");
        return Ok(());
    }

    let worker_count = concurrency.max(1).min(modules.len());
    sink.write_line(&format!(
        "build: building {} module(s) (concurrency: {})...",
        modules.len(),
        worker_count
    ));

    let next_index = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel();

    thread::scope(|s| {
        for _ in 0..worker_count {
            let tx = tx.clone();
            let next_index = &next_index;
            let modules = &modules[..];
            let cancellation = &cancellation;
            s.spawn(move || loop {
                if cancellation.is_cancelled() {
                    break;
                }
                let idx = next_index.fetch_add(1, Ordering::Relaxed);
                if idx >= modules.len() {
                    break;
                }
                let module = modules[idx];
                let res = build_module_with_context_and_sink_and_control(
                    context,
                    root,
                    module,
                    sink,
                    timeout,
                    cancellation.clone(),
                );
                let _ = tx.send((idx, module.name.clone(), res));
            });
        }
        drop(tx);
    });

    let mut outcomes = std::iter::repeat_with(|| None)
        .take(modules.len())
        .collect::<Vec<Option<(String, Result<()>)>>>();
    for (idx, mod_name, res) in rx {
        outcomes[idx] = Some((mod_name, res));
    }
    let mut failed: Vec<(String, VedError)> = outcomes
        .into_iter()
        .enumerate()
        .filter_map(|(idx, outcome)| {
            let (name, result) = outcome.unwrap_or_else(|| {
                let module = modules[idx];
                (
                    module.name.clone(),
                    Err(VedError::ProcessOperation {
                        operation: "build".into(),
                        resource: format!("module '{}' (not started)", module.name),
                        source: ved_process::RunError::Cancelled {
                            executable: module.name.clone(),
                        },
                    }),
                )
            });
            result.err().map(|err| (name, err))
        })
        .collect();

    if failed.len() == 1 {
        if let Some((_, err)) = failed.pop() {
            return Err(err);
        }
    } else if !failed.is_empty() {
        return Err(VedError::BuildFailures {
            count: failed.len(),
            failures: failed,
        });
    }

    sink.write_line("build: all modules built successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_resolve_module_command_gradle_wrapper() {
        let tmp =
            std::env::temp_dir().join(format!("ved-build-test-gradle-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        fs::write(tmp.join("gradlew"), "#!/bin/sh\necho gradle").unwrap();

        let module = DetectedModule {
            name: "service-a".to_string(),
            path: "./service-a".to_string(),
            kinds: vec![ModuleKind::Gradle],
            build_cmd: "gradle build".to_string(),
            has_dockerfile: false,
            default_port: 8080,
        };

        let (cmd, args) = resolve_module_command(&module, &tmp);
        assert_eq!(cmd, "./gradlew");
        assert_eq!(args, vec!["build"]);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolve_module_command_maven_wrapper() {
        let tmp = std::env::temp_dir().join(format!("ved-build-test-mvn-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        fs::write(tmp.join("mvnw"), "#!/bin/sh\necho mvn").unwrap();

        let module = DetectedModule {
            name: "service-b".to_string(),
            path: "./service-b".to_string(),
            kinds: vec![ModuleKind::Maven],
            build_cmd: "mvn package".to_string(),
            has_dockerfile: false,
            default_port: 8080,
        };

        let (cmd, args) = resolve_module_command(&module, &tmp);
        assert_eq!(cmd, "./mvnw");
        assert_eq!(args, vec!["package", "-DskipTests"]);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolve_module_command_fallback() {
        let tmp =
            std::env::temp_dir().join(format!("ved-build-test-fallback-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);

        let node_mod = DetectedModule {
            name: "frontend".to_string(),
            path: "./frontend".to_string(),
            kinds: vec![ModuleKind::Node],
            build_cmd: "npm run build".to_string(),
            has_dockerfile: false,
            default_port: 3000,
        };
        let (cmd, args) = resolve_module_command(&node_mod, &tmp);
        assert_eq!(cmd, "npm");
        assert_eq!(args, vec!["run", "build"]);

        let go_mod = DetectedModule {
            name: "backend".to_string(),
            path: "./backend".to_string(),
            kinds: vec![ModuleKind::Go],
            build_cmd: "go build ./...".to_string(),
            has_dockerfile: false,
            default_port: 8080,
        };
        let (cmd, args) = resolve_module_command(&go_mod, &tmp);
        assert_eq!(cmd, "go");
        assert_eq!(args, vec!["build", "./..."]);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_default_concurrency() {
        let conc = default_concurrency();
        assert!(conc >= 1);
    }

    #[test]
    fn test_concurrent_module_builds() {
        let tmp = std::env::temp_dir().join(format!("ved-build-concurrent-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let _ = fs::create_dir_all(&tmp);

        let mod1 = tmp.join("svc1");
        fs::create_dir_all(&mod1).unwrap();
        fs::write(mod1.join("build.gradle"), "").unwrap();
        let gradlew1 = mod1.join("gradlew");
        fs::write(&gradlew1, "#!/bin/sh\nexit 0\n").unwrap();

        let mod2 = tmp.join("svc2");
        fs::create_dir_all(&mod2).unwrap();
        fs::write(mod2.join("build.gradle"), "").unwrap();
        let gradlew2 = mod2.join("gradlew");
        fs::write(&gradlew2, "#!/bin/sh\nexit 0\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms1 = fs::metadata(&gradlew1).unwrap().permissions();
            perms1.set_mode(0o755);
            fs::set_permissions(&gradlew1, perms1).unwrap();

            let mut perms2 = fs::metadata(&gradlew2).unwrap().permissions();
            perms2.set_mode(0o755);
            fs::set_permissions(&gradlew2, perms2).unwrap();
        }

        let res = build_all_concurrent(&tmp, None, 2);
        assert!(res.is_ok(), "Concurrent build failed: {res:?}");

        let _ = fs::remove_dir_all(&tmp);
    }
}
