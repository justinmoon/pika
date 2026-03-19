#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/lib/pika-env.sh
source "$ROOT/scripts/lib/pika-env.sh"

load_local_env "$ROOT"
set_agent_api_base_url_default remote-demo
require_agent_api_nsec
set_agent_incus_lane_defaults
set_agent_runtime_defaults pi

echo "Agent ensure API base URL: $PIKA_AGENT_API_BASE_URL"
echo "Agent ensure provider: $PIKA_AGENT_VM_PROVIDER"
echo "Agent ensure runtime kind: $PIKA_AGENT_MICROVM_KIND"
echo "Agent ensure runtime backend: $PIKA_AGENT_MICROVM_BACKEND"
echo "Agent ensure Incus endpoint: $PIKA_AGENT_INCUS_ENDPOINT"
echo "Running Incus agent ensure demo..."
exec "$ROOT/scripts/pikachat-cli.sh" agent new "$@"
