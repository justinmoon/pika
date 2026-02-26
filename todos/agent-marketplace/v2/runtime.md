# Runtime Abstractions Spec (V2 Target Internal Model)

Status: deferred reference for post-v1 freeze; use v1 docs for active implementation decisions.

## 1. Scope

This spec defines server-internal runtime orchestration in `pika-server`.

Design intent:
- backend-neutral internal model
- lifecycle-first reliability model
- wire schema remains minimal and backend-agnostic

## 2. Current Code Snapshot (From `marmot-followups`)

Current implementation facts:
- `ProtocolKind` is ACP-only.
- `ProviderKind` is Fly + MicroVM.
- No active workers/cloudflare runtime provider path.
- Control-plane runtime phases currently include `Queued/Provisioning/Ready/Failed/Teardown`.
- Fly provision creates machine+volume, but teardown remains manual/advisory.
- MicroVM teardown performs real delete via vm-spawner.

This spec defines the target internal model that should be implemented in staged increments from that baseline.

## 3. Boundary (Wire vs Internal)

### 3.1 Wire-facing
Wire payloads remain backend-agnostic:
- order/status/result/error
- lease command/status

Wire schema MUST NOT require backend/workload internals.

### 3.2 Server-internal
Server owns:
- `ProvisionCommand`
- `RuntimeLease`
- `RuntimeRecord`
- `WorkloadSpec` (resolved internal form)
- `ExecutionBackend` contract
- lifecycle reaper and reconciliation loops

## 4. Canonical Internal Primitives

### 4.1 Workload Model

`WorkloadKind`:
- `OciRuntime`
- `NixAutostart`

`BuildSpec` (optional pre-provision):
- `None`
- `DockerBuild { source, dockerfile, args, target }`
- `NixBuild { source, flake_ref, attr_or_shell }`

`WorkloadSpec` (resolved for provisioning):
- `OciWorkload { image_ref, command, env, mounts }`
- `NixWorkload { flake_ref, dev_shell, autostart, env, files }`

Rule:
- provisioning consumes one validated, resolved `WorkloadSpec`.

### 4.2 Runtime Lease

Each runtime stores:
- `created_at`
- `expires_at`
- `grace_until` (optional)
- `last_activity_at` (optional)
- `max_idle_seconds` (optional)
- `teardown_policy`
- retry metadata for teardown failures

### 4.3 Backend Contract

Backend interface (conceptual):
- `provision(cmd) -> ProvisionedRuntime`
- `readiness(runtime) -> ReadinessState`
- `process_welcome(runtime, welcome) -> Result`
- `teardown(runtime) -> TeardownOutcome`
- `list_owned_resources(scope) -> [ExternalResource]`

Teardown outcomes must be structured and idempotent.

## 5. Lifecycle Model

Target phases:
- `Queued`
- `Provisioning`
- `WaitingForKeyPackage`
- `Ready`
- `Failed`
- `TearingDown`
- `Terminated`

Transition rules:
- `Ready -> TearingDown` on manual terminate, expiry, or idle timeout.
- `TearingDown -> Terminated` after cleanup success or resource confirmed absent.
- teardown failures stay retryable with persisted backoff metadata.

Implementation note:
- current code may keep `Teardown` as terminal placeholder until `Terminated` is added.

## 6. Reaper and Reconciliation

Two loops are required:

1. Lease reaper (short interval)
- expire/idle scan
- trigger teardown
- persist retry/backoff metadata

2. External reconciliation (longer interval)
- list backend resources
- detect orphans against runtime state
- delete orphans
- emit audit events

## 7. Build Plane Separation

Build and runtime orchestration should be separate:
- build plane: source/build -> immutable artifact
- runtime plane: artifact -> lease runtime

This is required before broad untrusted build enablement.

## 8. Staged Implementation Plan

Phase A (immediate):
- implement real Fly teardown
- add teardown idempotency handling

Phase B:
- add lease metadata persistence
- add reaper loop + retry persistence

Phase C:
- add reconciliation loop and orphan cleanup

Phase D:
- integrate distribution/build-plane model (separate draft)

## 9. Acceptance Checks

1. Wire schema stays backend-agnostic.
2. Deterministic mapping tests for request/policy -> provision + lease.
3. Lifecycle tests cover expiry teardown path.
4. Reaper restart test resumes pending teardown.
5. Reconciliation test removes orphans.
6. Fly and MicroVM teardown contract tests are deterministic.
