# VT Deployer

VT Deployer (`ved`) is a Rust CLI and library that compiles declarative service specifications into deployment artifacts.

| Target | CLI output with `--output-dir dist` | Scope |
| --- | --- | --- |
| Compose (`compose`) | `dist/docker-compose.yml` | Container workloads, dependencies, ports and healthchecks |
| Kubernetes (`k8s`) | Kustomize tree under `dist/k8s/` | Base resources and environment overlays |
| Helm (`helm`) | `dist/values.yaml` | Application version and ingress values for integration with your own chart |

Without `--output-dir`, Kubernetes defaults to `installer/k8s/` and Helm to `installer/values.yaml`; Compose uses its configured path (default `.ved/docker-compose.yml`). The in-memory library bundle has its own relative paths, including `helm/values.yaml`.

Helm output is values only; it does not generate a chart or promise compatibility with an arbitrary chart. Kubernetes generation checks YAML structure and configured namespaces; it does not establish cluster admission, CRD schema validity or application health. The CLI also manages environment exports and compilation state. See [public API and target limits](docs/public-api.md).

## Build and install from source

From an obtained source checkout, run these commands at the repository root. Install Rust through rustup and use its `cargo` shim so [rust-toolchain.toml](rust-toolchain.toml) selects Rust **1.88.0**, the minimum supported version. A native compiler/linker toolchain and access to dependency sources are needed for the first build.

Linux and macOS are supported; CI tests both, and release builds target AMD64 and ARM64 on each. Windows is not currently a supported release platform.

```sh
cargo build --locked -p ved --release
./target/release/ved --version
./target/release/ved compile --help

# Optional: install the CLI into Cargo's bin directory.
cargo install --locked --path crates/ved
ved --version
```

For the installed command, ensure Cargo's bin directory (normally `~/.cargo/bin`) is on your `PATH`. The next example uses the checkout's built binary directly, so installation is optional.

## Compile your first specification

Run this from the checkout root. It creates an isolated temporary workspace and generates Compose YAML. Compilation requires neither Docker nor a Kubernetes cluster.

```sh
ved_bin="$PWD/target/release/ved"
demo_dir=$(mktemp -d)
mkdir -p "$demo_dir/config"
cat > "$demo_dir/ved.yaml" <<'YAML'
app: hello-ved
version: "0.1.0"
env_files: []
services:
  web:
    image: nginx:alpine
    ports: ["8080:80"]
YAML

"$ved_bin" --dir "$demo_dir" \
  --config-dir "$demo_dir/config" \
  --work-dir "$demo_dir/.ved" \
  --output-dir "$demo_dir/dist" \
  compile --file "$demo_dir/ved.yaml" --target compose
cat "$demo_dir/dist/docker-compose.yml"
```

The output contains the `web` service, its `nginx:alpine` image and the `8080:80` port mapping. Explicit paths keep configuration, state and generated files in the temporary workspace. Use `--target k8s`, `--target helm` or `--target all` for other outputs. An explicitly selected configuration directory must already exist.

## Specifications and deployment

File loading supports YAML, Starlark (`Vedfile`, `.star`, or `.py`) and shorthand (`.ved`). A `.py` specification uses Starlark syntax, not unrestricted Python. Start with the [examples](examples/README.md) for richer workloads, configuration and optional deployment.

Deployment has additional prerequisites:

- Compose: Docker with the Compose plugin and a running daemon.
- Kubernetes: `kubectl`, an accessible target cluster and the permissions/resources required by your workloads.
- Helm values: Helm and a compatible chart for your integration; the generated values alone are not a deployable chart.

The `ved up`, `down`, `ps` and `test` commands operate on runtime resources. Review the example and selected target before using them. Rendering artifacts successfully does not prove that images exist or that deployment will succeed.

## Rust development and library use

Run `cargo test --locked --workspace` for the Rust tests, or `bash scripts/rust-quality.sh` for the standard format, Clippy, test and rustdoc gates. See the [Rust quality policy](docs/rust-quality.md) for MSRV, isolated testing, dependency checks, packaging and fuzzing instructions.

New library integrations should use `ved-core`'s immutable validated specification and in-memory compilation facade. The [public API guide](docs/public-api.md) explains examples, host effects and the 0.1 compatibility policy.

## Historical Python and Make workflows

The repository retains historical Python tooling and tests under [installer/ved](installer/ved/) and [tests](tests/). These are separate from the Rust build and compile instructions above. The [Makefile](Makefile) mixes Rust CLI wrappers with historical checks: `make compile` invokes `vew`, while `make unit-test` runs Cargo tests and `.venv/bin/pytest`. Python and a configured virtual environment are required for those Python checks, not for building or running the Rust CLI. `make demo` starts services and runs runtime checks.

The [vew wrapper](vew) prefers a local release binary but can attempt downloads when one is unavailable. Source builds above provide a checkout-local starting point without depending on published binary availability.
