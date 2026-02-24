use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use flume::{Receiver, Sender};
use uuid::Uuid;

pub mod channel_groups;
pub mod chat;
pub mod control;
pub mod permissions;
pub mod reconcile;
pub mod sim;
mod storage;
pub mod voice;
pub mod voice_media;

use channel_groups::ChannelGroupDirectory;
use chat::{ChatApplyOutcome, ChatEnvelope, ChatState, EpochKeyLookup};
use control::{ControlEnvelope, ControlState, GuildState};
use permissions::{has_permission, Permission};
use reconcile::desired_channel_members;
use storage::ControlStore;
use voice::{VoiceApplyOutcome, VoiceEnvelope, VoicePermissionLookup, VoiceState};

uniffi::setup_scaffolding!();

const DEFAULT_GREETING: &str = "Rapture ready";

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct GuildSummary {
    pub guild_id: String,
    pub name: String,
    pub channel_count: u32,
    pub member_count: u32,
    pub channels: Vec<ChannelSummary>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct ChannelSummary {
    pub channel_id: String,
    pub name: String,
    pub kind: ChannelKind,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct TimelineReactionSummary {
    pub emoji: String,
    pub actors: Vec<String>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct TimelineMessageSummary {
    pub message_id: String,
    pub author: String,
    pub content: String,
    pub edited: bool,
    pub deleted: bool,
    pub reactions: Vec<TimelineReactionSummary>,
    pub ts_ms: i64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct VoiceParticipantSummary {
    pub pubkey: String,
    pub muted: bool,
    pub speaking: bool,
    pub hand_raised: bool,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct VoiceRoomSummary {
    pub active_session_id: Option<String>,
    pub moq_url: Option<String>,
    pub participants: Vec<VoiceParticipantSummary>,
    pub track_count: u32,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct AppState {
    pub rev: u64,
    pub greeting: String,
    pub guilds: Vec<GuildSummary>,
    pub selected_guild_id: Option<String>,
    pub selected_channel_id: Option<String>,
    pub timeline: Vec<TimelineMessageSummary>,
    pub voice_room: Option<VoiceRoomSummary>,
    pub toast: Option<String>,
}

impl AppState {
    fn empty() -> Self {
        Self {
            rev: 0,
            greeting: DEFAULT_GREETING.to_string(),
            guilds: vec![],
            selected_guild_id: None,
            selected_channel_id: None,
            timeline: vec![],
            voice_room: None,
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
    SelectGuild {
        guild_id: String,
    },
    SelectChannel {
        guild_id: String,
        channel_id: String,
    },
    SendMessage {
        guild_id: String,
        channel_id: String,
        actor_pubkey: String,
        content: String,
    },
    EditMessage {
        guild_id: String,
        channel_id: String,
        actor_pubkey: String,
        message_id: String,
        content: String,
    },
    DeleteMessage {
        guild_id: String,
        channel_id: String,
        actor_pubkey: String,
        message_id: String,
    },
    PutReaction {
        guild_id: String,
        channel_id: String,
        actor_pubkey: String,
        message_id: String,
        emoji: String,
    },
    RemoveReaction {
        guild_id: String,
        channel_id: String,
        actor_pubkey: String,
        message_id: String,
        emoji: String,
    },
    JoinVoice {
        guild_id: String,
        channel_id: String,
        actor_pubkey: String,
    },
    LeaveVoice {
        guild_id: String,
        channel_id: String,
        actor_pubkey: String,
    },
    SetVoiceMuted {
        guild_id: String,
        channel_id: String,
        actor_pubkey: String,
        muted: bool,
    },
    SetVoiceSpeaking {
        guild_id: String,
        channel_id: String,
        actor_pubkey: String,
        speaking: bool,
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
            let mut control_ops: Vec<ControlEnvelope> = vec![];
            let mut chat_state = ChatState::default();
            let mut voice_state = VoiceState::default();
            let mut channel_groups = ChannelGroupDirectory::default();
            let mut epoch_keys: BTreeMap<(String, String, u64), [u8; 32]> = BTreeMap::new();
            let mut selected_guild_id: Option<String> = None;
            let mut selected_channel_id: Option<String> = None;
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
            match store.load_ops() {
                Ok(loaded_ops) => {
                    if !loaded_ops.is_empty() {
                        match ControlState::replay_sorted(&loaded_ops) {
                            Ok(replayed) => {
                                control_state = replayed;
                                control_ops = loaded_ops;
                            }
                            Err(e) => {
                                toast = Some(format!("failed to replay control log: {e}"));
                            }
                        }
                    }
                }
                Err(e) => {
                    toast = Some(format!("failed to load control log: {e}"));
                }
            }

            if let Some(sync_err) = sync_channel_groups(
                &control_state,
                &mut channel_groups,
                &mut epoch_keys,
            ) {
                toast = Some(sync_err);
            }
            normalize_selection(
                &control_state,
                &mut selected_guild_id,
                &mut selected_channel_id,
            );

            let mut rev = control_state.seen_op_ids.len() as u64;
            let mut state = build_state(
                rev,
                &greeting,
                &control_state,
                &chat_state,
                &voice_state,
                selected_guild_id.clone(),
                selected_channel_id.clone(),
                toast.take(),
            );

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
                            AppAction::SelectGuild { guild_id } => {
                                selected_guild_id = Some(guild_id);
                                selected_channel_id = None;
                            }
                            AppAction::SelectChannel {
                                guild_id,
                                channel_id,
                            } => {
                                selected_guild_id = Some(guild_id);
                                selected_channel_id = Some(channel_id);
                            }
                            AppAction::SendMessage {
                                guild_id,
                                channel_id,
                                actor_pubkey,
                                content,
                            } => {
                                local_toast = send_message(
                                    &control_state,
                                    &mut chat_state,
                                    &channel_groups,
                                    &mut epoch_keys,
                                    guild_id,
                                    channel_id,
                                    actor_pubkey,
                                    content,
                                );
                            }
                            AppAction::EditMessage {
                                guild_id,
                                channel_id,
                                actor_pubkey,
                                message_id,
                                content,
                            } => {
                                local_toast = edit_message(
                                    &control_state,
                                    &mut chat_state,
                                    &channel_groups,
                                    &mut epoch_keys,
                                    guild_id,
                                    channel_id,
                                    actor_pubkey,
                                    message_id,
                                    content,
                                );
                            }
                            AppAction::DeleteMessage {
                                guild_id,
                                channel_id,
                                actor_pubkey,
                                message_id,
                            } => {
                                local_toast = delete_message(
                                    &control_state,
                                    &mut chat_state,
                                    guild_id,
                                    channel_id,
                                    actor_pubkey,
                                    message_id,
                                );
                            }
                            AppAction::PutReaction {
                                guild_id,
                                channel_id,
                                actor_pubkey,
                                message_id,
                                emoji,
                            } => {
                                local_toast = put_reaction(
                                    &control_state,
                                    &mut chat_state,
                                    guild_id,
                                    channel_id,
                                    actor_pubkey,
                                    message_id,
                                    emoji,
                                );
                            }
                            AppAction::RemoveReaction {
                                guild_id,
                                channel_id,
                                actor_pubkey,
                                message_id,
                                emoji,
                            } => {
                                local_toast = remove_reaction(
                                    &control_state,
                                    &mut chat_state,
                                    guild_id,
                                    channel_id,
                                    actor_pubkey,
                                    message_id,
                                    emoji,
                                );
                            }
                            AppAction::JoinVoice {
                                guild_id,
                                channel_id,
                                actor_pubkey,
                            } => {
                                local_toast = join_voice(
                                    &control_state,
                                    &mut voice_state,
                                    guild_id,
                                    channel_id,
                                    actor_pubkey,
                                );
                            }
                            AppAction::LeaveVoice {
                                guild_id,
                                channel_id,
                                actor_pubkey,
                            } => {
                                local_toast = leave_voice(
                                    &control_state,
                                    &mut voice_state,
                                    guild_id,
                                    channel_id,
                                    actor_pubkey,
                                );
                            }
                            AppAction::SetVoiceMuted {
                                guild_id,
                                channel_id,
                                actor_pubkey,
                                muted,
                            } => {
                                local_toast = set_voice_muted(
                                    &control_state,
                                    &mut voice_state,
                                    guild_id,
                                    channel_id,
                                    actor_pubkey,
                                    muted,
                                );
                            }
                            AppAction::SetVoiceSpeaking {
                                guild_id,
                                channel_id,
                                actor_pubkey,
                                speaking,
                            } => {
                                local_toast = set_voice_speaking(
                                    &control_state,
                                    &mut voice_state,
                                    guild_id,
                                    channel_id,
                                    actor_pubkey,
                                    speaking,
                                );
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
                                local_toast = apply_control_op(
                                    &store,
                                    &mut control_state,
                                    &mut control_ops,
                                    op,
                                );
                                if local_toast.is_none() {
                                    local_toast = sync_channel_groups(
                                        &control_state,
                                        &mut channel_groups,
                                        &mut epoch_keys,
                                    );
                                }
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
                                local_toast = apply_control_op(
                                    &store,
                                    &mut control_state,
                                    &mut control_ops,
                                    op,
                                );
                                if local_toast.is_none() {
                                    local_toast = sync_channel_groups(
                                        &control_state,
                                        &mut channel_groups,
                                        &mut epoch_keys,
                                    );
                                }
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
                                local_toast = apply_control_op(
                                    &store,
                                    &mut control_state,
                                    &mut control_ops,
                                    op,
                                );
                                if local_toast.is_none() {
                                    local_toast = sync_channel_groups(
                                        &control_state,
                                        &mut channel_groups,
                                        &mut epoch_keys,
                                    );
                                }
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
                                local_toast = apply_control_op(
                                    &store,
                                    &mut control_state,
                                    &mut control_ops,
                                    op,
                                );
                                if local_toast.is_none() {
                                    local_toast = sync_channel_groups(
                                        &control_state,
                                        &mut channel_groups,
                                        &mut epoch_keys,
                                    );
                                }
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
                                local_toast = apply_control_op(
                                    &store,
                                    &mut control_state,
                                    &mut control_ops,
                                    op,
                                );
                                if local_toast.is_none() {
                                    local_toast = sync_channel_groups(
                                        &control_state,
                                        &mut channel_groups,
                                        &mut epoch_keys,
                                    );
                                }
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
                                local_toast = apply_control_op(
                                    &store,
                                    &mut control_state,
                                    &mut control_ops,
                                    op,
                                );
                                if local_toast.is_none() {
                                    local_toast = sync_channel_groups(
                                        &control_state,
                                        &mut channel_groups,
                                        &mut epoch_keys,
                                    );
                                }
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
                                local_toast = apply_control_op(
                                    &store,
                                    &mut control_state,
                                    &mut control_ops,
                                    op,
                                );
                                if local_toast.is_none() {
                                    local_toast = sync_channel_groups(
                                        &control_state,
                                        &mut channel_groups,
                                        &mut epoch_keys,
                                    );
                                }
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
                                local_toast = apply_control_op(
                                    &store,
                                    &mut control_state,
                                    &mut control_ops,
                                    op,
                                );
                                if local_toast.is_none() {
                                    local_toast = sync_channel_groups(
                                        &control_state,
                                        &mut channel_groups,
                                        &mut epoch_keys,
                                    );
                                }
                            }
                        }

                        normalize_selection(
                            &control_state,
                            &mut selected_guild_id,
                            &mut selected_channel_id,
                        );

                        rev += 1;
                        state = build_state(
                            rev,
                            &greeting,
                            &control_state,
                            &chat_state,
                            &voice_state,
                            selected_guild_id.clone(),
                            selected_channel_id.clone(),
                            local_toast,
                        );

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

struct EpochKeyMap<'a> {
    keys: &'a BTreeMap<(String, String, u64), [u8; 32]>,
}

impl EpochKeyLookup for EpochKeyMap<'_> {
    fn epoch_key(&self, guild_id: &str, channel_id: &str, epoch: u64) -> Option<[u8; 32]> {
        self.keys
            .get(&(guild_id.to_string(), channel_id.to_string(), epoch))
            .copied()
    }
}

fn send_message(
    control_state: &ControlState,
    chat_state: &mut ChatState,
    channel_groups: &ChannelGroupDirectory,
    epoch_keys: &mut BTreeMap<(String, String, u64), [u8; 32]>,
    guild_id: String,
    channel_id: String,
    actor_pubkey: String,
    content: String,
) -> Option<String> {
    let content = content.trim().to_string();
    if content.is_empty() {
        return Some("message cannot be empty".to_string());
    }

    if let Err(e) = require_channel_permission(
        control_state,
        &guild_id,
        &channel_id,
        &actor_pubkey,
        Permission::SendMessage,
    ) {
        return Some(e);
    }

    let (epoch, key) = match channel_groups.current_epoch_key(&guild_id, &channel_id) {
        Some(v) => v,
        None => {
            return Some(format!(
                "channel key not found: {guild_id}/{channel_id}; create channel first"
            ));
        }
    };
    epoch_keys.insert((guild_id.clone(), channel_id.clone(), epoch), key);

    let envelope = match ChatEnvelope::message_send(
        next_op_id(),
        now_ms(),
        guild_id,
        channel_id,
        actor_pubkey,
        next_message_id(),
        &content,
        epoch,
        key,
    ) {
        Ok(env) => env,
        Err(e) => return Some(format!("failed to build message: {e}")),
    };

    apply_chat_envelope(chat_state, epoch_keys, envelope)
}

fn edit_message(
    control_state: &ControlState,
    chat_state: &mut ChatState,
    channel_groups: &ChannelGroupDirectory,
    epoch_keys: &mut BTreeMap<(String, String, u64), [u8; 32]>,
    guild_id: String,
    channel_id: String,
    actor_pubkey: String,
    message_id: String,
    content: String,
) -> Option<String> {
    let content = content.trim().to_string();
    if content.is_empty() {
        return Some("edited message cannot be empty".to_string());
    }

    if let Err(e) = require_channel_permission(
        control_state,
        &guild_id,
        &channel_id,
        &actor_pubkey,
        Permission::ViewChannel,
    ) {
        return Some(e);
    }

    let Some(existing) = chat_state
        .timeline(&guild_id, &channel_id)
        .into_iter()
        .find(|m| m.message_id == message_id)
    else {
        return Some(format!("message not found: {message_id}"));
    };

    if let Err(e) = require_manage_or_author(
        control_state,
        &guild_id,
        &channel_id,
        &actor_pubkey,
        &existing.author,
    ) {
        return Some(e);
    }

    let (epoch, key) = match channel_groups.current_epoch_key(&guild_id, &channel_id) {
        Some(v) => v,
        None => {
            return Some(format!(
                "channel key not found: {guild_id}/{channel_id}; create channel first"
            ));
        }
    };
    epoch_keys.insert((guild_id.clone(), channel_id.clone(), epoch), key);

    let envelope = match ChatEnvelope::message_edit(
        next_op_id(),
        now_ms(),
        guild_id,
        channel_id,
        actor_pubkey,
        message_id,
        &content,
        epoch,
        key,
    ) {
        Ok(env) => env,
        Err(e) => return Some(format!("failed to build edit: {e}")),
    };

    apply_chat_envelope(chat_state, epoch_keys, envelope)
}

fn delete_message(
    control_state: &ControlState,
    chat_state: &mut ChatState,
    guild_id: String,
    channel_id: String,
    actor_pubkey: String,
    message_id: String,
) -> Option<String> {
    if let Err(e) = require_channel_permission(
        control_state,
        &guild_id,
        &channel_id,
        &actor_pubkey,
        Permission::ViewChannel,
    ) {
        return Some(e);
    }

    let Some(existing) = chat_state
        .timeline(&guild_id, &channel_id)
        .into_iter()
        .find(|m| m.message_id == message_id)
    else {
        return Some(format!("message not found: {message_id}"));
    };

    if let Err(e) = require_manage_or_author(
        control_state,
        &guild_id,
        &channel_id,
        &actor_pubkey,
        &existing.author,
    ) {
        return Some(e);
    }

    let envelope = ChatEnvelope::message_delete(
        next_op_id(),
        now_ms(),
        guild_id,
        channel_id,
        actor_pubkey,
        message_id,
    );

    let lookup = EpochKeyMap {
        keys: &BTreeMap::new(),
    };
    match chat_state.apply(envelope, &lookup) {
        Ok(ChatApplyOutcome::Applied | ChatApplyOutcome::Duplicate) => None,
        Err(e) => Some(format!("chat op failed: {e}")),
    }
}

fn put_reaction(
    control_state: &ControlState,
    chat_state: &mut ChatState,
    guild_id: String,
    channel_id: String,
    actor_pubkey: String,
    message_id: String,
    emoji: String,
) -> Option<String> {
    if let Err(e) = require_channel_permission(
        control_state,
        &guild_id,
        &channel_id,
        &actor_pubkey,
        Permission::SendMessage,
    ) {
        return Some(e);
    }

    let envelope = ChatEnvelope::reaction_put(
        next_op_id(),
        now_ms(),
        guild_id,
        channel_id,
        actor_pubkey,
        message_id,
        emoji,
    );
    let lookup = EpochKeyMap {
        keys: &BTreeMap::new(),
    };

    match chat_state.apply(envelope, &lookup) {
        Ok(ChatApplyOutcome::Applied | ChatApplyOutcome::Duplicate) => None,
        Err(e) => Some(format!("chat op failed: {e}")),
    }
}

fn remove_reaction(
    control_state: &ControlState,
    chat_state: &mut ChatState,
    guild_id: String,
    channel_id: String,
    actor_pubkey: String,
    message_id: String,
    emoji: String,
) -> Option<String> {
    if let Err(e) = require_channel_permission(
        control_state,
        &guild_id,
        &channel_id,
        &actor_pubkey,
        Permission::SendMessage,
    ) {
        return Some(e);
    }

    let envelope = ChatEnvelope::reaction_remove(
        next_op_id(),
        now_ms(),
        guild_id,
        channel_id,
        actor_pubkey,
        message_id,
        emoji,
    );
    let lookup = EpochKeyMap {
        keys: &BTreeMap::new(),
    };

    match chat_state.apply(envelope, &lookup) {
        Ok(ChatApplyOutcome::Applied | ChatApplyOutcome::Duplicate) => None,
        Err(e) => Some(format!("chat op failed: {e}")),
    }
}

struct ControlVoicePermissions<'a> {
    control: &'a ControlState,
}

impl VoicePermissionLookup for ControlVoicePermissions<'_> {
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

fn join_voice(
    control_state: &ControlState,
    voice_state: &mut VoiceState,
    guild_id: String,
    channel_id: String,
    actor_pubkey: String,
) -> Option<String> {
    if let Err(e) = require_voice_channel(control_state, &guild_id, &channel_id) {
        return Some(e);
    }

    let mut session_id = voice_state
        .room(&guild_id, &channel_id)
        .and_then(|room| room.active_session_id.clone());
    if session_id.is_none() {
        let new_session_id = next_voice_session_id();
        let start = VoiceEnvelope::session_start(
            next_op_id(),
            now_ms(),
            guild_id.clone(),
            channel_id.clone(),
            actor_pubkey.clone(),
            new_session_id.clone(),
            default_voice_moq_url(&guild_id, &channel_id, &new_session_id),
        );
        if let Some(err) = apply_voice_envelope(voice_state, control_state, start) {
            return Some(err);
        }
        session_id = Some(new_session_id);
    }

    let join = VoiceEnvelope::participant_join(
        next_op_id(),
        now_ms(),
        guild_id,
        channel_id,
        actor_pubkey,
        session_id.unwrap_or_default(),
    );
    apply_voice_envelope(voice_state, control_state, join)
}

fn leave_voice(
    control_state: &ControlState,
    voice_state: &mut VoiceState,
    guild_id: String,
    channel_id: String,
    actor_pubkey: String,
) -> Option<String> {
    if let Err(e) = require_voice_channel(control_state, &guild_id, &channel_id) {
        return Some(e);
    }

    let Some(room) = voice_state.room(&guild_id, &channel_id) else {
        return Some(format!("no active voice room for {guild_id}/{channel_id}"));
    };
    let Some(session_id) = room.active_session_id.clone() else {
        return Some(format!("no active voice session for {guild_id}/{channel_id}"));
    };
    if !room.participants.contains_key(&actor_pubkey) {
        return Some(format!("actor not joined in voice room: {actor_pubkey}"));
    }

    let leave = VoiceEnvelope::participant_leave(
        next_op_id(),
        now_ms(),
        guild_id.clone(),
        channel_id.clone(),
        actor_pubkey.clone(),
        session_id.clone(),
    );
    if let Some(err) = apply_voice_envelope(voice_state, control_state, leave) {
        return Some(err);
    }

    let should_end = voice_state
        .room(&guild_id, &channel_id)
        .map(|updated| updated.participants.is_empty() && updated.active_session_id.is_some())
        .unwrap_or(false);
    if should_end {
        let end = VoiceEnvelope::session_end(
            next_op_id(),
            now_ms(),
            guild_id,
            channel_id,
            actor_pubkey,
            session_id,
        );
        return apply_voice_envelope(voice_state, control_state, end);
    }

    None
}

fn set_voice_muted(
    control_state: &ControlState,
    voice_state: &mut VoiceState,
    guild_id: String,
    channel_id: String,
    actor_pubkey: String,
    muted: bool,
) -> Option<String> {
    if let Err(e) = require_voice_channel(control_state, &guild_id, &channel_id) {
        return Some(e);
    }

    let Some(room) = voice_state.room(&guild_id, &channel_id) else {
        return Some(format!("no active voice room for {guild_id}/{channel_id}"));
    };
    let Some(session_id) = room.active_session_id.clone() else {
        return Some(format!("no active voice session for {guild_id}/{channel_id}"));
    };
    let Some(me) = room.participants.get(&actor_pubkey) else {
        return Some(format!("actor not joined in voice room: {actor_pubkey}"));
    };

    let update = VoiceEnvelope::participant_state(
        next_op_id(),
        now_ms(),
        guild_id,
        channel_id,
        actor_pubkey.clone(),
        session_id,
        actor_pubkey,
        muted,
        me.speaking,
        me.hand_raised,
    );
    apply_voice_envelope(voice_state, control_state, update)
}

fn set_voice_speaking(
    control_state: &ControlState,
    voice_state: &mut VoiceState,
    guild_id: String,
    channel_id: String,
    actor_pubkey: String,
    speaking: bool,
) -> Option<String> {
    if let Err(e) = require_voice_channel(control_state, &guild_id, &channel_id) {
        return Some(e);
    }

    let Some(room) = voice_state.room(&guild_id, &channel_id) else {
        return Some(format!("no active voice room for {guild_id}/{channel_id}"));
    };
    let Some(session_id) = room.active_session_id.clone() else {
        return Some(format!("no active voice session for {guild_id}/{channel_id}"));
    };
    let Some(me) = room.participants.get(&actor_pubkey) else {
        return Some(format!("actor not joined in voice room: {actor_pubkey}"));
    };

    let update = VoiceEnvelope::participant_state(
        next_op_id(),
        now_ms(),
        guild_id,
        channel_id,
        actor_pubkey.clone(),
        session_id,
        actor_pubkey,
        me.muted,
        speaking,
        me.hand_raised,
    );
    apply_voice_envelope(voice_state, control_state, update)
}

fn apply_voice_envelope(
    voice_state: &mut VoiceState,
    control_state: &ControlState,
    envelope: VoiceEnvelope,
) -> Option<String> {
    let perms = ControlVoicePermissions {
        control: control_state,
    };
    match voice_state.apply(envelope, &perms) {
        Ok(VoiceApplyOutcome::Applied | VoiceApplyOutcome::Duplicate) => None,
        Err(e) => Some(format!("voice op failed: {e}")),
    }
}

fn require_voice_channel(
    control_state: &ControlState,
    guild_id: &str,
    channel_id: &str,
) -> Result<(), String> {
    let guild = control_state
        .guilds
        .get(guild_id)
        .ok_or_else(|| format!("guild not found: {guild_id}"))?;
    let channel = guild
        .channels
        .get(channel_id)
        .ok_or_else(|| format!("channel not found: {guild_id}/{channel_id}"))?;
    if channel.kind != ChannelKind::Voice {
        return Err(format!(
            "channel is not voice: {guild_id}/{channel_id} ({:?})",
            channel.kind
        ));
    }
    Ok(())
}

fn default_voice_moq_url(guild_id: &str, channel_id: &str, session_id: &str) -> String {
    format!("moq://local-relay/room/{guild_id}/{channel_id}/{session_id}")
}

fn require_manage_or_author(
    control_state: &ControlState,
    guild_id: &str,
    channel_id: &str,
    actor: &str,
    author: &str,
) -> Result<(), String> {
    if actor == author {
        return Ok(());
    }

    require_channel_permission(
        control_state,
        guild_id,
        channel_id,
        actor,
        Permission::ManageMessages,
    )
}

fn require_channel_permission(
    control_state: &ControlState,
    guild_id: &str,
    channel_id: &str,
    actor: &str,
    permission: Permission,
) -> Result<(), String> {
    let guild = control_state
        .guilds
        .get(guild_id)
        .ok_or_else(|| format!("guild not found: {guild_id}"))?;

    if !guild.channels.contains_key(channel_id) {
        return Err(format!("channel not found: {guild_id}/{channel_id}"));
    }

    if has_permission(guild, actor, Some(channel_id), permission) {
        return Ok(());
    }

    Err(format!(
        "permission denied: actor {actor} lacks {permission:?} for {guild_id}/{channel_id}"
    ))
}

fn apply_chat_envelope(
    chat_state: &mut ChatState,
    epoch_keys: &BTreeMap<(String, String, u64), [u8; 32]>,
    envelope: ChatEnvelope,
) -> Option<String> {
    let lookup = EpochKeyMap { keys: epoch_keys };
    match chat_state.apply(envelope, &lookup) {
        Ok(ChatApplyOutcome::Applied | ChatApplyOutcome::Duplicate) => None,
        Err(e) => Some(format!("chat op failed: {e}")),
    }
}

fn sync_channel_groups(
    control_state: &ControlState,
    channel_groups: &mut ChannelGroupDirectory,
    epoch_keys: &mut BTreeMap<(String, String, u64), [u8; 32]>,
) -> Option<String> {
    channel_groups.ensure_from_control(control_state);

    for (guild_id, channel_id) in channel_groups.channel_refs() {
        let desired = match desired_channel_members(control_state, &guild_id, &channel_id) {
            Ok(members) => members,
            Err(e) => return Some(format!("reconcile desired-members failed: {e}")),
        };

        if let Err(e) = channel_groups.reconcile_members(&guild_id, &channel_id, desired) {
            return Some(format!("reconcile channel-group members failed: {e}"));
        }

        if let Some((epoch, key)) = channel_groups.current_epoch_key(&guild_id, &channel_id) {
            epoch_keys.insert((guild_id.clone(), channel_id.clone(), epoch), key);
        }
    }

    None
}

fn normalize_selection(
    control_state: &ControlState,
    selected_guild_id: &mut Option<String>,
    selected_channel_id: &mut Option<String>,
) {
    if control_state.guilds.is_empty() {
        *selected_guild_id = None;
        *selected_channel_id = None;
        return;
    }

    if selected_guild_id
        .as_ref()
        .is_none_or(|gid| !control_state.guilds.contains_key(gid))
    {
        *selected_guild_id = first_guild_id(control_state);
        *selected_channel_id = None;
    }

    let guild_id = match selected_guild_id.as_ref() {
        Some(gid) => gid,
        None => {
            *selected_channel_id = None;
            return;
        }
    };

    let Some(guild) = control_state.guilds.get(guild_id) else {
        *selected_channel_id = None;
        return;
    };

    if guild.channels.is_empty() {
        *selected_channel_id = None;
        return;
    }

    if selected_channel_id
        .as_ref()
        .is_none_or(|cid| !guild.channels.contains_key(cid))
    {
        *selected_channel_id = first_channel_id(guild);
    }
}

fn first_guild_id(control_state: &ControlState) -> Option<String> {
    let mut guilds: Vec<&GuildState> = control_state.guilds.values().collect();
    guilds.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.guild_id.cmp(&b.guild_id))
    });
    guilds.first().map(|g| g.guild_id.clone())
}

fn first_channel_id(guild: &GuildState) -> Option<String> {
    let mut channels: Vec<_> = guild.channels.values().collect();
    channels.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.channel_id.cmp(&b.channel_id))
    });
    channels.first().map(|c| c.channel_id.clone())
}

