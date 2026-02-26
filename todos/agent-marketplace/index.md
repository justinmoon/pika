# Agent Marketplace Docs

Single authority:
- This file is the source of truth for track status (`active` vs `deferred`).
- If any v1/v2 file conflicts with this status, this file wins.

## Active Track

Implement **v1 first**.

- [v1/overview.md](v1/overview.md)
- [v1/wire.md](v1/wire.md)
- [v1/runtime.md](v1/runtime.md)
- [v1/plan.md](v1/plan.md)
- [v1/tasks.md](v1/tasks.md)

## Next Track (After v1 Freeze)

- [v2/wire.md](v2/wire.md)
- [v2/runtime.md](v2/runtime.md)
- [v2/roadmap.md](v2/roadmap.md)
- [v2/fly-teardown.md](v2/fly-teardown.md)
- [v2/distribution.md](v2/distribution.md)
- [v2/tasks.md](v2/tasks.md)

## Decision Rule

1. Until March 12, 2026, use v1 docs for implementation decisions.
2. Treat v2 as architecture target and deferred design.
3. Promote v2 work only after v1 freeze checklist passes.

## v1 -> v2 Wire Compatibility Rule

1. v1 is a strict subset of v2 for currently implemented families.
2. v1 active families are:
- `agent.offer.v0`
- `agent.order.create.v0`
- `agent.order.status.v0`
- `agent.order.result.v0`
- `agent.order.error.v0`
- `agent.lease.command.v0`
3. v2 adds families (additive, not replacement):
- `agent.lease.status.v0`
- `agent.checkpoint.v0`
4. During v1 implementation window, do not require or depend on v2-only families.
5. At/after freeze, enable v2-only families behind explicit compatibility notes and test coverage.
