# V2-EC2 Wire Contract

Status: active.

## Contract Goal

Expose a lean lease-centric wire contract that can run:
- `microvm.nix` Linux today
- EC2-like Linux/macOS/Windows later
- headed or headless modes via additive fields

## Canonical Families (v0)

- `agent.offer.v0`
- `agent.order.create.v0`
- `agent.order.status.v0`
- `agent.order.result.v0`
- `agent.order.error.v0`
- `agent.lease.command.v0`
- `agent.lease.status.v0`
- `agent.checkpoint.v0`

## Lease Spec Shape (Order Payload)

`agent.order.create.v0` carries a `lease_spec` object:

- `provider_class`: `microvm` | `ec2` (future adapters can be added)
- `os_family`: `linux` | `macos` | `windows`
- `display_mode`: `headless` | `headed`
- `image_ref`: optional immutable image identifier
- `bootstrap`:
- `kind`: `nixos_config` | `cloud_init` | `powershell`
- `ref`: optional config reference (flake/configuration path, script ref, etc.)
- `machine`:
- `vcpu`
- `memory_mb`
- `disk_gb`
- `requested_ttl_sec`
- `relay_urls`

Compatibility:
- All fields above are additive extensions for current ACP control envelopes.
- Unknown optional fields must be ignored by older servers.

## Lease Commands

`agent.lease.command.v0` supports:
- `status_query`
- `terminate`
- `extend` (optional server policy)
- `checkpoint_now` (optional)

## Security + Ownership

- Non-offer payloads remain encrypted.
- Every lease command validates owner pubkey.
- Idempotency remains keyed by `(client_pubkey, idempotency_key)`.
