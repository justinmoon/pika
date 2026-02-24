use std::collections::BTreeSet;

use rapture_core::channel_groups::ChannelGroupDirectory;
use rapture_core::control::{ControlEnvelope, ControlState};
use rapture_core::ChannelKind;

#[test]
fn initial_keys_are_not_deterministic_from_public_ids() {
    let control = control_with_channel();

    let mut a = ChannelGroupDirectory::default();
    let mut b = ChannelGroupDirectory::default();
    a.ensure_from_control(&control);
    b.ensure_from_control(&control);

    let (_, key_a) = a.current_epoch_key("g-1", "c-1").expect("key a");
    let (_, key_b) = b.current_epoch_key("g-1", "c-1").expect("key b");
    assert_ne!(key_a, key_b);
}

#[test]
fn membership_change_rotates_to_fresh_random_key() {
    let control = control_with_channel();
    let mut directory = ChannelGroupDirectory::default();
    directory.ensure_from_control(&control);

    let (epoch0, key0) = directory.current_epoch_key("g-1", "c-1").expect("epoch0");
    assert_eq!(epoch0, 0);

    let mut members = BTreeSet::new();
    members.insert("alice".to_string());
    assert!(directory
        .reconcile_members("g-1", "c-1", members.clone())
        .expect("reconcile 1"));

    let (epoch1, key1) = directory.current_epoch_key("g-1", "c-1").expect("epoch1");
    assert_eq!(epoch1, 1);
    assert_ne!(key1, key0);

    assert!(!directory
        .reconcile_members("g-1", "c-1", members.clone())
        .expect("reconcile noop"));

    members.insert("bob".to_string());
    assert!(directory
        .reconcile_members("g-1", "c-1", members)
        .expect("reconcile 2"));
    let (epoch2, key2) = directory.current_epoch_key("g-1", "c-1").expect("epoch2");
    assert_eq!(epoch2, 2);
    assert_ne!(key2, key1);
}

fn control_with_channel() -> ControlState {
    let mut control = ControlState::default();
    control
        .apply(ControlEnvelope::guild_create(
            "op-1".to_string(),
            1,
            "g-1".to_string(),
            "alice".to_string(),
            "Guild One".to_string(),
        ))
        .expect("guild");
    control
        .apply(ControlEnvelope::channel_create(
            "op-2".to_string(),
            2,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            "general".to_string(),
            ChannelKind::Text,
        ))
        .expect("channel");
    control
}
