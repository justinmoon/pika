use std::collections::{BTreeMap, BTreeSet};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CHAT_SCHEMA_V1: &str = "rapture.chat.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatEnvelope {
    pub schema: String,
    pub guild_id: String,
    pub channel_id: String,
    pub actor: String,
    pub op_id: String,
    pub ts_ms: i64,
    #[serde(flatten)]
    pub body: ChatBody,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", content = "body")]
pub enum ChatBody {
    #[serde(rename = "message.send")]
    MessageSend {
        message_id: String,
        epoch: u64,
        nonce_b64: String,
        ciphertext_b64: String,
    },
    #[serde(rename = "message.edit")]
    MessageEdit {
        message_id: String,
        epoch: u64,
        nonce_b64: String,
        ciphertext_b64: String,
    },
    #[serde(rename = "message.delete")]
    MessageDelete { message_id: String },
    #[serde(rename = "reaction.put")]
    ReactionPut { message_id: String, emoji: String },
    #[serde(rename = "reaction.remove")]
    ReactionRemove { message_id: String, emoji: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChatState {
    pub channels: BTreeMap<(String, String), ChannelTimeline>,
    pub seen_op_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChannelTimeline {
    pub messages: Vec<TimelineMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineMessage {
    pub message_id: String,
    pub author: String,
    pub content: String,
    pub edited: bool,
    pub deleted: bool,
    pub reactions: BTreeMap<String, BTreeSet<String>>,
    pub ts_ms: i64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChatError {
    #[error("unsupported schema: {0}")]
    UnknownSchema(String),
    #[error("missing key for {guild_id}/{channel_id} epoch {epoch}")]
    MissingEpochKey {
        guild_id: String,
        channel_id: String,
        epoch: u64,
    },
    #[error("invalid nonce")]
    InvalidNonce,
    #[error("invalid ciphertext")]
    InvalidCiphertext,
    #[error("decryption failed")]
    DecryptFailed,
    #[error("message not found: {0}")]
    MessageNotFound(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatApplyOutcome {
    Applied,
    Duplicate,
}

pub trait EpochKeyLookup {
    fn epoch_key(&self, guild_id: &str, channel_id: &str, epoch: u64) -> Option<[u8; 32]>;
}

impl ChatEnvelope {
    pub fn message_send(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        channel_id: String,
        actor: String,
        message_id: String,
        plaintext: &str,
        epoch: u64,
        key: [u8; 32],
    ) -> Result<Self, ChatError> {
        let nonce = nonce_from_op_id(&op_id);
        let ciphertext = encrypt(&key, &nonce, plaintext.as_bytes())?;
        Ok(Self {
            schema: CHAT_SCHEMA_V1.to_string(),
            guild_id,
            channel_id,
            actor,
            op_id,
            ts_ms,
            body: ChatBody::MessageSend {
                message_id,
                epoch,
                nonce_b64: B64.encode(nonce),
                ciphertext_b64: B64.encode(ciphertext),
            },
        })
    }

    pub fn message_edit(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        channel_id: String,
        actor: String,
        message_id: String,
        plaintext: &str,
        epoch: u64,
        key: [u8; 32],
    ) -> Result<Self, ChatError> {
        let nonce = nonce_from_op_id(&op_id);
        let ciphertext = encrypt(&key, &nonce, plaintext.as_bytes())?;
        Ok(Self {
            schema: CHAT_SCHEMA_V1.to_string(),
            guild_id,
            channel_id,
            actor,
            op_id,
            ts_ms,
            body: ChatBody::MessageEdit {
                message_id,
                epoch,
                nonce_b64: B64.encode(nonce),
                ciphertext_b64: B64.encode(ciphertext),
            },
        })
    }

    pub fn message_delete(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        channel_id: String,
        actor: String,
        message_id: String,
    ) -> Self {
        Self {
            schema: CHAT_SCHEMA_V1.to_string(),
            guild_id,
            channel_id,
            actor,
            op_id,
            ts_ms,
            body: ChatBody::MessageDelete { message_id },
        }
    }

    pub fn reaction_put(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        channel_id: String,
        actor: String,
        message_id: String,
        emoji: String,
    ) -> Self {
        Self {
            schema: CHAT_SCHEMA_V1.to_string(),
            guild_id,
            channel_id,
            actor,
            op_id,
            ts_ms,
            body: ChatBody::ReactionPut { message_id, emoji },
        }
    }

    pub fn reaction_remove(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        channel_id: String,
        actor: String,
        message_id: String,
        emoji: String,
    ) -> Self {
        Self {
            schema: CHAT_SCHEMA_V1.to_string(),
            guild_id,
            channel_id,
            actor,
            op_id,
            ts_ms,
            body: ChatBody::ReactionRemove { message_id, emoji },
        }
    }
}

impl ChatState {
    pub fn timeline(&self, guild_id: &str, channel_id: &str) -> Vec<TimelineMessage> {
        self.channels
            .get(&(guild_id.to_string(), channel_id.to_string()))
            .map(|t| t.messages.clone())
            .unwrap_or_default()
    }

    pub fn apply<K: EpochKeyLookup>(
        &mut self,
        envelope: ChatEnvelope,
        keys: &K,
    ) -> Result<ChatApplyOutcome, ChatError> {
        if envelope.schema != CHAT_SCHEMA_V1 {
            return Err(ChatError::UnknownSchema(envelope.schema));
        }
        if self.seen_op_ids.contains(&envelope.op_id) {
            return Ok(ChatApplyOutcome::Duplicate);
        }

        let chan_key = (envelope.guild_id.clone(), envelope.channel_id.clone());
        let timeline = self.channels.entry(chan_key).or_default();

        match envelope.body {
            ChatBody::MessageSend {
                message_id,
                epoch,
                nonce_b64,
                ciphertext_b64,
            } => {
                let plaintext = decrypt_body(
                    keys,
                    &envelope.guild_id,
                    &envelope.channel_id,
                    epoch,
                    &nonce_b64,
                    &ciphertext_b64,
                )?;
                timeline.messages.push(TimelineMessage {
                    message_id,
                    author: envelope.actor,
                    content: plaintext,
                    edited: false,
                    deleted: false,
                    reactions: BTreeMap::new(),
                    ts_ms: envelope.ts_ms,
                });
            }
            ChatBody::MessageEdit {
                message_id,
                epoch,
                nonce_b64,
                ciphertext_b64,
            } => {
                let plaintext = decrypt_body(
                    keys,
                    &envelope.guild_id,
                    &envelope.channel_id,
                    epoch,
                    &nonce_b64,
                    &ciphertext_b64,
                )?;
                let idx = find_message_idx(&timeline.messages, &message_id)
                    .ok_or_else(|| ChatError::MessageNotFound(message_id.clone()))?;
                timeline.messages[idx].content = plaintext;
                timeline.messages[idx].edited = true;
            }
            ChatBody::MessageDelete { message_id } => {
                let idx = find_message_idx(&timeline.messages, &message_id)
                    .ok_or_else(|| ChatError::MessageNotFound(message_id.clone()))?;
                timeline.messages[idx].content = "[deleted]".to_string();
                timeline.messages[idx].deleted = true;
                timeline.messages[idx].reactions.clear();
            }
            ChatBody::ReactionPut { message_id, emoji } => {
                let idx = find_message_idx(&timeline.messages, &message_id)
                    .ok_or_else(|| ChatError::MessageNotFound(message_id.clone()))?;
                timeline.messages[idx]
                    .reactions
                    .entry(emoji)
                    .or_default()
                    .insert(envelope.actor);
            }
            ChatBody::ReactionRemove { message_id, emoji } => {
                let idx = find_message_idx(&timeline.messages, &message_id)
                    .ok_or_else(|| ChatError::MessageNotFound(message_id.clone()))?;
                if let Some(actors) = timeline.messages[idx].reactions.get_mut(&emoji) {
                    actors.remove(&envelope.actor);
                    if actors.is_empty() {
                        timeline.messages[idx].reactions.remove(&emoji);
                    }
                }
            }
        }

        self.seen_op_ids.insert(envelope.op_id);
        Ok(ChatApplyOutcome::Applied)
    }
}

fn find_message_idx(messages: &[TimelineMessage], message_id: &str) -> Option<usize> {
    messages.iter().position(|m| m.message_id == message_id)
}

fn decrypt_body<K: EpochKeyLookup>(
    keys: &K,
    guild_id: &str,
    channel_id: &str,
    epoch: u64,
    nonce_b64: &str,
    ciphertext_b64: &str,
) -> Result<String, ChatError> {
    let key =
        keys.epoch_key(guild_id, channel_id, epoch)
            .ok_or_else(|| ChatError::MissingEpochKey {
                guild_id: guild_id.to_string(),
                channel_id: channel_id.to_string(),
                epoch,
            })?;
    let nonce_raw = B64
        .decode(nonce_b64.as_bytes())
        .map_err(|_| ChatError::InvalidNonce)?;
    if nonce_raw.len() != 12 {
        return Err(ChatError::InvalidNonce);
    }
    let ciphertext = B64
        .decode(ciphertext_b64.as_bytes())
        .map_err(|_| ChatError::InvalidCiphertext)?;
    let plaintext = decrypt(&key, &nonce_raw, &ciphertext)?;
    String::from_utf8(plaintext).map_err(|_| ChatError::DecryptFailed)
}

fn encrypt(key: &[u8; 32], nonce: &[u8; 12], plaintext: &[u8]) -> Result<Vec<u8>, ChatError> {
    let aead = ChaCha20Poly1305::new(Key::from_slice(key));
    aead.encrypt(Nonce::from_slice(nonce), plaintext)
        .map_err(|_| ChatError::DecryptFailed)
}

fn decrypt(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, ChatError> {
    if nonce.len() != 12 {
        return Err(ChatError::InvalidNonce);
    }
    let aead = ChaCha20Poly1305::new(Key::from_slice(key));
    aead.decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| ChatError::DecryptFailed)
}

fn nonce_from_op_id(op_id: &str) -> [u8; 12] {
    let digest = Sha256::digest(op_id.as_bytes());
    let mut nonce = [0_u8; 12];
    nonce.copy_from_slice(&digest[..12]);
    nonce
}
