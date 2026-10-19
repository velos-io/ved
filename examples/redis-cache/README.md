# Redis Cache Example

A single-service caching deployment with persistent data volume storage and TCP health checks.

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

Test Redis connectivity:

```bash
nc -zv localhost 6379
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
