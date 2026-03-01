# V2-EC2 Roadmap

Status: active.

## Phase A (Now): Microvm-Nix Lease Core

1. Add lease intent fields for OS/display/bootstrap to control-plane schema.
2. Add EC2 provider scaffold while routing demo workloads to existing microvm runtime path.
3. Keep ACP-only control-plane behavior.
4. Harden lifecycle: TTL metadata, explicit teardown, reaper correctness.

Acceptance:
- User can request a microvm.nix VM lease with explicit lease intent metadata.
- Runtime is provisioned, visible, and terminates cleanly on command/expiry.

## Phase B: EC2 Linux Provider

1. Implement real EC2 adapter for Linux AMIs.
2. Support `cloud_init` bootstrap path.
3. Persist cloud handles required for deterministic teardown.
4. Add quotas and instance-type policy gates.

Acceptance:
- Same wire/API as Phase A, provider backed by actual EC2 APIs.

## Phase C: Headed + Additional OS

1. Windows lane (`powershell` bootstrap, WinRM health checks).
2. macOS lane (dedicated host provider constraints).
3. `display_mode=headed` with explicit transport policy (RDP/VNC/DCV/WebRTC).

Acceptance:
- No protocol break; additive provider capabilities only.

## Build Plane Strategy

Default position:
- no mandatory build plane for first demo.

When required later:
- run image/build workflows in a separate builder service plane
- can initially lease managed build workers instead of building in runtime hosts
- keep runtime lease API unchanged; build service only produces immutable refs
