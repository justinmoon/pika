#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/lib/pika-env.sh
source "$ROOT/scripts/lib/pika-env.sh"

load_local_env "$ROOT"

# Remote chat demos intentionally default to the hosted pika-server unless callers
# explicitly point PIKA_AGENT_API_BASE_URL or PIKA_SERVER_URL elsewhere.
set_agent_api_base_url_default remote-demo
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

STATE_DIR="${PIKA_AGENT_DEMO_STATE_DIR:-$ROOT/.tmp/agent-cli-e2e}"
LISTEN_TIMEOUT="${PIKA_AGENT_DEMO_LISTEN_TIMEOUT:-90}"
POLL_ATTEMPTS="${PIKA_AGENT_DEMO_POLL_ATTEMPTS:-45}"
POLL_DELAY_SEC="${PIKA_AGENT_DEMO_POLL_DELAY_SEC:-2}"
MESSAGE="${*:-CLI demo check: reply with ACK and one short sentence.}"

echo "Agent chat API base URL: $PIKA_AGENT_API_BASE_URL"
echo "Agent chat microVM backend: $PIKA_AGENT_MICROVM_BACKEND"
echo "Agent chat kind: $PIKA_AGENT_MICROVM_KIND"

rm -rf "$STATE_DIR"
mkdir -p "$STATE_DIR"

echo "Running live agent chat demo (ensure/reuse + wait + send + listen)..."
exec "$ROOT/scripts/pikachat-cli.sh" \
  --state-dir "$STATE_DIR" \
  agent chat \
  "$MESSAGE" \
  --listen-timeout "$LISTEN_TIMEOUT" \
  --poll-attempts "$POLL_ATTEMPTS" \
  --poll-delay-sec "$POLL_DELAY_SEC"
