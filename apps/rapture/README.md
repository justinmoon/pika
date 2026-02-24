# Rapture

A cross-platform app built with [RMP](https://github.com/nickthecook/rmp) (Rust Multiplatform).

## Quick Start

```bash
just --justfile apps/rapture/justfile doctor
just rapture run-ios
just rapture run-android
just --justfile apps/rapture/justfile pre-merge
```

## Current UI Scope

- Discord-style structural shell: server rail, channel navigation, timeline/composer.
- Control-plane actions: create guild/channel, invite/kick/ban member, role/policy updates.
- Chat timeline actions: send/edit/delete message, put/remove `:+1:` reaction.
- Voice actions: join/leave voice session, toggle mute, toggle speaking state.
