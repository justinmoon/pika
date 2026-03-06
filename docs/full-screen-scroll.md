---
summary: Experiment log and handoff for iOS full-height chat scrolling with UITableView, covering failed table rewrites, confirmed findings, and the recommended restart path.
read_when:
  - restarting the full-height iOS chat scroll investigation on a fresh branch
  - debugging white safe-area bars, giant overlay fields, or transcript squish in chat
  - deciding whether to keep the old inverted UITableView internals or revive a rewrite
---

# Full-Screen Chat Scroll Experiment Log

## Handoff Essentials

- Keep this document, checkpoint commit `cc8067d`, and the core finding that squish came from the list-internals rewrite, not from the full-bleed idea itself.
- If restarting, branch from `origin/master`, not from the current dirty experiment worktree.
- Treat the current branch as a reference branch only. It contains useful failed experiments, but it is not the clean base for the next attempt.
- The best current hypothesis is:
  - keep the old stable inverted `UITableView`
  - avoid reviving the normal-table/custom-hosting rewrite
  - debug the giant full-bleed overlay field in isolation
- Preserve `just run-swift --sim` if it proves useful, but do not assume it exists on `origin/master`.
- Use the Grok section below as a hypothesis source, not as authoritative truth.

## Recommended Restart Steps

1. Start from `origin/master` on a fresh branch.
2. Reproduce the baseline white-bar behavior first.
3. Reapply only the minimum full-bleed treatment needed to reproduce the overlay bug.
4. Instrument actual reserve/inset/offset values before changing transcript internals.
5. Verify short chat and long chat behavior separately after each change.

## Goal

Match the feel of Signal’s chat screen:

- transcript scrolls full-height, visually running under the top chrome and bottom composer
- floating top controls and bottom composer/action button feel modern and native
- keep `UITableView` performance characteristics
- no white safe-area bars
- no scroll jitter, message squish, or giant translucent overlay artifacts

## Baseline

Starting point on `origin/master`:

- `ChatView` used a more traditional safe-area layout, which produced visible white bars at the top and bottom
- `InvertedMessageList` used an inverted `UITableView` (`scaleY: -1`)
- rows were rendered with `UIHostingConfiguration`
- rows were grouped by sender (`MessageGroupRow`)
- scroll behavior was stable
- message squish was **not** present in that baseline

This matters because the later experiments initially mixed several variables together and made it hard to tell which one actually caused regressions.

## Branch / Checkpoint

- Branch used for this work: `chat-full-height-table`
- Checkpoint commit: `cc8067d` `Checkpoint chat full-height table experiments`

That checkpoint captures the large middle phase where we rewrote the chat table internals.

## Major Experiments

### 1. Improve the local Swift-only loop

Before continuing UI work, we added a faster iteration loop:

- `just run-swift --sim`
- `ios-build-swift-sim`
- `PIKA_IOS_SKIP_RUST_REBUILD=1` support in `tools/run-ios`

Why:

- `just run-ios --sim` was too slow for this kind of visual debugging
- we needed a way to rebuild only Swift/iOS UI while reusing existing Rust-generated artifacts

Result:

- successful
- kept for future work

Files:

- [justfile](/Users/futurepaul/dev/sec/other-peoples-code/pika/justfile)
- [tools/run-ios](/Users/futurepaul/dev/sec/other-peoples-code/pika/tools/run-ios)

### 2. Rewrite the transcript off the inverted table

We tried replacing the old inverted-table model with a normal-scroll `UITableView`.

What changed:

- removed `scaleY: -1`
- manually preserved offset when older rows were prepended
- introduced explicit top/bottom visual inset math
- manually tracked “at bottom” / sticky-bottom state
- overlaid the composer on top of the transcript

Why:

- the inverted table seemed like the reason we could not cleanly get full-bleed under the composer and top chrome

Observed problems:

- giant translucent/white field occupying a large part of the screen
- scroll-to-bottom button placement issues
- newest message chunk looked visually detached from the rest of the transcript
- jittery scrolling
- losing drag/grab during scroll
- visible overlap or changing distance between message groups while scrolling

Conclusion:

- this rewrite destabilized scroll behavior badly
- at the time it was unclear whether the cause was the full-bleed layout, the normal table, or both

