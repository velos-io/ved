# Postgres & Flyway Migration Example

A complete deployment example demonstrating automated database schema migration using Flyway before dependent services start.

## Architecture

1. **`db`** (Service): PostgreSQL database with persistent volume and health check (`pg_isready`).
2. **`migration`** (Job): Flyway migration container that runs `flyway migrate` against `db` after `db.healthy`.
3. **`api`** (Service): Web server configured with `wait_for=["migration.completed"]`. Docker Compose enforces `service_completed_successfully` so `api` starts strictly after database migrations finish.

## Usage

Compile the Docker Compose manifest:

```bash
ved compile -d examples/postgres-flyway -t compose
```

Inspect generated dependencies in `.ved/docker-compose.yml`:

```yaml
api:
  depends_on:
    migration:
      condition: service_completed_successfully
```

Start the stack:

```bash
ved up -d examples/postgres-flyway -t compose
```

Inspect container status and health:

```bash
ved ps -d examples/postgres-flyway -t compose
```

Stop and clean up:

```bash
ved down -d examples/postgres-flyway -t compose
```

## Kubernetes Compilation

Generate Kubernetes manifests and job definitions:

```bash
ved compile -d examples/postgres-flyway -t k8s
```
