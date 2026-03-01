## Spec

This work is being done because the current migration delivered a major structure improvement (library modules, Rust selector tests, and CI selector wiring), but the integration suite is not yet fully library-native end-to-end.

Today, test execution moved from ad-hoc shell entrypoints to Rust test selectors in many lanes, but much scenario logic is still CLI-shaped and command-oriented rather than fixture-contract-oriented. The outcome we want is a complete library-first integration testing architecture where all integration flows are authored as clean Rust tests using stable `pikahut::testing` APIs, with CLI/script wrappers reduced to optional convenience shims.

Intent and expected outcome:
1. `pikahut::testing` becomes the single orchestration API used by integration tests.
2. Integration tests are expressed as composable Rust code with typed scenario inputs/outputs, deterministic fixture lifecycle, and explicit capability gating.
3. CI lanes execute selector-based Rust tests mapped to deterministic/heavy/nondeterministic/manual tiers with no bespoke shell orchestration requirements.
4. Artifact capture is standardized so failures are actionable (logs/config snapshots/markers per scenario type).
5. Existing developer entrypoints (`just`, scripts) remain available where useful, but are thin wrappers over the same library/test selectors.

Exact build target when done:
1. No integration test logic is coupled to `test_harness` CLI argument structs.
2. `crates/pikahut/src/testing/scenarios.rs` is decomposed into modular domain scenario modules with typed APIs and shared fixture/command primitives.
3. All integration selectors live under explicit targets (`integration_deterministic`, `integration_openclaw`, `integration_public`, `integration_primal`, `integration_manual`).
4. Manual flows currently represented as shell-only operations (notably Primal lab and interop manual guidance) have Rust selector entrypoints or documented intentional exceptions.
5. CI and docs are aligned with actual selector targets and capability semantics.

Current state assessment (what already changed):
1. Integration selectors were added in `crates/pikahut/tests/*` and are executed by many `just` recipes.
2. `test_harness` is now mostly dispatch and scenario functions live under `pikahut::testing::scenarios`.
3. `Capabilities`, `TestContext`, `CommandSpec/Runner`, and `FixtureSpec/Handle` exist.
4. OpenClaw/public/primal selector tests exist and basic guardrail/docs were added.

Rebase delta from `origin/master` on 2026-03-01:
1. Core session relay handling and event verification changed (`rust/src/core/session.rs`, `rust/src/core/mod.rs`, `rust/src/core/storage.rs`, `rust/src/external_signer.rs`).
2. `app_flows` timing/race behavior changed (`rust/tests/app_flows.rs` timeout + logout wait fix).
3. The integration refactor plan must include selector coverage that protects these new core behaviors in realistic end-to-end flows.

Current state gaps (what remains to fully refactor):
1. Scenario API still depends on CLI-shaped types from `test_harness` (reverse dependency from library to CLI contract).
2. `scenarios.rs` remains monolithic and uses substantial ad-hoc command/process logic instead of consistently routing through `FixtureHandle` + `CommandRunner` abstractions.
3. `FixtureSpec/FixtureHandle` APIs are underused by real scenarios; reusable fixture composition is incomplete.
4. Manual scenario class has no dedicated Rust test target even though matrix/docs track it as a first-class tier.
5. Public/deployed and Primal flows still lean on shell scripts internally for key operations rather than first-class Rust orchestration primitives.
6. Artifact schema is not yet unified across all scenario types (paths/metadata/summary conventions are inconsistent).
7. Selector/doc/workflow consistency checks should enforce all mapped selectors exist and are actually referenced by lanes.

Scope of full completion:
1. Deterministic local suite: CLI smoke (+media), local UI e2e (android/ios/desktop), interop baseline, OpenClaw deterministic scenarios.
2. Heavy deterministic suite: OpenClaw gateway e2e.
3. Nondeterministic suite: public UI e2e (android/ios/all), deployed-bot call flow.
4. Primal suite: lean nightly smoke in CI plus manual lab flows with explicit manual selector tier.
5. Compatibility wrappers: `tools/*` and `pikachat-openclaw/scripts/*` wrappers preserved only where they add DX value.
6. Post-rebase regression coverage: selector tests that assert invalid-event rejection behavior and robust session/logout convergence (no race-prone wait contracts) in integration contexts.

Non-goals:
1. Eliminating every shell script in the repo.
2. Making public/deployed flows deterministic.
3. Forcing manual-only exploratory tooling to run in CI.

