# Cerbos Authorization Service Example

Cerbos Policy Decision Point (PDP) deployed with a direct exec-form health check.

## Shell-less Containers and Health Checks

The official `ghcr.io/cerbos/cerbos` container image is distroless and does not contain `/bin/sh` or any shell interpreter.

Traditional string health checks (such as `Health.exec("cerbos healthcheck")`) wrap the command in `CMD-SHELL` for Docker Compose and `["sh", "-c", "..."]` for Kubernetes, which immediately fails in shell-less containers:

```text
OCI runtime exec failed: exec: "/bin/sh": stat /bin/sh: no such file or directory
```

## Solution: Exec Array Form

Using an array in `Health.exec`:

```python
health = Health.exec(["cerbos", "healthcheck"])
```

Ved compiles this directly into the native exec form without shell invocation:
- **Docker Compose**: `["CMD", "cerbos", "healthcheck"]`
- **Kubernetes**: `exec.command = ["cerbos", "healthcheck"]`

## Entrypoint & Command Support

Ved supports custom entrypoints and commands for containers:

```python
entrypoint = ["cerbos"]
command = ["server", "--config=/dev/null"]
```

- **Docker Compose**: Emitted as `entrypoint: ["cerbos"]` and `command: ["server", "--config=/dev/null"]`.
- **Kubernetes**: Emitted as `command: ["cerbos"]` and `args: ["server", "--config=/dev/null"]`.

## Compilation

```bash
ved compile -t compose
ved compile -t k8s
```
