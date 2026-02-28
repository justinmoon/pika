# V2 Implementation Map (Phases 0-A-B-C)

Status: implemented on `marketplace-v2` as the canonical always-on control-plane path.

## 1. File-Level Map

### Wire/schema files touched
- `crates/pika-agent-control-plane/src/lib.rs`
  - Added additive V2 commands:
    - `get_capabilities`
    - `resolve_distribution`
    - `submit_build`
    - `get_build`
    - `cancel_build`
  - Added additive `ProvisionCommand` fields:
    - `build_id`
    - `artifact_ref`
    - `advanced_workload_json`
  - Preserved existing V1 envelope schemas and kinds.

### Server handler/runtime files touched
- `crates/pika-server/src/agent_control.rs`
  - Added handlers for capability/distribution/build commands on the always-on v2 control-plane path.
  - Added build-plane boundary interface (`BuildServiceAdapter`) with default implementation.
  - Integrated `build -> artifact -> runtime provision` flow via `build_id`/`artifact_ref`.
  - Added source-build isolation checks, rate/concurrency quotas, and audit log persistence.
  - Added artifact cache state + GC maintenance on reaper ticks.
  - Added state migration backfills for new build/artifact persistence fields.

### Provider client files touched
- `crates/pika-server/src/agent_clients/fly_machines.rs`
  - `create_machine` now accepts an optional image override.
  - Fly provisioning can consume immutable OCI artifact refs as image inputs.

### CLI files touched
- `cli/src/main.rs`
  - Added agent v2 subcommands:
    - `get-capabilities`
    - `resolve-distribution`
    - `submit-build`
    - `get-build`
    - `cancel-build`

### Test files touched
- `crates/pika-agent-control-plane/src/lib.rs` (serde round-trip coverage for new command family)
- `crates/pika-server/src/agent_control.rs`
  - Added tests for always-on v2 command availability, source policy/advanced-mode checks, rate limits, cancel behavior, build->provision integration, ownership side-effect safety, and artifact GC.
- `crates/pika-server/src/agent_clients/fly_machines.rs`
  - Added image-override contract test.
- `cli/src/main.rs`
  - Added clap parse tests for new agent subcommands.

## 2. Compatibility Matrix

### `v1 client` <-> `v2 server`
- Existing V1 commands remain unchanged and continue to work.
- V2 commands are additive and optional.
- V2 command families are always available (no `*_disabled` phase errors).

### `v2 client` <-> `v1 server`
- V2 client must feature-detect and fall back to V1 commands.
- Build/distribution/capability commands are optional; V1 provision/list/get/teardown remain usable.

### Mixed family presence/absence behavior
- V2 command presence does not alter V1 command semantics.
- `build_id`/`artifact_ref` fields are additive; omitting them preserves prior provision behavior.

## 3. Rollout Flags and Defaults

V2 command surface is always enabled (no phase flags for capabilities/distribution/build).

Risky-operation flag:
- `PIKA_AGENT_CONTROL_V2_ADVANCED_WORKLOAD_ENABLED` (default `0`)
- Governs source-build submission path only.

Operational policy env:
- `PIKA_AGENT_CONTROL_BUILD_ALLOWED_SOURCE_PREFIXES`
- `PIKA_AGENT_CONTROL_BUILD_MAX_ACTIVE`
- `PIKA_AGENT_CONTROL_BUILD_MAX_SUBMISSIONS_PER_HOUR`
- `PIKA_AGENT_CONTROL_BUILD_MAX_CONTEXT_BYTES`
- `PIKA_AGENT_CONTROL_BUILD_DEFAULT_TIMEOUT_SECS`
- `PIKA_AGENT_CONTROL_BUILD_MAX_TIMEOUT_SECS`
- `PIKA_AGENT_CONTROL_BUILD_ARTIFACT_TTL_SECS`
- `PIKA_AGENT_CONTROL_AUDIT_MAX_ENTRIES`
