#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/lib/pika-env.sh
source "$ROOT/scripts/lib/pika-env.sh"

load_local_env "$ROOT"
set_agent_api_base_url_default local
require_agent_api_nsec
export PIKA_AGENT_MICROVM_KIND="${PIKA_AGENT_MICROVM_KIND:-pi}"
case "$PIKA_AGENT_MICROVM_KIND" in
  openclaw)
    set_agent_microvm_backend_default native
    ;;
  *)
    set_agent_microvm_backend_default acp
    ;;
esac

echo "Agent ensure API base URL: $PIKA_AGENT_API_BASE_URL"
echo "Agent ensure microVM backend: $PIKA_AGENT_MICROVM_BACKEND"
echo "Agent ensure kind: $PIKA_AGENT_MICROVM_KIND"
echo "Running agent HTTP ensure demo..."
exec "$ROOT/scripts/pikachat-cli.sh" agent new "$@"
