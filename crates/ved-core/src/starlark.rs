use crate::error::{Result, SpecLoadPhase, StarlarkError, VedError};
use starlark::environment::{GlobalsBuilder, LibraryExtension, Module};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect};
use std::path::Path;
use std::time::Instant;

use crate::resources::{LoadOptions, LoadSession, ResourceLimitError, ResourceLimitKind};

use starlark::starlark_module;

use crate::model::{AppSpec, ServiceSpec};

#[starlark_module]
fn ved_env_module(builder: &mut GlobalsBuilder) {
    fn ved_get_env(
        name: &str,
        default: Option<&str>,
        eval: &mut Evaluator,
    ) -> starlark::Result<String> {
        let def = default.unwrap_or("");
        let snapshot = eval
            .extra
            .and_then(|extra| extra.downcast_ref::<crate::config::ConfigSnapshot>())
            .ok_or_else(|| starlark::Error::new_other(StarlarkError::MissingConfigSnapshot))?;
        Ok(snapshot.get_prop(name, def))
    }
}

const PRELUDE: &str = r#"
def _mount_volume(name, path):
    return "{}:{}".format(name, path)

def _mount_ro(src, dst):
    return "{}:{}:ro".format(src, dst)

def _mount_bind(src, dst):
    return "{}:{}".format(src, dst)

Mount = struct(
    volume = _mount_volume,
    ro = _mount_ro,
    bind = _mount_bind,
)

def Port(target=None, container=None, host=None):
    c = container if container != None else target
    if host != None and c != None:
        return "{}:{}".format(host, c)
    if c != None:
        return "{}:{}".format(c, c)
    return ""

def _sec_drop_all():
    return struct(dropAll=True, caps=[], escalate=False)

def Security(capabilities=[], allow_escalation=False, user=None, group=None, fs_group=None, drop_all=False):
    if drop_all:
        return struct(dropAll=True, caps=[], escalate=False)
    user_val = None
    if user != None and group != None:
        user_val = "{}:{}".format(user, group)
    elif user != None:
        user_val = str(user)
    return struct(
        caps = capabilities,
        escalate = allow_escalation,
        user = user_val,
        fsGroup = fs_group,
        dropAll = False,
    )

def _health_exec(cmd):
    return cmd

def _health_tcp(port):
    return "tcp:{}".format(port)

def _health_http(path="/", port=None):
    if type(path) == "int":
        return "http://localhost:{}/".format(path)
    if str(path).isdigit():
        return "http://localhost:{}/".format(path)
    if path.startswith("http://") or path.startswith("https://"):
        return path
    if path.startswith("localhost:") or path.startswith("127.0.0.1:"):
        return "http://" + path
    clean_path = path if path.startswith("/") else "/" + path
    if port != None:
        return "http://localhost:{}{}".format(port, clean_path)
    return clean_path

Health = struct(
    exec = _health_exec,
    tcp = _health_tcp,
    http = _health_http,
)

def Ingress(host="app", path="/"):
    return host

def _env_secret(name, default=""):
    return ved_get_env(name, default)

def _env_bool(name, default=False):
    val = ved_get_env(name, "")
    if val != "":
        return val in ["1", "true", "True", "yes"]
    return default

env = struct(
    secret = _env_secret,
    bool = _env_bool,
)

def _normalize_wait(w):
    if "." in w and ":" not in w:
        parts = w.split(".")
        return parts[0] + ":" + parts[1]
    return w

def _is_enabled(name, explicit):
    if explicit != None:
        return explicit
    flag_name = name.replace("-", "_").upper() + "_ENABLED"
    val = ved_get_env(flag_name, "true").lower()
    return val not in ["0", "false", "no", "off"]

_activated_stacks = []

def _stack_add(name):
    if name not in _activated_stacks:
        _activated_stacks.append(name)

Stack = struct(
    add = _stack_add,
)

