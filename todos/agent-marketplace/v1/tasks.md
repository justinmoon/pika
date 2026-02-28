# V1 Concrete Task List (Archived)

Status: archived reference task list (v2 is canonical).
Freeze target: March 12, 2026.

## P0. Fly Teardown (Must Ship)

1. Add Fly stop/delete machine API calls.
- File: `crates/pika-server/src/agent_clients/fly_machines.rs`
- Add methods for stop machine, delete machine, delete volume.
- Handle 404 as `already_gone`.

2. Replace manual Fly teardown payload with real cleanup logic.
- File: `crates/pika-server/src/agent_control.rs`
- Update Fly adapter `teardown()` to call new client methods.
- Return structured outcome with retryability.

3. Add deterministic teardown contract tests.
- File: `crates/pika-server/src/agent_clients/fly_machines.rs`
- File: `crates/pika-server/src/agent_control.rs`
- Cases: success, already gone, volume conflict, provider error.

## P1. Lease + Expiry Reaper

1. Add lease fields to runtime persistence.
- File: `crates/pika-server/src/agent_control.rs`
- Add `created_at`, `expires_at`, retry metadata to runtime state model.
- Add migration handling for existing state file.

2. Add periodic reaper loop.
- File: `crates/pika-server/src/agent_control.rs`
- Trigger teardown for expired runtimes.
- Persist retry metadata/backoff on failure.

3. Add reaper tests.
- File: `crates/pika-server/src/agent_control.rs` (tests module)
- Cases: expire -> teardown, retry scheduling, restart resume.

## P2. Operator Reliability

1. Ensure interactive flow teardown on Ctrl-C is best-effort and explicit.
- File: `cli/src/main.rs`
- File: `cli/src/agent/session.rs` / harness path as needed.

2. Keep script mode stable.
- File: `cli/src/main.rs`
- Validate `--json` and non-interactive paths do not regress.

3. Update docs to frozen v1 shape at freeze date.
- Files: `docs/agent-marketplace/v1/*`

## Validation Commands

- `cargo test -p pika-agent-control-plane`
- `cargo test -p pika-server`
- `cargo test -p pikachat`
- `cargo check --workspace`

## Freeze Checklist (March 12, 2026)

1. No known leaking Fly runtimes in normal/failure paths.
2. Expired runtimes are cleaned automatically.
3. Wire + runtime docs match implementation.
4. E2E provision -> chat -> teardown passes for Fly + MicroVM.
