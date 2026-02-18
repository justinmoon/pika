use std::collections::BTreeSet;

use rapture_core::control::{ControlEnvelope, ControlState};
use rapture_core::reconcile::{reconcile_channel, InMemoryMembershipBackend};
use rapture_core::ChannelKind;

#[test]
fn add_remove_diff_correctness() {
    let state = base_state();
    let mut backend = InMemoryMembershipBackend::default();
    backend.set_actual("g-1", "c-1", set_of(&["alice", "bob", "stale-user"]));

    let report = reconcile_channel(&state, "g-1", "c-1", &mut backend).expect("reconcile");

    assert_eq!(report.diff.to_add, set_of(&["carol"]));
    assert_eq!(report.diff.to_remove, set_of(&["stale-user"]));
    assert_eq!(
        backend.members("g-1", "c-1"),
        set_of(&["alice", "bob", "carol"])
    );
}

#[test]
fn partial_failure_retry() {
    let state = base_state();
    let mut backend = InMemoryMembershipBackend::default();
    backend.fail_add_once("g-1", "c-1", "carol");

    let first = reconcile_channel(&state, "g-1", "c-1", &mut backend).expect("first reconcile");
    assert_eq!(first.added, set_of(&["alice", "bob"]));
    assert_eq!(first.failed_add, set_of(&["carol"]));
    assert_eq!(backend.members("g-1", "c-1"), set_of(&["alice", "bob"]));

    let second = reconcile_channel(&state, "g-1", "c-1", &mut backend).expect("second reconcile");
    assert!(second.failed_add.is_empty());
    assert_eq!(second.added, set_of(&["carol"]));
    assert_eq!(
        backend.members("g-1", "c-1"),
        set_of(&["alice", "bob", "carol"])
    );
}

#[test]
fn idempotent_rerun() {
    let state = base_state();
    let mut backend = InMemoryMembershipBackend::default();

    let first = reconcile_channel(&state, "g-1", "c-1", &mut backend).expect("first reconcile");
    assert!(first.converged());
    assert_eq!(
        backend.members("g-1", "c-1"),
        set_of(&["alice", "bob", "carol"])
    );

    let second = reconcile_channel(&state, "g-1", "c-1", &mut backend).expect("second reconcile");
    assert!(second.diff.to_add.is_empty());
    assert!(second.diff.to_remove.is_empty());
    assert!(second.added.is_empty());
    assert!(second.removed.is_empty());
}

fn base_state() -> ControlState {
    let mut state = ControlState::default();
    let ops = vec![
        ControlEnvelope::guild_create(
            "op-1".to_string(),
            1,
            "g-1".to_string(),
            "alice".to_string(),
            "Guild One".to_string(),
        ),
        ControlEnvelope::member_add(
            "op-2".to_string(),
            2,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ),
        ControlEnvelope::member_add(
            "op-3".to_string(),
            3,
            "g-1".to_string(),
            "alice".to_string(),
            "carol".to_string(),
        ),
        ControlEnvelope::channel_create(
            "op-4".to_string(),
            4,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            "general".to_string(),
            ChannelKind::Text,
        ),
    ];

    for op in ops {
        state.apply(op).expect("op applies");
    }
    state
}

fn set_of(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|v| v.to_string()).collect()
}
