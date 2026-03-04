---
summary: iOS share extension architecture and phased implementation plan.
read_when:
  - planning or reviewing iOS share extension work
---

# iOS Share Extension Plan

**Issue:** [#386](https://github.com/sledtools/pika/issues/386)
**Status:** Proposal / awaiting review
**Date:** 2026-03-03

## Problem

Users cannot share content (text, URLs, images) from other apps into Pika conversations via the iOS share sheet. Adding a Share Extension target would let Pika appear as a share destination, with a conversation picker so users can share directly to a specific chat.

## Architecture decision: file-based queue

The share extension runs in a **separate process** with a ~120 MB memory limit. It **cannot** use the Rust core (`FfiApp`) because:

- The MLS database uses SQLCipher with locking constraints
- `FfiApp` requires a Tokio runtime and relay connections
- The main app may be running simultaneously (database conflicts)

Instead, the share extension communicates with the main app through files in the shared **App Group container** (`group.$(PIKA_APP_BUNDLE_ID)`):

```
Main app                          Share extension
    │                                    │
    ├── writes share_chat_list.json ────►├── reads chat list for picker UI
    │   (on every state update)          │
    │                                    ├── user picks a conversation
    │                                    │
    │◄── reads share_queue/*.json ───────├── writes queued payload
    │   (on foreground)                  │   (text/URL/image + chat_id)
    │                                    │
    ├── dispatches via FfiApp            ├── calls completeRequest()
    │   (SendMessage / SendChatMedia)    │   (extension dismissed)
    └── deletes queue file               └
```

This is the same shared-container pattern used by the existing Notification Service Extension (`PikaNotificationService`), which reads the keychain and MLS database from the app group.

---

## Phase 1: Shared data layer (`ShareQueueManager`)

**Goal:** Create the shared Swift module that both the main app and extension use for file I/O.

### New file: `ios/ShareExtension/ShareQueueManager.swift`

A `ShareQueueManager` enum with static methods:

```swift
enum ShareQueueManager {
    // Chat list cache (main app writes, extension reads)
    static func writeChatListCache(_ chats: [ShareableChatSummary])
    static func readChatListCache() -> [ShareableChatSummary]

    // Share queue (extension writes, main app reads + deletes)
    static func enqueue(_ item: ShareQueueItem) throws
    static func dequeueAll() -> [ShareQueueItem]
    static func deleteQueueItem(_ item: ShareQueueItem)

    // Login check (reads app-group UserDefaults)
    static func isLoggedIn() -> Bool
}
```

### Data structures (Codable)

**`ShareableChatSummary`** — simplified projection of `ChatSummary` (from `rust/src/state.rs`):

| Field | Type | Source |
|-------|------|--------|
| `chatId` | `String` | `ChatSummary.chat_id` |
| `displayName` | `String` | `ChatSummary.display_name` |
| `isGroup` | `Bool` | `ChatSummary.is_group` |
| `subtitle` | `String?` | `ChatSummary.subtitle` |
| `lastMessagePreview` | `String` | `ChatSummary.last_message_preview` |
| `lastMessageAt` | `Int64?` | `ChatSummary.last_message_at` |
| `members` | `[ShareableMember]` | `ChatSummary.members` (npub, name, pictureUrl only) |

**`ShareQueueItem`** — queued payload:

| Field | Type | Notes |
|-------|------|-------|
| `id` | `String` | UUID, also the filename |
| `chatId` | `String` | Target conversation |
| `contentType` | `text \| url \| image` | Determines how main app dispatches |
| `text` | `String` | Message text or URL string |
| `mediaFilename` | `String?` | Original filename for images |
| `mediaMimeType` | `String?` | e.g. `image/jpeg` |
| `mediaPath` | `String?` | Relative path within app group container |
| `createdAt` | `Int64` | Unix timestamp (for expiry) |

### File locations (within app group container)

```
<app_group>/Library/Application Support/
├── share_chat_list.json          # chat list cache
└── share_queue/
    ├── <uuid>.json               # queue item metadata
    └── media/
        └── <uuid>.jpg            # image data (if applicable)
```

### Login detection

The main app writes a boolean flag to app-group `UserDefaults`:
- Key: `pika.share.is_logged_in`
- Set `true` on login, `false` on logout
- The extension reads this to decide whether to show the picker or a "please log in" message

This is simpler than replicating the full keychain auth detection (which uses `UserDefaults.standard`, inaccessible from extensions).

---

## Phase 2: Extension target setup

**Goal:** Create the Xcode target, Info.plist, and entitlements.

### Add target to `ios/project.yml`

```yaml
PikaShareExtension:
  type: app-extension
  platform: iOS
  sources:
    - path: ShareExtension
    - path: Sources/Views/AvatarView.swift      # reuse existing avatar
    - path: Sources/Helpers/OnChangeCompat.swift # needed by AvatarView
  settings:
    base:
      PRODUCT_NAME: PikaShareExtension
      GENERATE_INFOPLIST_FILE: YES
      INFOPLIST_FILE: ShareExtension/Info.plist
      CODE_SIGN_ENTITLEMENTS: ShareExtension/ShareExtension.entitlements
      PRODUCT_BUNDLE_IDENTIFIER: $(PIKA_APP_BUNDLE_ID).share-extension
  postBuildScripts:
    - name: "Stamp build number"
      script: |
        if [ -f "${PROJECT_DIR}/.build-number" ]; then
          BUILD_NUMBER=$(cat "${PROJECT_DIR}/.build-number")
        else
          BUILD_NUMBER=$(date +"%Y%m%d%H%M")
        fi
        /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $BUILD_NUMBER" \
          "${BUILT_PRODUCTS_DIR}/${INFOPLIST_PATH}"
      basedOnDependencyAnalysis: false
```

Add to the main `Pika` target:
- **Dependencies:** `- target: PikaShareExtension`
- **Sources:** `- path: ShareExtension/ShareQueueManager.swift` (so the main app can also use `ShareQueueManager`)

### New file: `ios/ShareExtension/Info.plist`

```xml
<key>NSExtension</key>
<dict>
    <key>NSExtensionPointIdentifier</key>
    <string>com.apple.share-services</string>
    <key>NSExtensionPrincipalClass</key>
    <string>$(PRODUCT_MODULE_NAME).ShareViewController</string>
    <key>NSExtensionAttributes</key>
    <dict>
        <key>NSExtensionActivationRule</key>
        <dict>
            <key>NSExtensionActivationSupportsText</key>
            <true/>
            <key>NSExtensionActivationSupportsWebURLWithMaxCount</key>
            <integer>1</integer>
            <key>NSExtensionActivationSupportsImageWithMaxCount</key>
            <integer>1</integer>
        </dict>
        <key>IntentsSupported</key>
        <array>
            <string>INSendMessageIntent</string>
        </array>
    </dict>
</dict>
```

Uses the dictionary activation rule (not `TRUEPREDICATE`) to pass App Store review.

### New file: `ios/ShareExtension/ShareExtension.entitlements`

Same as `NotificationService.entitlements` but without `aps-environment`:

```xml
<key>com.apple.security.application-groups</key>
<array><string>$(PIKA_APP_GROUP)</string></array>
<key>keychain-access-groups</key>
<array><string>$(AppIdentifierPrefix)$(PIKA_APP_BUNDLE_ID).shared</string></array>
```

---

## Phase 3: Share extension UI

**Goal:** Build the SwiftUI conversation picker.

### New file: `ios/ShareExtension/ShareViewController.swift`

A `UIViewController` subclass (required as extension principal class) that hosts SwiftUI via `UIHostingController`.

### New file: `ios/ShareExtension/ShareExtensionView.swift`

SwiftUI view with:

1. **Navigation bar** — "Cancel" (left), "Send" (right, disabled until chat selected)
2. **Content preview** — snippet of shared text/URL, or image thumbnail
3. **Searchable conversation list** — reads `ShareQueueManager.readChatListCache()`, shows avatar + display name + last message preview
4. **Empty states:**
   - Not logged in: "Open Pika and sign in to share content"
   - No conversations: "No conversations yet. Start a chat in Pika first."

### Content extraction from `NSExtensionContext`

On appear, iterate `extensionContext.inputItems` / `NSItemProvider` attachments:

| UTType | Action | Queue `contentType` |
|--------|--------|---------------------|
| `.plainText` | Extract string | `text` |
| `.url` | Extract URL, store `.absoluteString` | `url` |
| `.image` | Load data, JPEG-compress, save to `share_queue/media/` | `image` |

Priority: image > URL > text (if multiple types present).

Images are downscaled to max 2048px on the longest side and JPEG-compressed at 0.85 quality to stay within the extension's ~120 MB memory limit.

### Avatar rendering

Reuses `AvatarView.swift` and its dependencies (`CachedAsyncImage`, `ImageLoader`, `ImageCache`, `OnChangeCompat`) from the main app by including those source files in the extension target.

---

## Phase 4: Main app integration

**Goal:** Wire up cache writing and queue processing in the main app.

### Modify: `ios/Sources/AppManager.swift`

**1. Chat list cache writing:**

```swift
private func updateShareChatListCache() {
    let shareable = state.chatList.map { chat in
        ShareableChatSummary(
            chatId: chat.chatId,
            displayName: chat.displayName,
            isGroup: chat.isGroup,
            subtitle: chat.subtitle,
            lastMessagePreview: chat.lastMessagePreview,
            lastMessageAt: chat.lastMessageAt,
            members: chat.members.map { m in
                ShareableMember(npub: m.npub, name: m.name, pictureUrl: m.pictureUrl)
            }
        )
    }
    ShareQueueManager.writeChatListCache(shareable)
}
```

Called in `apply(update:)` on `.fullState`:

```swift
case .fullState(let s):
    state = s
    callAudioSession.apply(activeCall: s.activeCall)
    updateShareChatListCache()  // <-- NEW
```

**2. Queue draining:**

```swift
func processShareQueue() {
    let items = ShareQueueManager.dequeueAll()
    for item in items {
        switch item.contentType {
        case .text, .url:
            dispatch(.sendMessage(chatId: item.chatId, content: item.text,
                                  kind: nil, replyToMessageId: nil))
        case .image:
            // Load image data from app group, base64-encode, dispatch SendChatMedia
            // Then delete the media file
        }
        ShareQueueManager.deleteQueueItem(item)
    }
}
```

Called from `onForeground()`:

```swift
func onForeground() {
    NSLog("[PikaAppManager] onForeground dispatching Foregrounded")
    dispatch(.foregrounded)
    processShareQueue()  // <-- NEW
}
```

**3. Login flag:**

In login/logout methods, write to app-group UserDefaults:

```swift
// After successful login:
UserDefaults(suiteName: appGroupId)?.set(true, forKey: "pika.share.is_logged_in")

// On logout:
UserDefaults(suiteName: appGroupId)?.set(false, forKey: "pika.share.is_logged_in")
```

---

## Edge cases

| Scenario | Behavior |
|----------|----------|
| User not logged in | Extension shows "please log in" message, Send hidden |
| Empty chat list | Extension shows "no conversations yet" message |
| Stale chat list cache | Acceptable — `chat_id` remains valid |
| Queue items accumulate | Discard items older than 7 days |
| App crash mid-processing | Queue items persist, retried next foreground |
| Concurrent writes | Atomic file writes; one file per queue item (no collisions) |
| Large images | Downscale + JPEG compress before saving to stay under memory limit |

---

## File summary

### New files (7)

| File | Lines (est.) |
|------|-------------|
| `ios/ShareExtension/ShareQueueManager.swift` | ~150 |
| `ios/ShareExtension/ShareViewController.swift` | ~30 |
| `ios/ShareExtension/ShareExtensionView.swift` | ~200 |
| `ios/ShareExtension/Info.plist` | ~30 |
| `ios/ShareExtension/ShareExtension.entitlements` | ~15 |

### Modified files (2)

| File | Change |
|------|--------|
| `ios/project.yml` | Add `PikaShareExtension` target, add dependency, add shared sources |
| `ios/Sources/AppManager.swift` | Add `updateShareChatListCache()`, `processShareQueue()`, login flag writes |

### Reused files (shared via project.yml sources, no changes)

| File | Used for |
|------|----------|
| `ios/Sources/Views/AvatarView.swift` | Avatar rendering in conversation picker |
| `ios/Sources/Helpers/OnChangeCompat.swift` | Dependency of AvatarView |

---

## Future enhancements (not in v1)

- **Conversation suggestions** in the share sheet top row via `INSendMessageIntent` donations
- **Multiple images** (currently limited to 1)
- **Video and file sharing**
- **Send immediately from extension** (would require a lightweight Rust library like PikaNSE)

---

## Testing

### Manual QA checklist

- [ ] Share text from Safari to Pika — message appears in selected chat
- [ ] Share URL from Safari — URL sent as text message
- [ ] Share image from Photos — image sent as media attachment
- [ ] Conversation list shows correct chats with avatars
- [ ] Search filters conversations
- [ ] Cancel button dismisses without sending
- [ ] Not-logged-in state shows correct message
- [ ] Empty chat list shows correct message
- [ ] Queue item processed correctly after app foreground
- [ ] Multiple queued items all processed
- [ ] Large image downscaled without crashing

### Unit tests

- `ShareQueueManager` serialization round-trip (write + read chat list)
- Queue enqueue/dequeue round-trip
- Queue item Codable conformance for each content type
- Edge cases: empty cache, malformed JSON, missing media file
