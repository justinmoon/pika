use std::time::Duration;

use pika_media::session::InMemoryRelay;
use rapture_core::control::{ControlEnvelope, ControlState};
use rapture_core::permissions::{
    has_permission, Permission, PERM_CONNECT_VOICE, PERM_MUTE_MEMBERS, PERM_SPEAK_VOICE,
    PERM_VIEW_CHANNEL,
};
use rapture_core::voice::{VoiceEnvelope, VoiceError, VoicePermissionLookup, VoiceState};
use rapture_core::voice_media::{connect_in_memory_peer, default_broadcast_base};
use rapture_core::ChannelKind;

const RELAY_AUTH: &str = "capv1_1111111111111111111111111111111111111111111111111111111111111111";
const READY_TIMEOUT: Duration = Duration::from_millis(300);
const FRAME_TIMEOUT: Duration = Duration::from_secs(1);

#[test]
#[ignore = "requires RAPTURE_E2E_MOQ=1"]
fn join_leave_voice_channel() {
    if std::env::var("RAPTURE_E2E_MOQ").ok().as_deref() != Some("1") {
        return;
    }

    let control = build_voice_control_state();
    let perms = ControlPermissions { control: &control };
    let mut voice = VoiceState::default();

    voice
        .apply(
            VoiceEnvelope::session_start(
                "voice-1".to_string(),
                1,
                "g-1".to_string(),
                "v-1".to_string(),
                "alice".to_string(),
                "sess-1".to_string(),
                "moq://local-relay/room/sess-1".to_string(),
            ),
            &perms,
        )
        .expect("session start");
    voice
        .apply(
            VoiceEnvelope::participant_join(
                "voice-2".to_string(),
                2,
                "g-1".to_string(),
                "v-1".to_string(),
                "alice".to_string(),
                "sess-1".to_string(),
            ),
            &perms,
        )
        .expect("alice join");
    voice
        .apply(
            VoiceEnvelope::participant_join(
                "voice-3".to_string(),
                3,
                "g-1".to_string(),
                "v-1".to_string(),
                "bob".to_string(),
                "sess-1".to_string(),
            ),
            &perms,
        )
        .expect("bob join");

    let (session_id, moq_url) = {
        let room = voice.room("g-1", "v-1").expect("room");
        (
            room.active_session_id.clone().expect("active session id"),
            room.moq_url.clone().expect("moq url"),
        )
    };
    let broadcast_base = default_broadcast_base("g-1", "v-1", &session_id);
    let relay = InMemoryRelay::new();
    let alice_peer = connect_in_memory_peer(
        relay.clone(),
        &moq_url,
        RELAY_AUTH,
        &broadcast_base,
        "alice",
    )
    .expect("alice media peer");
    let bob_peer = connect_in_memory_peer(relay, &moq_url, RELAY_AUTH, &broadcast_base, "bob")
        .expect("bob media peer");
    let alice_rx = alice_peer
        .subscribe_to(&bob_peer.participant_label)
        .expect("alice subscribe to bob");
    let bob_rx = bob_peer
        .subscribe_to(&alice_peer.participant_label)
        .expect("bob subscribe to alice");
    alice_rx
        .wait_ready(READY_TIMEOUT)
        .expect("alice subscription ready");
    bob_rx
        .wait_ready(READY_TIMEOUT)
        .expect("bob subscription ready");

    let bob_payload = vec![7_u8, 9_u8, 11_u8];
    let bob_delivered = bob_peer
        .publish_audio_frame(1, 20_000, bob_payload.clone())
        .expect("bob publish");
    assert_eq!(bob_delivered, 1, "bob publish should reach alice");
    let bob_to_alice = alice_rx
        .recv_timeout(FRAME_TIMEOUT)
        .expect("alice receives bob frame");
    assert_eq!(bob_to_alice.seq, 1);
    assert_eq!(bob_to_alice.timestamp_us, 20_000);
    assert_eq!(bob_to_alice.payload, bob_payload);

    let alice_payload = vec![1_u8, 3_u8, 5_u8];
    let alice_delivered = alice_peer
        .publish_audio_frame(2, 40_000, alice_payload.clone())
        .expect("alice publish");
    assert_eq!(alice_delivered, 1, "alice publish should reach bob");
    let alice_to_bob = bob_rx
        .recv_timeout(FRAME_TIMEOUT)
        .expect("bob receives alice frame");
    assert_eq!(alice_to_bob.seq, 2);
    assert_eq!(alice_to_bob.timestamp_us, 40_000);
    assert_eq!(alice_to_bob.payload, alice_payload);

    voice
        .apply(
            VoiceEnvelope::participant_leave(
                "voice-4".to_string(),
                4,
                "g-1".to_string(),
                "v-1".to_string(),
                "bob".to_string(),
                "sess-1".to_string(),
            ),
            &perms,
        )
        .expect("bob leave");

    let room = voice.room("g-1", "v-1").expect("room");
    assert_eq!(room.participants.len(), 1);
    assert!(room.participants.contains_key("alice"));
}

