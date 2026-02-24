use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rapture_core::control::ControlEnvelope;
use rapture_core::{AppAction, AppReconciler, AppUpdate, ChannelKind, FfiApp};
use tempfile::tempdir;

fn wait_until(what: &str, timeout: Duration, mut pred: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if pred() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("{what}: condition not met within {timeout:?}");
}

struct Collector {
    updates: Arc<Mutex<Vec<AppUpdate>>>,
}

impl Collector {
    fn new() -> (Self, Arc<Mutex<Vec<AppUpdate>>>) {
        let updates = Arc::new(Mutex::new(vec![]));
        (
            Self {
                updates: updates.clone(),
            },
            updates,
        )
    }
}

impl AppReconciler for Collector {
    fn reconcile(&self, update: AppUpdate) {
        self.updates.lock().expect("lock updates").push(update);
    }
}

#[test]
fn create_guild_and_channel_updates_state_and_rev() {
    let dir = tempdir().expect("tempdir");
    let app = FfiApp::new(dir.path().to_string_lossy().to_string());
    let (collector, updates) = Collector::new();
    app.listen_for_updates(Box::new(collector));

    app.dispatch(AppAction::CreateGuild {
        guild_id: "g-1".to_string(),
        name: "Guild One".to_string(),
        actor_pubkey: "alice".to_string(),
    });

    wait_until("guild created", Duration::from_secs(2), || {
        app.state().guilds.len() == 1
    });

    app.dispatch(AppAction::CreateChannel {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        name: "general".to_string(),
        kind: ChannelKind::Text,
        actor_pubkey: "alice".to_string(),
    });

    wait_until("channel created", Duration::from_secs(2), || {
        app.state()
            .guilds
            .iter()
            .find(|g| g.guild_id == "g-1")
            .map(|g| g.channel_count == 1)
            .unwrap_or(false)
    });

    let s = app.state();
    assert!(s.rev >= 2);
    assert_eq!(s.guilds.len(), 1);
    assert_eq!(s.guilds[0].member_count, 1);
    assert_eq!(s.guilds[0].channel_count, 1);

    wait_until("updates emitted", Duration::from_secs(2), || {
        updates.lock().expect("updates lock").len() >= 3
    });

    let up = updates.lock().expect("updates lock");
    let mut prev = 0_u64;
    for u in up.iter() {
        let rev = match u {
            AppUpdate::FullState(s) => s.rev,
        };
        assert!(rev >= prev);
        prev = rev;
    }
}

#[test]
fn replay_restores_guild_and_channel_from_disk() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().to_string_lossy().to_string();

    let app1 = FfiApp::new(path.clone());
    app1.dispatch(AppAction::CreateGuild {
        guild_id: "g-1".to_string(),
        name: "Guild One".to_string(),
        actor_pubkey: "alice".to_string(),
    });
    wait_until("guild created", Duration::from_secs(2), || {
        app1.state().guilds.len() == 1
    });

    app1.dispatch(AppAction::CreateChannel {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        name: "general".to_string(),
        kind: ChannelKind::Text,
        actor_pubkey: "alice".to_string(),
    });
    wait_until("channel created", Duration::from_secs(2), || {
        app1.state()
            .guilds
            .first()
            .map(|g| g.channel_count)
            .unwrap_or(0)
            == 1
    });

    let app2 = FfiApp::new(path);
    wait_until("restored state", Duration::from_secs(2), || {
        app2.state()
            .guilds
            .first()
            .map(|g| g.channel_count)
            .unwrap_or(0)
            == 1
    });

    let s2 = app2.state();
    assert_eq!(s2.guilds.len(), 1);
    assert_eq!(s2.guilds[0].guild_id, "g-1");
    assert_eq!(s2.guilds[0].channel_count, 1);
}

