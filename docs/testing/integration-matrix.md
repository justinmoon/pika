# Integration Test Matrix (Library-First Migration)

This matrix locks the migration scope for the library-first `pikahut::testing` rollout.

- Canonical execution model: Rust tests and Rust scenario helpers in `crates/pikahut/tests/*` and `crates/pikahut/src/testing/scenarios/*`.
- Compatibility rule: existing `just` targets and shell scripts remain callable during migration, but they dispatch into the same Rust-owned scenario/test internals.
- Tier meanings:
  - `deterministic`: required in pre-merge unless capability is unavailable.
  - `heavy`: deterministic but expensive; can be path-scoped in pre-merge and always in nightly.
  - `nondeterministic`: ignored by default; selected in nightly/manual runs.
  - `manual`: intentionally user-driven tooling, still backed by shared library primitives.

## Capability Keys

- `host-macos`: macOS runner required.
- `xcode`: Xcode + iOS simulator runtime required.
- `android`: Android SDK + emulator/AVD required.
- `openclaw-repo`: `openclaw/openclaw` checkout available.
- `interop-rust-repo`: `marmot-interop-lab-rust` checkout available.
- `primal-repo`: local/CI Primal iOS repo checkout available.
- `secret-pika-test-nsec`: `PIKA_TEST_NSEC` available.
- `public-network`: internet/public relay reachability available.

## Canonical Mapping

