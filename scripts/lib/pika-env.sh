#!/usr/bin/env bash
set -euo pipefail

# Shared env helpers for local pikachat workflows and agent demos.

set_default() {
  local key="$1"
  local value="$2"
  if [ -z "${!key:-}" ]; then
    export "$key=$value"
  fi
}

require_env() {
  local key
  for key in "$@"; do
    if [ -z "${!key:-}" ]; then
      echo "error: missing required env var: $key" >&2
      return 1
    fi
  done
}

load_local_env() {
  local root="${1:-$PWD}"
  # Reuse existing no-override dotenv loader.
  # shellcheck source=tools/lib/dotenv.sh
  source "$root/tools/lib/dotenv.sh"
  load_dotenv_no_override "$root"
}

set_agent_api_base_url_default() {
  local mode="${1:-local}"
  local default_url

  case "$mode" in
    local)
      default_url="http://127.0.0.1:8080"
      ;;
    remote-demo)
      default_url="https://api.pikachat.org"
      ;;
    *)
      echo "error: unsupported agent api base-url mode: $mode" >&2
      return 1
      ;;
  esac

  if [ -z "${PIKA_AGENT_API_BASE_URL:-}" ]; then
    if [ -n "${PIKA_SERVER_URL:-}" ]; then
      export PIKA_AGENT_API_BASE_URL="$PIKA_SERVER_URL"
    else
      export PIKA_AGENT_API_BASE_URL="$default_url"
    fi
  fi
}

resolve_agent_api_nsec() {
  if [ -n "${PIKA_AGENT_API_NSEC:-}" ]; then
    export PIKA_AGENT_API_NSEC
    return 0
  fi

  if [ -n "${PIKA_TEST_NSEC:-}" ]; then
    export PIKA_AGENT_API_NSEC="$PIKA_TEST_NSEC"
    return 0
  fi

  # Preserve the legacy alias for older local demo shells.
  if [ -n "${AGENT_API_NSEC:-}" ]; then
    export PIKA_AGENT_API_NSEC="$AGENT_API_NSEC"
  fi
}

require_agent_api_nsec() {
  resolve_agent_api_nsec
  if [ -z "${PIKA_AGENT_API_NSEC:-}" ]; then
    echo "PIKA_AGENT_API_NSEC (or PIKA_TEST_NSEC / AGENT_API_NSEC) is required." >&2
    return 1
  fi
}

set_agent_microvm_backend_default() {
  local default_backend="${1:-acp}"
  set_default "PIKA_AGENT_MICROVM_BACKEND" "$default_backend"
}

set_agent_vm_provider_default() {
  local provider="${1:-incus}"
  set_default "PIKA_AGENT_VM_PROVIDER" "$provider"
}

set_agent_runtime_defaults() {
  local default_kind="${1:-pi}"
  set_default "PIKA_AGENT_MICROVM_KIND" "$default_kind"
  case "${PIKA_AGENT_MICROVM_KIND}" in
    openclaw)
      set_agent_microvm_backend_default native
      ;;
    *)
      set_agent_microvm_backend_default acp
      ;;
  esac
}

set_agent_incus_lane_defaults() {
  set_agent_vm_provider_default incus
  set_default "PIKA_AGENT_INCUS_ENDPOINT" "https://pika-build:8443"
  set_default "PIKA_AGENT_INCUS_PROJECT" "pika-managed-agents"
  set_default "PIKA_AGENT_INCUS_PROFILE" "pika-agent-dev"
  set_default "PIKA_AGENT_INCUS_STORAGE_POOL" "default"
  set_default "PIKA_AGENT_INCUS_IMAGE_ALIAS" "pika-agent/dev"
}
