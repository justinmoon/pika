use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::channel_groups::{ChannelGroupDirectory, ChannelGroupError};
use crate::chat::{ChatEnvelope, ChatError, ChatState, EpochKeyLookup, TimelineMessage};
use crate::control::{ControlEnvelope, ControlError, ControlState};
use crate::reconcile::{desired_channel_members, ReconcileError};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SimError {
    #[error("control error: {0}")]
    Control(#[from] ControlError),
    #[error("channel-group error: {0}")]
    ChannelGroup(#[from] ChannelGroupError),
    #[error("chat error: {0}")]
    Chat(#[from] ChatError),
    #[error("unknown client: {0}")]
    UnknownClient(String),
    #[error("actor is not a channel member: {actor} ({guild_id}/{channel_id})")]
    ActorNotMember {
        actor: String,
        guild_id: String,
        channel_id: String,
    },
    #[error("channel group not found: {guild_id}/{channel_id}")]
    GroupNotFound {
        guild_id: String,
        channel_id: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct LocalRelay {
    control: ControlState,
    groups: ChannelGroupDirectory,
    clients: BTreeMap<String, SimClient>,
    next_chat_op: u64,
}

#[derive(Debug, Clone, Default)]
pub struct SimClient {
    keyring: BTreeMap<(String, String), BTreeMap<u64, [u8; 32]>>,
    chat: ChatState,
    decrypt_failures: usize,
}

impl LocalRelay {
    pub fn register_client(&mut self, user: &str) {
        self.clients.entry(user.to_string()).or_default();
    }

    pub fn control(&self) -> &ControlState {
        &self.control
    }

    pub fn apply_control(&mut self, op: ControlEnvelope) -> Result<(), SimError> {
        self.control.apply(op)?;
        self.groups.ensure_from_control(&self.control);
        self.reconcile_groups()?;
        Ok(())
    }

    pub fn send_message(
        &mut self,
        guild_id: &str,
        channel_id: &str,
        actor: &str,
        message_id: &str,
        plaintext: &str,
    ) -> Result<ChatEnvelope, SimError> {
        let group =
            self.groups
                .get(guild_id, channel_id)
                .ok_or_else(|| SimError::GroupNotFound {
                    guild_id: guild_id.to_string(),
                    channel_id: channel_id.to_string(),
                })?;
        if !group.members.contains(actor) {
            return Err(SimError::ActorNotMember {
                actor: actor.to_string(),
                guild_id: guild_id.to_string(),
                channel_id: channel_id.to_string(),
            });
        }

        self.next_chat_op = self.next_chat_op.saturating_add(1);
        let op_id = format!("chat-op-{}", self.next_chat_op);
        let envelope = ChatEnvelope::message_send(
            op_id,
            now_ms(),
            guild_id.to_string(),
            channel_id.to_string(),
            actor.to_string(),
            message_id.to_string(),
            plaintext,
            group.epoch,
            group.key,
        )?;
        self.broadcast_chat(envelope.clone());
        Ok(envelope)
    }

    pub fn timeline(&self, user: &str, guild_id: &str, channel_id: &str) -> Vec<TimelineMessage> {
        self.clients
            .get(user)
            .map(|c| c.chat.timeline(guild_id, channel_id))
            .unwrap_or_default()
    }

    pub fn decrypt_failures(&self, user: &str) -> Option<usize> {
        self.clients.get(user).map(|c| c.decrypt_failures)
    }

    fn reconcile_groups(&mut self) -> Result<(), SimError> {
        for (guild_id, channel_id) in self.groups.channel_refs() {
            let desired = match desired_channel_members(&self.control, &guild_id, &channel_id) {
                Ok(desired) => desired,
                Err(ReconcileError::GuildNotFound(g)) => {
                    return Err(SimError::Control(ControlError::GuildNotFound(g)));
                }
                Err(ReconcileError::ChannelNotFound {
                    guild_id,
                    channel_id,
                }) => {
                    return Err(SimError::Control(ControlError::ChannelNotFound {
                        guild_id,
                        channel_id,
                    }));
                }
                Err(ReconcileError::Backend(_)) => {
                    return Err(SimError::GroupNotFound {
                        guild_id,
                        channel_id,
                    });
                }
            };
            let changed = self
                .groups
                .reconcile_members(&guild_id, &channel_id, desired.clone())?;
            if changed {
                self.distribute_current_key(&guild_id, &channel_id, &desired)?;
            } else {
                self.ensure_keys_exist(&guild_id, &channel_id, &desired)?;
            }
        }
        Ok(())
    }

    fn distribute_current_key(
        &mut self,
        guild_id: &str,
        channel_id: &str,
        members: &BTreeSet<String>,
    ) -> Result<(), SimError> {
        let (epoch, key) = self
            .groups
            .current_epoch_key(guild_id, channel_id)
            .ok_or_else(|| SimError::GroupNotFound {
                guild_id: guild_id.to_string(),
                channel_id: channel_id.to_string(),
            })?;

        for member in members {
            if let Some(client) = self.clients.get_mut(member) {
                client
                    .keyring
                    .entry((guild_id.to_string(), channel_id.to_string()))
                    .or_default()
                    .insert(epoch, key);
            }
        }
        Ok(())
    }

    fn ensure_keys_exist(
        &mut self,
        guild_id: &str,
        channel_id: &str,
        members: &BTreeSet<String>,
    ) -> Result<(), SimError> {
        let (epoch, key) = self
            .groups
            .current_epoch_key(guild_id, channel_id)
            .ok_or_else(|| SimError::GroupNotFound {
                guild_id: guild_id.to_string(),
                channel_id: channel_id.to_string(),
            })?;

        for member in members {
            if let Some(client) = self.clients.get_mut(member) {
                client
                    .keyring
                    .entry((guild_id.to_string(), channel_id.to_string()))
                    .or_default()
                    .entry(epoch)
                    .or_insert(key);
            }
        }
        Ok(())
    }

    fn broadcast_chat(&mut self, envelope: ChatEnvelope) {
        for client in self.clients.values_mut() {
            let lookup = ClientLookup {
                keyring: &client.keyring,
            };
            if let Err(err) = client.chat.apply(envelope.clone(), &lookup) {
                if matches!(err, ChatError::MissingEpochKey { .. }) {
                    client.decrypt_failures = client.decrypt_failures.saturating_add(1);
                }
            }
        }
    }
}

struct ClientLookup<'a> {
    keyring: &'a BTreeMap<(String, String), BTreeMap<u64, [u8; 32]>>,
}

impl EpochKeyLookup for ClientLookup<'_> {
    fn epoch_key(&self, guild_id: &str, channel_id: &str, epoch: u64) -> Option<[u8; 32]> {
        self.keyring
            .get(&(guild_id.to_string(), channel_id.to_string()))
            .and_then(|epochs| epochs.get(&epoch).copied())
    }
}

fn now_ms() -> i64 {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0));
    d.as_millis() as i64
}