fn apply_control_op(
    store: &ControlStore,
    control_state: &mut ControlState,
    control_ops: &mut Vec<ControlEnvelope>,
    op: ControlEnvelope,
) -> Option<String> {
    if control_ops.iter().any(|existing| existing.op_id == op.op_id) {
        return Some("duplicate op ignored".to_string());
    }

    let mut next_ops = control_ops.clone();
    next_ops.push(op.clone());

    match ControlState::replay_sorted(&next_ops) {
        Ok(next_state) => {
            if let Err(e) = store.append_op(&op) {
                return Some(format!("failed to append op: {e}"));
            }

            *control_state = next_state;
            *control_ops = next_ops;

            if let Err(e) = store.write_snapshot(control_state) {
                return Some(format!(
                    "failed to write snapshot (log append succeeded): {e}"
                ));
            }
            None
        }
        Err(e) => Some(e.to_string()),
    }
}

fn build_state(
    rev: u64,
    greeting: &str,
    control_state: &ControlState,
    chat_state: &ChatState,
    voice_state: &VoiceState,
    selected_guild_id: Option<String>,
    selected_channel_id: Option<String>,
    toast: Option<String>,
) -> AppState {
    let mut guilds: Vec<GuildSummary> = control_state
        .guilds
        .values()
        .map(|g| {
            let mut channels: Vec<ChannelSummary> = g
                .channels
                .values()
                .map(|c| ChannelSummary {
                    channel_id: c.channel_id.clone(),
                    name: c.name.clone(),
                    kind: c.kind.clone(),
                })
                .collect();
            channels.sort_by(|a, b| {
                a.name
                    .cmp(&b.name)
                    .then_with(|| a.channel_id.cmp(&b.channel_id))
            });

            GuildSummary {
                guild_id: g.guild_id.clone(),
                name: g.name.clone(),
                channel_count: channels.len() as u32,
                member_count: g.members.len() as u32,
                channels,
            }
        })
        .collect();
    guilds.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.guild_id.cmp(&b.guild_id))
    });

    let mut timeline = match (&selected_guild_id, &selected_channel_id) {
        (Some(gid), Some(cid)) => chat_state
            .timeline(gid, cid)
            .into_iter()
            .map(|m| TimelineMessageSummary {
                message_id: m.message_id,
                author: m.author,
                content: m.content,
                edited: m.edited,
                deleted: m.deleted,
                reactions: m
                    .reactions
                    .into_iter()
                    .map(|(emoji, actors)| TimelineReactionSummary {
                        emoji,
                        actors: actors.into_iter().collect(),
                    })
                    .collect(),
                ts_ms: m.ts_ms,
            })
            .collect(),
        _ => vec![],
    };

    timeline.sort_by(|a, b| {
        a.ts_ms
            .cmp(&b.ts_ms)
            .then_with(|| a.message_id.cmp(&b.message_id))
    });

    let voice_room = match (&selected_guild_id, &selected_channel_id) {
        (Some(gid), Some(cid)) => {
            voice_state.room(gid, cid).map(|room| {
                let mut participants: Vec<VoiceParticipantSummary> = room
                    .participants
                    .iter()
                    .map(|(pubkey, p)| VoiceParticipantSummary {
                        pubkey: pubkey.clone(),
                        muted: p.muted,
                        speaking: p.speaking,
                        hand_raised: p.hand_raised,
                    })
                    .collect();
                participants.sort_by(|a, b| a.pubkey.cmp(&b.pubkey));

                VoiceRoomSummary {
                    active_session_id: room.active_session_id.clone(),
                    moq_url: room.moq_url.clone(),
                    participants,
                    track_count: room.tracks.len() as u32,
                }
            })
        }
        _ => None,
    };

    AppState {
        rev,
        greeting: greeting.to_string(),
        guilds,
        selected_guild_id,
        selected_channel_id,
        timeline,
        voice_room,
        toast,
    }
}

