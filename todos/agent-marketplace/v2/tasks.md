# V2 Concrete Task List

Status: active implementation queue (canonical path).

Authority:
- Active implementation queue is v2.
- Use this file as implementation and maintenance guidance.

## Phase 0. Concretization Pass (Required Before Coding)

1. Produce a file-level implementation map before writing v2 code:
- wire/schema files touched
- server handler files touched
- storage/migration files touched
- test files and fixtures touched
2. Define compatibility matrix:
- v1 client <-> v2 server
- v2 client <-> v1 server
- mixed family presence/absence behavior
3. Define rollout flags and default states for each new family.

## Phase A. Capability/Distribution Layer

1. Define additive wire families:
- `agent.capabilities.v0`
- `agent.distribution.v0`

2. Implement server capability publication.
- workload kinds, limits, policy flags, regions.

3. Implement distribution resolution path.
- `distribution_ref + preset + allowed overrides -> internal provision command`.

## Phase B. Build Plane Boundary

1. Introduce build service interface.
- submit/get/cancel build.
- immutable artifact outputs only.

2. Integrate build interface into provisioning pipeline.
- build -> artifact -> runtime provision.

3. Keep advanced raw workload mode policy-gated.

## Phase C. Security + Operability Hardening

1. Builder isolation and quota enforcement.
2. Artifact cache lifecycle + GC.
3. Abuse controls and audit trails.
4. Migration and compatibility strategy from v1.

## Current Operation

- V2 commands are treated as always-on control-plane surface (ACP-only).
- Operational safety gates (allowlist, advanced workload toggle, quotas, source allowlist, artifact immutability) remain required.
