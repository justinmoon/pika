# Chat Reliability / Stability TODOs

## 1. Reconnect on foreground
`AppAction::Foregrounded` re-opens MDK but never calls `recompute_subscriptions()` or nudges the relay pool. After a long background sleep, WebSocket connections may be dead. Adding a subscription refresh on foreground would fix "messages don't appear until I switch chats".

**Key files:** `rust/src/core/session.rs` (recompute_subscriptions), `rust/src/core/mod.rs` (Foregrounded handler ~line 4433)

## 2. Auto-retry on network recovery
Messages sent offline are immediately marked `Failed` — no `NWPathMonitor` (iOS) / `ConnectivityManager` (Android) integration. Adding a network observer that flushes failed sends on reconnect would eliminate manual tap-to-retry for transient drops.

**Key files:** `rust/src/core/mod.rs` (RetryMessage), `pika/` (iOS network observer), `android/` (Android connectivity)

## 3. Connection status indicator
No relay connection state is surfaced to the UI. Users can't tell if they're connected. Expose a `ConnectionState` enum in `AppState` and show a banner/bar.

**Key files:** `rust/src/core/mod.rs` (AppState), iOS/Android chat views

## 4. Handle `RecvError::Lagged`
The notification loop at `session.rs:138` silently drops events when the broadcast channel overflows. A re-fetch of recent events from relays after a lag would prevent message loss during bursts.

**Key files:** `rust/src/core/session.rs` (~line 138, notifications loop)

## 5. MLS `Unprocessable` recovery
`Unprocessable` messages from `mdk.process_message()` are silently discarded. At minimum surface a diagnostic ("group may be out of sync"). Ideally detect epoch gaps and re-fetch missed Commit events from relays.

**Key files:** `rust/src/core/mod.rs` (handle_group_message ~line 3884)

## 6. Multi-relay delivery verification
`send_event_first_ack` succeeds when *any one* relay ACKs. If that relay is unreliable, other group members on different relays may never see the message. Consider requiring ACK from all group relays or a majority.

**Key files:** `rust/src/core/chat_media.rs` (send_event_first_ack)

## 7. Offline send queue
Rather than failing immediately when offline, queue messages and auto-flush on reconnect. Ties into #2 above — network recovery triggers flush.

**Key files:** `rust/src/core/chat_media.rs` (publish_chat_message_with_tags), `rust/src/core/mod.rs` (pending_sends, local_outbox)
