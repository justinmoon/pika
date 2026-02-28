# Draft Spec: Distribution-First Agent Marketplace

Status: active v2 distribution design reference (non-normative where explicitly marked).

Activation policy:
- Keep broad advanced/raw build support policy-gated.
- Maintain Fly teardown + lease reaper + reconciliation safety requirements before any broader source-build rollout.

## 1. Why This Layer Exists

We need three separate concerns:
1. Server flexibility (can run many runtime/build modes).
2. Simple product UX (users should not fill low-level infra forms).
3. Safe operations (untrusted build/provision requests need policy isolation).

The missing abstraction is an **Agent Distribution** between raw server capabilities and end-user order requests.

## 2. Three-Layer Model

### 2.1 Server Capabilities (infra truth)
Server advertises what it can safely support.

Examples:
- workload kinds (`oci`, `nix`)
- build support (`docker_build`, `nix_build`, none)
- resource ranges (cpu/memory/disk)
- lease/TTL bounds
- volume/network policy limits
- regions and policy flags

### 2.2 Agent Distribution (portable product)
A signed, versioned manifest defining:
- workload template
- required inputs
- defaults
- presets (`small`, `medium`, `large`)
- override policy (what users may change)
- minimum capability requirements

### 2.3 Lease Order (instance request)
Default app order sends:
- `distribution_ref` (id + version/digest)
- selected preset
- minimal overrides (optional TTL, region hint)

Advanced/raw workload submission is optional and policy-gated.

## 3. Wire Extension Direction

Keep existing v0 families stable.
Additive families (draft):
- `agent.capabilities.v0`
- `agent.distribution.v0`

Order modes (exactly one):
1. `distribution_order` (default)
2. `advanced_workload_order` (power user, optional)

Server returns deterministic validation errors if mode is invalid or disabled.

## 4. Build and Runtime Separation

### 4.1 Required boundary
- Build plane resolves source/build inputs to immutable artifact refs.
- Runtime plane provisions lease runtimes from immutable artifacts.

### 4.2 Why
This prevents runtime orchestration code from becoming an unbounded build executor and reduces blast radius.

### 4.3 Canonical artifact outputs
- `OciArtifact { image_digest }`
- `NixArtifact { closure_hash }`

## 5. What Could Go Wrong (Primary Risks)

1. Security
- build scripts exfiltrate secrets
- lateral movement via network access
- privilege escape attempts

2. Cost and abuse
- crypto-mining workloads
- giant build contexts and cache flooding
- retry storms and queue starvation

3. Determinism and supply chain
- mutable tags/inputs causing drift
- non-reproducible outputs
- poisoned dependencies

4. Lifecycle leaks
- orphaned artifacts/caches/volumes
- failed teardown and stuck resources

5. Product complexity
- exposing too many knobs to app users
- unclear failure semantics for non-technical users

## 6. What To Worry About First

1. isolation model for untrusted builds
2. policy model (allow/deny and hard limits)
3. immutable pinning/provenance
4. quotas/concurrency/timeout guardrails
5. artifact GC and teardown reliability

## 7. What Can Be Deferred

1. raw advanced mode in mobile clients
2. cross-server artifact portability
3. deep provenance framework beyond basic signed manifests + immutable refs
4. multi-region builder clusters
5. first-class repo-clone protocol fields (can remain runtime/bootstrap behavior initially)

## 8. Iteration Plan

Phase 0:
- distribution mode only
- prebuilt immutable artifacts only
- no raw build submission

Phase 1:
- trusted publisher build support (allowlist)
- builder service behind strict policy

Phase 2:
- optional advanced workload mode for desktop/dev users
- policy-gated and rate-limited

Phase 3:
- broader rollout after hard metrics gates are stable

Phase gates (must pass before entering next phase):
- Phase 0 -> 1: deterministic runtime lifecycle tests + no teardown leak regressions
- Phase 1 -> 2: builder isolation/quotas validated under load test
- Phase 2 -> 3: abuse controls + artifact GC proven in production-like soak

## 9. Builder Service Strategy

### 9.1 Should this be another service?
Yes. Recommended: separate build service boundary.

`pika-server` should submit normalized build requests and receive immutable artifacts.

### 9.2 Leased external builder as initial step
Reasonable for early iteration if:
- dedicated tenant/isolation boundary is enforced
- no production secrets are exposed to build workers
- hard resource/egress limits are still enforced by us
- we keep a provider-agnostic internal build API for later migration

### 9.3 Self-host trigger points
Move to self-hosted builders when one or more occur:
- cost exceeds target threshold
- security/compliance requirements tighten
- queue latency/SLA misses are persistent
- provider lock-in becomes a delivery risk

## 10. Minimal Build API (Draft)

`submit_build(request) -> build_id`
`get_build(build_id) -> { phase, artifact_ref?, error? }`
`cancel_build(build_id)`

Build phases:
- `queued`
- `validating`
- `fetching_source`
- `building`
- `publishing_artifact`
- `succeeded`
- `failed`
- `canceled`

## 11. Acceptance Checks (Draft)

1. Distribution order resolves deterministically to internal command + lease.
2. Policy-denied overrides return deterministic non-retryable errors.
3. Build service enforces hard time/resource/network limits.
4. Artifact refs are immutable and retained per policy.
5. GC removes expired artifacts/caches without affecting active leases.
6. Advanced mode can be globally disabled and audited.
