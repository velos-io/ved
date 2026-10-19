# Nginx & Postgres Example

A minimal two-tier application stack with an Nginx web server and a Postgres database with persistent storage.

## Installation

Install `ved` via Homebrew:

```bash
brew install velos-io/tap/ved
```

## Usage

Start the stack with Docker Compose:

```bash
ved up -t compose
```

Inspect service health and port mappings:

```bash
ved ps -t compose
```

Verify Nginx is serving traffic:

```bash
curl http://localhost:8080
```

Stop the stack:

```bash
ved down -t compose
```

## Kubernetes Compilation

Generate standard Kubernetes manifests:

```bash
ved compile -t k8s
```
