use rapture_core::control::ControlEnvelope;
use rapture_core::sim::LocalRelay;
use rapture_core::ChannelKind;

#[test]
#[ignore = "requires RAPTURE_E2E_LOCAL=1"]
fn guild_invite_channel_join_encrypted_send_receive() {
    if std::env::var("RAPTURE_E2E_LOCAL").ok().as_deref() != Some("1") {
        return;
    }

    let mut relay = LocalRelay::default();
    relay.register_client("alice");
    relay.register_client("bob");

    relay
        .apply_control(ControlEnvelope::guild_create(
            "op-1".to_string(),
            1,
            "g-1".to_string(),
            "alice".to_string(),
            "Guild One".to_string(),
        ))
        .expect("guild create");
    relay
        .apply_control(ControlEnvelope::member_add(
            "op-2".to_string(),
            2,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ))
        .expect("member add");
    relay
        .apply_control(ControlEnvelope::channel_create(
            "op-3".to_string(),
            3,
            "g-1".to_string(),
            "alice".to_string(),
            "c-general".to_string(),
            "general".to_string(),
            ChannelKind::Text,
        ))
        .expect("channel create");

    relay
        .send_message("g-1", "c-general", "alice", "m-1", "hello from alice")
        .expect("send message");

    let alice_tl = relay.timeline("alice", "g-1", "c-general");
    let bob_tl = relay.timeline("bob", "g-1", "c-general");
    assert_eq!(alice_tl.len(), 1);
    assert_eq!(bob_tl.len(), 1);
    assert_eq!(alice_tl[0].content, "hello from alice");
    assert_eq!(bob_tl[0].content, "hello from alice");
}

#[test]
#[ignore = "requires RAPTURE_E2E_LOCAL=1"]
fn removed_member_cannot_decrypt_subsequent_messages() {
    if std::env::var("RAPTURE_E2E_LOCAL").ok().as_deref() != Some("1") {
        return;
    }

    let mut relay = LocalRelay::default();
    relay.register_client("alice");
    relay.register_client("bob");

    relay
        .apply_control(ControlEnvelope::guild_create(
            "op-1".to_string(),
            1,
            "g-1".to_string(),
            "alice".to_string(),
            "Guild One".to_string(),
        ))
        .expect("guild create");
    relay
        .apply_control(ControlEnvelope::member_add(
            "op-2".to_string(),
            2,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ))
        .expect("member add");
    relay
        .apply_control(ControlEnvelope::channel_create(
            "op-3".to_string(),
            3,
            "g-1".to_string(),
            "alice".to_string(),
            "c-general".to_string(),
            "general".to_string(),
            ChannelKind::Text,
        ))
        .expect("channel create");

    relay
        .send_message("g-1", "c-general", "alice", "m-1", "before removal")
        .expect("first send");

    relay
        .apply_control(ControlEnvelope::member_remove(
            "op-4".to_string(),
            4,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ))
        .expect("member remove");

    relay
        .send_message("g-1", "c-general", "alice", "m-2", "after removal")
        .expect("second send");

    let alice_tl = relay.timeline("alice", "g-1", "c-general");
    let bob_tl = relay.timeline("bob", "g-1", "c-general");

    assert_eq!(alice_tl.len(), 2);
    assert_eq!(alice_tl[1].content, "after removal");
    assert_eq!(bob_tl.len(), 1);
    assert_eq!(bob_tl[0].content, "before removal");
    assert_eq!(relay.decrypt_failures("bob"), Some(1));
}
