# Nginx + Postgres + OpenTelemetry Stack Example

Demonstrates a web proxy and relational database stack enhanced with the `otel` curated stack (`jaeger`, `prometheus`, `otel-collector`, and `grafana`).

## Architecture
- **`web`**: Nginx Alpine reverse proxy on port `8080` (probes container port `80`).
- **`db`**: PostgreSQL 16 Alpine on port `5432` with persistent volume `db-data`.
- **`otel` stack**:
  - `jaeger`: Distributed tracing UI (`16686`) and OTLP receivers (`14317`, `14318`).
  - `prometheus`: Metrics store and scraper (`9090`).
  - `otel-collector`: OTLP pipeline collector (`4317`, `4318`, `8888`, `8889`).
  - `grafana`: Observability dashboard (`3000`).

## Usage
Compile and inspect the Docker Compose output:
```bash
ved compile -t compose
```

Launch the core stack:
```bash
ved up
```

Launch with the `otel` profile enabled:
```bash
OTEL_ENABLED=true ved up
```
