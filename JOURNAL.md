# Journal

## 2026-02-26

### Validation blockers encountered

1. `cargo test -p pika-server` (full package) fails in DB-backed model tests due missing local Postgres socket.
- Failing tests:
  - `models::test::test_register`
  - `models::test::test_register_update`
  - `models::test::test_subscribe_groups`
- Error: `connection to server on socket .../crates/pika-server/.pgdata/.s.PGSQL.5432 failed: No such file or directory`.
- Agent-control and Fly teardown tests pass; this blocker is infra-only for model tests.

2. `cargo check --workspace` fails in this environment while building `openh264-sys2`.
- Failure occurs in C/C++ compile stage with `clang++ --target=arm64-apple-macosx` under Nix wrapper.
- Representative errors include unknown integer typedefs (`int64_t`, `uint8_t`, `uint64_t`) and SDK header failures.
- This appears to be a toolchain/environment issue unrelated to the agent marketplace changes.

### V2 scope notes (marketplace-v2 branch)

- Implemented in this branch:
  - Phase 0 concretization artifact: `todos/agent-marketplace/v2/implementation-map.md` (updated through Phase C).
  - Additive V2 wire command support in ACP schema:
    - `get_capabilities`
    - `resolve_distribution`
    - `submit_build`
    - `get_build`
    - `cancel_build`
  - Additive provision fields:
    - `build_id`
    - `artifact_ref`
    - `advanced_workload_json`
  - Server handlers for capability publication + distribution resolution, both behind rollout flags:
    - `PIKA_AGENT_CONTROL_V2_CAPABILITIES_ENABLED`
    - `PIKA_AGENT_CONTROL_V2_DISTRIBUTION_ENABLED`
    - `PIKA_AGENT_CONTROL_V2_BUILD_ENABLED`
    - `PIKA_AGENT_CONTROL_V2_ADVANCED_WORKLOAD_ENABLED`
    - Historical note: this rollout-flag snapshot was superseded by the 2026-02-27 "V2 canonical path switch" entry (phase flags removed; advanced-workload safety gate retained).
  - Build-plane boundary implemented:
    - explicit build service interface (`submit/get/cancel`) with persisted build records
    - immutable artifact handling
    - build-to-provision integration (`build_id`/`artifact_ref` resolved into runtime provisioning)
  - Security/operability hardening implemented:
    - source build isolation via allowlisted prefixes
    - quota/rate limits for build submissions
    - persisted audit trail ring buffer
    - artifact cache lifecycle with GC during maintenance ticks
    - backward-compatible state migration defaults for new persisted fields
  - CLI surface added for v2 commands:
    - `agent get-capabilities`
    - `agent resolve-distribution`
    - `agent submit-build`
    - `agent get-build`
    - `agent cancel-build`
  - Deterministic tests added for:
    - build command flag gating
    - source policy/advanced mode enforcement
    - rate limiting
    - pending build cancellation
    - build->provision artifact metadata flow
    - artifact GC behavior

- Re-validated on `marketplace-v2`: blockers above remain unchanged (`pika-server` DB tests require local Postgres socket; workspace check fails at `openh264-sys2` toolchain build).

## 2026-02-27

### V2 canonical path switch

- Removed v1-vs-v2 phase gating for control commands.
  - `get_capabilities`, `resolve_distribution`, `submit_build`, `get_build`, `cancel_build` are now always available when requester passes provision allowlist policy.
  - Removed error paths:
    - `v2_capabilities_disabled`
    - `v2_distribution_disabled`
    - `v2_build_disabled`
- Retained operational safety controls:
  - `provision_unauthorized` allowlist enforcement
  - advanced workload toggle for source builds (`PIKA_AGENT_CONTROL_V2_ADVANCED_WORKLOAD_ENABLED`)
  - build quotas/rate/context limits
  - immutable artifact reference validation
  - source allowlist policy

### Security hardening and bug fix

- Fixed ownership side-effect bug by moving owner checks before refresh/poll behavior:
  - provision path using `build_id`
  - `get_build`
- Unauthorized callers no longer trigger build state transitions, artifact creation, or persistence writes.
- Hardened source allowlist matching:
  - replaced naive raw `starts_with` checks
  - source refs and allowlist entries are parsed and normalized (`git+` prefix, scheme, host, port, path segments)
  - host + path segment boundary matching prevents prefix-confusion bypasses.
  - normalized `.`/`..` path segments so traversal-like path tricks cannot bypass allowlist boundaries.

### Test coverage additions

- Added explicit always-on availability tests for:
  - `get_build`
  - `cancel_build`
- Extended source allowlist confusion coverage to include path traversal-style ref input (`.../repo/../evil`).

### Docs alignment

- Updated `todos/agent-marketplace/index.md` to mark v2 as active and v1 as archived/reference.
- Marked `todos/agent-marketplace/v1/overview.md` as archived reference.
- Updated `todos/agent-marketplace/v2/tasks.md` to active/canonical status.
- Follow-up consistency pass:
  - Marked `todos/agent-marketplace/v2/wire.md`, `v2/runtime.md`, `v2/roadmap.md`, and `v2/fly-teardown.md` as active canonical v2 docs.
  - Marked `todos/agent-marketplace/v2/distribution.md` as active v2 design reference.
  - Marked `todos/agent-marketplace/v1/wire.md`, `v1/runtime.md`, `v1/tasks.md`, and `v1/plan.md` as archived reference/history.
