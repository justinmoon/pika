use rapture_core::control::{
    ApplyOutcome, ControlBody, ControlEnvelope, ControlError, ControlState,
};
use rapture_core::permissions::{
    has_permission, Permission, PERM_MANAGE_MESSAGES, PERM_SEND_MESSAGE, PERM_VIEW_CHANNEL,
};
use rapture_core::ChannelKind;

#[test]
fn replay_is_deterministic() {
    let mut s1 = ControlState::default();
    let mut s2 = ControlState::default();
    let ops = sample_ops();

    for op in &ops {
        let out1 = s1.apply(op.clone()).expect("s1 apply");
        let out2 = s2.apply(op.clone()).expect("s2 apply");
        assert_eq!(out1, ApplyOutcome::Applied);
        assert_eq!(out2, ApplyOutcome::Applied);
    }

    assert_eq!(s1, s2);
}

#[test]
fn duplicate_op_id_is_noop() {
    let mut state = ControlState::default();
    let op = ControlEnvelope::guild_create(
        "op-1".to_string(),
        1,
        "g-1".to_string(),
        "alice".to_string(),
        "Guild One".to_string(),
    );

    let first = state.apply(op.clone()).expect("first apply");
    let second = state.apply(op).expect("second apply");

    assert_eq!(first, ApplyOutcome::Applied);
    assert_eq!(second, ApplyOutcome::Duplicate);
    assert_eq!(state.guilds.len(), 1);
    assert_eq!(state.seen_op_ids.len(), 1);
}

#[test]
fn unknown_schema_version_is_rejected() {
    let mut state = ControlState::default();
    let op = ControlEnvelope {
        schema: "rapture.control.v999".to_string(),
        guild_id: "g-1".to_string(),
        actor: "alice".to_string(),
        op_id: "bad-1".to_string(),
        ts_ms: 1,
        body: ControlBody::GuildCreate {
            name: "Guild One".to_string(),
        },
    };

    let err = state.apply(op).expect_err("must fail");
    assert_eq!(
        err,
        ControlError::UnknownSchema("rapture.control.v999".to_string())
    );
}

#[test]
fn invalid_actor_is_rejected_for_channel_create() {
    let mut state = ControlState::default();
    state
        .apply(ControlEnvelope::guild_create(
            "op-1".to_string(),
            1,
            "g-1".to_string(),
            "alice".to_string(),
            "Guild One".to_string(),
        ))
        .expect("guild create");

    let err = state
        .apply(ControlEnvelope::channel_create(
            "op-2".to_string(),
            2,
            "g-1".to_string(),
            "mallory".to_string(),
            "c-1".to_string(),
            "general".to_string(),
            ChannelKind::Text,
        ))
        .expect_err("non-member actor must fail");

    assert_eq!(
        err,
        ControlError::PermissionDenied {
            actor: "mallory".to_string(),
            permission: "ManageChannels".to_string(),
        }
    );
}

#[test]
fn banned_member_cannot_be_reinvited() {
    let mut state = ControlState::default();
    state
        .apply(ControlEnvelope::guild_create(
            "op-1".to_string(),
            1,
            "g-1".to_string(),
            "alice".to_string(),
            "Guild One".to_string(),
        ))
        .expect("guild create");
    state
        .apply(ControlEnvelope::member_add(
            "op-2".to_string(),
            2,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ))
        .expect("member add");
    state
        .apply(ControlEnvelope::member_ban(
            "op-3".to_string(),
            3,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ))
        .expect("member ban");

    let err = state
        .apply(ControlEnvelope::member_add(
            "op-4".to_string(),
            4,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ))
        .expect_err("banned member must fail");
    assert_eq!(
        err,
        ControlError::MemberBanned {
            guild_id: "g-1".to_string(),
            member: "bob".to_string(),
        }
    );
}

#[test]
fn channel_create_conflict_is_rejected() {
    let mut state = ControlState::default();
    state
        .apply(ControlEnvelope::guild_create(
            "op-1".to_string(),
            1,
            "g-1".to_string(),
            "alice".to_string(),
            "Guild One".to_string(),
        ))
        .expect("guild create");
    state
        .apply(ControlEnvelope::channel_create(
            "op-2".to_string(),
            2,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            "general".to_string(),
            ChannelKind::Text,
        ))
        .expect("first create");

    let err = state
        .apply(ControlEnvelope::channel_create(
            "op-3".to_string(),
            3,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            "general".to_string(),
            ChannelKind::Text,
        ))
        .expect_err("duplicate channel must fail");

    assert_eq!(
        err,
        ControlError::ChannelExists {
            guild_id: "g-1".to_string(),
            channel_id: "c-1".to_string(),
        }
    );
}

#[test]
fn remove_member_from_channel_sets_deny_policy() {
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
        ControlEnvelope::channel_create(
            "op-3".to_string(),
            3,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            "general".to_string(),
            ChannelKind::Text,
        ),
        ControlEnvelope::channel_member_remove(
            "op-4".to_string(),
            4,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            "bob".to_string(),
        ),
    ];
    for op in ops {
        state.apply(op).expect("op apply");
    }

    let guild = state.guilds.get("g-1").expect("guild");
    assert!(!has_permission(
        guild,
        "bob",
        Some("c-1"),
        Permission::ViewChannel
    ));
}

fn sample_ops() -> Vec<ControlEnvelope> {
    vec![
        ControlEnvelope::guild_create(
            "op-1".to_string(),
            1,
            "g-1".to_string(),
            "alice".to_string(),
            "Guild One".to_string(),
        ),
        ControlEnvelope::role_upsert(
            "op-2".to_string(),
            2,
            "g-1".to_string(),
            "alice".to_string(),
            "role-mod".to_string(),
            "Moderator".to_string(),
            PERM_VIEW_CHANNEL | PERM_SEND_MESSAGE | PERM_MANAGE_MESSAGES,
            10,
        ),
        ControlEnvelope::member_add(
            "op-3".to_string(),
            3,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ),
        ControlEnvelope::member_roles_set(
            "op-4".to_string(),
            4,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
            vec!["role-mod".to_string()],
        ),
        ControlEnvelope::channel_create(
            "op-5".to_string(),
            5,
            "g-1".to_string(),
            "alice".to_string(),
            "c-general".to_string(),
            "general".to_string(),
            ChannelKind::Text,
        ),
    ]
}