## Plan

1. Lock a definitive “as-built vs target” integration inventory and gap list.
Acceptance criteria: `docs/testing/integration-matrix.md` and `docs/testing/library-first-migration-checklist.md` are updated to explicitly mark completed selector-backed flows vs remaining shell-backed/manual-only gaps, with no ambiguous “planned” entries.

2. Introduce library-native scenario input/output types that do not reference CLI structs.
Acceptance criteria: `crates/pikahut/src/testing/scenarios/*` consumes typed structs/enums defined under `testing` (not `test_harness`); `test_harness` maps CLI args into those types.

3. Split monolithic `scenarios.rs` into domain modules with stable boundaries.
Acceptance criteria: scenario code is decomposed into modules such as `scenarios/deterministic.rs`, `scenarios/openclaw.rs`, `scenarios/public.rs`, `scenarios/primal.rs`, `scenarios/interop.rs`; shared helpers move to `testing/common` module(s).

4. Refactor scenario execution to consistently use `TestContext`, `FixtureHandle`, and `CommandRunner`.
Acceptance criteria: direct ad-hoc process spawning in scenario code is replaced by `CommandRunner` where feasible; fixture lifecycle is driven by `FixtureSpec`/`start_fixture` instead of duplicated local cleanup guards.

5. Expand fixture contract so scenarios can express all required service graphs without bespoke code.
Acceptance criteria: `FixtureSpec` can model all currently used component combinations and overlays for local relay, relay+bot, backend, and scenario-specific ports/timeouts; missing knobs are added with tests.

6. Standardize artifact layout and metadata across scenario classes.
Acceptance criteria: every scenario writes artifacts under predictable subpaths with a common summary record (command outcomes, key env/capability decisions, preserved state paths); failure artifacts include tail excerpts and primary logs.

7. Add/complete the `integration_manual` selector target for manual-tier flows.
Acceptance criteria: `crates/pikahut/tests/integration_manual.rs` exists and covers manual interop/primal lab runbook selectors as explicit `#[ignore]` manual contracts, or docs clearly and intentionally mark unsupported conversions.

8. Remove remaining test-logic dependence on shell scripts by moving core operations into Rust helpers.
Acceptance criteria: public UI and primal smoke selectors invoke Rust orchestration paths directly for core flow logic; scripts remain optional wrappers that call selectors, not logic owners.

9. Tighten capability gating and skip semantics across all selectors.
Acceptance criteria: all selector tests emit consistent skip messages and CI notices; gates include required env vars, repos, tools, and platform/runtime prerequisites with deterministic behavior.

10. Add selector integrity guardrails to prevent doc/CI drift.
Acceptance criteria: guardrail tests verify that all selectors referenced in docs/workflows/just recipes exist and that deprecated command paths are absent from required lanes.

11. Add explicit post-rebase regression selectors for core session/event behavior.
Acceptance criteria: deterministic selectors cover at least (a) malformed/invalid-signature event rejection in an integration flow boundary and (b) logout/session convergence behavior that does not rely on brittle short waits; assertions are stable on CI runners.

12. Align `just` and workflow lanes to final tiering policy.
Acceptance criteria: deterministic pre-merge lanes, heavy path-scoped lanes, nightly ignored/nondeterministic lanes, and manual-only selectors are all encoded in `justfile` and `.github/workflows/pre-merge.yml` with matching documentation.

13. Preserve compatibility wrappers with explicit deprecation policy.
Acceptance criteria: wrappers in `tools/` and `pikachat-openclaw/scripts/` are either retained as thin selector dispatchers with argument pass-through or removed with replacement notes; no wrapper contains independent fixture orchestration.

14. Validate full suite reliability with targeted dry runs per tier.
Acceptance criteria: `cargo test -p pikahut` passes; deterministic selectors run in at least one CI-like environment; heavy/nondeterministic selectors are invoked with known-good prerequisites and produce expected artifacts and skip behavior.

15. Manual QA gate (user-run): confirm end-to-end operability and authoring ergonomics.
Acceptance criteria: user runs representative flows from each tier (deterministic, heavy OpenClaw, nondeterministic public, primal nightly smoke, manual lab) and confirms (a) behavior parity, (b) artifact quality, and (c) new-scenario authoring via library APIs is clean without adding shell orchestration.
