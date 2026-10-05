#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/../../config/load_config.sh"
_load_config

TARGET="$(get_prop DEPLOYMENT_TARGET compose)"
SLOT="$(get_prop K8S_APP_SLOT 1)"

if [ "$TARGET" = "k8s" ]; then
  echo "reset: deleting k8s namespaces for slot ${SLOT}"
  NS_INFRA="$(get_prop K8S_INFRA_NAMESPACE infra)"
  NS_TOOLS="$(get_prop K8S_TOOLS_NAMESPACE tools)"
  kubectl delete namespace "ns-${SLOT}" --ignore-not-found
  kubectl delete namespace "$NS_INFRA" --ignore-not-found
  kubectl delete namespace "$NS_TOOLS" --ignore-not-found
  echo "ok: namespaces deleted"
else
  echo "reset: removing all compose resources (volumes + images)"
  COMPOSE_PROFILES="*" docker compose --env-file config/.env down --remove-orphans --volumes --rmi local 2>/dev/null || true
  echo "ok: compose resources removed"
fi
