# ved-core public API

New integrations should use the root `PureCompiler`, `CompilationTarget`,
`ArtifactBundle`, `ConfigSnapshot`, `WorkspaceContext` and `ValidatedSpec` APIs
shown in the crate's executable rustdoc examples. Compilation accepts an immutable
`ValidatedSpec` and explicit configuration, returning relative artifact paths and bytes in memory.
Loading specifications, capturing workspace configuration, synchronizing catalogs,
building images, publishing artifacts and deploying are host adapter operations.

## Rendering-only dependency

```toml
[dependencies]
ved-core = { version = "0.1", default-features = false }
```

The default `host` feature retains the existing 0.1 APIs, including `VedCompiler`,
file loading, ambient configuration, build execution and persistence. The CLI
explicitly enables it. Disabling defaults leaves validation and explicit-input
rendering available without Starlark, `ved-process`, Unix `rustix`, or signal
registration dependencies in the normal dependency graph. Host-only module paths
and constructors require `host`; default consumers retain source compatibility.
Features are additive: another dependency enabling `host` enables it for the shared
crate instance. This is a build boundary, not a security sandbox.

Use `ConfigSnapshot::from_values` and `WorkspaceContext::from_explicit_paths` with
the example below. The independent consumer in
`tests/fixtures/rendering-consumer` demonstrates this configuration. Run
`python3 scripts/check-core-features.py` to check its locked normal dependency
graph and compare exact artifact paths and bytes with `host` enabled.

## In-memory compilation

```rust
use std::{collections::BTreeMap, path::PathBuf};
use ved_core::{AppSpec, CompilationTarget, ConfigSnapshot, PureCompiler,
               ValidatedSpec, WorkspaceContext};

let root = PathBuf::from(if cfg!(windows) { r"C:\virtual" } else { "/virtual" });
let context = WorkspaceContext::from_explicit_paths(
    &root, root.join("config"), ConfigSnapshot::from_values(BTreeMap::new()),
)?;
let spec = ValidatedSpec::try_from(AppSpec {
    app: "example".into(), version: "1".into(), ..Default::default()
})?;
let bundle = PureCompiler::new(context)?.compile(&spec, CompilationTarget::All)?;
let compose = &bundle.files()[std::path::Path::new("docker-compose.yml")];
assert!(!compose.is_empty());
```

The equivalent example in crate rustdoc runs in the doctest suite. The supplied
workspace need not exist. Paths affect generated environment references; they do
not authorize or trigger persistence. `files()` borrows the ordered map and
`into_files()` transfers ownership to a caller's publisher.

## Inputs and validation

Construct `AppSpec` in memory and convert it with `ValidatedSpec::try_from`.
Validation checks path-sensitive identifiers, workload name collisions, port,
command and mount decoding, dependency conditions/references and dependency cycles,
including disabled workloads and profiles. It freezes the validated snapshot; editing
the original DTO does not edit that snapshot. This does not prove image existence,
resource availability, application health, or acceptance by a container runtime or
Kubernetes API server. Configuration selects activation and namespace routing.

The host `VedCompiler::load_validated_spec` supports YAML, Starlark (`Vedfile`,
`.star`, `.py`, or recognized Starlark content), and shorthand (`.ved` or recognized
shorthand content). Starlark takes precedence in format detection. `.py` here means
Starlark syntax, not unrestricted Python. File loading can read referenced files,
resolve configuration and load stack catalogs; it is outside the pure compiler.
The pure API accepts an already resolved specification and does not parse files,
interpolate source text, merge catalogs or discover a project.

## Artifacts and target limits

Compose emits `docker-compose.yml`. Kubernetes emits a Kustomize tree under
`k8s/`, with base resources and local/dev overlays. Helm emits `helm/values.yaml`;
it does not produce a chart or promise compatibility with an arbitrary chart.
Target support differs: Helm currently emits global application version and ingress
suffix values, while Compose and Kubernetes synthesize workloads. Compilation
success is not deployment success. Kubernetes emission checks YAML document
structure and configured namespaces, not cluster admission or CRD schemas.

