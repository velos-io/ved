# Ved Examples

Practical, self-contained examples demonstrating `Vedfile` specifications for common application architectures.

## Build the Rust CLI

Follow the [source build and minimal compile example](../README.md) from the repository root. Python is not required for the Rust CLI. To install from a checkout:

```sh
cargo install --locked --path crates/ved
ved --version
```

Ensure Cargo's bin directory is on your `PATH`. You can instead use the absolute path to `target/release/ved` after `cargo build --locked -p ved --release`.

## Examples

| Directory | Architecture | Description |
|---|---|---|
| [`full/`](full/) | Complete Microservices Stack | End-to-end stack: API gateway, Lambda emulator, Postgres, Flyway migrations, smoke tests, and OpenTelemetry |
| [`nginx-postgres/`](nginx-postgres/) | Two-tier web + DB | Nginx web server with HTTP healthcheck and Postgres database with persistent volume |
| [`postgres-flyway/`](postgres-flyway/) | DB + Flyway Migration + API | Automated database schema migration with Flyway running to completion before API starts |
| [`redis-cache/`](redis-cache/) | In-memory cache | Redis 7 Alpine caching tier with TCP healthcheck and volume persistence |

## Compile an example

Enter an example directory and generate Compose artifacts before choosing whether to deploy:

```sh
ved compile -t compose
```

Compilation does not require a Docker daemon or cluster. These examples can reference configuration and local assets; consult the directory's README. Kubernetes compilation emits a Kustomize tree and checks structure/namespaces, not cluster admission or CRD schemas. Helm output is values only for a compatible chart; see [target limits](../docs/public-api.md).

## Optional deployment

Compose deployment requires Docker with the Compose plugin and a running daemon. Kubernetes deployment requires `kubectl`, an accessible cluster and appropriate permissions. Deployment changes runtime resources.

Enter the chosen example directory and run:

```bash
# Start stack locally with Docker Compose
ved up -t compose

# Inspect status and service ports
ved ps -t compose

# Stop stack
ved down -t compose
```

To compile Kubernetes manifests instead:

```bash
ved compile -t k8s
```
