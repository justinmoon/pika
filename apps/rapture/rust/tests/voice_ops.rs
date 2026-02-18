use rapture_core::control::{ControlEnvelope, ControlState};
use rapture_core::permissions::{has_permission, Permission, PERM_CONNECT_VOICE, PERM_SPEAK_VOICE};
use rapture_core::voice::{
    VoiceApplyOutcome, VoiceEnvelope, VoiceError, VoicePermissionLookup, VoiceState,
};
use rapture_core::ChannelKind;

#[test]
fn voice_join_requires_connect_permission() {
    let control = build_control();
    let perms = ControlPermissions { control: &control };
    let mut voice = VoiceState::default();

    voice
        .apply(
            VoiceEnvelope::session_start(
                "v-op-1".to_string(),
                1,
                "g-1".to_string(),
                "v-1".to_string(),
                "alice".to_string(),
                "sess-1".to_string(),
                "moq://local/1".to_string(),
            ),
            &perms,
        )
        .expect("session start");

    let err = voice
        .apply(
            VoiceEnvelope::participant_join(
                "v-op-2".to_string(),
                2,
                "g-1".to_string(),
                "v-1".to_string(),
                "mallory".to_string(),
                "sess-1".to_string(),
            ),
            &perms,
        )
        .expect_err("must fail");

    assert_eq!(
        err,
        VoiceError::PermissionDenied {
            actor: "mallory".to_string(),
            permission: Permission::ConnectVoice,
        }
    );
}

#[test]
fn voice_track_advertise_requires_speak_permission() {
    let mut control = build_control();
    control
        .apply(ControlEnvelope::member_add(
            "op-9".to_string(),
            9,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ))
        .expect("member add");
    control
        .apply(ControlEnvelope::member_roles_set(
            "op-10".to_string(),
            10,
            "g-1".to_string(),
            "alice".to_string(),
            "bob".to_string(),
            vec!["role-voice".to_string()],
        ))
        .expect("role set");

    let perms = ControlPermissions { control: &control };
    let mut voice = VoiceState::default();
    voice
        .apply(
            VoiceEnvelope::session_start(
                "v-op-1".to_string(),
                1,
                "g-1".to_string(),
                "v-1".to_string(),
                "alice".to_string(),
                "sess-1".to_string(),
                "moq://local/1".to_string(),
            ),
            &perms,
        )
        .expect("session start");
    voice
        .apply(
            VoiceEnvelope::participant_join(
                "v-op-2".to_string(),
                2,
                "g-1".to_string(),
                "v-1".to_string(),
                "bob".to_string(),
                "sess-1".to_string(),
            ),
            &perms,
        )
        .expect("bob join");

    let err = voice
        .apply(
            VoiceEnvelope::track_advertise(
                "v-op-3".to_string(),
                3,
                "g-1".to_string(),
                "v-1".to_string(),
                "bob".to_string(),
                "sess-1".to_string(),
                "audio-main".to_string(),
                "opus".to_string(),
            ),
            &perms,
        )
        .expect_err("no speak permission");
    assert_eq!(
        err,
        VoiceError::PermissionDenied {
            actor: "bob".to_string(),
            permission: Permission::SpeakVoice,
        }
    );
}

#[test]
fn duplicate_voice_op_is_noop() {
    let control = build_control();
    let perms = ControlPermissions { control: &control };
    let mut voice = VoiceState::default();
    let op = VoiceEnvelope::session_start(
        "v-op-1".to_string(),
        1,
        "g-1".to_string(),
        "v-1".to_string(),
        "alice".to_string(),
        "sess-1".to_string(),
        "moq://local/1".to_string(),
    );
    let first = voice.apply(op.clone(), &perms).expect("first");
    let second = voice.apply(op, &perms).expect("second");
    assert_eq!(first, VoiceApplyOutcome::Applied);
    assert_eq!(second, VoiceApplyOutcome::Duplicate);
}

struct ControlPermissions<'a> {
    control: &'a ControlState,
}

impl VoicePermissionLookup for ControlPermissions<'_> {
    fn has_permission(
        &self,
        guild_id: &str,
        channel_id: &str,
        actor: &str,
        permission: Permission,
    ) -> bool {
        let Some(guild) = self.control.guilds.get(guild_id) else {
            return false;
        };
        has_permission(guild, actor, Some(channel_id), permission)
    }
}

fn build_control() -> ControlState {
    let mut state = ControlState::default();
    let ops = vec![
        ControlEnvelope::guild_create(
            "op-1".to_string(),
            1,
            "g-1".to_string(),
            "alice".to_string(),
            "Guild One".to_string(),
        ),
        ControlEnvelope::channel_create(
            "op-2".to_string(),
            2,
            "g-1".to_string(),
            "alice".to_string(),
            "v-1".to_string(),
            "voice".to_string(),
            ChannelKind::Voice,
        ),
        ControlEnvelope::role_upsert(
            "op-3".to_string(),
            3,
            "g-1".to_string(),
            "alice".to_string(),
            "role-voice".to_string(),
            "Voice".to_string(),
            PERM_CONNECT_VOICE,
            10,
        ),
        ControlEnvelope::role_upsert(
            "op-4".to_string(),
            4,
            "g-1".to_string(),
            "alice".to_string(),
            "role-speaker".to_string(),
            "Speaker".to_string(),
            PERM_SPEAK_VOICE,
            11,
        ),
    ];
    for op in ops {
        state.apply(op).expect("apply");
    }
    state
}
