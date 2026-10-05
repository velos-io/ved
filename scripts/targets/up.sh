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
  NS_AGENT="ns-${SLOT}"

  echo "up: applying kustomize overlays (infra -> tools -> agent)"

  kubectl create namespace "$NS_INFRA" --dry-run=client -o yaml | kubectl apply -f -
  kubectl create namespace "$NS_TOOLS" --dry-run=client -o yaml | kubectl apply -f -
  kubectl create namespace "$NS_AGENT" --dry-run=client -o yaml | kubectl apply -f -

  kubectl apply -k "installer/k8s/overlays/local/${NS_INFRA}"
  kubectl apply -k "installer/k8s/overlays/local/${NS_TOOLS}"
  kubectl apply -k "installer/k8s/overlays/local/${NS_AGENT}"

  echo "up: waiting for postgres rollout in $NS_INFRA"
  kubectl rollout status deployment/postgres -n "$NS_INFRA" --timeout=120s

  echo "up: waiting for lambda-emulator rollout in $NS_AGENT"
  kubectl rollout status deployment/lambda-emulator -n "$NS_AGENT" --timeout=120s

  echo "up: waiting for api-gateway rollout in $NS_AGENT"
  kubectl rollout status deployment/api-gateway -n "$NS_AGENT" --timeout=120s
else
  # Docker Compose — always-on core profile; add otel if OTEL_ENABLED=true
  PROFILES="core"
  [ "$(get_prop OTEL_ENABLED false)" = "true" ] && PROFILES="${PROFILES},otel"
  echo "up: starting compose with profiles=${PROFILES}"
  COMPOSE_PROFILES="$PROFILES" docker compose --env-file config/.env up -d --build --no-recreate
fi
