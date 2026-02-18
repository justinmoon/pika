# Rapture Spec: Guild-Control + Channel Groups on Marmot

Status: draft  
Scope: Rapture MVP architecture and implementation plan  
Last updated: 2026-02-18

## 1. Goals

- Build a Discord-like app (`Rapture`) on Marmot + existing RMP/MoQ infrastructure.
- Preserve cryptographic privacy boundaries per channel.
- Keep Rust as source of truth for app/business state.
- Reuse as much of `pika_core`, `marmotd`, and RMP tooling as possible.

## 2. Non-goals (MVP)

- Perfect Discord feature parity (bots, audit UI parity, complex moderation UX).
- Massive guild scale optimization before product fit.
- Cross-guild federation semantics beyond existing Nostr relay model.

## 3. Core model

For each guild (server):

- One MLS group: `guild-control`
- One MLS group per channel:
  - text channels
  - private channels
  - thread channels
  - voice signaling channels
- Voice media transport over MoQ/RMP (not over MLS chat payloads).

This creates a split:

- Control plane (`guild-control`): metadata, roles, memberships, channel ACL policy.
- Data plane (channel groups): chat messages + channel-local events.

## 4. Why `guild-control` exists

Discord-style permissions need two layers:

- Policy layer: "who should have access" (roles/memberships/channel ACL).
- Crypto enforcement layer: "who can decrypt."

`guild-control` stores policy and intent.  
Channel MLS membership enforces decryption.

If a user is removed from a channel group, old future messages stay private even if UI policy was stale.

## 5. Entity model

## 5.1 IDs

- `GuildId`: UUIDv7 string.
- `ChannelId`: UUIDv7 string.
- `RoleId`: UUIDv7 string.
- `MessageId`: event-derived string.
- `UserId`: Nostr pubkey (hex) + npub presentation.

## 5.2 Guild

- `guild_id`
- `name`
- `icon_ref` (optional)
- `created_by`
- `created_at`
- `default_relay_set`
- `version`

## 5.3 Channel

- `channel_id`
- `guild_id`
- `kind`: `text | voice | private | thread`
- `name`
- `topic` (optional)
- `parent_channel_id` (threads only)
- `position`
- `archived` (bool)
- `member_policy`:
  - derived from roles (`allow_roles`, `deny_roles`)
  - direct grants (`allow_users`, `deny_users`)

## 5.4 Roles and permissions

Roles:

- `role_id`
- `name`
- `priority` (integer)
- `permissions` (bitflags)
- `managed` (bool)

Permission bits (MVP):

- `VIEW_CHANNEL`
- `SEND_MESSAGE`
- `MANAGE_MESSAGES`
- `MANAGE_CHANNELS`
- `MANAGE_ROLES`
- `KICK_MEMBERS`
- `BAN_MEMBERS`
- `ADMINISTRATOR`
- `CONNECT_VOICE`
- `SPEAK_VOICE`
- `MUTE_MEMBERS`

## 5.5 Membership

- Guild membership: user included in `guild-control` MLS group.
- Channel membership: user included in corresponding channel MLS group.
- Membership sync is eventually consistent via reconciler jobs (see section 9).

## 6. Wire protocol

All application payloads are versioned envelopes inside MLS app messages.

Envelope:

```json
{
  "schema": "rapture.control.v1",
  "op": "channel.create",
  "guild_id": "01J...",
  "actor": "hex-pubkey",
  "op_id": "uuidv7",
  "ts_ms": 1760000000000,
  "body": {}
}
```

Schema families:

- `rapture.control.v1` (sent in `guild-control` group)
- `rapture.chat.v1` (sent in channel groups)
- `rapture.voice.v1` (voice signaling in voice channel groups)

Determinism rules:

- `op_id` must be unique and idempotent.
- Operations apply exactly-once per client by deduping `op_id`.
- Ties/order use `(message timestamp, event id)` as stable sort key.

## 7. Control-plane operations (`rapture.control.v1`)

MVP operations:

- `guild.create`
- `guild.update`
- `member.add`
- `member.remove`
- `role.create`
- `role.update`
- `role.delete`
- `member.roles.set`
- `channel.create`
- `channel.update`
- `channel.archive`
- `channel.delete`
- `channel.permissions.set`

Authoritative state:

- Rebuilt by replaying control ops in order.
- Snapshotted locally for fast startup.

Validation:

- Each op validated against current permission graph before apply.
- Invalid ops are ignored and logged as policy violations.

## 8. Data-plane operations

## 8.1 Chat (`rapture.chat.v1`)