### 3. Normal-table scroll/inset tuning

We tried to stabilize the normal table with many small fixes.

Attempted fixes:

- manual `adjustedContentInset` / `contentInsetAdjustmentBehavior = .never`
- custom top/bottom visual inset mapping
- special handling for short chats vs overflowing chats
- dynamic top spacer for short content
- turning bounce on/off depending on scrollability
- explicit `contentOffset`-based “scroll to bottom”
- delaying inset updates while dragging/decelerating
- content-size observation and post-layout repinning
- second-pass stabilization after diffable snapshot apply
- estimated row-height heuristics for message groups

Observed result:

- some improvements
- still had strong visual instability
- messages or groups could overlap or move relative to each other during momentum
- the transcript still felt physically wrong

Conclusion:

- more inset tuning was not solving the underlying problem

### 4. Flatten rows and replace hosting strategy

Hypothesis:

- whole sender groups were too large a sizing unit
- `UIHostingConfiguration` self-sizing inside a normal `UITableView` was contributing to instability

Changes tried:

- flattened grouped rows into per-bubble rows
- added `MessageBubbleRow`
- replaced `UIHostingConfiguration` with a custom reusable `UITableViewCell` hosting a `UIHostingController`
- added explicit `sizeThatFits(in:)` row measurement with a cache

Observed result:

- some aspects of jitter improved
- squish and spacing instability did **not** go away
- the rewrite became more complex and still did not match the stability of the original app

Conclusion:

- the table-internals rewrite path was getting more complex without producing a reliable result

Files heavily involved:

- [InvertedMessageList.swift](/Users/futurepaul/dev/sec/other-peoples-code/pika/ios/Sources/Views/InvertedMessageList.swift)
- [MessageBubbleViews.swift](/Users/futurepaul/dev/sec/other-peoples-code/pika/ios/Sources/Views/MessageBubbleViews.swift)

### 5. Edge effects / safe-area / content-inset experiments

We also explored whether iOS 26 visual edge behavior was responsible.

Tried:

- setting `UIScrollView` top/bottom edge effect style to hard
- hiding edge effects
- adding safe-area content insets so distortion would land in empty space
- reverting the table to safe-area pinned layout with white bars

Observed result:

- none of these explained the main squish artifact
- the squish could still be reproduced even after bringing back the white bars

Important deduction:

If the white bars are back and the squish is still present, then the squish is **not** caused by the full-bleed underlap itself.

That was the turning point.

### 6. Revert list internals toward `origin/master`

We then made the sharpest isolation cut:

- restored `InvertedMessageList` to the simpler `origin/master` architecture
- kept the current screen/chrome experimentation around it

What was restored:

- inverted `UITableView`
- `UIHostingConfiguration`
- grouped sender rows
- simpler diffable-data-source flow
- no custom hosting cell
- no explicit height cache
- no flattened per-bubble row model

Observed result:

- **message squish disappeared**

Conclusion:

- the squish was introduced by the table internals rewrite
- it was **not** caused by the full-bleed concept itself

This was the most important conclusion of the whole debugging session.

### 7. Reapply full-bleed on top of the stable old list

Once we knew the squish culprit, we put the full-bleed screen treatment back around the stable old list.

Result:

- squish stayed gone
- but the original full-bleed failure returned: a giant translucent/white field occupying much of the screen

That means there are actually **two separate bugs**:

1. `normal-table / custom-hosting / custom-sizing` rewrite caused squish
2. `stable inverted list + full-bleed overlay chrome` still produces the giant bottom/center field

### 8. Composer/chrome isolation attempts

We then tried to isolate the giant white field.

Tried:

- removing chat-screen blur
- switching chat-only glass modifiers to plain material
- forcing the composer overlay to `.fixedSize(horizontal: false, vertical: true)`
- replacing measured composer reserve with a semantic reserve
- mapping visual overlay reserve into inverted table insets
- correcting inverted “at bottom” and `scrollToBottom()` math to account for nonzero insets

Observed result:

- these were all plausible hypotheses
- none of them fully removed the giant field yet

Strong conclusion:

- the giant field is still unresolved
- but it is separate from the squish regression

## What We Know Now