fn next_op_id() -> String {
    format!("local-op-{}", Uuid::new_v4())
}

fn next_message_id() -> String {
    format!("msg-{}", Uuid::new_v4())
}

fn next_voice_session_id() -> String {
    format!("sess-{}", Uuid::new_v4())
}

fn now_ms() -> i64 {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0));
    d.as_millis() as i64
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::apply_control_op;
    use crate::control::{ControlEnvelope, ControlState};

    #[test]
    fn live_apply_uses_deterministic_replay_order() {
        let dir = tempdir().expect("tempdir");
        let store = crate::storage::ControlStore::new(PathBuf::from(dir.path()));
        let mut state = ControlState::default();
        let mut ops = vec![];

        let guild = ControlEnvelope::guild_create(
            "op-1".to_string(),
            1,
            "g-1".to_string(),
            "alice".to_string(),
            "Guild One".to_string(),
        );
        assert_eq!(apply_control_op(&store, &mut state, &mut ops, guild), None);

        let channel = ControlEnvelope::channel_create(
            "op-2".to_string(),
            2,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            "general".to_string(),
            crate::ChannelKind::Text,
        );
        assert_eq!(apply_control_op(&store, &mut state, &mut ops, channel), None);

        let newer_policy = ControlEnvelope::channel_permissions_set(
            "op-4".to_string(),
            4,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            vec![],
            vec![],
            vec!["bob".to_string()],
            vec![],
        );
        assert_eq!(
            apply_control_op(&store, &mut state, &mut ops, newer_policy),
            None
        );

        let older_policy = ControlEnvelope::channel_permissions_set(
            "op-3".to_string(),
            3,
            "g-1".to_string(),
            "alice".to_string(),
            "c-1".to_string(),
            vec![],
            vec![],
            vec![],
            vec!["bob".to_string()],
        );
        assert_eq!(
            apply_control_op(&store, &mut state, &mut ops, older_policy),
            None
        );

        let guild = state.guilds.get("g-1").expect("guild exists");
        let channel = guild.channels.get("c-1").expect("channel exists");
        assert!(channel.policy.allow_users.contains("bob"));
        assert!(!channel.policy.deny_users.contains("bob"));
    }
}
