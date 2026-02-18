use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::permissions::Permission;

pub const VOICE_SCHEMA_V1: &str = "rapture.voice.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceEnvelope {
    pub schema: String,
    pub guild_id: String,
    pub channel_id: String,
    pub actor: String,
    pub op_id: String,
    pub ts_ms: i64,
    #[serde(flatten)]
    pub body: VoiceBody,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", content = "body")]
pub enum VoiceBody {
    #[serde(rename = "voice.session.start")]
    SessionStart { session_id: String, moq_url: String },
    #[serde(rename = "voice.session.end")]
    SessionEnd { session_id: String },
    #[serde(rename = "voice.participant.join")]
    ParticipantJoin { session_id: String },
    #[serde(rename = "voice.participant.leave")]
    ParticipantLeave { session_id: String },
    #[serde(rename = "voice.participant.state")]
    ParticipantState {
        session_id: String,
        target_pubkey: String,
        muted: bool,
        speaking: bool,
        hand_raised: bool,
    },
    #[serde(rename = "voice.track.advertise")]
    TrackAdvertise {
        session_id: String,
        track_name: String,
        codec: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VoiceState {
    pub rooms: BTreeMap<(String, String), VoiceRoomState>,
    pub seen_op_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VoiceRoomState {
    pub active_session_id: Option<String>,
    pub moq_url: Option<String>,
    pub participants: BTreeMap<String, VoiceParticipant>,
    pub tracks: Vec<VoiceTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VoiceParticipant {
    pub muted: bool,
    pub speaking: bool,
    pub hand_raised: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceTrack {
    pub track_name: String,
    pub codec: String,
    pub advertised_by: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceApplyOutcome {
    Applied,
    Duplicate,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VoiceError {
    #[error("unsupported schema: {0}")]
    UnknownSchema(String),
    #[error("permission denied: actor {actor} lacks {permission:?}")]
    PermissionDenied {
        actor: String,
        permission: Permission,
    },
    #[error("voice session mismatch: expected {expected:?}, got {actual}")]
    SessionMismatch {
        expected: Option<String>,
        actual: String,
    },
    #[error("participant not in room: {0}")]
    ParticipantNotInRoom(String),
}

pub trait VoicePermissionLookup {
    fn has_permission(
        &self,
        guild_id: &str,
        channel_id: &str,
        actor: &str,
        permission: Permission,
    ) -> bool;
}

impl VoiceEnvelope {
    pub fn session_start(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        channel_id: String,
        actor: String,
        session_id: String,
        moq_url: String,
    ) -> Self {
        Self {
            schema: VOICE_SCHEMA_V1.to_string(),
            guild_id,
            channel_id,
            actor,
            op_id,
            ts_ms,
            body: VoiceBody::SessionStart {
                session_id,
                moq_url,
            },
        }
    }

    pub fn session_end(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        channel_id: String,
        actor: String,
        session_id: String,
    ) -> Self {
        Self {
            schema: VOICE_SCHEMA_V1.to_string(),
            guild_id,
            channel_id,
            actor,
            op_id,
            ts_ms,
            body: VoiceBody::SessionEnd { session_id },
        }
    }

    pub fn participant_join(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        channel_id: String,
        actor: String,
        session_id: String,
    ) -> Self {
        Self {
            schema: VOICE_SCHEMA_V1.to_string(),
            guild_id,
            channel_id,
            actor,
            op_id,
            ts_ms,
            body: VoiceBody::ParticipantJoin { session_id },
        }
    }

    pub fn participant_leave(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        channel_id: String,
        actor: String,
        session_id: String,
    ) -> Self {
        Self {
            schema: VOICE_SCHEMA_V1.to_string(),
            guild_id,
            channel_id,
            actor,
            op_id,
            ts_ms,
            body: VoiceBody::ParticipantLeave { session_id },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn participant_state(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        channel_id: String,
        actor: String,
        session_id: String,
        target_pubkey: String,
        muted: bool,
        speaking: bool,
        hand_raised: bool,
    ) -> Self {
        Self {
            schema: VOICE_SCHEMA_V1.to_string(),
            guild_id,
            channel_id,
            actor,
            op_id,
            ts_ms,
            body: VoiceBody::ParticipantState {
                session_id,
                target_pubkey,
                muted,
                speaking,
                hand_raised,
            },
        }
    }

    pub fn track_advertise(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        channel_id: String,
        actor: String,
        session_id: String,
        track_name: String,
        codec: String,
    ) -> Self {
        Self {
            schema: VOICE_SCHEMA_V1.to_string(),
            guild_id,
            channel_id,
            actor,
            op_id,
            ts_ms,
            body: VoiceBody::TrackAdvertise {
                session_id,
                track_name,
                codec,
            },
        }
    }
}

impl VoiceState {
    pub fn room(&self, guild_id: &str, channel_id: &str) -> Option<&VoiceRoomState> {
        self.rooms
            .get(&(guild_id.to_string(), channel_id.to_string()))
    }

    pub fn apply<P: VoicePermissionLookup>(
        &mut self,
        env: VoiceEnvelope,
        perms: &P,
    ) -> Result<VoiceApplyOutcome, VoiceError> {
        if env.schema != VOICE_SCHEMA_V1 {
            return Err(VoiceError::UnknownSchema(env.schema));
        }
        if self.seen_op_ids.contains(&env.op_id) {
            return Ok(VoiceApplyOutcome::Duplicate);
        }

        let room_key = (env.guild_id.clone(), env.channel_id.clone());
        let room = self.rooms.entry(room_key).or_default();

        match env.body {
            VoiceBody::SessionStart {
                session_id,
                moq_url,
            } => {
                require(
                    perms,
                    &env.guild_id,
                    &env.channel_id,
                    &env.actor,
                    Permission::ConnectVoice,
                )?;
                room.active_session_id = Some(session_id);
                room.moq_url = Some(moq_url);
                room.participants.clear();
                room.tracks.clear();
            }
            VoiceBody::SessionEnd { session_id } => {
                require(
                    perms,
                    &env.guild_id,
                    &env.channel_id,
                    &env.actor,
                    Permission::ConnectVoice,
                )?;
                ensure_session(room, &session_id)?;
                room.active_session_id = None;
                room.moq_url = None;
                room.participants.clear();
                room.tracks.clear();
            }
            VoiceBody::ParticipantJoin { session_id } => {
                require(
                    perms,
                    &env.guild_id,
                    &env.channel_id,
                    &env.actor,
                    Permission::ConnectVoice,
                )?;
                ensure_session(room, &session_id)?;
                room.participants.entry(env.actor).or_default();
            }
            VoiceBody::ParticipantLeave { session_id } => {
                require(
                    perms,
                    &env.guild_id,
                    &env.channel_id,
                    &env.actor,
                    Permission::ConnectVoice,
                )?;
                ensure_session(room, &session_id)?;
                room.participants.remove(&env.actor);
            }
            VoiceBody::ParticipantState {
                session_id,
                target_pubkey,
                muted,
                speaking,
                hand_raised,
            } => {
                ensure_session(room, &session_id)?;
                if target_pubkey != env.actor {
                    require(
                        perms,
                        &env.guild_id,
                        &env.channel_id,
                        &env.actor,
                        Permission::MuteMembers,
                    )?;
                }
                if speaking {
                    require(
                        perms,
                        &env.guild_id,
                        &env.channel_id,
                        &env.actor,
                        Permission::SpeakVoice,
                    )?;
                }
                let participant = room
                    .participants
                    .get_mut(&target_pubkey)
                    .ok_or_else(|| VoiceError::ParticipantNotInRoom(target_pubkey.clone()))?;
                participant.muted = muted;
                participant.speaking = speaking;
                participant.hand_raised = hand_raised;
            }
            VoiceBody::TrackAdvertise {
                session_id,
                track_name,
                codec,
            } => {
                ensure_session(room, &session_id)?;
                if !room.participants.contains_key(&env.actor) {
                    return Err(VoiceError::ParticipantNotInRoom(env.actor));
                }
                require(
                    perms,
                    &env.guild_id,
                    &env.channel_id,
                    &env.actor,
                    Permission::SpeakVoice,
                )?;
                room.tracks.push(VoiceTrack {
                    track_name,
                    codec,
                    advertised_by: env.actor,
                });
            }
        }

        self.seen_op_ids.insert(env.op_id);
        Ok(VoiceApplyOutcome::Applied)
    }
}

fn require<P: VoicePermissionLookup>(
    perms: &P,
    guild_id: &str,
    channel_id: &str,
    actor: &str,
    permission: Permission,
) -> Result<(), VoiceError> {
    if perms.has_permission(guild_id, channel_id, actor, permission) {
        return Ok(());
    }
    Err(VoiceError::PermissionDenied {
        actor: actor.to_string(),
        permission,
    })
}

fn ensure_session(room: &VoiceRoomState, session_id: &str) -> Result<(), VoiceError> {
    if room.active_session_id.as_deref() == Some(session_id) {
        return Ok(());
    }
    Err(VoiceError::SessionMismatch {
        expected: room.active_session_id.clone(),
        actual: session_id.to_string(),
    })
}
