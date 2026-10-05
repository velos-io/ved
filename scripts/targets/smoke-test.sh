#!/usr/bin/env bash
set -euo pipefail
_CLI_TARGET="${TARGET:-}"
source "$(dirname "$0")/../../config/load_config.sh"
_load_config

TARGET="${_CLI_TARGET:-$(get_prop DEPLOYMENT_TARGET compose)}"
SLOT="$(get_prop K8S_APP_SLOT 1)"

if [ "$TARGET" = "k8s" ]; then
  NS_AGENT="ns-${SLOT}"
  echo "smoke-test: checking Job in ${NS_AGENT}"
  kubectl wait --for=condition=complete job/smoke-test -n "${NS_AGENT}" --timeout=60s
  kubectl logs -n "${NS_AGENT}" -l job-name=smoke-test --tail=20
else
  echo "smoke-test: running smoke-test container"
  COMPOSE_PROFILES="core" docker compose --env-file config/.env run --rm smoke-test
fi
