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

## 17. Rebaseline (2026-02-18)

This project is now best described as:

- **Core/protocol ahead of UI**.
- Rust control/chat/voice foundations are largely implemented and tested.
- iOS + Android now expose a Discord-style structural shell (server rail, channel navigation, timeline/composer) backed by Rust timeline state.

Current status by area:

- [x] App scaffolding + `apps/rapture` + `just rapture ...` command surface.
- [x] Deterministic control replay (`ts_ms`, `op_id`) across load/sim/live apply paths.
- [x] Durable append-before-commit behavior for control ops.
- [x] Unique op IDs (`uuid v4`) for local control actions.
- [x] Randomized channel group key rotation in simulation harness (non-deterministic, not MLS-grade).
- [x] iOS + Android UI support for:
  - greeting/set-name
  - create guild/channel
  - invite/kick/ban member
  - set member roles
  - set channel permissions
  - remove member from channel
  - visible error/status toast + rev + guild summaries
- [x] Timeline/chat UI (send/edit/delete/reactions) on iOS/Android via Rust `AppState.timeline`.
- [x] Voice UI (join/leave/mute + speaking state) on iOS/Android via Rust `AppState.voice_room`.
- [ ] Replayable `agent-device` JSON smoke scripts committed under `scripts/agent-device/`.

## 18. Execution plan (UI-first from here)

## Sprint A (done): Control-plane parity in mobile UI

Deliverables:

- [x] iOS + Android expose current control actions from `AppAction`.
- [x] UI displays `AppState.rev`, guild summaries, and error toast.
- [x] Emulator launch UX for `just rapture run-android` behaves like React Native/Flutter expectations (auto-start visible emulator if needed).

Required commands:

- `just rapture run-ios`
- `just rapture run-android`
- `just --justfile apps/rapture/justfile pre-merge`

Acceptance criteria:

- Both apps launch and dispatch all control actions without terminal-only tooling.
- Permission failures surface in-app via toast.

## Sprint B (done): Timeline/chat UI slice

Deliverables:

- [x] Add channel/timeline state projections for frontend consumption.
- [x] Add iOS + Android timeline UI for send/read/edit/delete/reactions.
- [x] Wire local encrypted channel flow to visible UI (not just Rust tests).

Required tests/commands:

- `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test chat_ops`
- `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test app_flows timeline_send_edit_react_delete_round_trip`
- `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test app_flows timeline_permissions_are_enforced`
- `just rapture run-ios`
- `just rapture run-android`
- `just --justfile apps/rapture/justfile pre-merge`

Acceptance criteria:

- `timeline_send_edit_react_delete_round_trip` passes and verifies send/edit/reaction/remove/delete through `FfiApp`.
- `timeline_permissions_are_enforced` passes and verifies denied send before membership + allowed send after invite.
- Both mobile apps launch and render the same Rust-selected guild/channel timeline slice (no platform-local timeline source).
- Existing encrypted multi-client relay guard remains covered by `RAPTURE_E2E_LOCAL=1 cargo test --manifest-path apps/rapture/rust/Cargo.toml --test e2e_local_relay -- --ignored --nocapture`.

## Sprint C (done): Voice UI slice

Deliverables:

- [x] Add iOS + Android voice controls (join/leave/mute/speaking state).
- [x] Surface voice permission denials in UI.

Required tests/commands:

- `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test voice_ops`
- `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test app_flows voice_join_mute_leave_updates_state`
- `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test app_flows voice_permission_denial_surfaces_toast`
- `just rapture run-ios`
- `just rapture run-android`
- `just --justfile apps/rapture/justfile pre-merge`

Acceptance criteria:

- `voice_join_mute_leave_updates_state` passes and verifies session start/join/mute/speaking/leave through `FfiApp`.
- `voice_permission_denial_surfaces_toast` passes and verifies denied voice join is surfaced as UI toast.
- iOS + Android render and dispatch voice controls from the same Rust `voice_room` projection.
- Existing MoQ integration remains covered by `RAPTURE_E2E_MOQ=1 cargo test --manifest-path apps/rapture/rust/Cargo.toml --test e2e_local_moq_voice -- --ignored --nocapture`.

## Sprint D (after UI parity): Monorepo reshape

Deliverables:

- [ ] Decide timing for moving Pika fully to `apps/pika`.
- [x] Path-filtered CI lane selection remains in place.
- [ ] Keep lane ownership clear (`pre-merge-pika`, `pre-merge-rapture`, shared lanes).

Acceptance criteria:

- Rapture-only PRs run only relevant lanes.
- Shared changes fan out correctly.

## 19. Command checklist for PRs (updated)

Minimum for Rapture feature PRs:

1. `cargo test -p rapture_core --lib --tests`
2. `just --justfile apps/rapture/justfile pre-merge`

For chat/timeline changes:

1. `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test chat_ops`
2. `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test app_flows timeline_send_edit_react_delete_round_trip`
3. `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test app_flows timeline_permissions_are_enforced`
4. `RAPTURE_E2E_LOCAL=1 cargo test --manifest-path apps/rapture/rust/Cargo.toml --test e2e_local_relay -- --ignored --nocapture`

For voice changes:

1. `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test voice_ops`
2. `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test app_flows voice_join_mute_leave_updates_state`
3. `cargo test --manifest-path apps/rapture/rust/Cargo.toml --test app_flows voice_permission_denial_surfaces_toast`
4. `RAPTURE_E2E_MOQ=1 cargo test --manifest-path apps/rapture/rust/Cargo.toml --test e2e_local_moq_voice -- --ignored --nocapture`

Before merge to `master`:

1. `just pre-merge-rapture`
2. `just pre-merge` (repo-wide gate)

## 20. QA strategy

Default:

- Automated Rust tests + platform UI tests + scripted device flows.

Fallback:

- Human manual QA only when scripted flow fails and root cause is unknown.

Manual smoke assertions (current UI baseline):

- Launch app.
- Create guild.
- Create channel.
- Send a message.
- Edit the message.
- Toggle `:+1:` reaction.
- Delete the message.
- Enter a voice channel, join voice, toggle mute/speaking, then leave.
- Invite a non-voice user and verify denied voice join shows toast.
- Trigger denied send action as a non-member and verify toast message.
- Kill/relaunch app and verify guild/channel summary persists.

## 21. Risks and mitigations

- Core/UI drift.
  - Mitigation: UI-first sprint gates; no “phase complete” without UI tests.
- Replay/order regressions under distributed ingest.
  - Mitigation: deterministic replay tests at control core + startup/load + live apply.
- Crypto model confusion (simulation vs MLS).
  - Mitigation: explicit labeling in code/docs; upgrade path to Marmot MLS integration.

## 22. Immediate next tasks

- [x] Rebaseline plan to reflect core-ahead-of-UI reality.
- [x] Ship control-plane UI slice on iOS + Android.
- [x] Add chat timeline UI slice (Sprint B).
- [x] Add voice UI slice (Sprint C).
- [ ] Add first UI automated tests (`RaptureGuildFlow` iOS/Android).
- [ ] Add first `agent-device` replay scripts for iOS/Android smoke.
