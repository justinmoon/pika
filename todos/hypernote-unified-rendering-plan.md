---
summary: Remaining Hypernote rendering migration work after the first desktop/Iced Hypernote slice landed
read_when:
  - continuing Hypernote rendering migration
  - removing Hypernote AST JSON from mobile clients
  - unifying ordinary chat markdown through Rust-owned parsing
  - removing client markdown dependencies
---

# Hypernote Unified Rendering Plan

## Current Status

The first desktop Hypernote slice is done.

What already landed:

1. `hypernote-mdx` was updated to the newer semantic-accessor API surface.
2. `hypernote-protocol` now owns the pure-Rust lowered Hypernote document plus
   typed submit-action extraction.
3. `pika_core` dual-writes typed `document` and `default_form_state` while
   still preserving legacy `ast_json` and `default_state` for compatibility.
4. Desktop/Iced now renders Hypernotes from typed data and dispatches
   `AppAction::HypernoteAction`.
5. Desktop keeps ordinary non-Hypernote markdown on `pulldown-cmark` for now.

That means the main remaining work is no longer "can desktop render
Hypernotes?" It can.

The remaining work is:

1. finish removing Hypernote AST JSON from iOS and Android
2. remove the legacy JSON fields from `HypernoteData`
3. unify ordinary chat rendering through a Rust-owned typed content path
4. remove `MarkdownUI`, `compose-markdown`, and `pulldown-cmark`

## Problem Statement

Pika still has two kinds of duplication:

1. Hypernotes:
   Rust parses them, but iOS and Android still consume legacy JSON fields
   instead of the typed document.
2. Ordinary chat messages:
   iOS, Android, and desktop still use separate markdown stacks:
   `MarkdownUI`, `compose-markdown`, and `pulldown-cmark`.

So the project is only partially complete. We fixed the first desktop consumer
and the Rust ownership boundary, but we have not finished the cross-platform
migration.

## Target End State

1. Rust owns parsing for both Hypernotes and ordinary markdown-ish chat
   content.
2. `hypernote-protocol` owns the generic typed document/lowering layer plus
   Pika-specific component/action semantics.
3. `pika_core` projects that into UniFFI-safe records along with message
   metadata and app-specific state.
4. SwiftUI, Kotlin, and Iced stay thin and render from Rust-owned Hypernote
   concepts instead of reparsing.
5. `ast_json`, `default_state`, `MarkdownUI`, `compose-markdown`, and
   `pulldown-cmark` are removed.

## Architecture Notes

The current architecture cut is the right one:

1. `hypernote-mdx`
   parser plus parser-semantic helpers
2. `hypernote-protocol`
   owned Hypernote document, lowering, built-in markdown node set, and
   protocol-level component/action semantics
3. `pika_core`
   message integration, form-state normalization, responder/tally state, and
   UniFFI projection
4. clients
   thin renderers over markdown node kinds and protocol component kinds

One upstream nice-to-have still exists:

1. A typed semantic tree export from `hypernote-mdx` could simplify
   `hypernote-protocol` further.

But that is no longer a blocker for Pika.

## Acceptance Criteria

This migration is done when all of the following are true:

1. No Hypernote AST JSON transport remains in `HypernoteData`.
2. No client reparses Hypernotes locally from JSON.
3. Ordinary chat messages render from a Rust-owned typed content path on all
   clients.
4. `MarkdownUI`, `compose-markdown`, and `pulldown-cmark` are removed.
5. Automated coverage and manual QA are strong enough to trust the migration.
6. We have a credible benchmark story for cold parse and repeated render.

## Remaining Stages

### Stage 1: Finish Existing Hypernote JSON Removal

Goal:

Switch iOS and Android to the typed Hypernote document, then remove the legacy
JSON fields from `HypernoteData`.

Already done:

1. typed `document` exists
2. typed `default_form_state` exists
3. submit-action extraction no longer depends on AST JSON
4. desktop is already rendering the typed path
5. `hypernote-protocol` owns the lowered document

Remaining work:

1. Update
   [HypernoteRenderer.swift](/Users/futurepaul/dev/sec/other-peoples-code/pika/ios/Sources/Views/HypernoteRenderer.swift)
   to render from `hypernote.document` and seed local form state from
   `hypernote.defaultFormState`.
2. Update
   [HypernoteRenderer.kt](/Users/futurepaul/dev/sec/other-peoples-code/pika/android/app/src/main/java/com/pika/app/ui/screens/HypernoteRenderer.kt)
   to render from the typed document and typed default form state instead of
   local JSON parsing.
3. Remove `ast_json` and `default_state` from
   [state.rs](/Users/futurepaul/dev/sec/other-peoples-code/pika/rust/src/state.rs).
4. Remove the remaining legacy JSON plumbing in
   [hypernote.rs](/Users/futurepaul/dev/sec/other-peoples-code/pika/rust/src/hypernote.rs)
   and
   [storage.rs](/Users/futurepaul/dev/sec/other-peoples-code/pika/rust/src/core/storage.rs).
5. Regenerate bindings after the field removal.

Verification:

1. existing iOS Hypernote UI tests stay green
2. existing Android Hypernote UI tests stay green
3. Rust storage/tests stop asserting against `ast_json`
4. final grep-level check finds no non-generated `ast_json`, `astJson`,
   `default_state`, or client-side Hypernote JSON decode left in source

### Stage 2: Unify Ordinary Message Parsing

Goal:

Route ordinary chat markdown through a Rust-owned typed content path instead of
per-client markdown parsing.

Notes:

1. This may reuse the exact `HypernoteDocument` model or a close sibling typed
   content model.
2. Message-derived parse/render data should be treated as immutable derived
   state and cached conservatively.
3. A temporary desktop bridge is acceptable if any parser-parity edge remains
   while the shared path is being proven out.

Open design questions:

1. whether ordinary messages should use the exact Hypernote document or a
   sibling content model
2. where runtime caching should live
3. whether any temporary desktop bridge is worth keeping during rollout

Acceptance:

1. regular messages no longer depend on client-local markdown parsing, except
   for any explicitly temporary bridge still called out during rollout
2. repeated display of the same message does not repeatedly reparse content
3. the shared path covers the message shapes Pika actually sends in chat

### Stage 3: Remove Client Markdown Dependencies

Goal:

Delete the client markdown libraries once the shared path is trusted.

Acceptance:

1. iOS no longer depends on `MarkdownUI`
2. Android no longer depends on `compose-markdown`
3. desktop no longer depends on `pulldown-cmark`
4. no hidden fallback path quietly preserves the old libraries

### Stage 4: QA and Benchmarking

Goal:

Prove the shared path is correct and worth keeping.

What to validate:

1. headings, emphasis, links, lists, code blocks, images, and blockquotes
2. Hypernote components such as `Details`, `SubmitButton`, and form defaults
3. scrolling and repeated re-render behavior in long chats
4. cold parse versus warm repeated render on representative message fixtures

Acceptance:

1. the shared renderer is stable in normal chat usage on iOS, Android, and
   desktop
2. performance claims are backed by measurements from real client paths

## Immediate Next Step

The next practical slice is:

1. switch iOS Hypernotes off `astJson` and `defaultState`
2. switch Android Hypernotes off the same legacy fields
3. then remove the legacy JSON fields in one cleanup pass

That keeps the current desktop win, finishes the Hypernote transport
migration, and leaves ordinary-message unification as the next project.