def Service(
    name,
    image=None,
    build=None,
    group="default",
    ports=[],
    memory=None,
    ingress=None,
    env={},
    env_file=None,
    mounts=[],
    security=None,
    health=None,
    wait_for=[],
    restart=None,
    enabled=None,
    command=None,
):
    norm_wait = [_normalize_wait(w) for w in wait_for]
    act_enabled = _is_enabled(name, enabled)
    return struct(
        name=name,
        image=image,
        build=build,
        group=group,
        ports=ports,
        memory=memory,
        ingress=ingress,
        env=env,
        env_file=env_file,
        mounts=mounts,
        security=security,
        health=health,
        waitFor=norm_wait,
        restart=restart,
        enabled=act_enabled,
        command=command,
    )

def Job(
    name,
    image,
    group="default",
    command=[],
    schedule=None,
    hook=None,
    mounts=[],
    env={},
    wait_for=[],
    restart=None,
    when=None,
    enabled=None,
):
    norm_wait = [_normalize_wait(w) for w in wait_for]
    act_enabled = _is_enabled(name, enabled)
    return struct(
        name=name,
        image=image,
        group=group,
        command=command,
        schedule=schedule,
        hook=hook,
        mounts=mounts,
        env=env,
        waitFor=norm_wait,
        restart=restart,
        when=when,
        enabled=act_enabled,
    )

def App(name, version, network="default", ingress_suffix=None, env_files=None):
    if env_files == None:
        env_files = ["config/.env", "config/.local.env"]
    _services = []
    _jobs = []
    _active_profile = [None]
    _profile_services = {}
    _profile_jobs = {}
    _profile_when = {}
    _tasks = {}

    def _add_services(*args):
        prof = _active_profile[0]
        for s in args:
            if prof != None:
                if prof not in _profile_services:
                    _profile_services[prof] = []
                _profile_services[prof].append(s)
            else:
                _services.append(s)

    def _add_jobs(*args):
        prof = _active_profile[0]
        for j in args:
            if prof != None:
                if prof not in _profile_jobs:
                    _profile_jobs[prof] = []
                _profile_jobs[prof].append(j)
            else:
                _jobs.append(j)

    def _profile(name, enabled=True):
        _active_profile[0] = name
        _profile_when[name] = enabled
        return True

    def _task_decorator(fn):
        return fn

    return struct(
        app = name,
        version = version,
        network = network,
        ingress = ingress_suffix,
        env_files = env_files,
        services = struct(add = _add_services, list = _services),
        jobs = struct(add = _add_jobs, list = _jobs),
        stacks = struct(add = _stack_add, list = _activated_stacks),
        profile = _profile,
        task = _task_decorator,
        _profile_services = _profile_services,
        _profile_jobs = _profile_jobs,
        _profile_when = _profile_when,
        _tasks = _tasks,
    )

def _stack_services_to_dict(services):
    res = {}
    for s in services:
        res[s.name] = {
            "image": s.image,
            "build": s.build,
            "group": s.group,
            "ports": s.ports,
            "memory": s.memory,
            "ingress": s.ingress,
            "env": s.env,
            "env_file": s.env_file,
            "mounts": s.mounts,
            "security": s.security,
            "health": s.health,
            "waitFor": s.waitFor,
            "restart": s.restart,
            "enabled": s.enabled,
            "command": s.command,
        }
    return res