| Current entrypoint | Current invocation | Target Rust test/selector | Tier | Owner lane | Required capabilities | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `just cli-smoke` | `cargo run -q -p pikahut -- test cli-smoke` | `cargo test -p pikahut --test integration_deterministic cli_smoke_local` | deterministic | pre-merge-pikachat | none | Local relay fixture, no external services. |
| `just cli-smoke-media` | `cargo run -q -p pikahut -- test cli-smoke --with-media` | `cargo test -p pikahut --test integration_deterministic cli_smoke_media_local` | deterministic | pre-merge-pikachat | public-network | Uses Blossom upload path. |
| `just android-ui-e2e-local` | `cargo run -q -p pikahut -- test ui-e2e-local --platform android` | `cargo test -p pikahut --test integration_deterministic ui_e2e_local_android -- --ignored` | heavy | nightly-pika-ui-android and manual | android | Skip with explicit reason when Android tools/AVD are missing. |
| `just ios-ui-e2e-local` | `cargo run -q -p pikahut -- test ui-e2e-local --platform ios` | `cargo test -p pikahut --test integration_deterministic ui_e2e_local_ios -- --ignored` | heavy | nightly macOS/manual | host-macos, xcode | Skip with explicit reason on non-macOS/no simulator. |
| `just desktop-e2e-local` | `cargo run -q -p pikahut -- test ui-e2e-local --platform desktop` | `cargo test -p pikahut --test integration_deterministic ui_e2e_local_desktop` | deterministic | pre-merge-pikachat | none | Uses local relay+bot fixture. |
| `just interop-rust-baseline` | `cargo run -q -p pikahut -- test interop-rust-baseline` | `cargo test -p pikahut --test integration_deterministic interop_rust_baseline -- --ignored` | heavy | nightly/manual | interop-rust-repo | Skip with explicit reason if external repo missing. |
| `just interop-rust-manual` | `cargo run -q -p pikahut -- test interop-rust-baseline --manual` | `cargo test -p pikahut --test integration_manual interop_rust_manual_guidance -- --ignored` | manual | manual only | interop-rust-repo, android or xcode | Manual-mode helper backed by same scenario primitives. |
| `just openclaw-pikachat-deterministic` (invite/chat) | `pikahut test scenario invite-and-chat` | `cargo test -p pikahut --test integration_deterministic openclaw_scenario_invite_and_chat` | deterministic | pre-merge-pikachat | none | Rust scenario executed in local deterministic fixture. |
| `just openclaw-pikachat-deterministic` (rust bot) | `pikahut test scenario invite-and-chat-rust-bot` | `cargo test -p pikahut --test integration_deterministic openclaw_scenario_invite_and_chat_rust_bot` | deterministic | pre-merge-pikachat | none | Same scenario module as CLI wrapper. |
| `just openclaw-pikachat-deterministic` (daemon) | `pikahut test scenario invite-and-chat-daemon` | `cargo test -p pikahut --test integration_deterministic openclaw_scenario_invite_and_chat_daemon` | deterministic | pre-merge-pikachat | none | Same scenario module as CLI wrapper. |
| `just openclaw-pikachat-deterministic` (audio) | `pikahut test scenario audio-echo` | `cargo test -p pikahut --test integration_deterministic openclaw_scenario_audio_echo` | deterministic | pre-merge-pikachat | none | Deterministic local audio echo contract. |
| `just openclaw-pikachat-e2e` | `pikahut test openclaw-e2e` | `cargo test -p pikahut --test integration_openclaw openclaw_gateway_e2e -- --ignored` | heavy | pre-merge path-scoped + nightly-pikachat | openclaw-repo, public-network | Preserve OpenClaw config/log/state artifacts on failure. |
| `just nightly-pikachat` | `just openclaw-pikachat-e2e` | `cargo test -p pikahut --test integration_openclaw openclaw_gateway_e2e -- --ignored --nocapture` | heavy | nightly-pikachat | openclaw-repo, public-network | Nightly canonical selector. |
| `just e2e-public-relays` | `./tools/ui-e2e-public --platform all` | `cargo test -p pikahut --test integration_public ui_e2e_public_all -- --ignored` | nondeterministic | nightly/manual | secret-pika-test-nsec, public-network, android, host-macos, xcode | Unified Rust orchestration for public UI E2E. |
| `just ios-ui-e2e` | `./tools/ui-e2e-public --platform ios` | `cargo test -p pikahut --test integration_public ui_e2e_public_ios -- --ignored` | nondeterministic | nightly/manual | secret-pika-test-nsec, public-network, host-macos, xcode | iOS-only public relay path. |
| `just android-ui-e2e` | `./tools/ui-e2e-public --platform android` | `cargo test -p pikahut --test integration_public ui_e2e_public_android -- --ignored` | nondeterministic | nightly/manual | secret-pika-test-nsec, public-network, android | Android-only public relay path. |
| `just e2e-deployed-bot` | `cargo test -p pika_core --test e2e_calls call_deployed_bot -- --ignored --nocapture` | `cargo test -p pikahut --test integration_public deployed_bot_call_flow -- --ignored` | nondeterministic | nightly-pika-e2e/manual | secret-pika-test-nsec, public-network | Keep legacy `pika_core` test callable during migration. |
| `just nightly-primal-ios-interop` | `./tools/primal-ios-interop-nightly` | `cargo test -p pikahut --test integration_primal primal_nostrconnect_smoke -- --ignored` | heavy | nightly-primal-ios-interop | host-macos, xcode, primal-repo | Single confidence smoke: URL handoff + marker-file contract. |
| `just primal-ios-lab` | `./tools/primal-ios-interop-lab run` | `cargo test -p pikahut --test integration_manual primal_lab_runbook -- --ignored` | manual | manual only | host-macos, xcode, primal-repo | Manual lab remains, built on shared fixture primitives. |
| `just primal-ios-lab-patch-primal` | `./tools/primal-ios-interop-lab patch-primal` | `cargo test -p pikahut --test integration_manual primal_lab_patch_helper -- --ignored` | manual | manual only | host-macos, primal-repo | Kept out of CI by design. |
| `just primal-ios-lab-seed-capture` | `./tools/primal-ios-interop-lab seed-capture` | `cargo test -p pikahut --test integration_manual primal_lab_seed_capture -- --ignored` | manual | manual only | host-macos, xcode | Explicitly manual-only; not part of nightly CI. |
| `just primal-ios-lab-seed-reset` | `./tools/primal-ios-interop-lab seed-reset` | `cargo test -p pikahut --test integration_manual primal_lab_seed_reset -- --ignored` | manual | manual only | host-macos, xcode | Explicitly manual-only; not part of nightly CI. |
| `just primal-ios-lab-dump-debug` | `./tools/primal-ios-interop-lab dump-debug` | `cargo test -p pikahut --test integration_manual primal_lab_dump_debug -- --ignored` | manual | manual only | host-macos, xcode | Debug support retained after CI simplification. |
| `pikachat-openclaw/scripts/phase1.sh` | wrapper to `run-scenario.sh invite-and-chat` | `integration_deterministic::openclaw_scenario_invite_and_chat` | deterministic | pre-merge-pikachat | none | Script remains compatibility wrapper only. |
| `pikachat-openclaw/scripts/phase2.sh` | wrapper to `run-scenario.sh invite-and-chat-rust-bot` | `integration_deterministic::openclaw_scenario_invite_and_chat_rust_bot` | deterministic | pre-merge-pikachat | none | Script remains compatibility wrapper only. |
| `pikachat-openclaw/scripts/phase3.sh` | wrapper to `run-scenario.sh invite-and-chat-daemon` | `integration_deterministic::openclaw_scenario_invite_and_chat_daemon` | deterministic | pre-merge-pikachat | none | Script remains compatibility wrapper only. |
| `pikachat-openclaw/scripts/phase3_audio.sh` | wrapper to `pikahut test scenario audio-echo` | `integration_deterministic::openclaw_scenario_audio_echo` | deterministic | pre-merge-pikachat | none | Script remains compatibility wrapper only. |
| `pikachat-openclaw/scripts/phase4_openclaw_pikachat.sh` | wrapper to `run-openclaw-e2e.sh` | `integration_openclaw::openclaw_gateway_e2e` | heavy | pre-merge path-scoped + nightly-pikachat | openclaw-repo, public-network | Script remains compatibility wrapper only. |
| `pikachat-openclaw/scripts/run-scenario.sh` | generic wrapper to `pikahut test scenario <name>` | `testing::scenarios::openclaw::*` | deterministic | pre-merge-pikachat | varies by scenario | Thin wrapper target only. |
| `pikachat-openclaw/scripts/run-openclaw-e2e.sh` | generic wrapper to `pikahut test openclaw-e2e` | `testing::scenarios::openclaw::gateway_e2e` | heavy | pre-merge path-scoped + nightly-pikachat | openclaw-repo, public-network | Thin wrapper target only. |

## Workflow Lane Targeting (Post-Migration Contract)

| Lane | Planned canonical selectors |
| --- | --- |
| `pre-merge-pikachat` | deterministic `integration_deterministic` selectors; optional path-scoped `integration_openclaw::openclaw_gateway_e2e` when OpenClaw/plugin paths changed |
| `pre-merge` fixture lane | `cargo test -p pikahut` plus harness unit tests that enforce capability/skip behavior |
| `nightly-pikachat` | `integration_openclaw::openclaw_gateway_e2e` |
| `nightly-pika-e2e` | ignored heavy selectors including public/deployed call flows |
| `nightly-pika-ui-android` | `integration_deterministic::ui_e2e_local_android` |
| `nightly-primal-ios-interop` | `integration_primal::primal_nostrconnect_smoke` only |

## Migration Notes

- Until the migration is complete, legacy just/script entrypoints remain as compatibility shims.
- Capability-dependent tests must skip with explicit reason text instead of failing by default.
- Failure artifact expectations (logs, config snapshots, emitted URLs) are required for heavy and nondeterministic lanes.