#[test]
fn denied_actions_surface_error_without_mutating_state() {
    let dir = tempdir().expect("tempdir");
    let app = FfiApp::new(dir.path().to_string_lossy().to_string());

    app.dispatch(AppAction::CreateGuild {
        guild_id: "g-1".to_string(),
        name: "Guild One".to_string(),
        actor_pubkey: "alice".to_string(),
    });
    wait_until("guild created", Duration::from_secs(2), || {
        app.state().guilds.len() == 1
    });

    app.dispatch(AppAction::InviteMember {
        guild_id: "g-1".to_string(),
        member_pubkey: "carol".to_string(),
        actor_pubkey: "alice".to_string(),
    });
    wait_until("carol invited", Duration::from_secs(2), || {
        app.state()
            .guilds
            .iter()
            .find(|g| g.guild_id == "g-1")
            .map(|g| g.member_count == 2)
            .unwrap_or(false)
    });

    let rev_before_denied_channel = app.state().rev;
    app.dispatch(AppAction::CreateChannel {
        guild_id: "g-1".to_string(),
        channel_id: "c-denied".to_string(),
        name: "denied".to_string(),
        kind: ChannelKind::Text,
        actor_pubkey: "bob".to_string(),
    });
    wait_until(
        "denied channel attempt processed",
        Duration::from_secs(2),
        || app.state().rev > rev_before_denied_channel,
    );

    let s1 = app.state();
    assert_eq!(s1.guilds[0].channel_count, 0);
    assert!(s1
        .toast
        .as_deref()
        .unwrap_or("")
        .contains("permission denied"));

    let rev_before_denied_kick = app.state().rev;
    app.dispatch(AppAction::KickMember {
        guild_id: "g-1".to_string(),
        member_pubkey: "carol".to_string(),
        actor_pubkey: "bob".to_string(),
    });
    wait_until(
        "denied kick attempt processed",
        Duration::from_secs(2),
        || app.state().rev > rev_before_denied_kick,
    );

    let s2 = app.state();
    assert_eq!(s2.guilds[0].member_count, 2);
    assert!(s2
        .toast
        .as_deref()
        .unwrap_or("")
        .contains("permission denied"));
}

#[test]
fn append_failure_does_not_commit_in_memory_state() {
    let dir = tempdir().expect("tempdir");
    let blocked = dir.path().join("not-a-directory");
    std::fs::write(&blocked, b"blocked").expect("write blocking file");

    let app = FfiApp::new(blocked.to_string_lossy().to_string());
    let rev_before = app.state().rev;
    app.dispatch(AppAction::CreateGuild {
        guild_id: "g-1".to_string(),
        name: "Guild One".to_string(),
        actor_pubkey: "alice".to_string(),
    });

    wait_until("append failure processed", Duration::from_secs(2), || {
        app.state().rev > rev_before
    });

    let state = app.state();
    assert!(state.guilds.is_empty());
    assert!(state
        .toast
        .as_deref()
        .unwrap_or("")
        .contains("failed to append op"));
}

#[test]
fn startup_log_replay_is_sorted_by_timestamp_then_op_id() {
    let dir = tempdir().expect("tempdir");
    let base = dir.path();

    let ops = vec![
        ControlEnvelope::channel_create(
            "op-3".to_string(),
            3,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            "general".to_string(),
            ChannelKind::Text,
        ),
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
    ];

    let mut log = String::new();
    for op in ops {
        log.push_str(&serde_json::to_string(&op).expect("serialize op"));
        log.push('\n');
    }
    std::fs::write(base.join("control_ops.jsonl"), log).expect("write op log");

    let app = FfiApp::new(base.to_string_lossy().to_string());
    wait_until(
        "sorted replay restored state",
        Duration::from_secs(2),
        || {
            app.state()
                .guilds
                .first()
                .map(|g| g.channel_count == 1 && g.member_count == 2)
                .unwrap_or(false)
        },
    );
}

