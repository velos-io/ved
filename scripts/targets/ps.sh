#!/usr/bin/env bash
set -euo pipefail
_CLI_TARGET="${TARGET:-}"
source "$(dirname "$0")/../../config/load_config.sh"
_load_config

TARGET="${_CLI_TARGET:-$(get_prop DEPLOYMENT_TARGET compose)}"
SLOT="$(get_prop K8S_APP_SLOT 1)"
SUFFIX="$(get_prop INGRESS_SUFFIX app.localapp)"

if [ "$TARGET" = "k8s" ]; then
  echo "── Kubernetes pods (ns-${SLOT}) ──────────────────────────"
  kubectl get pods -n "ns-${SLOT}" 2>/dev/null || echo "  (namespace not found)"
  echo ""
  echo "── Service URLs ─────────────────────────────────────────"
  echo "  lambda-emulator: http://app${SLOT}.${SUFFIX}/health"
  echo "  otel-collector:  http://otel.${SUFFIX}"
else
  echo "── Docker Compose status ────────────────────────────────"
  COMPOSE_PROFILES="*" docker compose --env-file config/.env ps
  echo ""
  echo "── Service URLs ─────────────────────────────────────────"
  echo "  lambda-emulator: http://localhost:8080/health"
  echo "  postgres:        localhost:5432"
fi