Artifacts can contain configured secret values. Callers must handle returned bytes
and persistence destinations appropriately. Artifact debug output reports metadata
without displaying contents. Generated environment exports, stack assets, routing
fingerprints and cache metadata belong to CLI orchestration, outside the pure bundle.

## Effects and compatibility

The pure facade uses only its supplied specification and configuration snapshot.
Use the explicit path/snapshot constructor to avoid ambient environment and file
reads during setup. Legacy workspace constructors capture configuration from the
host and may resolve relative paths against the current directory. Existing
`VedCompiler` load methods read files; its Kubernetes compile methods write through
the confined filesystem adapter. Build and stacks module operations may run
subprocesses, read catalogs, synchronize Git repositories and copy files.
The CLI publisher owns transactional publication and stale-file cleanup.

During 0.1.x the existing public module paths and `VedCompiler` signatures remain
available with the default `host` feature for source compatibility. Implementation modules are hidden from default
rustdoc; this is a documentation boundary, not access control. The root facade is
the supported entry point for new compilation integrations. A future breaking
release may make legacy modules private or move host adapters, with release notes
and migration instructions before removal. No legacy module is removed in T78.

## Checked ports and 0.1 migration

Use `CheckedPortSpec::new(host, container)`, `single(port)` or `parse(text)` for
nonzero ports. These return `Result<CheckedPortSpec, PortValidationError>`;
`host()` and `container()` expose read-only values. Private `NonZeroU16` fields
prevent callers from mutating an accepted port to zero. Checked serde accepts
integers, numeric strings and `"host:container"` strings, rejects zero, malformed
or out-of-range values, and serializes canonically as `"host:container"`, matching
legacy serialization for valid ports.

```rust
use ved_core::{CheckedPortSpec, PortSpec};
let checked = CheckedPortSpec::new(9000, 8080)?;
assert_eq!(checked.container(), 8080);
assert!(CheckedPortSpec::single(0).is_err());
let legacy: PortSpec = checked.into();
assert_eq!(CheckedPortSpec::try_from(legacy)?, checked);
# Ok::<(), ved_core::PortValidationError>(())
```

`ValidatedWorkload::checked_ports()` borrows the checked port slice. Its existing
`ports()` still returns `&[PortSpec]`; both private caches originate from the same
checked values and cannot be mutated by callers. Singular and list ports, including
profiles, pass through semantic validation before rendering.

During 0.1, `PortSpec` retains its public `u16` fields, infallible constructors and
legacy parser/serde behavior, including the ability to construct zero. Such values
are rejected at the validated compilation boundary. Convert legacy values using
`CheckedPortSpec::try_from` when integrating new code. An explicitly approved
breaking release may replace the public DTO fields and legacy slice accessor with
checked values and remove the compatibility cache; T86 does not make that change.

## Fallible host workspace setup

With `host`, prefer `WorkspaceContext::try_new`, `try_with_paths` and
`try_with_snapshot`. They return `Result<WorkspaceContext, WorkspaceResolutionError>`.
Absolute inputs do not need cwd resolution. Relative paths resolve against one
captured absolute cwd; failure returns the original I/O cause through the typed
error and `std::error::Error::source`. Path resolution finishes before
`try_with_paths` captures configuration. These constructors do not canonicalize
paths or require directories to exist.

The CLI propagates cwd and context construction errors before workload operations.
The legacy public `new`, `with_paths` and `with_snapshot` keep their 0.1 signatures
and their historical dot fallback when cwd cannot be read. New code should use
the fallible variants; replacing the old return types requires an explicitly
approved breaking release. For rendering-only integrations,
`from_explicit_paths` remains the ambient-free constructor in either feature mode.

## Options-based host builds

With `host`, prefer `ved_core::build::{BuildOptions, build_workspace}`:

```rust
use std::time::Duration;
use ved_core::{build::{BuildOptions, build_workspace}, SilentSink, WorkspaceContext};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let context = WorkspaceContext::try_new("/workspace")?;
    let options = BuildOptions::new()
        .module("api")
        .concurrency(2)?
        .timeout(Duration::from_secs(30))?;
    build_workspace(&context, context.root(), &options, &SilentSink)?;
    Ok(())
}
```

