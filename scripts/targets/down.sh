#!/usr/bin/env bash
set -euo pipefail
_CLI_TARGET="${TARGET:-}"
source "$(dirname "$0")/../../config/load_config.sh"
_load_config

TARGET="${_CLI_TARGET:-$(get_prop DEPLOYMENT_TARGET compose)}"
SLOT="$(get_prop K8S_APP_SLOT 1)"

if [ "$TARGET" = "k8s" ]; then
  NS_INFRA="$(get_prop K8S_INFRA_NAMESPACE infra)"
  NS_TOOLS="$(get_prop K8S_TOOLS_NAMESPACE tools)"
  echo "down: scaling all deployments to zero in ns-${SLOT}, ${NS_INFRA}, ${NS_TOOLS}"
  kubectl scale deployment --all -n "ns-${SLOT}" --replicas=0 2>/dev/null || true
  kubectl scale deployment --all -n "${NS_INFRA}" --replicas=0 2>/dev/null || true
  kubectl scale deployment --all -n "${NS_TOOLS}" --replicas=0 2>/dev/null || true
  kubectl delete job --all -n "ns-${SLOT}" 2>/dev/null || true
  kubectl delete job --all -n "${NS_INFRA}" 2>/dev/null || true
else
  echo "down: stopping all compose services"
  COMPOSE_PROFILES="*" docker compose --env-file config/.env down --remove-orphans
fi