def _app_to_spec(app):
    services_map = {}
    for s in app.services.list:
        services_map[s.name] = {
            "image": s.image,
            "build": s.build,
            "group": s.group,
            "ports": s.ports,
            "memory": s.memory,
            "ingress": s.ingress,
            "env": s.env,
            "env_file": s.env_file,
            "mounts": s.mounts,
            "security": s.security,
            "health": s.health,
            "waitFor": s.waitFor,
            "restart": s.restart,
            "enabled": s.enabled,
            "command": s.command,
        }

    jobs_map = {}
    for j in app.jobs.list:
        jobs_map[j.name] = {
            "image": j.image,
            "group": j.group,
            "command": j.command,
            "schedule": j.schedule,
            "hook": j.hook,
            "mounts": j.mounts,
            "env": j.env,
            "waitFor": j.waitFor,
            "restart": j.restart,
            "when": j.when,
            "enabled": j.enabled,
        }

    profiles_map = {}
    for p_name, p_svcs in app._profile_services.items():
        p_svc_map = {}
        for s in p_svcs:
            p_svc_map[s.name] = {
                "image": s.image,
                "build": s.build,
                "group": s.group,
                "ports": s.ports,
                "memory": s.memory,
                "ingress": s.ingress,
                "env": s.env,
                "env_file": s.env_file,
                "mounts": s.mounts,
                "security": s.security,
                "health": s.health,
                "waitFor": s.waitFor,
                "restart": s.restart,
                "enabled": s.enabled,
                "command": s.command,
            }
        profiles_map[p_name] = {
            "when": app._profile_when.get(p_name, True),
            "services": p_svc_map,
            "jobs": {},
        }

    return {
        "app": app.app,
        "version": app.version,
        "network": app.network,
        "ingress": app.ingress,
        "envFiles": list(app.env_files),
        "services": services_map,
        "jobs": jobs_map,
        "profiles": profiles_map,
        "tasks": app._tasks,
        "stacks": _activated_stacks,
    }
"#;

pub fn sanitize_starlark(raw: &str) -> String {
    let mut out = String::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("from ved import") || trimmed.starts_with("@app.task") {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        } else if trimmed.starts_with("with app.profile(") && trimmed.ends_with(':') {
            let replaced = line.replacen("with app.profile(", "if app.profile(", 1);
            out.push_str(&replaced);
            out.push('\n');
        } else if trimmed.contains("Security.drop_all()") {
            let replaced = line.replace("Security.drop_all()", "Security(drop_all=True)");
            out.push_str(&replaced);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

pub fn evaluate_stack_file(spec_path: &Path) -> Result<Vec<(String, ServiceSpec)>> {
    let context = crate::WorkspaceContext::new(spec_path.parent().unwrap_or(Path::new(".")));
    let options = LoadOptions::from_config(context.config())?;
    let mut session = LoadSession::new(&options)?;
    let content = session.read_source(spec_path)?;
    evaluate_stack_contents_with_context_in_session(&content, spec_path, &context, &mut session)
}

/// Evaluate catalogue content that the caller has already charged to this session.
pub(crate) fn evaluate_stack_contents_with_context_in_session(
    content: &str,
    spec_path: &Path,
    context: &crate::WorkspaceContext,
    session: &mut LoadSession,
) -> Result<Vec<(String, ServiceSpec)>> {
    let json = evaluate_json(
        content,
        spec_path,
        context,
        session,
        "stack_res = _stack_services_to_dict(services)",
        "stack_res",
    )?;
    let services: indexmap::IndexMap<String, ServiceSpec> = serde_json::from_str(&json)
        .map_err(|e| VedError::spec_load(spec_path, SpecLoadPhase::Convert, e))?;
    session.check_elapsed(spec_path)?;
    Ok(services.into_iter().collect())
}

pub fn load_starlark_spec(path: &Path) -> Result<AppSpec> {
    let spec = load_starlark_spec_wire(path)?;
    spec.validate()
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Validate, e))?;
    Ok(spec)
}

pub(crate) fn load_starlark_spec_wire(path: &Path) -> Result<AppSpec> {
    let context = crate::WorkspaceContext::new(path.parent().unwrap_or(Path::new(".")));
    load_starlark_spec_wire_with_context(path, &context)
}
pub(crate) fn load_starlark_spec_wire_with_context(
    path: &Path,
    context: &crate::WorkspaceContext,
) -> Result<AppSpec> {
    let options = LoadOptions::from_config(context.config())?;
    let mut session = LoadSession::new(&options)?;
    let content = session.read_source(path)?;
    load_starlark_spec_wire_from_source_in_session(path, context, &content, &mut session)
}