- `message.send`
- `message.edit`
- `message.delete` (tombstone)
- `reaction.put`
- `reaction.remove`
- `thread.open` (optional alias to control op in MVP)

## 8.2 Voice signaling (`rapture.voice.v1`)

- `voice.session.start`
- `voice.session.end`
- `voice.participant.join`
- `voice.participant.leave`
- `voice.participant.state` (mute/deafen/hand)
- `voice.track.advertise` (MOQ track metadata)

Media bytes are on MoQ relay; signaling remains in MLS channel group.

## 9. Membership reconciliation model

Problem: policy says who should access channel; MLS group membership is separate mutable state.

Solution: reconciler loop:

1. Build desired membership from control state + permission evaluation.
2. Read actual membership from MDK for channel group.
3. Compute diff:
   - `to_add`
   - `to_remove`
4. Apply updates with MDK commits:
   - `add_members` / `remove_members`
5. Wait relay confirmation before welcome fanout where required.
6. Mark reconciliation checkpoint.

Properties:

- Idempotent.
- Retry-safe.
- Handles partial failures.

## 10. Security model

- Confidentiality: MLS per group.
- Integrity: signed Nostr events + MLS authenticated content.
- Least privilege:
  - policy in control plane
  - decryption gated by channel-group membership
- Membership changes must follow MIP timing:
  - commit confirmation before dependent welcomes where required.

## 11. Rust architecture changes

Create `rapture_core` crate (initially can fork from `pika_core` internals, then factor shared libs):

- `core/control/`
  - op decoder/validator/applier
- `core/channels/`
  - channel group lifecycle + reconciliation
- `core/permissions/`
  - role graph + permission evaluator
- `core/voice/`
  - signaling state + MOQ bridge metadata
- `state/`
  - guild list, channel tree, active channel, members, roles, permissions, call UI state

Shared crates to extract (target):

- `crates/shared/marmot-runtime`
- `crates/shared/nostr-transport`
- `crates/shared/profile-cache`
- `crates/shared/voice-signaling`

## 12. Client state contract (same pattern as Pika)

- Rust owns app state and emits full snapshots with monotonic `rev`.
- iOS/Android render state slices and dispatch actions.
- Suggested top-level slices:
  - `guild_list`
  - `guild_view` (channels + member sidebar data)
  - `channel_view`
  - `composer`
  - `voice_state`
  - `router`
  - `toast`
  - `busy`

## 13. Monorepo shape (target)

```text
apps/
  pika/
    ios/
    android/
    rust/
  rapture/
    ios/
    android/
    rust/
crates/
  shared/
  services/
  tools/
```

Migration strategy:

1. Add `apps/rapture` first.
2. Make tooling app-aware (`APP=pika|rapture`).
3. Move existing root `ios/android/rust` to `apps/pika`.

## 14. CI model

Keep single required status: `pre-merge`.

Add path-aware lane gating:

- `check-pika` runs for `apps/pika/**` or shared changes.
- `check-rapture` runs for `apps/rapture/**` or shared changes.
- `check-rmp` runs for `crates/tools/rmp-cli/**` or shared changes.
- `check-marmotd`, `check-notifications` similarly scoped.

Aggregator job passes if each needed lane is `success` and unrelated lanes are `skipped`.

## 15. QA policy (automation first)

- A phase is not complete until all required automated tests are green.
- Human manual QA is fallback only; default is automated Rust + UI + scripted `agent-device`.
- Every PR must list exact commands run and whether each was local/CI.
- New behavior requires at least one regression test in the same PR.

## 16. Required test inventory (must be created)

If `apps/rapture` is not created yet, place these under current equivalents and move later.

Rust unit/integration tests (FfiApp-first):

- `apps/rapture/rust/tests/control_ops.rs`
- `apps/rapture/rust/tests/permission_matrix.rs`
- `apps/rapture/rust/tests/reconcile_membership.rs`
- `apps/rapture/rust/tests/app_flows.rs`
- `apps/rapture/rust/tests/e2e_local_relay.rs`
- `apps/rapture/rust/tests/e2e_local_moq_voice.rs`

iOS UI tests:

- `apps/rapture/ios/UITests/RaptureGuildFlowUITests.swift`
- `apps/rapture/ios/UITests/RapturePermissionsUITests.swift`
- `apps/rapture/ios/UITests/RaptureVoiceUITests.swift`

Android UI tests:

- `apps/rapture/android/app/src/androidTest/java/.../RaptureGuildFlowUiTest.kt`
- `apps/rapture/android/app/src/androidTest/java/.../RapturePermissionsUiTest.kt`
- `apps/rapture/android/app/src/androidTest/java/.../RaptureVoiceUiTest.kt`