The example requires a real workspace when executed; the crate's `no_run` doctest
and standalone consumer compile the API without invoking build tools. The context,
scan root and output sink stay explicit; the scan root may differ from the context
root as in the legacy API. Selection matches module names or existing path forms;
`all_modules()` clears it. Missing or empty selections report a missing module.

Defaults select all modules, use `default_concurrency()` (logical parallelism,
fallback four), inherit the existing per-child timeout policy, and create a fresh
cancellation token. Concurrency is capped by the selected module count. New options
reject zero concurrency and zero timeout before discovery or tool execution.
`configured_timeout()` clears a timeout override. Resolution remains explicit
option, positive `VED_PROCESS_TIMEOUT_SECS` in the context, positive process
environment value, then the runner default (30 minutes unless configured). A timeout
bounds each child, not the entire batch; there is no new unlimited mode.

Pass your application token with `cancellation_token(token)` to share cancellation.
`cancellation()` exposes the option's token; cloned options share it, while separate
default options have independent tokens. The CLI passes its invocation token to
explicit builds and auto-builds. Cancellation terminates active process groups and
prevents pending tool launches. Aggregate failures remain in module discovery order,
including cancelled pending work, regardless of completion order.

All existing `build_all_*` signatures remain available during 0.1 and delegate to
the same implementation. They retain zero-concurrency normalization to one and an
explicit zero timeout's immediate-deadline behavior. The CLI likewise preserves its
existing zero-concurrency normalization. Lower-level single-module functions remain
available. Removing compatibility wrappers requires a future breaking migration;
T87 adds no separate scheduler, launcher or release workflow.

## Structured validation and file errors

Semantic checks construct a `ValidationError` at the failing check, with a
`ValidationReason` and read-only profile, workload kind/name and field accessors.
Reasons cover identifier, port, command, mount, malformed dependency, dependency
condition/reference/hostname, duplicate workload and cycle failures. Classification
never parses rendered messages. Use `VedError::validation_error()` to inspect the
typed reason, and `is_validation_error()` for validation/specification classification.
The reason and file-phase enums are non-exhaustive; allow future categories.

File adapters retain a `SpecLoadError` with `path()` and `phase()` (`Read`, `Parse`,
`Validate`, `Evaluate` or `Convert`). `VedError::spec_load_error()` finds this wrapper.
Standard `std::error::Error::source` traversal retains concrete I/O, YAML, JSON and
semantic errors, even through file and anyhow context wrappers. Starlark 0.14.2
provides `into_anyhow()` rather than implementing `Error` directly; the supported
conversion preserves its diagnostic and available native sources. Its wrapper is
private upstream, and some Starlark error kinds have no deeper standard source.
No string fallback is presented as an original source.

Normal loader display shows phase/file and safe semantic field context, omitting
raw parser excerpts and invalid command/mount/port values. Default core/CLI debug
formatting also avoids dumping retained source chains. Sources are deliberately
inspectable and may contain interpolated configuration or interpreter snippets;
treat explicit source traversal and upstream debug output as sensitive. A caller
providing custom details, paths or context must avoid putting secrets into them.
Legacy string constructors keep their existing caller-provided text.

During 0.1, all public `VedError` variants, payloads and constructors remain
available. New typed errors travel through the existing `Anyhow` variant; loaders
now use that envelope instead of flattening new failures into `SpecParse` or
`Starlark` text. Migrate loader error handling to the classification/accessor APIs
rather than expecting an individual legacy variant. Standard source traversal now
includes the anyhow root instead of skipping it. A future explicitly approved
breaking release may introduce dedicated variants; T88 changes no enum payloads.

The CLI retains a boxed core cause for validation failures. Validation/specification
errors return exit 2 through operation wrappers; unrelated operational errors return
1. A nonempty aggregate is exit 2 only when every failure is validation, otherwise
1. Aggregate counts and display summaries derive from their stored failures. The
public legacy core `BuildFailures::count` field remains for compatibility.
