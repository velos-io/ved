#!/usr/bin/env bash
# config/load_config.sh — source this file; never execute it directly.
#
# Provides three utilities:
#   _load_config       — loads the three env files in order (later wins)
#   get_prop KEY [DEF] — returns value of KEY from env, or DEF if unset/empty
#   safe_curl [FLAGS] URL — curl wrapper that bypasses proxy for k8s ingress
#
# Usage in every script:
#   source "$(dirname "$0")/../../config/load_config.sh"
#   _load_config
#   TARGET="$(get_prop DEPLOYMENT_TARGET compose)"

# Guard: prevent double-sourcing (would break later-wins load order)
if [ "${_CONFIG_LOADED:-}" = "1" ]; then
  return 0 2>/dev/null || true
fi

# Resolve the config directory relative to this file's location
_CONFIG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
_REPO_DIR="$(dirname "$_CONFIG_DIR")"

_parse_env_file() {
  local _f="$1"
  [ -f "$_f" ] || return 0
  while IFS='=' read -r _k _v; do
    [[ "$_k" =~ ^[[:space:]]*# ]] && continue
    [[ -z "${_k// /}" ]] && continue
    _v="${_v%%#*}"
    _v="${_v#\"}" && _v="${_v%\"}"
    _v="${_v#\'}" && _v="${_v%\'}"
    _v="${_v#"${_v%%[![:space:]]*}"}"
    _v="${_v%"${_v##*[![:space:]]}"}"
    export "$_k"="$_v"
  done < "$_f"
}

_load_config() {
  set -a  # auto-export every variable assigned from here

  # Load order: .generated.env first (spec defaults and VELOS_ENV_FILES)
  _parse_env_file "${_CONFIG_DIR}/.generated.env"

  # Determine cascade list from VED_ENV_FILES / VELOS_ENV_FILES or default files
  local _env_files=()
  local _files_to_check="${VED_ENV_FILES:-${VELOS_ENV_FILES:-}}"
  if [ -n "$_files_to_check" ]; then
    for _rel in $_files_to_check; do
      if [ -f "${_REPO_DIR}/${_rel}" ]; then
        _env_files+=("${_REPO_DIR}/${_rel}")
      elif [ -f "${_CONFIG_DIR}/${_rel}" ]; then
        _env_files+=("${_CONFIG_DIR}/${_rel}")
      elif [ -f "$_rel" ]; then
        _env_files+=("$_rel")
      fi
    done
  else
    _env_files=("${_CONFIG_DIR}/.env" "${_CONFIG_DIR}/.local.env")
  fi

  for _f in "${_env_files[@]}"; do
    _parse_env_file "$_f"
  done

  set +a
  export _CONFIG_LOADED=1
}

# get_prop KEY [DEFAULT]
# Returns the value of KEY from the current environment, or DEFAULT if unset/empty.
get_prop() {
  local _key="$1"
  local _default="${2:-}"
  local _val="${!_key:-}"
  echo "${_val:-$_default}"
}

# safe_curl [FLAGS] URL
# curl wrapper for k8s ingress: when APP_BE_HOST is set, bypasses the corporate
# proxy for 127.0.0.1 and injects a Host: header so nginx routes correctly.
safe_curl() {
  local _url="${@: -1}"

  if [ -n "${APP_BE_HOST:-}" ]; then
    # Replace hostname in URL with 127.0.0.1; inject Host header
    local _host
    _host="$(echo "$_url" | sed -E 's|https?://([^/]+).*|\1|')"
    local _path
    _path="$(echo "$_url" | sed -E 's|https?://[^/]+(/.*)|\1|')"
    local _scheme
    _scheme="$(echo "$_url" | sed -E 's|([a-z]+)://.*|\1|')"
    local _rewritten="${_scheme}://127.0.0.1${_path}"

    # Unset proxy for local k8s traffic
    unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY
    if [ $# -gt 1 ]; then
      curl -H "Host: ${_host}" "${@:1:$#-1}" "$_rewritten"
    else
      curl -H "Host: ${_host}" "$_rewritten"
    fi
  else
    curl "$@"
  fi
}
