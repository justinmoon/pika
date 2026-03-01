# V2-EC2 Concrete Tasks

Status: active implementation list.

## 1. Schema + CLI Surface

1. Extend control-plane schema with additive VM lease intent fields:
- os family
- display mode
- bootstrap kind/reference
- optional image reference and disk size
2. Add `ec2` provider enum value (backward-compatible additive change).
3. Expose provider and lease intent fields in CLI `pikachat agent new`.

Target files:
- `crates/pika-agent-control-plane/src/lib.rs`
- `cli/src/main.rs`
- `crates/pika-agent-microvm/src/lib.rs`

## 2. Server Provider Path

1. Add EC2 provider adapter scaffold implementing existing adapter trait.
2. For first demo, route EC2 scaffold to microvm-compatible provisioning backend.
3. Include lease intent metadata in runtime descriptor payload for observability.

Target files:
- `crates/pika-server/src/agent_control.rs`

## 3. Lifecycle Hardening

1. Verify runtime records carry lease timestamps and ownership.
2. Ensure teardown path remains deterministic for both microvm and scaffolded ec2 provider.
3. Reaper behavior must terminate expired leases and persist state.

Target files:
- `crates/pika-server/src/agent_control.rs`
- related tests in same module

## 4. Test Coverage

1. Serialization round-trips for new schema fields and provider kind.
2. CLI parse tests for `--provider ec2` and lease intent flags.
3. Server routing tests proving `ProviderKind::Ec2` resolves adapter path.
4. Existing agent control tests remain green.

Target commands:
- `cargo test -p pika-agent-control-plane`
- `cargo test -p pikachat --bin pikachat`
- `cargo test -p pika-server agent_control::tests::`

## 5. Deferred Explicitly

- real AWS API integration
- Windows/macOS runtime provisioning execution
- headed remote display transport implementation
