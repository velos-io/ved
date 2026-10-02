#!/usr/bin/env bash
set -euo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# scripts/sonar-scanner.sh — Run SonarQube analysis for Ved / vt-deployer
# ─────────────────────────────────────────────────────────────────────────────

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if ! command -v sonar-scanner >/dev/null 2>&1; then
    echo "Error: 'sonar-scanner' executable not found in PATH." >&2
    echo "Install via Homebrew: brew install sonar-scanner" >&2
    exit 1
fi

PROJECT_KEY="${SONAR_PROJECT_KEY:-ved}"
if [ $# -gt 0 ] && [[ "$1" != -* ]]; then
    PROJECT_KEY="$1"
    shift
fi

HOST_URL="${SONAR_HOST_URL:-http://localhost:9000}"
TOKEN="${SONAR_TOKEN:-}"

SONAR_ARGS=(
    "-Dsonar.projectKey=${PROJECT_KEY}"
    "-Dsonar.projectName=${PROJECT_KEY}"
    "-Dsonar.host.url=${HOST_URL}"
    "-Dsonar.sources=crates/ved/src,tests,scripts"
    "-Dsonar.exclusions=**/target/**,**/.venv/**,**/.ved/**,**/.git/**,**/agent-work/**"
)

if [ -n "$TOKEN" ]; then
    SONAR_ARGS+=("-Dsonar.token=${TOKEN}")
fi

echo "==> Running SonarScanner for '${PROJECT_KEY}' at ${HOST_URL}..."
exec sonar-scanner "${SONAR_ARGS[@]}" "$@"