#[test]
fn timeline_send_edit_react_delete_round_trip() {
    let dir = tempdir().expect("tempdir");
    let app = FfiApp::new(dir.path().to_string_lossy().to_string());

    app.dispatch(AppAction::CreateGuild {
        guild_id: "g-1".to_string(),
        name: "Guild One".to_string(),
        actor_pubkey: "alice".to_string(),
    });
    app.dispatch(AppAction::CreateChannel {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        name: "general".to_string(),
        kind: ChannelKind::Text,
        actor_pubkey: "alice".to_string(),
    });
    wait_until("channel ready", Duration::from_secs(2), || {
        app.state()
            .guilds
            .iter()
            .find(|g| g.guild_id == "g-1")
            .map(|g| g.channel_count == 1)
            .unwrap_or(false)
    });

    app.dispatch(AppAction::SendMessage {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        actor_pubkey: "alice".to_string(),
        content: "hello".to_string(),
    });
    wait_until("message sent", Duration::from_secs(2), || {
        app.state().timeline.len() == 1
    });

    let sent = app.state().timeline[0].clone();
    assert_eq!(sent.author, "alice");
    assert_eq!(sent.content, "hello");
    assert!(!sent.edited);
    assert!(!sent.deleted);
    let message_id = sent.message_id.clone();

    app.dispatch(AppAction::EditMessage {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        actor_pubkey: "alice".to_string(),
        message_id: message_id.clone(),
        content: "hello edited".to_string(),
    });
    wait_until("message edited", Duration::from_secs(2), || {
        app.state()
            .timeline
            .first()
            .map(|m| m.edited && m.content == "hello edited")
            .unwrap_or(false)
    });

    app.dispatch(AppAction::PutReaction {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        actor_pubkey: "alice".to_string(),
        message_id: message_id.clone(),
        emoji: ":+1:".to_string(),
    });
    wait_until("reaction added", Duration::from_secs(2), || {
        app.state()
            .timeline
            .first()
            .map(|m| {
                m.reactions
                    .iter()
                    .any(|r| r.emoji == ":+1:" && r.actors.contains(&"alice".to_string()))
            })
            .unwrap_or(false)
    });

    app.dispatch(AppAction::RemoveReaction {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        actor_pubkey: "alice".to_string(),
        message_id: message_id.clone(),
        emoji: ":+1:".to_string(),
    });
    wait_until("reaction removed", Duration::from_secs(2), || {
        app.state()
            .timeline
            .first()
            .map(|m| m.reactions.iter().all(|r| r.emoji != ":+1:"))
            .unwrap_or(false)
    });

    app.dispatch(AppAction::DeleteMessage {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        actor_pubkey: "alice".to_string(),
        message_id,
    });
    wait_until("message deleted", Duration::from_secs(2), || {
        app.state()
            .timeline
            .first()
            .map(|m| m.deleted && m.content == "[deleted]")
            .unwrap_or(false)
    });
}

#[test]
fn timeline_permissions_are_enforced() {
    let dir = tempdir().expect("tempdir");
    let app = FfiApp::new(dir.path().to_string_lossy().to_string());

    app.dispatch(AppAction::CreateGuild {
        guild_id: "g-1".to_string(),
        name: "Guild One".to_string(),
        actor_pubkey: "alice".to_string(),
    });
    app.dispatch(AppAction::CreateChannel {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        name: "general".to_string(),
        kind: ChannelKind::Text,
        actor_pubkey: "alice".to_string(),
    });
    wait_until("channel ready", Duration::from_secs(2), || {
        app.state()
            .guilds
            .iter()
            .find(|g| g.guild_id == "g-1")
            .map(|g| g.channel_count == 1)
            .unwrap_or(false)
    });

    let rev_before_denied_send = app.state().rev;
    app.dispatch(AppAction::SendMessage {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        actor_pubkey: "bob".to_string(),
        content: "hello from bob".to_string(),
    });
    wait_until("denied send processed", Duration::from_secs(2), || {
        app.state().rev > rev_before_denied_send
    });
    let denied = app.state();
    assert!(denied.timeline.is_empty());
    assert!(denied
        .toast
        .as_deref()
        .unwrap_or("")
        .contains("permission denied"));

    app.dispatch(AppAction::InviteMember {
        guild_id: "g-1".to_string(),
        member_pubkey: "bob".to_string(),
        actor_pubkey: "alice".to_string(),
    });
    wait_until("bob invited", Duration::from_secs(2), || {
        app.state()
            .guilds
            .iter()
            .find(|g| g.guild_id == "g-1")
            .map(|g| g.member_count == 2)
            .unwrap_or(false)
    });

    app.dispatch(AppAction::SendMessage {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        actor_pubkey: "bob".to_string(),
        content: "hello from bob".to_string(),
    });
    wait_until("bob send succeeds", Duration::from_secs(2), || {
        app.state()
            .timeline
            .first()
            .map(|m| m.author == "bob")
            .unwrap_or(false)
    });

    let message_id = app.state().timeline[0].message_id.clone();
    let rev_before_denied_edit = app.state().rev;
    app.dispatch(AppAction::EditMessage {
        guild_id: "g-1".to_string(),
        channel_id: "c-1".to_string(),
        actor_pubkey: "alice".to_string(),
        message_id,
        content: "owner edit".to_string(),
    });
    wait_until("owner edit processed", Duration::from_secs(2), || {
        app.state().rev > rev_before_denied_edit
    });
    assert_eq!(app.state().timeline[0].content, "owner edit");
}