/// Load already charged source while preserving the legacy catalogue publication path.
pub(crate) fn load_starlark_spec_wire_from_source_in_session(
    path: &Path,
    context: &crate::WorkspaceContext,
    content: &str,
    session: &mut LoadSession,
) -> Result<AppSpec> {
    let mut spec = evaluate_starlark_spec_in_session(path, context, content, session)?;
    crate::stacks::apply_stacks_with_context_in_session(
        &spec.stacks.clone(),
        &mut spec,
        context,
        session,
    )
    .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Evaluate, e))?;
    Ok(spec)
}

/// Evaluate project content already charged by the parser, sharing all catalogue accounting.
pub(crate) fn load_starlark_spec_wire_with_context_and_dependencies_in_session(
    path: &Path,
    context: &crate::WorkspaceContext,
    content: &str,
    session: &mut LoadSession,
) -> Result<crate::stacks::StackApplication> {
    let spec = evaluate_starlark_spec_in_session(path, context, content, session)?;
    let applied = crate::stacks::load_stacks_with_context_in_session(
        &spec.stacks.clone(),
        &spec,
        context,
        session,
    )
    .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Evaluate, e))?;
    session.check_elapsed(path)?;
    Ok(applied)
}

fn evaluate_starlark_spec_in_session(
    path: &Path,
    context: &crate::WorkspaceContext,
    content: &str,
    session: &mut LoadSession,
) -> Result<AppSpec> {
    let json = evaluate_json(
        content,
        path,
        context,
        session,
        "spec = _app_to_spec(app)",
        "spec",
    )?;
    let spec: AppSpec = serde_json::from_str(&json)
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Convert, e))?;
    spec.validate_identifiers()
        .map_err(|e| VedError::spec_load(path, SpecLoadPhase::Validate, e))?;
    session.check_elapsed(path)?;
    Ok(spec)
}

