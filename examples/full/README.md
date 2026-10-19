# Full Reference Stack Example

A complete microservices architecture demonstrating the full feature set of `ved`: services, background jobs, database migrations, profiles, and health checks.

## Architecture

- **`api-gateway`**: Node.js reverse proxy and ingress entry point.
- **`lambda-emulator`**: Serverless function runner executing containerized business logic.
- **`postgres`**: Relational database with persistent volume storage.
- **`redis`**: In-memory cache conditionally toggled by feature flags.
- **`flyway-demo`**: Pre-run migration job that runs to completion before application startup.
- **`smoke-test`**: Post-install test job validating cluster readiness.
- **`cleanup-job`**: Scheduled maintenance job.
- **`otel-collector`**: Observability profile for telemetry collection.

## Usage

Start the stack with Docker Compose:

```bash
ved up -t compose
```

Inspect service health and port mappings:

```bash
ved ps -t compose
```

Run smoke tests:

```bash
ved test -t compose
```

Stop the stack:

```bash
ved down -t compose
```

## Compilation

Generate Docker Compose or Kubernetes manifests:

```bash
# Docker Compose
ved compile -t compose

# Kubernetes (Kustomize)
ved compile -t k8s
```
