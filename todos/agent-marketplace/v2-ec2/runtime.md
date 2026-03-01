# V2-EC2 Runtime Model

Status: active.

## Runtime Unit

A runtime is a **VM lease** with explicit lifecycle metadata:
- `runtime_id`
- `owner_pubkey`
- `provider`
- `created_at`
- `expires_at`
- `lifecycle_phase`
- provider handle (instance/vm identifiers)

## Provider Abstraction

Provider adapters implement:
1. `provision(lease_spec)`
2. `process_welcome` (if required)
3. `teardown`

Current demo provider target:
- `microvm` via existing vm-spawner + `flake_ref` workflow.

Planned provider classes:
- `ec2` Linux
- `ec2` Windows
- `ec2-mac` / dedicated mac hosts

## Bootstrap Model

Bootstrap defines how the VM is configured before it is marked ready:
- `nixos_config`: apply flake/configuration for microvm.nix path
- `cloud_init`: Linux cloud-init/user-data style
- `powershell`: Windows startup script path

For the first demo, `microvm.nix` is the only fully implemented bootstrap lane.

## What Can Go Wrong

High-risk areas:
1. Leaked infrastructure from teardown failures.
2. Lease expiry races (agent still running after billing window).
3. Privilege escalation or secret leakage from bootstrap config.
4. Cost explosions from unconstrained lease sizes/TTLs.
5. Provider API partial failures (instance terminated, volume left behind).

Required guards:
- hard TTL caps + periodic reaper
- ownership checks on all runtime/build commands
- audit log of lifecycle transitions
- bounded retries with dead-letter visibility
- deterministic teardown semantics (best-effort cleanup with explicit status)

## Deferred (Intentionally)

- generalized build service and container artifact routing
- multi-provider discovery market mechanics
- portability guarantees for checkpoints across providers