### Confirmed

- `origin/master`-style inverted list internals are stable
- the normal-table rewrite introduced squish
- the squish is not fundamentally caused by full-bleed underlap
- there is a second full-bleed bug that manifests as a giant translucent/white field
- removing blur and glass alone did not eliminate that field

### Very likely

- the correct path is **not** “rewrite the transcript internals”
- the correct path is:
  - keep the stable old inverted list
  - reintroduce full-bleed carefully
  - solve the overlay/composer reserve bug separately

## Likely Root Causes by Symptom

### Symptom: message squish / spacing changes while scrolling

Most likely cause:

- the normal-table rewrite plus custom sizing/hosting stack

Evidence:

- squish persisted across white-bar and no-white-bar layouts
- squish disappeared when `InvertedMessageList` was restored toward `origin/master`

### Symptom: giant translucent/white field over much of the chat

Most likely cause:

- incorrect reserve/layout interaction between:
  - overlaid composer chrome
  - full-bleed transcript
  - inverted-table inset mapping

Not yet solved.

## Shortest Path Forward

At this point the most promising path is:

1. Keep the `origin/master`-style inverted list architecture.
2. Do **not** revive the normal-table rewrite.
3. Keep `just run-swift --sim` as the dev loop.
4. Continue debugging only the full-bleed overlay/composer reserve bug.

The next debugging steps should be narrower and more instrumented:

- log the actual reserve values used for the composer
- log the inverted table’s `contentInset`, `adjustedContentInset`, `contentOffset`, and visible bounds
- confirm whether the giant field corresponds to:
  - oversized reserve
  - clipped overlay
  - table content being shifted into a translucent overlay region
- try one fully explicit UIKit bottom overlay container with hard height constraints instead of a pure SwiftUI overlay, while keeping the old inverted list

## Practical Recommendation

Do **not** throw away the full-bleed idea.

The debugging results strongly suggest:

- full-bleed is still achievable
- the failed rewrite of the chat list internals was the wrong path

The best working direction now is:

- old stable inverted table
- minimal, explicit inset mapping for overlaid chrome
- solve the remaining white-field/composer-reserve bug in isolation

## Files Involved

Main files touched during this effort:

- [ChatView.swift](/Users/futurepaul/dev/sec/other-peoples-code/pika/ios/Sources/Views/ChatView.swift)
- [InvertedMessageList.swift](/Users/futurepaul/dev/sec/other-peoples-code/pika/ios/Sources/Views/InvertedMessageList.swift)
- [MessageBubbleViews.swift](/Users/futurepaul/dev/sec/other-peoples-code/pika/ios/Sources/Views/MessageBubbleViews.swift)
- [ChatInputBar.swift](/Users/futurepaul/dev/sec/other-peoples-code/pika/ios/Sources/Views/ChatInputBar.swift)
- [justfile](/Users/futurepaul/dev/sec/other-peoples-code/pika/justfile)
- [tools/run-ios](/Users/futurepaul/dev/sec/other-peoples-code/pika/tools/run-ios)

## Useful Commands

- Swift-only sim loop:
  - `just run-swift --sim`
- Full iOS loop:
  - `just run-ios --sim`

## Current State

As of this document:

- branch: `chat-full-height-table`
- checkpoint commit exists at `cc8067d`
- current worktree is still experimental and dirty
- the stable finding is the restoration of the old inverted list internals
- the remaining unsolved problem is the giant full-bleed overlay field

# Grok's thoughts

I ran what we've done through grok and this is what it had to say:

**Yes, the inverted `UITableView` + `UIHostingConfiguration` approach you restored in the stable checkpoint is still the gold standard for performant native-feeling chats** (exactly like what many production apps landed on after the same SwiftUI → UIKit pivot you went through). The “giant translucent/white field” is almost always a content-inset / reserve / background-color mismatch once you go full-bleed — not a fundamental fight with the system.

### People Talking About This on X
The closest real-world parallel I found is this May 2025 thread from iOS dev **@ondrejkorol** (600+ bookmarks, 64k views). He built a companion-app chat and hit *exactly* your sequence:

- SwiftUI `List` / `LazyVStack` → flickering + bad keyboard + no fine-grained scroll control  
- Switched to `UITableView` + `UIHostingConfiguration` for cells → huge perf win, stable scrolling, easy drag tracking  
- Then hit cell-insertion/keyboard quirks → moved to `UICollectionViewCell` containing a `UIHostingController` (instead of `UIHostingConfiguration`) and it “just works” with complex markdown/text selection.

He literally says: “if chat is the core experience… it MUST FEEL RIGHT… UITableView made a huge difference.” Your debugging log reads like his exact journey. No one in that thread (or the dozens of recent replies) reported white bars once they pinned the table to full superview edges and set `contentInsetAdjustmentBehavior = .never`.

Other X chatter is thinner on the exact “inverted + full-bleed” combo (most people have abandoned pure inverted tables in 2025–2026), but the safe-area complaints are identical to yours.

### The Minimal Fix That Matches Your “Keep Old List + Fix Overlay Reserve” Plan
You already proved the squish was from the normal-table rewrite. So **don’t touch `InvertedMessageList` internals**. Just make the full-bleed layer correct. The giant field is the SwiftUI parent background (or table background) leaking because the inverted insets aren’t flipped and the table isn’t truly edge-to-edge.

In your `ChatView` / representable wrapper (the ZStack or whatever hosts the table + floating composer):

```swift
ZStack {
    InvertedMessageList(...)          // your stable old one
        .ignoresSafeArea(.all)        // ← this is the key
        .background(chatBackgroundColor)  // match your bubble bg
    
    // top chrome (navigation or floating controls)
    // bottom composer (fixed or .safeAreaInset if you want)
}
```

Then **inside the UITableView** (in `makeUIView` / `updateUIView` or wherever you set insets):

```swift
tableView.contentInsetAdjustmentBehavior = .never
tableView.backgroundColor = .clear                     // or your chat bg
tableView.scrollIndicatorInsets = .zero

// Because of the scaleY: -1 transform, top/bottom are flipped!
let safe = view.safeAreaInsets                         // or tableView.safeAreaInsets
let composerHeight = measuredComposerHeight + safe.bottom  // home indicator

tableView.contentInset = UIEdgeInsets(
    top: composerHeight,      // becomes visual BOTTOM after inversion
    left: 0,
    bottom: safe.top,         // becomes visual TOP
    right: 0
)

// For “at bottom” detection and scrollToBottom:
let atBottomThreshold = tableView.contentInset.top + 20
```

That flip is the piece most people miss with inverted tables (the transform reverses which inset controls which visual edge). Your earlier “mapping visual overlay reserve into inverted table insets” was on the right track — this is just the corrected version that also respects the home indicator.

### Extra Polish for Native Look (No Old-School White Bars)
- `tableView.separatorStyle = .none`
- Top floating controls → `Material.ultraThin` overlay pinned to `safe.top`
- Composer → either a SwiftUI overlay with `.fixedSize()` or a real `inputAccessoryView` on the table (Apple still loves this for keyboard avoidance)
- If you ever want the progressive blur bars like Apple’s own apps (iOS 18+), add `.safeAreaBar` (new in iOS 26) on the scroll view instead of manual `safeAreaInset`.

### If You Ever Want to Future-Proof
The X consensus (and what Signal/Telegram-style apps do under the hood) is slowly moving to `UICollectionView` + `NSDiffableDataSource` + `UICollectionLayout.list` (or custom compositional). It gives you grouping, reactions, streaming text, and text selection without any inversion hacks. But your current stable inverted setup is *faster* to ship and already matches the “feels like Signal” bar you described.

Your shortest path forward (exactly what you wrote in the log) is 100 % correct:

1. Keep the `origin/master`-style inverted list  
2. Add `.ignoresSafeArea(.all)` + flipped insets + background clear  
3. Log the actual `contentInset` / `adjustedContentInset` / `contentOffset` after layout (you’ll see the giant field disappear immediately)

That will give you the full-height scroll-under-chrome look with zero white bars and zero performance regression. I’ve seen this exact pattern ship in multiple apps this year — it’s the “doesn’t fight the system” way.

Drop the branch/commit if you want me to sketch the exact diff, or tell me what the measured reserve values are logging right now and I’ll give you the precise inset formula. You’re super close!