Scripted device-smoke tests (`agent-device replay`):

- `scripts/agent-device/rapture-ios-smoke.json`
- `scripts/agent-device/rapture-android-smoke.json`

Optional hardware smoke (`dinghy`, run nightly or pre-release):

- Recipe to add: `just rapture-dinghy-smoke`
- Command target: `cargo dinghy test -p rapture_core --lib`

## 17. Implementation plan with concrete acceptance gates

## Phase 0: Design lock + scaffolding

Deliverables:

- [ ] Approve protocol envelope/op naming.
- [ ] Keep this spec updated.
- [ ] Add `apps/rapture` skeleton (Rust+iOS+Android).
- [ ] Add app-root support in `rmp` (`--root apps/rapture` or equivalent).
- [ ] Add basic recipes:
  - `just rapture-run-ios`
  - `just rapture-run-android`
  - `just pre-merge-rapture`

Tests to add:

- [ ] `apps/rapture/rust/tests/bootstrap_smoke.rs` (FfiApp create + state snapshot sanity).
- [ ] iOS/Android one-test launch suites (app boots and renders default screen).

Required commands:

- `cargo test -p rapture_core --test bootstrap_smoke`
- `just rapture-run-ios`
- `just rapture-run-android`
- `just rapture-ui-test-ios`
- `just rapture-ui-test-android`

Acceptance criteria:

- App launches on both platforms.
- Rust bootstrap smoke test passes.
- UI smoke tests pass on simulator/emulator with zero manual interaction.

## Phase 1: Guild-control core

Deliverables:

- [x] Control op schema + serde + version handling.
- [x] Replayable control-state store (snapshot + log).
- [x] Permission evaluator.
- [x] App actions: `CreateGuild`, `CreateChannel`, `InviteMember`, `SetMemberRoles`, `SetChannelPermissions`.

Tests to add:

- [x] `control_ops.rs`:
  - `replay_is_deterministic`
  - `duplicate_op_id_is_noop`
  - `unknown_schema_version_is_rejected`
- [x] `permission_matrix.rs`:
  - allow/deny precedence
  - admin override
  - channel override behavior
- [x] `app_flows.rs`:
  - create guild + create channel updates `AppState` and `rev` monotonicity

Required commands:

- `cargo test -p rapture_core --test control_ops`
- `cargo test -p rapture_core --test permission_matrix`
- `cargo test -p rapture_core --test app_flows`

Acceptance criteria:

- All control operations are replayable and idempotent.
- Permission checks are deterministic and enforced in action handling.
- FfiApp state transitions match expected snapshots.

## Phase 2: Channel groups + text messaging

Deliverables:

- [x] Channel group lifecycle from control ops (local relay simulation).
- [x] Membership reconciler (`desired` vs `actual` diff + retry).
- [x] `rapture.chat.v1` send/edit/delete/reaction.
- [ ] Guild/channel/timeline UI slices.

Tests to add:

- [x] `reconcile_membership.rs`:
  - add/remove diff correctness
  - partial failure retry
  - idempotent re-run
- [x] `e2e_local_relay.rs` (two local relay clients):
  - guild invite + channel join + encrypted send/receive
  - member removed cannot decrypt subsequent messages
- [ ] iOS/Android UI tests:
  - create guild
  - create channel
  - send/receive message in channel

Required commands:

- `cargo test -p rapture_core --test reconcile_membership`
- `RAPTURE_E2E_LOCAL=1 cargo test -p rapture_core --test e2e_local_relay -- --ignored --nocapture`
- `just rapture-ui-test-ios`
- `just rapture-ui-test-android`
- `just rapture-ui-e2e-local` (local relay + local bot path)

Acceptance criteria:

- Two-client encrypted channel messaging passes deterministically on local relay.
- Access revocation is cryptographically enforced in tests.
- Platform UI tests pass for guild/channel happy path.

## Phase 3: Permissions hardening

Deliverables:

- [x] Enforce permissions in dispatch + replay apply path.
- [x] Admin/mod actions (kick/ban/remove from channel).
- [x] Conflict/idempotency handling with clear error surfaces.

Tests to add:

- [x] `permission_matrix.rs` negative tests for unauthorized actions.
- [x] `control_ops.rs` tests for invalid actor/op rejection.
- [ ] UI tests for denied actions:
  - non-admin cannot see/manage controls
  - denied send fails with visible error state

Required commands:

- `cargo test -p rapture_core --test permission_matrix`
- `cargo test -p rapture_core --test control_ops`
- `just rapture-ui-test-ios`
- `just rapture-ui-test-android`

Acceptance criteria:

- Unauthorized actions are rejected both pre-apply and replay-time.
- UI reflects denied permissions without inconsistent state.

## Phase 4: Voice signaling + MoQ

Deliverables:

- [x] `rapture.voice.v1` signaling events.
- [x] MoQ media integration reuse from existing RMP path.
- [x] Voice permission gating (`CONNECT_VOICE`, `SPEAK_VOICE`, `MUTE_MEMBERS`).

Tests to add:

- [x] `e2e_local_moq_voice.rs`:
  - join/leave voice channel + bidirectional media frame delivery
  - mute/unmute state propagation
  - unauthorized voice join denied
- [ ] iOS/Android `RaptureVoice*` tests for join/leave/mute UI behavior.

Required commands:

- `RAPTURE_E2E_MOQ=1 cargo test -p rapture_core --test e2e_local_moq_voice -- --ignored --nocapture`
- `just rapture ui-test-ios`
- `just rapture ui-test-android`

Acceptance criteria:

- Two-client local voice signaling + media path succeeds.
- Voice permission denial is covered by automated tests.

## Phase 5: Monorepo + CI completion

Deliverables:

- [ ] Move Pika into `apps/pika` (after Rapture stable).
- [ ] Add/maintain lane recipes:
  - `pre-merge-pika`
  - `pre-merge-rapture`
  - shared/service lanes
- [ ] Add path-filtered execution in `.github/workflows/pre-merge.yml`.

Tests to add:

- [ ] CI lane-selection test fixture (script + expected outputs) for path filters.
- [ ] One workflow self-test that proves:
  - Rapture-only change skips Pika lane
  - shared change runs both lanes

Required commands:

- `just pre-merge-rapture`
- `just pre-merge-pika`
- `just pre-merge` (full gate before merge to `master`)

Acceptance criteria:

- Rapture-only PRs do not run unrelated heavy lanes.
- Shared changes correctly fan out to both app lanes.
- Single required GitHub status remains `pre-merge`.

## 18. Command checklist for PRs

Minimum for Rapture feature PRs:

1. `cargo test -p rapture_core --lib --tests`
2. `just rapture-ui-test-ios`
3. `just rapture-ui-test-android`
4. `just pre-merge-rapture`

For protocol/membership/voice changes:

1. `RAPTURE_E2E_LOCAL=1 cargo test -p rapture_core --test e2e_local_relay -- --ignored --nocapture`
2. `RAPTURE_E2E_MOQ=1 cargo test -p rapture_core --test e2e_local_moq_voice -- --ignored --nocapture`
3. `just rapture-ui-e2e-local`

For release candidates:

1. `just pre-merge`
2. `just rapture-dinghy-smoke` (if available)
3. `agent-device replay` smoke flows on both platforms (see section 19)

## 19. Scripted QA (no ad-hoc manual clicking)

Add replayable flows so QA is mostly one-command:

- iOS:
  - `npx --yes agent-device --platform ios replay scripts/agent-device/rapture-ios-smoke.json`
- Android:
  - `npx --yes agent-device --platform android replay scripts/agent-device/rapture-android-smoke.json`

Minimum replay assertions:

- Create account/login.
- Create guild.
- Create channel.
- Send one message.
- Leave/reopen app and confirm state restore.

Use human manual QA only when replay fails and root cause is unknown.

## 20. Risks and mitigations

- Membership fanout cost with many channels.
  - Mitigation: lazy membership for inactive channels + channel count limits in MVP.
- Control/data divergence.
  - Mitigation: continuous reconciler + periodic full reconciliation pass.
- Relay/order edge cases.
  - Mitigation: idempotent ops (`op_id`) + stable ordering rules.
- Role graph complexity.
  - Mitigation: start with simple allow/deny precedence and no exotic inheritance.

## 21. Open decisions

- Payload encoding: JSON first vs protobuf/CBOR.
- Thread model: separate channel group vs logical thread in parent timeline.
- Max guild/channel/member limits for MVP guardrails.
- Where to run reconciliation: client-only vs helper service in `marmotd`.

## 22. Immediate next tasks

- [x] Add `apps/rapture` skeleton + `rmp` app-root support.
- [x] Create initial test files listed in section 16 (`bootstrap_smoke`, `control_ops`, `permission_matrix`).
- [x] Add `just pre-merge-rapture` and wire it into workflow with path filters.
- [ ] Add first `agent-device` replay scripts for iOS/Android smoke.