#[test]
#[ignore = "requires RAPTURE_E2E_MOQ=1"]
fn mute_unmute_state_propagation() {
    if std::env::var("RAPTURE_E2E_MOQ").ok().as_deref() != Some("1") {
        return;
    }

    let control = build_voice_control_state();
    let perms = ControlPermissions { control: &control };
    let mut voice = VoiceState::default();
    start_room(&mut voice, &perms);

    voice
        .apply(
            VoiceEnvelope::participant_state(
                "voice-10".to_string(),
                10,
                "g-1".to_string(),
                "v-1".to_string(),
                "alice".to_string(),
                "sess-1".to_string(),
                "bob".to_string(),
                true,
                false,
                false,
            ),
            &perms,
        )
        .expect("alice mutes bob");
    assert!(
        voice
            .room("g-1", "v-1")
            .expect("room")
            .participants
            .get("bob")
            .expect("bob state")
            .muted
    );

    voice
        .apply(
            VoiceEnvelope::participant_state(
                "voice-11".to_string(),
                11,
                "g-1".to_string(),
                "v-1".to_string(),
                "bob".to_string(),
                "sess-1".to_string(),
                "bob".to_string(),
                false,
                true,
                false,
            ),
            &perms,
        )
        .expect("bob unmutes self and speaks");
    let room = voice.room("g-1", "v-1").expect("room");
    let bob = room.participants.get("bob").expect("bob state");
    assert!(!bob.muted);
    assert!(bob.speaking);

    voice
        .apply(
            VoiceEnvelope::track_advertise(
                "voice-12".to_string(),
                12,
                "g-1".to_string(),
                "v-1".to_string(),
                "bob".to_string(),
                "sess-1".to_string(),
                "audio-bob-main".to_string(),
                "opus".to_string(),
            ),
            &perms,
        )
        .expect("track advertise");
    assert_eq!(room_track_count(&voice), 1);
}

#[test]
#[ignore = "requires RAPTURE_E2E_MOQ=1"]
fn unauthorized_voice_join_denied() {
    if std::env::var("RAPTURE_E2E_MOQ").ok().as_deref() != Some("1") {
        return;
    }

    let control = build_voice_control_state();
    let perms = ControlPermissions { control: &control };
    let mut voice = VoiceState::default();
    start_room(&mut voice, &perms);

    let err = voice
        .apply(
            VoiceEnvelope::participant_join(
                "voice-20".to_string(),
                20,
                "g-1".to_string(),
                "v-1".to_string(),
                "mallory".to_string(),
                "sess-1".to_string(),
            ),
            &perms,
        )
        .expect_err("mallory must be denied");

    assert_eq!(
        err,
        VoiceError::PermissionDenied {
            actor: "mallory".to_string(),
            permission: Permission::ConnectVoice,
        }
    );
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

fn build_voice_control_state() -> ControlState {
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
            "role-voice-user".to_string(),
            "VoiceUser".to_string(),
            PERM_VIEW_CHANNEL | PERM_CONNECT_VOICE | PERM_SPEAK_VOICE,
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
            vec!["role-voice-user".to_string()],
        ),
        ControlEnvelope::channel_create(
            "op-5".to_string(),
            5,
            "g-1".to_string(),
            "alice".to_string(),
            "v-1".to_string(),
            "voice".to_string(),
            ChannelKind::Voice,
        ),
        ControlEnvelope::role_upsert(
            "op-6".to_string(),
            6,
            "g-1".to_string(),
            "alice".to_string(),
            "role-mod".to_string(),
            "Moderator".to_string(),
            PERM_MUTE_MEMBERS,
            20,
        ),
        ControlEnvelope::member_roles_set(
            "op-7".to_string(),
            7,
            "g-1".to_string(),
            "alice".to_string(),
            "alice".to_string(),
            vec!["role-owner".to_string(), "role-mod".to_string()],
        ),
    ];

    for op in ops {
        state.apply(op).expect("control op apply");
    }
    state
}

fn start_room(voice: &mut VoiceState, perms: &ControlPermissions<'_>) {
    voice
        .apply(
            VoiceEnvelope::session_start(
                "voice-start".to_string(),
                1,
                "g-1".to_string(),
                "v-1".to_string(),
                "alice".to_string(),
                "sess-1".to_string(),
                "moq://local-relay/room/sess-1".to_string(),
            ),
            perms,
        )
        .expect("session start");
    voice
        .apply(
            VoiceEnvelope::participant_join(
                "voice-join-a".to_string(),
                2,
                "g-1".to_string(),
                "v-1".to_string(),
                "alice".to_string(),
                "sess-1".to_string(),
            ),
            perms,
        )
        .expect("alice join");
    voice
        .apply(
            VoiceEnvelope::participant_join(
                "voice-join-b".to_string(),
                3,
                "g-1".to_string(),
                "v-1".to_string(),
                "bob".to_string(),
                "sess-1".to_string(),
            ),
            perms,
        )
        .expect("bob join");
}

fn room_track_count(voice: &VoiceState) -> usize {
    voice
        .room("g-1", "v-1")
        .map(|r| r.tracks.len())
        .unwrap_or_default()
}
