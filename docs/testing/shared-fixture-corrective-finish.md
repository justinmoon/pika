---
summary: Canonical shared-fixture corrective finish reference
read_when:
  - evaluating strict-vs-shared rollout boundaries
  - deciding whether a lane/profile can run shared fixture mode
  - reviewing promotion evidence and rollback rules for shared fixture defaults
---

# Shared Fixture Corrective Finish Reference

This document is the canonical source for strict-vs-shared capability status, rollout boundaries, and promotion evidence rules for the shared-fixture corrective finish effort.

## Capability Matrix (Step 1)

Status legend:
- `SharedSupported`: shared mode is the documented default for this target.
- `StrictOnly`: shared mode is not allowed for this target in the current cycle.
- `Experimental`: shared mode exists only as a bounded validation path and must not be treated as a default.

| Target | Profile / Selector Scope | Status | Notes |
| --- | --- | --- | --- |
| Local deterministic CLI selectors | `integration_deterministic::{cli_smoke_local,cli_smoke_media_local}` | `StrictOnly` | Canonical deterministic lanes remain strict while corrective gates are incomplete. |
| Deterministic boundary/interop selectors | `integration_deterministic::{post_rebase_invalid_event_rejection_boundary,post_rebase_logout_session_convergence_boundary,interop_rust_baseline}` | `StrictOnly` | Boundary and interop deterministic contracts remain strict-only in this cycle. |
| Local deterministic OpenClaw selectors | `integration_deterministic::openclaw_scenario_*` | `StrictOnly` | Shared default is rolled back pending explicit parity/isolation/reliability evidence. |
| Local deterministic UI selectors | `integration_deterministic::{ui_e2e_local_android,ui_e2e_local_ios,ui_e2e_local_desktop}` | `StrictOnly` | Heavy deterministic fixtures remain strict by default. |
| OpenClaw gateway E2E selector | `integration_openclaw::openclaw_gateway_e2e` | `StrictOnly` | No shared-mode promotion in this corrective cycle. |
| Public relay selectors | `integration_public::ui_e2e_public_*`, `integration_public::deployed_bot_call_flow` | `StrictOnly` | Nondeterministic/public-network flows are outside shared-mode promotion scope. |
| Primal interop selector | `integration_primal::primal_nostrconnect_smoke` | `StrictOnly` | Nightly interop remains strict pending dedicated shared evidence. |
| Manual runbook selectors | `integration_manual::{manual_interop_rust_runbook_contract,manual_primal_lab_runbook_contract}` | `StrictOnly` | Manual selectors remain strict-only contracts. |
| Shared fixture infra validation (candidate) | Relay + MoQ + Postgres shared infra validation in deterministic harness | `Experimental` | Allowed only as explicit validation runs with recorded evidence artifacts. |

Current default policy: no lane/profile is `SharedSupported` yet in this finish cycle.
