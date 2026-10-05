#!/usr/bin/env bash
# up-smart.sh — idempotent Docker Compose start.
# Restarts unhealthy or exited services. Skips services that are already running and healthy.
set -euo pipefail
source "$(dirname "$0")/../../config/load_config.sh"
_load_config

PROFILES="core"
[ "$(get_prop OTEL_ENABLED false)" = "true" ] && PROFILES="${PROFILES},otel"

echo "up-smart: checking current service states (profiles=${PROFILES})"

# Get all services in a parseable form
RUNNING=$(COMPOSE_PROFILES="$PROFILES" docker compose ps --format "{{.Name}}:{{.State}}:{{.Health}}" 2>/dev/null || true)

NEEDS_RESTART=()
while IFS=':' read -r name state health; do
  [ -z "$name" ] && continue
  if [ "$state" = "exited" ] || [ "$health" = "unhealthy" ]; then
    echo "  restart needed: ${name} (state=${state}, health=${health:-n/a})"
    NEEDS_RESTART+=("$name")
  else
    echo "  ok: ${name} (state=${state}, health=${health:-n/a})"
  fi
done <<< "$RUNNING"

if [ "${#NEEDS_RESTART[@]}" -gt 0 ]; then
  echo "up-smart: restarting ${#NEEDS_RESTART[@]} service(s)"
  COMPOSE_PROFILES="$PROFILES" docker compose restart "${NEEDS_RESTART[@]}"
fi

# Start any services not yet running
echo "up-smart: ensuring all services are up"
COMPOSE_PROFILES="$PROFILES" docker compose up -d --no-recreate
echo "ok: stack is up"
