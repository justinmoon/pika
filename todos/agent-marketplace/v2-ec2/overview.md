# V2-EC2 Overview (Canonical On This Branch)

Status: active.

## Product Direction

Treat the marketplace as **leased machine infrastructure**, not container/build orchestration.

Primary user story:
1. Rent a VM lease.
2. Apply OS config (microvm.nix first, later macOS/Windows variants).
3. Start coding agent.
4. Talk to the running agent over Marmot.
5. Enforce lease expiry and hard teardown.

## Why This Direction

This removes the biggest complexity tax in early iterations:
- no required ECR/ECS build pipeline in the first demo
- no Dockerfile/Nix build API exposed to end users
- one lifecycle model: `create -> ready -> active -> expired/terminated -> cleaned`

## Scope

In scope now:
- ACP-only control plane
- lease lifecycle correctness (ownership, TTL, teardown, reaper)
- microvm.nix first-class path
- additive VM intent fields that can represent Linux/macOS/Windows and headed/headless

Out of scope for first demo:
- multi-cloud discovery
- dynamic image factory service
- full Windows/macOS provisioning implementation
- GUI streaming stack (RDP/VNC/DCV/WebRTC)

## Design Principle

Keep server capabilities broad, keep client UX opinionated.

- Server API accepts explicit VM lease parameters.
- App clients mostly use distribution/preset wrappers.
- Advanced users can still submit lower-level lease requests.
