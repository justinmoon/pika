use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use flume::{Receiver, Sender};

pub mod channel_groups;
pub mod chat;
pub mod control;
pub mod permissions;
pub mod reconcile;
pub mod sim;
mod storage;
pub mod voice;
pub mod voice_media;

use control::{ApplyOutcome, ControlEnvelope, ControlState};
use storage::ControlStore;

uniffi::setup_scaffolding!();

const DEFAULT_GREETING: &str = "Rapture ready";

static NEXT_OP_ID: AtomicU64 = AtomicU64::new(1);

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct GuildSummary {
    pub guild_id: String,
    pub name: String,
    pub channel_count: u32,
    pub member_count: u32,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct AppState {
    pub rev: u64,
    pub greeting: String,
    pub guilds: Vec<GuildSummary>,
    pub toast: Option<String>,
}

impl AppState {
    fn empty() -> Self {
        Self {
            rev: 0,
            greeting: DEFAULT_GREETING.to_string(),
            guilds: vec![],
            toast: None,
        }
    }
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ChannelKind {
    Text,
    Voice,
    Private,
    Thread,
}

#[derive(uniffi::Enum, Clone, Debug)]
pub enum AppAction {
    SetName {
        name: String,
    },
    CreateGuild {
        guild_id: String,
        name: String,
        actor_pubkey: String,
    },
    CreateChannel {
        guild_id: String,
        channel_id: String,
        name: String,
        kind: ChannelKind,
        actor_pubkey: String,
    },
    InviteMember {
        guild_id: String,
        member_pubkey: String,
        actor_pubkey: String,
    },
    KickMember {
        guild_id: String,
        member_pubkey: String,
        actor_pubkey: String,
    },
    BanMember {
        guild_id: String,
        member_pubkey: String,
        actor_pubkey: String,
    },
    SetMemberRoles {
        guild_id: String,
        member_pubkey: String,
        role_ids: Vec<String>,
        actor_pubkey: String,
    },
    SetChannelPermissions {
        guild_id: String,
        channel_id: String,
        allow_roles: Vec<String>,
        deny_roles: Vec<String>,
        allow_users: Vec<String>,
        deny_users: Vec<String>,
        actor_pubkey: String,
    },
    RemoveMemberFromChannel {
        guild_id: String,
        channel_id: String,
        member_pubkey: String,
        actor_pubkey: String,
    },
}

#[derive(uniffi::Enum, Clone, Debug)]
pub enum AppUpdate {
    FullState(AppState),
}

#[uniffi::export(callback_interface)]
pub trait AppReconciler: Send + Sync + 'static {
    fn reconcile(&self, update: AppUpdate);
}

enum CoreMsg {
    Action(AppAction),
}

#[derive(uniffi::Object)]
pub struct FfiApp {
    core_tx: Sender<CoreMsg>,
    update_rx: Receiver<AppUpdate>,
    listening: AtomicBool,
    shared_state: Arc<RwLock<AppState>>,
}

#[uniffi::export]
impl FfiApp {
    #[uniffi::constructor]
    pub fn new(data_dir: String) -> Arc<Self> {
        let (update_tx, update_rx) = flume::unbounded();
        let (core_tx, core_rx) = flume::unbounded::<CoreMsg>();
        let shared_state = Arc::new(RwLock::new(AppState::empty()));

        let shared_for_core = shared_state.clone();
        thread::spawn(move || {
            let mut control_state = ControlState::default();
            let mut greeting = DEFAULT_GREETING.to_string();
            let mut toast: Option<String> = None;

            let store = ControlStore::new(PathBuf::from(&data_dir));
            match store.load_state() {
                Ok(loaded) => {
                    control_state = loaded;
                }
                Err(e) => {
                    toast = Some(format!("failed to load control state: {e}"));
                }
            }

            let mut rev = control_state.seen_op_ids.len() as u64;
            let mut state = build_state(rev, &greeting, &control_state, toast.take());

            {
                let snapshot = state.clone();
                match shared_for_core.write() {
                    Ok(mut g) => *g = snapshot.clone(),
                    Err(p) => *p.into_inner() = snapshot.clone(),
                }
                let _ = update_tx.send(AppUpdate::FullState(snapshot));
            }

            while let Ok(msg) = core_rx.recv() {
                match msg {
                    CoreMsg::Action(action) => {
                        let mut local_toast: Option<String> = None;
                        match action {
                            AppAction::SetName { name } => {
                                if name.trim().is_empty() {
                                    greeting = DEFAULT_GREETING.to_string();
                                } else {
                                    greeting = format!("Rapture ready, {}", name.trim());
                                }
                            }
                            AppAction::CreateGuild {
                                guild_id,
                                name,
                                actor_pubkey,
                            } => {
                                let op = ControlEnvelope::guild_create(
                                    next_op_id(),
                                    now_ms(),
                                    guild_id,
                                    actor_pubkey,
                                    name,
                                );
                                local_toast = apply_control_op(&store, &mut control_state, op);
                            }
                            AppAction::CreateChannel {
                                guild_id,
                                channel_id,
                                name,
                                kind,
                                actor_pubkey,
                            } => {
                                let op = ControlEnvelope::channel_create(
                                    next_op_id(),
                                    now_ms(),
                                    guild_id,
                                    actor_pubkey,
                                    channel_id,
                                    name,
                                    kind,
                                );
                                local_toast = apply_control_op(&store, &mut control_state, op);
                            }
                            AppAction::InviteMember {
                                guild_id,
                                member_pubkey,
                                actor_pubkey,
                            } => {
                                let op = ControlEnvelope::member_add(
                                    next_op_id(),
                                    now_ms(),
                                    guild_id,
                                    actor_pubkey,
                                    member_pubkey,
                                );
                                local_toast = apply_control_op(&store, &mut control_state, op);
                            }
                            AppAction::KickMember {
                                guild_id,
                                member_pubkey,
                                actor_pubkey,
                            } => {
                                let op = ControlEnvelope::member_remove(
                                    next_op_id(),
                                    now_ms(),
                                    guild_id,
                                    actor_pubkey,
                                    member_pubkey,
                                );
                                local_toast = apply_control_op(&store, &mut control_state, op);
                            }
                            AppAction::BanMember {
                                guild_id,
                                member_pubkey,
                                actor_pubkey,
                            } => {
                                let op = ControlEnvelope::member_ban(
                                    next_op_id(),
                                    now_ms(),
                                    guild_id,
                                    actor_pubkey,
                                    member_pubkey,
                                );
                                local_toast = apply_control_op(&store, &mut control_state, op);
                            }
                            AppAction::SetMemberRoles {
                                guild_id,
                                member_pubkey,
                                role_ids,
                                actor_pubkey,
                            } => {
                                let op = ControlEnvelope::member_roles_set(
                                    next_op_id(),
                                    now_ms(),
                                    guild_id,
                                    actor_pubkey,
                                    member_pubkey,
                                    role_ids,
                                );
                                local_toast = apply_control_op(&store, &mut control_state, op);
                            }
                            AppAction::SetChannelPermissions {
                                guild_id,
                                channel_id,
                                allow_roles,
                                deny_roles,
                                allow_users,
                                deny_users,
                                actor_pubkey,
                            } => {
                                let op = ControlEnvelope::channel_permissions_set(
                                    next_op_id(),
                                    now_ms(),
                                    guild_id,
                                    actor_pubkey,
                                    channel_id,
                                    allow_roles,
                                    deny_roles,
                                    allow_users,
                                    deny_users,
                                );
                                local_toast = apply_control_op(&store, &mut control_state, op);
                            }
                            AppAction::RemoveMemberFromChannel {
                                guild_id,
                                channel_id,
                                member_pubkey,
                                actor_pubkey,
                            } => {
                                let op = ControlEnvelope::channel_member_remove(
                                    next_op_id(),
                                    now_ms(),
                                    guild_id,
                                    actor_pubkey,
                                    channel_id,
                                    member_pubkey,
                                );
                                local_toast = apply_control_op(&store, &mut control_state, op);
                            }
                        }

                        rev += 1;
                        state = build_state(rev, &greeting, &control_state, local_toast);

                        let snapshot = state.clone();
                        match shared_for_core.write() {
                            Ok(mut g) => *g = snapshot.clone(),
                            Err(p) => *p.into_inner() = snapshot.clone(),
                        }
                        let _ = update_tx.send(AppUpdate::FullState(snapshot));
                    }
                }
            }
        });

        Arc::new(Self {
            core_tx,
            update_rx,
            listening: AtomicBool::new(false),
            shared_state,
        })
    }

    pub fn state(&self) -> AppState {
        match self.shared_state.read() {
            Ok(g) => g.clone(),
            Err(poison) => poison.into_inner().clone(),
        }
    }

    pub fn dispatch(&self, action: AppAction) {
        let _ = self.core_tx.send(CoreMsg::Action(action));
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn AppReconciler>) {
        if self
            .listening
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let rx = self.update_rx.clone();
        thread::spawn(move || {
            while let Ok(update) = rx.recv() {
                reconciler.reconcile(update);
            }
        });
    }
}

fn apply_control_op(
    store: &ControlStore,
    control_state: &mut ControlState,
    op: ControlEnvelope,
) -> Option<String> {
    match control_state.apply(op.clone()) {
        Ok(ApplyOutcome::Applied) => {
            if let Err(e) = store.append_op(&op) {
                return Some(format!("failed to append op: {e}"));
            }
            if let Err(e) = store.write_snapshot(control_state) {
                return Some(format!("failed to write snapshot: {e}"));
            }
            None
        }
        Ok(ApplyOutcome::Duplicate) => Some("duplicate op ignored".to_string()),
        Err(e) => Some(e.to_string()),
    }
}

fn build_state(
    rev: u64,
    greeting: &str,
    control_state: &ControlState,
    toast: Option<String>,
) -> AppState {
    let mut guilds: Vec<GuildSummary> = control_state
        .guilds
        .values()
        .map(|g| GuildSummary {
            guild_id: g.guild_id.clone(),
            name: g.name.clone(),
            channel_count: g.channels.len() as u32,
            member_count: g.members.len() as u32,
        })
        .collect();
    guilds.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.guild_id.cmp(&b.guild_id))
    });

    AppState {
        rev,
        greeting: greeting.to_string(),
        guilds,
        toast,
    }
}

fn next_op_id() -> String {
    let id = NEXT_OP_ID.fetch_add(1, Ordering::SeqCst);
    format!("local-op-{id}")
}

fn now_ms() -> i64 {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0));
    d.as_millis() as i64
}
