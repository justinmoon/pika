# V2 Roadmap (Deferred)

Status: deferred until v1 freeze checklist passes (target gate: March 12, 2026).

Authority:
- Track status is controlled by `../index.md`.
- If this file conflicts with `../index.md`, `../index.md` wins.

## 1. Entry Gate

Start v2 only when all are true:
1. v1 freeze checklist is complete.
2. Fly teardown leak regression is acceptable in soak testing.
3. Lease expiry/reaper behavior is stable and observable.

## 2. Phase A: Additive Wire Extension

1. Add v2-only message families:
- `agent.lease.status.v0`
- `agent.checkpoint.v0`
- optional capability/distribution families if needed by implementation
2. Preserve v1 families unchanged.
3. Add compatibility tests for mixed v1/v2 peers.

## 3. Phase B: Distribution Layer

1. Add distribution manifests (`distribution_ref`, presets, bounded overrides).
2. Keep default order flow distribution-first for app UX.
3. Keep raw advanced workload mode policy-gated and off by default.

## 4. Phase C: Build Plane Boundary

1. Add explicit build service interface (submit/get/cancel).
2. Accept immutable artifacts as runtime inputs.
3. Enforce quotas, isolation, and audit trails before enabling untrusted builds broadly.

## 5. Phase D: Operability + Federation (Later)

1. Extend metrics/audit signals for distribution/build paths.
2. Add migration and rollback playbooks.
3. Treat multi-server discovery/decentralization as a separate follow-up track.
