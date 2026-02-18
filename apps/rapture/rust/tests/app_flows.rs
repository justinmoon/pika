use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
