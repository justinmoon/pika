# Agent Marketplace Docs

Single authority:
- This file is the source of truth for track status (`active` vs `reference`).
- If any sub-doc conflicts with this status, this file wins.

## Active Track

Implement **v2-ec2** as the canonical protocol/runtime path on this branch.

- [v2-ec2/overview.md](v2-ec2/overview.md)
- [v2-ec2/wire.md](v2-ec2/wire.md)
- [v2-ec2/runtime.md](v2-ec2/runtime.md)
- [v2-ec2/roadmap.md](v2-ec2/roadmap.md)
- [v2-ec2/tasks.md](v2-ec2/tasks.md)
- [v2-ec2/testing.md](v2-ec2/testing.md)

## Reference Tracks

Previous tracks remain for historical context and fallback ideas.

- Prior v1/v2 drafts live in earlier `marketplace-v2` worktree history.
- This rebased branch keeps only the active EC2-first track docs.

## Decision Rule

1. Use v2-ec2 docs for implementation decisions.
2. Keep ACP-only behavior and operational safety controls enabled by default.
3. Keep old v2/v1 docs for reference only unless explicitly re-promoted.
4. Treat lease lifecycle correctness (create, heartbeat/expiry, teardown, reaper) as non-negotiable foundation.

## Compatibility Rule

1. Existing v1 families remain valid during migration:
- `agent.offer.v0`
- `agent.order.create.v0`
- `agent.order.status.v0`
- `agent.order.result.v0`
- `agent.order.error.v0`
- `agent.lease.command.v0`
2. v2-ec2 keeps these families and adds lease visibility families:
- `agent.lease.status.v0`
- `agent.checkpoint.v0`
3. Keep payload changes additive through the two-week rapid-iteration window; freeze once lease model stabilizes.
