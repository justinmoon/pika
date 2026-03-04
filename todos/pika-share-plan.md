---
summary: Shared Rust queue contract for iOS + Android share flows without embedding FfiApp in extensions.
read_when:
  - planning iOS and Android share-sheet parity
  - reducing duplicated Swift/Kotlin share logic
  - defining share queue semantics and test coverage
---

# Pika Share Plan

## 1) Self-contained Problem Statement

Pika currently has an iOS share extension flow implemented with Swift-side queue logic.  
We need iOS and Android share-sheet functionality to converge on a shared Rust core so we avoid duplicating business logic in Swift and Kotlin.

The share surface runs in constrained, separate OS contexts. It must not instantiate the full `FfiApp` runtime in extension/process boundaries because that introduces database locking and lifecycle contention risk.

## 2) Acceptance Criteria

1. iOS share flow and Android share flow use a shared Rust queue/policy contract.
2. Native code remains focused on OS integration (intent/item-provider ingestion, UI, app handoff).
3. At-least-once delivery semantics are preserved.
4. Queue failures are observable through a bidirectional result channel.
5. No extension/process path attempts to run the full Rust app runtime (`FfiApp`).

## 3) Constraints

### Musts

- Keep policy/state transitions in Rust where shared behavior matters.
- Preserve app-group/shared-storage queue model (no DB lock fights with main app runtime).
- Keep the bridge narrow and testable.

### Must-Nots

- Do not instantiate `FfiApp` from iOS share extension or Android share entry process.
- Do not create diverging Swift/Kotlin queue semantics.
- Do not couple share extension lifecycle to relay/network runtime.

### Preferences

- iOS migration first with no UX regression.
- Android implementation reuses same Rust queue API.
- Keep API small enough for UniFFI bindings and deterministic tests.

### Escalations (Ask User)

- If Android incoming-share UX constraints require a materially different product flow than iOS.
- If OS limits force dropping required payload types in one platform.
- If queue guarantees need stronger semantics than at-least-once (exactly-once or transactional send receipts).

## 4) Decomposition (Phased)

## Phase 1: Rust Queue Contract + iOS Migration

### Scope

- Introduce `crates/pika-share`.
- Move queue semantics from Swift into Rust:
  - enqueue
  - dequeue claim/lease
  - ack/requeue/finalize
  - result reporting
  - cleanup/GC
- Keep iOS extension UI and payload extraction native.
- Keep iOS main app send dispatch native (`AppAction.SendMessage` / `SendChatMedia`) while draining via Rust contract.

### Contract (v1)

- `share_enqueue(root_dir, request) -> ShareQueueReceipt`
- `share_dequeue_batch(root_dir, now_ms, limit) -> Vec<ShareDispatchJob>`
- `share_ack(root_dir, ack) -> ()`
- `share_list_recent_results(root_dir, limit) -> Vec<ShareResult>`
- `share_gc(root_dir, now_ms) -> ShareGcStats`

### Exit Criteria

- Existing iOS behavior still works.
- Queue correctness covered by Rust unit tests.
- No Swift queue policy logic beyond bridge calls + UI.

## Phase 2: Android Share Integration

### Scope

- Add Android share ingress (`ACTION_SEND` / URL/text/image).
- Normalize payload natively, enqueue via `pika-share`.
- Drain queue in app foreground/entry via same Rust contract.
- Dispatch existing Rust core actions from Android app runtime after dequeue.

### Exit Criteria

- Android can share text/URL/image via same contract semantics as iOS.
- No platform-specific queue forks.

## Phase 3: UX/Parity Polish

### Scope

- Shared result statuses surfaced in native UI.
- Retry/failure messaging consistency across iOS and Android.
- Optional chooser improvements and conversation suggestions.

### Exit Criteria

- Cross-platform share UX is comparable and consistent.
- Failures are visible and diagnosable from result channel.

## 5) Evaluation Design

### Rust Unit Tests (Required)

- enqueue/dequeue ordering by `created_at_ms`
- duplicate suppression by `client_request_id`
- lease expiration reclaim (`inflight -> pending`)
- retryable ack path (requeue with backoff)
- terminal ack path (result persisted, media cleanup)
- malformed file handling and safe path validation
- GC behavior (stale queue entries, stale results, orphan media/index cleanup)

### Optional Rust Property Tests

- Queue state invariants across random operation sequences:
  - no item simultaneously in `pending` and `inflight`
  - terminal items not returned by dequeue
  - result timestamps monotonic per item

### Manual QA (Required)

- iOS: share text/url/image from external app; verify queue progress and eventual send.
- Android: same coverage once integrated.
- Failure drill: induce retryable and permanent errors and verify result visibility.

## Architecture Notes

- This is a bounded mini-RMP adapter, not a mini full-app runtime.
- Native owns OS lifecycle/UI; Rust owns share queue policy/state.
- At-least-once semantics are intentional for extension reliability and simplicity.

## Current Known Limitation (Android)

- In current Phase 2 behavior, after selecting a recipient from Android share flow, navigation into the target chat can be delayed while state catches up. Share enqueue/send succeeds, but immediate post-send routing UX is not yet polished to parity with iOS.
