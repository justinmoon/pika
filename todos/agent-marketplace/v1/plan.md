# 2-Week Plan (Experiment -> Freeze)

Window:
- Start: February 26, 2026
- Freeze target: March 12, 2026

## Week 1 (Feb 26 - Mar 4)

1. Implement real Fly teardown.
2. Add minimal lease fields (`created_at`, `expires_at`).
3. Add basic expiry reaper with retry metadata.
4. Add deterministic tests for Fly/MicroVM teardown outcomes.

## Week 2 (Mar 5 - Mar 12)

1. Stabilize wire fields (no more churn after freeze decision).
2. Harden error codes and retry semantics.
3. Run focused soak tests for runtime leak detection.
4. Decide freeze vs second experiment.

## Freeze Criteria (Must Pass)

1. No known leaking Fly runtimes in normal and failure teardown paths.
2. Expired runtimes are cleaned automatically by reaper.
3. Wire contract is documented and consistent with implementation.
4. End-to-end provision -> chat -> teardown works for Fly + MicroVM.

## If Criteria Fail by March 12, 2026

Run one additional short experiment (max 1 week):
1. Keep wire changes minimal and intentional.
2. Fix only blockers for lifecycle reliability.
3. Re-freeze with explicit diff from this track.