#[test]
fn voice_join_mute_leave_updates_state() {
    let dir = tempdir().expect("tempdir");
    let app = FfiApp::new(dir.path().to_string_lossy().to_string());

    app.dispatch(AppAction::CreateGuild {
        guild_id: "g-1".to_string(),
        name: "Guild One".to_string(),
        actor_pubkey: "alice".to_string(),
    });
    app.dispatch(AppAction::CreateChannel {
        guild_id: "g-1".to_string(),
        channel_id: "v-1".to_string(),
        name: "voice".to_string(),
        kind: ChannelKind::Voice,
        actor_pubkey: "alice".to_string(),
    });
    wait_until("voice channel ready", Duration::from_secs(2), || {
        app.state()
            .guilds
            .iter()
            .find(|g| g.guild_id == "g-1")
            .map(|g| g.channel_count == 1)
            .unwrap_or(false)
    });

    app.dispatch(AppAction::SelectChannel {
        guild_id: "g-1".to_string(),
        channel_id: "v-1".to_string(),
    });
    app.dispatch(AppAction::JoinVoice {
        guild_id: "g-1".to_string(),
        channel_id: "v-1".to_string(),
        actor_pubkey: "alice".to_string(),
    });
    wait_until("alice joined voice", Duration::from_secs(2), || {
        app.state()
            .voice_room
            .as_ref()
            .map(|r| {
                r.active_session_id.is_some()
                    && r.participants.iter().any(|p| p.pubkey == "alice")
            })
            .unwrap_or(false)
    });

    app.dispatch(AppAction::SetVoiceMuted {
        guild_id: "g-1".to_string(),
        channel_id: "v-1".to_string(),
        actor_pubkey: "alice".to_string(),
        muted: true,
    });
    wait_until("alice muted", Duration::from_secs(2), || {
        app.state()
            .voice_room
            .as_ref()
            .and_then(|r| r.participants.iter().find(|p| p.pubkey == "alice"))
            .map(|p| p.muted)
            .unwrap_or(false)
    });

    app.dispatch(AppAction::SetVoiceSpeaking {
        guild_id: "g-1".to_string(),
        channel_id: "v-1".to_string(),
        actor_pubkey: "alice".to_string(),
        speaking: true,
    });
    wait_until("alice speaking", Duration::from_secs(2), || {
        app.state()
            .voice_room
            .as_ref()
            .and_then(|r| r.participants.iter().find(|p| p.pubkey == "alice"))
            .map(|p| p.speaking)
            .unwrap_or(false)
    });

    app.dispatch(AppAction::LeaveVoice {
        guild_id: "g-1".to_string(),
        channel_id: "v-1".to_string(),
        actor_pubkey: "alice".to_string(),
    });
    wait_until("alice left voice", Duration::from_secs(2), || {
        app.state()
            .voice_room
            .as_ref()
            .map(|r| r.active_session_id.is_none() && r.participants.is_empty())
            .unwrap_or(false)
    });
}

#[test]
fn voice_permission_denial_surfaces_toast() {
    let dir = tempdir().expect("tempdir");
    let app = FfiApp::new(dir.path().to_string_lossy().to_string());

    app.dispatch(AppAction::CreateGuild {
        guild_id: "g-1".to_string(),
        name: "Guild One".to_string(),
        actor_pubkey: "alice".to_string(),
    });
    app.dispatch(AppAction::CreateChannel {
        guild_id: "g-1".to_string(),
        channel_id: "v-1".to_string(),
        name: "voice".to_string(),
        kind: ChannelKind::Voice,
        actor_pubkey: "alice".to_string(),
    });
    app.dispatch(AppAction::InviteMember {
        guild_id: "g-1".to_string(),
        member_pubkey: "bob".to_string(),
        actor_pubkey: "alice".to_string(),
    });
    wait_until("bob invited", Duration::from_secs(2), || {
        app.state()
            .guilds
            .iter()
            .find(|g| g.guild_id == "g-1")
            .map(|g| g.member_count == 2)
            .unwrap_or(false)
    });

    app.dispatch(AppAction::SelectChannel {
        guild_id: "g-1".to_string(),
        channel_id: "v-1".to_string(),
    });
    let rev_before_denied_join = app.state().rev;
    app.dispatch(AppAction::JoinVoice {
        guild_id: "g-1".to_string(),
        channel_id: "v-1".to_string(),
        actor_pubkey: "bob".to_string(),
    });
    wait_until("denied join processed", Duration::from_secs(2), || {
        app.state().rev > rev_before_denied_join
    });
    let denied = app.state();
    assert!(denied
        .toast
        .as_deref()
        .unwrap_or("")
        .contains("permission denied"));
    assert!(denied
        .voice_room
        .as_ref()
        .map(|r| r.participants.is_empty())
        .unwrap_or(true));
}