/// Supported upstream limits are cooperative; native calls and parsing cannot be
/// interrupted in-process. In particular, the heap budget does not bound process RSS.
fn evaluate_json(
    content: &str,
    path: &Path,
    context: &crate::WorkspaceContext,
    session: &mut LoadSession,
    conversion: &str,
    result_name: &str,
) -> Result<String> {
    session.check_elapsed(path)?;
    let sanitized = sanitize_starlark(content);
    let full_script = format!("{PRELUDE}\n\n{sanitized}\n\n{conversion}\n");
    let dialect = Dialect {
        enable_top_level_stmt: true,
        enable_keyword_only_arguments: true,
        ..Dialect::Standard
    };
    let ast = AstModule::parse(&path.to_string_lossy(), full_script, &dialect).map_err(|e| {
        VedError::spec_load(path, SpecLoadPhase::Parse, StarlarkError::evaluation(e))
    })?;
    session.check_elapsed(path)?;
    let globals =
        GlobalsBuilder::extended_by(&[LibraryExtension::StructType, LibraryExtension::Json])
            .with(ved_env_module)
            .build();
    Module::with_temp_heap(|module| -> Result<String> {
        let mut eval = Evaluator::new(&module);
        eval.extra = Some(context.config());
        let remaining_ticks = session.remaining_ticks();
        if remaining_ticks == 0 {
            return Err(ResourceLimitError::new(
                ResourceLimitKind::Ticks,
                path,
                session.budget().max_ticks,
            )
            .into());
        }
        let heap_limit = session.budget().max_heap_bytes;
        let stack_limit = session.budget().max_callstack_frames;
        eval.set_max_tick_count(remaining_ticks).map_err(|e| {
            VedError::spec_load(path, SpecLoadPhase::Evaluate, StarlarkError::evaluation(e))
        })?;
        eval.set_max_heap_size(heap_limit).map_err(|e| {
            VedError::spec_load(path, SpecLoadPhase::Evaluate, StarlarkError::evaluation(e))
        })?;
        eval.set_max_callstack_size(stack_limit).map_err(|e| {
            VedError::spec_load(path, SpecLoadPhase::Evaluate, StarlarkError::evaluation(e))
        })?;
        let deadline = session.deadline();
        eval.set_check_cancelled(Box::new(move || Instant::now() >= deadline));
        let outcome = eval.eval_module(ast, &globals);
        // Upstream checks only periodically. Inspect usage after both success and
        // failure, before handing any partially evaluated value to conversion.
        session.check_elapsed(path)?;
        let used_ticks = eval.get_total_tick_count();
        session.charge_ticks(path, used_ticks)?;
        let heap_used = eval
            .heap()
            .peak_allocated_bytes()
            .saturating_add(eval.frozen_heap().allocated_bytes());
        if heap_used > heap_limit {
            return Err(ResourceLimitError::new(
                ResourceLimitKind::HeapBytes,
                path,
                heap_limit as u64,
            )
            .into());
        }
        outcome.map_err(|e| {
            if matches!(e.kind(), starlark::ErrorKind::StackOverflow(_)) {
                ResourceLimitError::new(
                    ResourceLimitKind::CallstackFrames,
                    path,
                    stack_limit as u64,
                )
                .into()
            } else {
                VedError::spec_load(path, SpecLoadPhase::Evaluate, StarlarkError::evaluation(e))
            }
        })?;
        let value = module.get(result_name).ok_or_else(|| {
            VedError::spec_load(
                path,
                SpecLoadPhase::Evaluate,
                StarlarkError::MissingResult(result_name.to_string()),
            )
        })?;
        let json = value.to_json().map_err(|e| {
            VedError::spec_load(path, SpecLoadPhase::Convert, StarlarkError::evaluation(e))
        })?;
        // Conversion allocates native Rust memory; this post-allocation cap is
        // representation protection, not a hard allocation limit.
        session.check_representation(path, json.as_bytes())?;
        Ok(json)
    })
}

#[cfg(test)]
mod budget_measurements {
    use super::*;
    #[test]
    fn committed_example_evaluation_has_default_tick_headroom() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let path = root.join("examples/full/Vedfile");
        let context = crate::WorkspaceContext::from_explicit_paths(
            &root,
            root.join("config"),
            crate::ConfigSnapshot::from_values(Default::default()),
        )
        .unwrap();
        let options = LoadOptions::default();
        let mut session = LoadSession::new(&options).unwrap();
        let content = session.read_source(&path).unwrap();
        let spec =
            evaluate_starlark_spec_in_session(&path, &context, &content, &mut session).unwrap();
        assert_eq!(spec.app, "vt-deployer");
        let ticks = session.ticks_used();
        eprintln!(
            "full Vedfile: {ticks} ticks; default {}",
            options.budget().max_ticks
        );
        assert!(ticks > 0);
        assert!(ticks < options.budget().max_ticks / 10);
    }

    #[test]
    fn missing_evaluator_config_snapshot_yields_typed_error() {
        let dialect = Dialect {
            enable_top_level_stmt: true,
            enable_keyword_only_arguments: true,
            ..Dialect::Standard
        };
        let ast = AstModule::parse("test.star", format!("{PRELUDE}\nx = env.secret('KEY')\n"), &dialect).unwrap();
        let globals = GlobalsBuilder::extended_by(&[LibraryExtension::StructType, LibraryExtension::Json])
            .with(ved_env_module)
            .build();
        Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            let err = eval.eval_module(ast, &globals).unwrap_err();
            assert!(
                err.to_string().contains("Configuration snapshot missing from Starlark evaluator context"),
                "expected typed missing snapshot error, got: {err}"
            );
        });
    }
}

