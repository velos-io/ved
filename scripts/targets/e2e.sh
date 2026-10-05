#!/usr/bin/env bash
set -euo pipefail
_CLI_TARGET="${TARGET:-}"
source "$(dirname "$0")/../../config/load_config.sh"
_load_config

TARGET="${_CLI_TARGET:-$(get_prop DEPLOYMENT_TARGET compose)}"
SLOT="$(get_prop K8S_APP_SLOT 1)"
SUFFIX="$(get_prop INGRESS_SUFFIX app.localapp)"

if [ "$TARGET" = "k8s" ]; then
  # Ingress hostname: app{slot}.{suffix} — e.g. app1.app.localapp
  # Ensure /etc/hosts or DNS resolves this to 127.0.0.1 for local k8s.
  URL="http://app${SLOT}.${SUFFIX}/health"
else
  URL="http://localhost:8080/health"
fi

echo "E2E: hitting ${URL}"
STATUS=$(safe_curl -s -o /dev/null -w "%{http_code}" --max-time 10 --retry 3 --retry-delay 5 "$URL")
[ "$STATUS" = "200" ] || { echo "FAIL: expected 200, got ${STATUS}"; exit 1; }
echo "ok: lambda-emulator healthy (HTTP 200)"
