# V2-EC2 Testing Plan

Status: active.

## Current Branch Validation

Run:
1. `cargo test -p pika-agent-control-plane`
2. `cargo test -p pikachat --bin pikachat`
3. `cargo test -p pika-server agent_control::tests::`

Manual local smoke:
1. `just pikahub`
2. `just agent-ec2-local --json`
3. `just cli --relay "$RELAY_EU" --relay "$RELAY_US" agent list-runtimes`
4. `just cli --relay "$RELAY_EU" --relay "$RELAY_US" agent teardown --runtime-id <id>`

## Suggested Post-Implementation Validation (Linux/macOS/Windows)

Linux EC2:
1. Launch small Linux AMI via provider adapter.
2. Verify bootstrap script completion and Marmot agent heartbeat.
3. Validate lease expiry triggers instance + volume cleanup.

Windows EC2:
1. Use minimal Windows AMI with bootstrap powershell stub.
2. Validate readiness via WinRM/agent heartbeat, not SSH assumptions.
3. Validate teardown and orphaned EBS cleanup.

macOS:
1. Start with provider that supports dedicated mac hosts (capacity-limited).
2. Validate longer startup timings and host allocation edge cases.
3. Validate teardown path includes host release semantics.

Headed Mode:
1. Gate behind explicit capability flag.
2. Validate remote display setup/teardown does not block base lease cleanup.
3. Ensure headed transport failures still allow command/control and termination.

## Exit Criteria Before Widening Rollout

1. No leaked leases in a 24h soak (create/expire/terminate loops).
2. Deterministic teardown observed in audit logs.
3. Ownership checks enforced for all runtime operations.
4. No protocol regression for existing ACP clients.
