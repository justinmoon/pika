use rapture_core::control::{ControlEnvelope, ControlState};
use rapture_core::permissions::{has_permission, Permission, PERM_SEND_MESSAGE, PERM_VIEW_CHANNEL};
use rapture_core::ChannelKind;

#[test]
fn allow_deny_precedence_prefers_user_deny() {
    let mut state = base_state();
    state
        .apply(ControlEnvelope::channel_permissions_set(
            "op-perm-1".to_string(),
            10,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            vec!["role-mod".to_string()],
            vec![],
            vec![],
            vec!["bob".to_string()],
        ))
        .expect("set permissions");

    let guild = state.guilds.get("g-1").expect("guild exists");
    assert!(!has_permission(
        guild,
        "bob",
        Some("c-1"),
        Permission::SendMessage
    ));
}

#[test]
fn admin_override_beats_channel_denies() {
    let mut state = base_state();
    state
        .apply(ControlEnvelope::channel_permissions_set(
            "op-perm-2".to_string(),
            11,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            vec![],
            vec!["role-owner".to_string()],
            vec![],
            vec!["alice".to_string()],
        ))
        .expect("set permissions");

    let guild = state.guilds.get("g-1").expect("guild exists");
    assert!(has_permission(
        guild,
        "alice",
        Some("c-1"),
        Permission::ManageChannels
    ));
}

#[test]
fn channel_allow_user_grants_without_base_role() {
    let mut state = base_state();
    state
        .apply(ControlEnvelope::channel_permissions_set(
            "op-perm-3".to_string(),
            12,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            vec![],
            vec![],
            vec!["carol".to_string()],
            vec![],
        ))
        .expect("set permissions");

    let guild = state.guilds.get("g-1").expect("guild exists");
    assert!(has_permission(
        guild,
        "carol",
        Some("c-1"),
        Permission::SendMessage
    ));
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
        ControlEnvelope::role_upsert(
            "op-2".to_string(),
            2,
            "g-1".to_string(),
            "alice".to_string(),
            "role-mod".to_string(),
            "Mod".to_string(),
            PERM_VIEW_CHANNEL | PERM_SEND_MESSAGE,
            5,
        ),
        ControlEnvelope::member_add(
            "op-3".to_string(),
            3,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ),
        ControlEnvelope::member_add(
            "op-4".to_string(),
            4,
            "g-1".to_string(),
            "alice".to_string(),
            "carol".to_string(),
        ),
        ControlEnvelope::member_roles_set(
            "op-5".to_string(),
            5,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
            vec!["role-mod".to_string()],
        ),
        ControlEnvelope::channel_create(
            "op-6".to_string(),
            6,
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
