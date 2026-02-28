# Simple Wire Contract (Experiment)

Status: archived reference (superseded by v2 canonical wire contract).

## 1. Keep Only Essential Message Families

Use only:
- `agent.offer.v0`
- `agent.order.create.v0`
- `agent.order.status.v0`
- `agent.order.result.v0`
- `agent.order.error.v0`
- `agent.lease.command.v0`

Not in this track:
- `agent.lease.status.v0`
- `agent.checkpoint.v0`
- capability/distribution families

## 2. Required Fields (Minimal)

### `agent.order.create.v0`
- `request_id`
- `idempotency_key`
- `offer_id`
- `protocol` = `acp`
- `requested_ttl_sec`
- `relay_urls`

### `agent.order.status.v0`
- `request_id`
- `status_seq`
- `phase` in:
  - `queued`
  - `provisioning`
  - `readying` (optional; use only if needed)

### `agent.order.result.v0`
- `request_id`
- `lease_id`
- `bot_pubkey`
- `expires_at`

### `agent.order.error.v0`
- `request_id`
- `error_code`
- `hint`
- `retryable`

### `agent.lease.command.v0`
- `terminate`
- `status_query` (optional)

## 3. Versioning Rule During Experiment

- Additive and breaking changes are allowed during experiment window.
- Keep a short change log in commit messages.
- At freeze date, cut a stable schema snapshot and stop breaking changes.
